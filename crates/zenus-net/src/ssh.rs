use crate::nic;
use crate::socket;
use zenus_console::serial::SerialPort;
use zenus_sync::spinlock::SpinLock;
use zutils_common::{Args, Writer, OutputBuf};

const MAX_SSH_CLIENTS: usize = 4;
const MAX_LINE: usize = 256;
const MAX_OUTPUT: usize = 4096;
const CHUNK_SIZE: usize = 1024;

fn ssh_keystream_byte(seed: u32, pos: u32) -> u8 {
    let state = seed.wrapping_mul(0x9E3779B9).wrapping_add(pos);
    ((state >> 16) ^ (state >> 8) ^ state) as u8
}

fn derive_key(nonce: &[u8; 8], password: &[u8]) -> u32 {
    let mut h: u32 = 0x6A09E667;
    for &b in nonce {
        h = h.wrapping_mul(0x01000193).wrapping_add(b as u32);
    }
    for &b in password {
        h = h.wrapping_mul(0x01000193).wrapping_add(b as u32);
    }
    h ^ 0x9E3779B9
}

fn hex_byte(b: u8) -> (u8, u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    (HEX[(b >> 4) as usize], HEX[(b & 0x0f) as usize])
}

#[derive(Clone, Copy, PartialEq)]
enum ConnState {
    New,
    WaitAuth,
    AuthOk,
    AuthDenied,
    Shell,
    Closing,
}

#[derive(Clone, Copy)]
struct SshConnection {
    fd: Option<usize>,
    state: ConnState,
    nonce: [u8; 8],
    seed: u32,
    cipher_pos: u32,
    rx_buf: [u8; 512],
    rx_len: usize,
    line: [u8; MAX_LINE],
    line_len: usize,
    output: [u8; MAX_OUTPUT],
    output_len: usize,
    output_sent: usize,
}

impl SshConnection {
    const fn new() -> Self {
        SshConnection {
            fd: None,
            state: ConnState::Closing,
            nonce: [0; 8],
            seed: 0,
            cipher_pos: 0,
            rx_buf: [0; 512],
            rx_len: 0,
            line: [0; MAX_LINE],
            line_len: 0,
            output: [0; MAX_OUTPUT],
            output_len: 0,
            output_sent: 0,
        }
    }
}

pub struct SshServer {
    listen_fd: Option<usize>,
    running: bool,
    connections: [SshConnection; MAX_SSH_CLIENTS],
}

static SSH_SERVER: SpinLock<SshServer> = SpinLock::new(SshServer {
    listen_fd: None,
    running: false,
    connections: [SshConnection::new(); MAX_SSH_CLIENTS],
});

impl SshServer {
    pub fn new() -> Self {
        SshServer {
            listen_fd: None,
            running: false,
            connections: [SshConnection::new(); MAX_SSH_CLIENTS],
        }
    }

    pub fn start(iface_idx: usize, port: u16) -> bool {
        let fd = match socket::socket(socket::AF_INET, socket::SOCK_STREAM, 0) {
            Some(fd) => fd,
            None => return false,
        };
        if !socket::bind(fd, port) {
            socket::close(fd, iface_idx);
            return false;
        }
        if !socket::listen(fd, 4) {
            socket::close(fd, iface_idx);
            return false;
        }
        {
            let mut server = SSH_SERVER.lock();
            server.listen_fd = Some(fd);
            server.running = true;
        }
        let s = SerialPort::new(0x3F8);
        s.write_str("[SSH] Server started on port ");
        s.write_u64(port as u64);
        s.write_str("\n");
        true
    }

    pub fn poll(&mut self, iface_idx: usize) {
        if !self.running {
            return;
        }
        socket::poll_all(iface_idx);

        if let Some(lfd) = self.listen_fd {
            while let Some(cfd) = socket::accept(lfd, iface_idx) {
                let mut slot = None;
                for i in 0..MAX_SSH_CLIENTS {
                    if self.connections[i].fd.is_none() {
                        slot = Some(i);
                        break;
                    }
                }
                if let Some(idx) = slot {
                    let conn = &mut self.connections[idx];
                    conn.fd = Some(cfd);
                    conn.state = ConnState::New;
                    let r = zenus_arch::random::get_random_u64();
                    conn.nonce = [
                        r as u8,
                        (r >> 8) as u8,
                        (r >> 16) as u8,
                        (r >> 24) as u8,
                        (r >> 32) as u8,
                        (r >> 40) as u8,
                        (r >> 48) as u8,
                        (r >> 56) as u8,
                    ];
                    conn.seed = 0;
                    conn.cipher_pos = 0;
                    conn.rx_len = 0;
                    conn.line_len = 0;
                    conn.output_len = 0;
                    conn.output_sent = 0;
                    let s = SerialPort::new(0x3F8);
                    s.write_str("[SSH] Connection #");
                    s.write_u64(idx as u64);
                    s.write_str(" accepted (fd=");
                    s.write_u64(cfd as u64);
                    s.write_str(")\n");
                } else {
                    let s = SerialPort::new(0x3F8);
                    s.write_str("[SSH] Too many connections, rejecting\n");
                    socket::close(cfd, iface_idx);
                }
            }
        }

        for i in 0..MAX_SSH_CLIENTS {
            let conn = &mut self.connections[i];
            let fd = match conn.fd {
                Some(fd) => fd,
                None => continue,
            };

            if !socket::is_connected(fd) {
                let s = SerialPort::new(0x3F8);
                s.write_str("[SSH] Connection #");
                s.write_u64(i as u64);
                s.write_str(" disconnected\n");
                conn.fd = None;
                continue;
            }

            match conn.state {
                ConnState::New => {
                    let mut greeting = [0u8; 64];
                    let mut pos = 0;
                    let proto = b"ZENUS_SSH/1.0\n";
                    greeting[pos..pos + proto.len()].copy_from_slice(proto);
                    pos += proto.len();
                    for &b in &conn.nonce {
                        let (hi, lo) = hex_byte(b);
                        greeting[pos] = hi;
                        greeting[pos + 1] = lo;
                        pos += 2;
                    }
                    greeting[pos] = b'\n';
                    pos += 1;
                    if socket::send(fd, &greeting[..pos], iface_idx) {
                        conn.state = ConnState::WaitAuth;
                    }
                }
                ConnState::WaitAuth => {
                    let mut buf = [0u8; 128];
                    if let Some(len) = socket::recv(fd, &mut buf) {
                        let space = conn.rx_buf.len() - conn.rx_len;
                        let copy = len.min(space);
                        conn.rx_buf[conn.rx_len..conn.rx_len + copy].copy_from_slice(&buf[..copy]);
                        conn.rx_len += copy;

                        if let Some(nl) = conn.rx_buf[..conn.rx_len].iter().position(|&b| b == b'\n') {
                            let line = &conn.rx_buf[..nl];
                            if line.starts_with(b"AUTH ") {
                                let auth_data = &line[5..];
                                let password = b"zenus";
                                if auth_data.len() == password.len() {
                                    let ok = auth_data.iter().enumerate().all(|(j, &a)| {
                                        a ^ conn.nonce[j % 8] == password[j]
                                    });
                                    if ok {
                                        conn.seed = derive_key(&conn.nonce, password);
                                        conn.state = ConnState::AuthOk;
                                    } else {
                                        conn.state = ConnState::AuthDenied;
                                    }
                                } else {
                                    conn.state = ConnState::AuthDenied;
                                }
                            } else {
                                conn.state = ConnState::AuthDenied;
                            }
                            conn.rx_len = 0;
                        }
                    }
                }
                ConnState::AuthOk => {
                    if socket::send(fd, b"OK\n", iface_idx) {
                        conn.cipher_pos = 0;
                        conn.output_len = 0;
                        conn.output_sent = 0;
                        conn.line_len = 0;
                        conn.state = ConnState::Shell;
                    }
                }
                ConnState::AuthDenied => {
                    if socket::send(fd, b"DENIED\n", iface_idx) {
                        conn.state = ConnState::Closing;
                    }
                }
                ConnState::Shell => {
                    Self::poll_shell(conn, fd, iface_idx);
                }
                ConnState::Closing => {
                    socket::close(fd, iface_idx);
                    conn.fd = None;
                }
            }
        }
    }

    fn poll_shell(conn: &mut SshConnection, fd: usize, iface_idx: usize) {
        if conn.output_sent < conn.output_len {
            let remaining = conn.output_len - conn.output_sent;
            let chunk = remaining.min(CHUNK_SIZE);
            let mut enc = [0u8; CHUNK_SIZE];
            for j in 0..chunk {
                let pos = (conn.output_sent + j) as u32;
                enc[j] = conn.output[conn.output_sent + j] ^ ssh_keystream_byte(conn.seed, pos);
            }
            if socket::send(fd, &enc[..chunk], iface_idx) {
                conn.output_sent += chunk;
                if conn.output_sent >= conn.output_len {
                    conn.output_len = 0;
                    conn.output_sent = 0;
                }
            }
            return;
        }

        let mut buf = [0u8; 128];
        if let Some(len) = socket::recv(fd, &mut buf) {
            for &b in &buf[..len] {
                let decrypted = b ^ ssh_keystream_byte(conn.seed, conn.cipher_pos);
                conn.cipher_pos += 1;
                if decrypted == b'\n' {
                    let line = core::str::from_utf8(&conn.line[..conn.line_len]).unwrap_or("");
                    let trimmed = line.trim();
                    conn.line_len = 0;

                    if trimmed == "exit" || trimmed == "quit" || trimmed == "logout" {
                        conn.state = ConnState::Closing;
                        return;
                    }
                    if !trimmed.is_empty() {
                        execute_command(trimmed, &mut conn.output, &mut conn.output_len);
                    }
                    let prompt = b"zenus$ ";
                    let avail = MAX_OUTPUT - conn.output_len;
                    let n = prompt.len().min(avail);
                    conn.output[conn.output_len..conn.output_len + n].copy_from_slice(&prompt[..n]);
                    conn.output_len += n;
                    conn.output_sent = 0;
                    break;
                } else if conn.line_len < MAX_LINE - 1 {
                    conn.line[conn.line_len] = decrypted;
                    conn.line_len += 1;
                }
            }
        } else if conn.output_len == 0 && conn.line_len == 0 {
            let prompt = b"zenus$ ";
            conn.output[..prompt.len()].copy_from_slice(prompt);
            conn.output_len = prompt.len();
            conn.output_sent = 0;
        }
    }

    pub fn connection_count() -> usize {
        let server = SSH_SERVER.lock();
        let mut count = 0;
        for i in 0..MAX_SSH_CLIENTS {
            if server.connections[i].fd.is_some() {
                count += 1;
            }
        }
        count
    }

    pub fn is_running() -> bool {
        SSH_SERVER.lock().running
    }
}

fn execute_command(line: &str, output: &mut [u8; MAX_OUTPUT], out_len: &mut usize) {
    let args = Args::parse(line);
    if args.cmd.is_empty() {
        return;
    }

    let mut out = OutputBuf::new(output);

    match args.cmd {
        "help" => zutils_help::execute(&mut out),
        "echo" => zutils_echo::execute(&args, &mut out),
        "ls" => zutils_ls::execute(&args, &mut out),
        "cat" => zutils_cat::execute(&args, &mut out),
        "uname" | "version" => zutils_uname::execute(&args, &mut out),
        "id" => zutils_id::execute(&args, &mut out),
        "whoami" => zutils_whoami::execute(&args, &mut out),
        "ps" => zutils_ps::execute(&args, &mut out),
        "ifconfig" => {
            let count = nic::iface_count();
            for i in 0..count {
                if let Some(iface) = nic::get_iface(i) {
                    out.write_str("Interface ");
                    out.write_u64(i as u64);
                    out.write_str(":\r\n");
                    out.write_str("  MAC: ");
                    for (j, b) in iface.mac.iter().enumerate() {
                        if j > 0 { out.write_byte(b':'); }
                        out.write_hex(*b as u64);
                    }
                    out.write_str("\r\n  IP: ");
                    out.write_ip(iface.ip);
                    out.write_str("\r\n  Link: ");
                    if iface.link_up {
                        out.write_str("UP\r\n");
                    } else {
                        out.write_str("DOWN\r\n");
                    }
                }
            }
        }
        "meminfo" => zutils_meminfo::execute(&args, &mut out),
        "dmesg" => zutils_dmesg::execute(&args, &mut out),
        _ => {
            out.write_str("Unknown command: ");
            out.write_str(args.cmd);
            out.write_str("\n");
        }
    }
    *out_len = out.len();
}

use crate::nic;
use crate::socket;
use zenus_console::serial::SerialPort;

const MAX_SSH_CLIENTS: usize = 4;
const MAX_LINE: usize = 256;
const MAX_OUTPUT: usize = 4096;
const MAX_ARGS: usize = 16;
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

struct OutputBuffer<'a> {
    buf: &'a mut [u8; MAX_OUTPUT],
    pos: usize,
}

impl<'a> OutputBuffer<'a> {
    fn new(buf: &'a mut [u8; MAX_OUTPUT]) -> Self {
        OutputBuffer { buf, pos: 0 }
    }

    fn write_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let avail = self.buf.len() - self.pos;
        let n = bytes.len().min(avail);
        self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
        self.pos += n;
    }

    fn write_byte(&mut self, b: u8) {
        if self.pos < self.buf.len() {
            self.buf[self.pos] = b;
            self.pos += 1;
        }
    }

    fn write_u64(&mut self, v: u64) {
        if v == 0 {
            self.write_byte(b'0');
            return;
        }
        let mut tmp = [0u8; 20];
        let mut i = 20;
        let mut n = v;
        while n > 0 {
            i -= 1;
            tmp[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        let s = core::str::from_utf8(&tmp[i..]).unwrap_or("");
        self.write_str(s);
    }

    fn write_hex(&mut self, v: u64) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut started = false;
        for s in (0..16).rev() {
            let nib = ((v >> (s * 4)) & 0xf) as u8;
            if nib != 0 || started || s == 0 {
                self.write_byte(HEX[nib as usize]);
                started = true;
            }
        }
    }
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

static mut SSH_SERVER: SshServer = SshServer {
    listen_fd: None,
    running: false,
    connections: [SshConnection::new(); MAX_SSH_CLIENTS],
};

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
        unsafe {
            SSH_SERVER.listen_fd = Some(fd);
            SSH_SERVER.running = true;
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
        unsafe {
            let mut count = 0;
            for i in 0..MAX_SSH_CLIENTS {
                if SSH_SERVER.connections[i].fd.is_some() {
                    count += 1;
                }
            }
            count
        }
    }

    pub fn is_running() -> bool {
        unsafe { SSH_SERVER.running }
    }
}

fn execute_command(line: &str, output: &mut [u8; MAX_OUTPUT], out_len: &mut usize) {
    let mut parts: [&str; MAX_ARGS] = [""; MAX_ARGS];
    let mut count = 0;
    for arg in line.split_whitespace() {
        if count >= MAX_ARGS {
            break;
        }
        parts[count] = arg;
        count += 1;
    }
    if count == 0 {
        return;
    }

    let mut out = OutputBuffer::new(output);
    let cmd = parts[0];

    match cmd {
        "help" => cmd_help(&mut out),
        "echo" => cmd_echo(&parts[1..count], &mut out),
        "ls" => cmd_ls(&parts[1..count], &mut out),
        "cat" => cmd_cat(&parts[1..count], &mut out),
        "uname" | "version" => cmd_uname(&mut out),
        "id" => cmd_id(&mut out),
        "whoami" => cmd_whoami(&mut out),
        "ps" => cmd_ps(&mut out),
        "ifconfig" => cmd_ifconfig(&mut out),
        "meminfo" => cmd_meminfo(&mut out),
        "dmesg" => cmd_dmesg(&mut out),
        _ => {
            out.write_str("Unknown command: ");
            out.write_str(cmd);
            out.write_str("\n");
        }
    }
    *out_len = out.pos;
}

fn cmd_help(out: &mut OutputBuffer) {
    out.write_str("Commands:\n");
    out.write_str("  help     - Show this help\n");
    out.write_str("  echo     - Print text\n");
    out.write_str("  ls       - List directory\n");
    out.write_str("  cat      - Show file contents\n");
    out.write_str("  uname    - Show kernel version\n");
    out.write_str("  id       - Show user/group IDs\n");
    out.write_str("  whoami   - Show username\n");
    out.write_str("  ps       - List processes\n");
    out.write_str("  ifconfig - Show network interfaces\n");
    out.write_str("  meminfo  - Show memory usage\n");
    out.write_str("  dmesg    - Show kernel log\n");
    out.write_str("  exit     - Disconnect\n");
}

fn cmd_echo(args: &[&str], out: &mut OutputBuffer) {
    for (i, arg) in args.iter().enumerate() {
        if arg.is_empty() {
            continue;
        }
        if i > 0 {
            out.write_byte(b' ');
        }
        out.write_str(arg);
    }
    out.write_byte(b'\n');
}

fn cmd_uname(out: &mut OutputBuffer) {
    out.write_str("Zenus OS v0.1.0 x86_64\n");
}

fn cmd_id(out: &mut OutputBuffer) {
    let uid = zenus_sched::scheduler::current_uid();
    let euid = zenus_sched::scheduler::current_euid();
    let gid = zenus_sched::scheduler::current_gid();
    let egid = zenus_sched::scheduler::current_egid();
    out.write_str("uid=");
    out.write_u64(uid as u64);
    if euid != uid {
        out.write_str(" euid=");
        out.write_u64(euid as u64);
    }
    out.write_str(" gid=");
    out.write_u64(gid as u64);
    if egid != gid {
        out.write_str(" egid=");
        out.write_u64(egid as u64);
    }
    out.write_byte(b'\n');
}

fn cmd_whoami(out: &mut OutputBuffer) {
    out.write_str("root\n");
}

fn cmd_ps(out: &mut OutputBuffer) {
    out.write_str("PID\tUID\tGID\tState\n");
    let tasks = zenus_sched::scheduler::list_tasks();
    for info in tasks.iter().flatten() {
        out.write_u64(info.id);
        out.write_str("\t");
        out.write_u64(info.uid as u64);
        out.write_str("\t");
        out.write_u64(info.gid as u64);
        out.write_str("\t");
        out.write_str(info.state.to_str());
        if info.id == zenus_sched::scheduler::current_task_id() {
            out.write_str(" (current)");
        }
        out.write_byte(b'\n');
    }
}

fn cmd_ifconfig(out: &mut OutputBuffer) {
    let count = nic::iface_count();
    for i in 0..count {
        if let Some(iface) = nic::get_iface(i) {
            out.write_str("Interface ");
            out.write_u64(i as u64);
            out.write_str(":\n");
            out.write_str("  MAC: ");
            for (j, b) in iface.mac.iter().enumerate() {
                if j > 0 {
                    out.write_byte(b':');
                }
                out.write_hex(*b as u64);
            }
            out.write_str("\n  IP: ");
            out.write_u64(iface.ip[0] as u64);
            out.write_byte(b'.');
            out.write_u64(iface.ip[1] as u64);
            out.write_byte(b'.');
            out.write_u64(iface.ip[2] as u64);
            out.write_byte(b'.');
            out.write_u64(iface.ip[3] as u64);
            out.write_str("\n  Link: ");
            if iface.link_up {
                out.write_str("UP\n");
            } else {
                out.write_str("DOWN\n");
            }
        }
    }
}

fn cmd_meminfo(out: &mut OutputBuffer) {
    out.write_str("Physical frames:\n");
    let fa = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
    out.write_str("  Total: ");
    out.write_u64(fa.total_memory() / 4096);
    out.write_str(" frames (");
    out.write_u64(fa.total_memory() / (1024 * 1024));
    out.write_str(" MB)\n");
    out.write_str("  Used:  ");
    out.write_u64(fa.used_memory() / 4096);
    out.write_str(" frames (");
    out.write_u64(fa.used_memory() / (1024 * 1024));
    out.write_str(" MB)\n");
    out.write_str("  Free stack: ");
    out.write_u64(fa.free_frames_count() as u64);
    out.write_str(" frames\n");
}

fn cmd_dmesg(out: &mut OutputBuffer) {
    let snap = zenus_console::log::dmesg_snapshot();
    for i in 0..snap.count {
        let entry = &snap.entries[i];
        let len = core::cmp::min(entry.len as usize, entry.msg.len());
        let msg = core::str::from_utf8(&entry.msg[..len]).unwrap_or("");
        out.write_str(entry.level.prefix());
        out.write_str(" ");
        out.write_str(msg);
        out.write_byte(b'\n');
    }
    if snap.count == 0 {
        out.write_str("(no messages)\n");
    }
}

fn cmd_cat(args: &[&str], out: &mut OutputBuffer) {
    let path = match args.iter().find(|a| !a.is_empty()) {
        Some(p) => p,
        None => {
            out.write_str("cat: missing operand\n");
            return;
        }
    };

    match zenus_fs::vfs::open(path) {
        Some(node) => {
            let stat = node.fs.stat(node.inode);
            if stat.file_type == zenus_fs::vfs::FileType::Directory {
                out.write_str("cat: ");
                out.write_str(path);
                out.write_str(": Is a directory\n");
                return;
            }
            let mut buf = [0u8; 512];
            let mut offset: u64 = 0;
            let mut last_byte: u8 = 0;
            loop {
                match node.fs.read(node.inode, offset, &mut buf) {
                    Some(0) | None => break,
                    Some(n) => {
                        for i in 0..n as usize {
                            let b = buf[i];
                            out.write_byte(b);
                            last_byte = b;
                        }
                        offset += n;
                    }
                }
            }
            if offset > 0 && last_byte != b'\n' {
                out.write_byte(b'\n');
            }
        }
        None => {
            out.write_str("cat: ");
            out.write_str(path);
            out.write_str(": not found\n");
        }
    }
}

fn cmd_ls(args: &[&str], out: &mut OutputBuffer) {
    let long = args.iter().any(|a| *a == "-l");
    let path = args
        .iter()
        .find(|a| !a.is_empty() && **a != "-l")
        .unwrap_or(&"/");
    let path = if path.is_empty() { "/" } else { path };

    match zenus_fs::vfs::open(path) {
        Some(node) => {
            let entries = node.fs.read_dir(node.inode);
            let mut count = 0u64;
            for entry in entries {
                count += 1;
                if long {
                    let stat = node.fs.stat(entry.inode);
                    let perm_buf = zenus_fs::vfs::perm_str(stat.mode);
                    let perm = core::str::from_utf8(&perm_buf).unwrap_or("?????????");
                    out.write_str(perm);
                    out.write_byte(b' ');
                    out.write_u64(stat.uid as u64);
                    out.write_byte(b':');
                    out.write_u64(stat.gid as u64);
                    out.write_byte(b' ');
                    out.write_u64(stat.size);
                    out.write_byte(b' ');
                }
                out.write_str(entry.name.as_str());
                if entry.file_type == zenus_fs::vfs::FileType::Directory {
                    out.write_byte(b'/');
                }
                out.write_str("  ");
            }
            if count == 0 {
                out.write_str("(empty)\n");
            } else {
                out.write_byte(b'\n');
            }
        }
        None => {
            out.write_str("ls: ");
            out.write_str(path);
            out.write_str(": not found\n");
        }
    }
}

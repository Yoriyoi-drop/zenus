use crate::nic;
use crate::socket;

use zenus_sync::spinlock::SpinLock;
use zutils_common::{Args, OutputBuf, Writer};

const MAX_SSH_CLIENTS: usize = 4;
const MAX_LINE: usize = 256;
const MAX_OUTPUT: usize = 4096;
const CHUNK_SIZE: usize = 1024;

#[cfg(not(feature = "ssh_password"))]
const SSH_PASSWORD: &[u8] = b"zenus";
#[cfg(feature = "ssh_password")]
const SSH_PASSWORD: &[u8] = include_bytes!(env!("SSH_PASSWORD_PATH"));

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn ssh_keystream_byte(seed: u32, pos: u32) -> u8 {
    let state = seed.wrapping_mul(0x9E3779B9).wrapping_add(pos);
    ((state >> 16) ^ (state >> 8) ^ state) as u8
}

fn derive_key(nonce: &[u8; 16], password: &[u8]) -> u32 {
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

/// Build the `ZENUS_SSH/1.0` banner for a fresh connection.
///
/// Protocol string, then the 16-byte nonce as lowercase hex, then a newline:
/// `14 + 32 + 1` bytes. Extracted from `poll` so the layout is testable instead
/// of being re-implemented (and silently diverging) in the test suite.
fn build_greeting(nonce: &[u8; 16], buf: &mut [u8; 96]) -> usize {
    const PROTO: &[u8] = b"ZENUS_SSH/1.0\n";
    let mut pos = 0;
    buf[pos..pos + PROTO.len()].copy_from_slice(PROTO);
    pos += PROTO.len();
    for &b in nonce {
        let (hi, lo) = hex_byte(b);
        buf[pos] = hi;
        buf[pos + 1] = lo;
        pos += 2;
    }
    buf[pos] = b'\n';
    pos + 1
}

#[derive(Clone, Copy)]
struct SshConnection {
    fd: Option<usize>,
    state: ConnState,
    nonce: [u8; 16],
    seed: u32,
    cipher_pos: u32,
    rx_buf: [u8; 512],
    rx_len: usize,
    line: [u8; MAX_LINE],
    line_len: usize,
    output: [u8; MAX_OUTPUT],
    output_len: usize,
    output_sent: usize,
    auth_failures: u8,
}

impl SshConnection {
    const fn new() -> Self {
        SshConnection {
            fd: None,
            state: ConnState::Closing,
            nonce: [0; 16],
            seed: 0,
            cipher_pos: 0,
            rx_buf: [0; 512],
            rx_len: 0,
            line: [0; MAX_LINE],
            line_len: 0,
            output: [0; MAX_OUTPUT],
            output_len: 0,
            output_sent: 0,
            auth_failures: 0,
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
        zenus_console::kinfo!("SSH server started on port {}", port);
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
                    let r0 = zenus_arch::random::get_random_u64();
                    let r1 = zenus_arch::random::get_random_u64();
                    conn.nonce = [
                        r0 as u8,
                        (r0 >> 8) as u8,
                        (r0 >> 16) as u8,
                        (r0 >> 24) as u8,
                        (r0 >> 32) as u8,
                        (r0 >> 40) as u8,
                        (r0 >> 48) as u8,
                        (r0 >> 56) as u8,
                        r1 as u8,
                        (r1 >> 8) as u8,
                        (r1 >> 16) as u8,
                        (r1 >> 24) as u8,
                        (r1 >> 32) as u8,
                        (r1 >> 40) as u8,
                        (r1 >> 48) as u8,
                        (r1 >> 56) as u8,
                    ];
                    conn.seed = 0;
                    conn.cipher_pos = 0;
                    conn.rx_len = 0;
                    conn.line_len = 0;
                    conn.output_len = 0;
                    conn.output_sent = 0;
                    conn.auth_failures = 0;
                    zenus_console::kinfo!("SSH connection #{} accepted (fd={})", idx, cfd);
                } else {
                    zenus_console::kwarn!("SSH too many connections, rejecting");
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
                zenus_console::kinfo!("SSH connection #{} disconnected", i);
                conn.fd = None;
                continue;
            }

            match conn.state {
                ConnState::New => {
                    let mut greeting = [0u8; 96];
                    let len = build_greeting(&conn.nonce, &mut greeting);
                    if socket::send(fd, &greeting[..len], iface_idx) {
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

                        if let Some(nl) =
                            conn.rx_buf[..conn.rx_len].iter().position(|&b| b == b'\n')
                        {
                            let line = &conn.rx_buf[..nl];
                            if line.starts_with(b"AUTH ") {
                                let auth_data = &line[5..];
                                if conn.auth_failures >= 5 {
                                    conn.state = ConnState::Closing;
                                } else if constant_time_eq(auth_data, SSH_PASSWORD) {
                                    conn.seed = derive_key(&conn.nonce, SSH_PASSWORD);
                                    conn.state = ConnState::AuthOk;
                                } else {
                                    conn.auth_failures += 1;
                                    conn.state = ConnState::AuthDenied;
                                    for _ in 0..50000 {
                                        unsafe {
                                            core::arch::asm!("pause");
                                        }
                                    }
                                }
                            } else {
                                conn.auth_failures += 1;
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
            if conn.line_len >= MAX_LINE {
                conn.line_len = 0;
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

/// Host-side unit tests for the ZENUS_SSH framing helpers.
///
/// This protocol is *not* SSH: it is a line protocol with a hand-rolled
/// keystream (`ssh_keystream_byte`). These tests pin the properties that make
/// the toy protocol self-consistent — they are not a statement that it is
/// cryptographically sound, because it is not.
#[cfg(test)]
mod host_tests {
    use super::{constant_time_eq, derive_key, hex_byte, ssh_keystream_byte, SSH_PASSWORD};
    use alloc::vec::Vec;

    #[test]
    fn constant_time_eq_matches_and_rejects() {
        assert!(constant_time_eq(b"zenus", b"zenus"));
        assert!(!constant_time_eq(b"zenus", b"zenu"));
        assert!(!constant_time_eq(b"zenus", b"zenuss"));
        assert!(!constant_time_eq(b"", b"zenus"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn default_password_is_the_documented_one() {
        // Only true for the default build; the `ssh_password` feature embeds a
        // real secret instead, which is why the assertion is cfg-split.
        #[cfg(not(feature = "ssh_password"))]
        assert_eq!(SSH_PASSWORD, b"zenus");
        #[cfg(feature = "ssh_password")]
        assert!(!SSH_PASSWORD.is_empty());
    }

    /// Golden values: the keystream must stay bit-for-bit stable, because the
    /// client and the server both derive ciphertext from it and a change would
    /// break every existing session.
    #[test]
    fn keystream_matches_its_golden_values() {
        assert_eq!(ssh_keystream_byte(0xDEAD_BEEF, 0), 96);
        assert_eq!(ssh_keystream_byte(0xDEAD_BEEF, 1), 111);
        assert_eq!(ssh_keystream_byte(0xDEAD_BEEF, 2), 110);
        // Changing either input must change the output.
        assert_ne!(
            ssh_keystream_byte(0xDEAD_BEEF, 0),
            ssh_keystream_byte(0xDEADBEE0, 0)
        );
        assert_ne!(
            ssh_keystream_byte(0xDEAD_BEEF, 0),
            ssh_keystream_byte(0xDEAD_BEEF, 1)
        );
    }

    /// The keystream is weak and this pins *how*: the generator adds `pos` to
    /// the state, so consecutive keystream bytes are usually a small delta away
    /// (measured: 60 of 64 consecutive pairs differ by <= 5). That is roughly a
    /// couple of bits of entropy per byte, which is why `SECURITY.md` calls
    /// ZENUS_SSH/1.0 a toy protocol rather than SSH.
    ///
    /// The point of the test is not to approve of that. It is that a future
    /// change to the generator would silently invalidate every client; this
    /// makes the change deliberate.
    #[test]
    fn keystream_weakness_is_pinned_not_glossed_over() {
        let seed = 0x1234_5678;
        let deltas: Vec<u8> = (0..64u32)
            .map(|p| ssh_keystream_byte(seed, p).abs_diff(ssh_keystream_byte(seed, p + 1)))
            .collect();
        let near = deltas.iter().filter(|d| **d <= 5).count();
        assert!(
            near * 100 >= deltas.len() * 90,
            "consecutive-byte structure changed ({near}/{} deltas <= 5): {deltas:?}",
            deltas.len()
        );

        // The bytes themselves are not all equal — a truly constant stream
        // would also satisfy the delta check in a degenerate way.
        let distinct = (0..64u32)
            .map(|p| ssh_keystream_byte(seed, p))
            .collect::<Vec<u8>>();
        let mut sorted = distinct.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), distinct.len(), "keystream repeats bytes");
    }

    #[test]
    fn xor_round_trip_recovers_plaintext() {
        // The client XORs with the same stream; this is what makes the session
        // readable at all.
        let seed = 0x0BAD_F00D;
        let plain = b"zenus$ uname\r\n";
        let cipher: Vec<u8> = plain
            .iter()
            .enumerate()
            .map(|(i, &b)| b ^ ssh_keystream_byte(seed, i as u32))
            .collect();
        let back: Vec<u8> = cipher
            .iter()
            .enumerate()
            .map(|(i, &b)| b ^ ssh_keystream_byte(seed, i as u32))
            .collect();
        assert_eq!(back, plain);
        assert_ne!(cipher.as_slice(), plain.as_slice());
    }

    #[test]
    fn derive_key_depends_on_nonce_and_password() {
        let nonce_a = [0u8; 16];
        let mut nonce_b = [0u8; 16];
        nonce_b[15] = 1;

        let base = derive_key(&nonce_a, b"zenus");
        assert_eq!(base, derive_key(&nonce_a, b"zenus"), "deterministic");
        assert_ne!(base, derive_key(&nonce_b, b"zenus"), "nonce is mixed in");
        assert_ne!(base, derive_key(&nonce_a, b"other"), "password is mixed in");
    }

    #[test]
    fn hex_byte_lowercases_nibbles() {
        assert_eq!(hex_byte(0x00), (b'0', b'0'));
        assert_eq!(hex_byte(0x0F), (b'0', b'f'));
        assert_eq!(hex_byte(0xA5), (b'a', b'5'));
        assert_eq!(hex_byte(0xFF), (b'f', b'f'));
    }

    #[test]
    fn greeting_frame_matches_documented_layout() {
        let nonce = [0xABu8; 16];
        let mut greeting = [0u8; 96];
        let len = super::build_greeting(&nonce, &mut greeting);

        // 14 bytes of protocol string, 32 hex chars of nonce, one newline.
        assert_eq!(len, 14 + 32 + 1);
        assert_eq!(&greeting[..14], b"ZENUS_SSH/1.0\n");
        let mut expected_nonce = [0u8; 32];
        for i in 0..16 {
            expected_nonce[2 * i] = b'a';
            expected_nonce[2 * i + 1] = b'b';
        }
        assert_eq!(&greeting[14..46], &expected_nonce[..]);
        assert_eq!(greeting[46], b'\n');
        assert_eq!(greeting[47], 0, "nothing is written past the newline");
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
                        if j > 0 {
                            out.write_byte(b':');
                        }
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

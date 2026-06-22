use zenus_console::serial::SerialPort;

const MAX_LINE: usize = 256;
const MAX_ARGS: usize = 16;
const PROMPT: &str = "zenus$ ";

const MAX_ECHO_LISTENS: usize = 8;
const MAX_ECHO_CLIENTS: usize = 16;
static mut ECHO_LISTEN_FDS: [Option<usize>; MAX_ECHO_LISTENS] = [None; MAX_ECHO_LISTENS];
static mut ECHO_CLIENT_FDS: [Option<usize>; MAX_ECHO_CLIENTS] = [None; MAX_ECHO_CLIENTS];

pub struct Shell {
    serial: SerialPort,
}

impl Shell {
    pub fn new() -> Self {
        Shell {
            serial: SerialPort::new(0x3F8),
        }
    }

    pub fn run(&mut self) -> ! {
        let mut yield_count = 0u64;
        loop {
            yield_count += 1;
            if yield_count % 10 == 0 {
                zenus_sched::scheduler::yield_now();
            }
            if yield_count % 5 == 0 {
                zenus_net::nic::net_poll();
                Self::echo_server_poll();
            }
            self.serial.write_str(PROMPT);
            let line = match self.read_line() {
                Some(l) => l,
                None => continue,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            self.execute(trimmed);
        }
    }

    fn read_line(&mut self) -> Option<&'static str> {
        static mut BUF: [u8; MAX_LINE] = [0; MAX_LINE];
        static mut POS: usize = 0;

        unsafe { POS = 0 };

        loop {
            // Check if any byte available (non-blocking peek)
            if self.serial.is_data_available() {
                let c = self.serial.read_byte_serial();
                match c {
                    b'\r' | b'\n' => {
                        self.serial.write_str("\r\n");
                        unsafe {
                            let s = core::str::from_utf8(&BUF[..POS]).unwrap_or("");
                            POS = 0;
                            return if s.is_empty() { None } else { Some(s) };
                        }
                    }
                    b'\x7F' | b'\x08' => {
                        if unsafe { POS > 0 } {
                            unsafe { POS -= 1 };
                            self.serial.write_str("\x08 \x08");
                        }
                    }
                    0x20..=0x7E => {
                        unsafe {
                            if POS < MAX_LINE - 1 {
                                BUF[POS] = c;
                                POS += 1;
                                self.serial.write_byte_serial(c);
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                zenus_net::nic::net_poll();
                Self::echo_server_poll();
                zenus_sched::scheduler::check_yield();
            }
        }
    }

    fn execute(&mut self, line: &str) {
        let mut parts: [&str; MAX_ARGS] = [""; MAX_ARGS];
        let mut count = 0;
        for arg in line.split_whitespace() {
            if count >= MAX_ARGS { break; }
            parts[count] = arg;
            count += 1;
        }
        if count == 0 { return; }

        let cmd = parts[0];

        match cmd {
            "help" => self.cmd_help(),
            "echo" => self.cmd_echo(&parts[1..count]),
            "ls" => self.cmd_ls(&parts[1..count]),
            "cat" => self.cmd_cat(&parts[1..count]),
            "clear" => self.cmd_clear(),
            "timer" => self.cmd_timer(),
            "ps" => self.cmd_ps(),
            "kill" => self.cmd_kill(&parts[1..count]),
            "mkdir" => self.cmd_mkdir(&parts[1..count]),
            "rm" => self.cmd_rm(&parts[1..count]),
            "touch" => self.cmd_touch(&parts[1..count]),
            "ifconfig" => self.cmd_ifconfig(),
            "meminfo" => self.cmd_meminfo(),
            "reboot" => self.cmd_reboot(),
            "shutdown" => self.cmd_shutdown(),
            "uname" | "version" => self.cmd_uname(),
            "dmesg" => self.cmd_dmesg(),
            "readdev" => self.cmd_readdev(&parts[1..count]),
            "mount" => self.cmd_mount(),
            "bcache" => self.cmd_bcache(),
            "fsck" => self.cmd_fsck(),
            "journal-init" => self.cmd_journal_init(),
            "journal-test" => self.cmd_journal_test(),
            "tcp-listen" => self.cmd_tcp_listen(&parts[1..count]),
            "tcp-status" => self.cmd_tcp_status(),
            "tcp-send" => self.cmd_tcp_send(&parts[1..count]),
            "tcp-echo" => self.cmd_tcp_echo(&parts[1..count]),
            "tcp-connect" => self.cmd_tcp_connect(&parts[1..count]),
            "udp-bind" => self.cmd_udp_bind(&parts[1..count]),
            "udp-send" => self.cmd_udp_send(&parts[1..count]),
            "udp-recv" => self.cmd_udp_recv(&parts[1..count]),
            "dhcp" => self.cmd_dhcp(),
            "dhcp-server" => self.cmd_dhcp_server(&parts[1..count]),
            "resolve" => self.cmd_resolve(&parts[1..count]),
            "id" => self.cmd_id(),
            "whoami" => self.cmd_whoami(),
            "chmod" => self.cmd_chmod(&parts[1..count]),
            _ => {
                self.serial.write_str("Unknown command: ");
                self.serial.write_str(cmd);
                self.serial.write_str("\r\n");
            }
        }
    }

    fn cmd_help(&mut self) {
        self.serial.write_str("Commands:\r\n");
        self.serial.write_str("  help   - Show this help\r\n");
        self.serial.write_str("  echo   - Print text\r\n");
        self.serial.write_str("  ls     - List directory\r\n");
        self.serial.write_str("  ls -l  - List with permissions, owner, size\r\n");
        self.serial.write_str("  chmod <mode> <file> - Change file permissions (octal)\r\n");
        self.serial.write_str("  cat    - Show file contents\r\n");
        self.serial.write_str("  clear  - Clear screen\r\n");
        self.serial.write_str("  timer  - Show APIC timer tick count\r\n");
        self.serial.write_str("  ps     - List processes\r\n");
        self.serial.write_str("  kill   - Kill process\r\n");
        self.serial.write_str("  mkdir  - Create directory\r\n");
        self.serial.write_str("  rm     - Remove file/directory\r\n");
        self.serial.write_str("  touch  - Create empty file\r\n");
        self.serial.write_str("  ifconfig - Show network interfaces\r\n");
        self.serial.write_str("  meminfo  - Show memory usage\r\n");
        self.serial.write_str("  reboot   - Reboot the system\r\n");
        self.serial.write_str("  shutdown - Shutdown the system\r\n");
        self.serial.write_str("  uname    - Show kernel version info\r\n");
        self.serial.write_str("  version  - Alias for uname\r\n");
        self.serial.write_str("  dmesg    - Show kernel log buffer\r\n");
        self.serial.write_str("  readdev  - Hexdump first sector of a block device\r\n");
        self.serial.write_str("  tcp-listen <port> - Listen on TCP port\r\n");
        self.serial.write_str("  tcp-status        - Show TCP connection table\r\n");
        self.serial.write_str("  tcp-send <conn> <text> - Send data on connection\r\n");
        self.serial.write_str("  tcp-echo  - Start echo server on port 7\r\n");
        self.serial.write_str("  tcp-connect <port> [ip] - Connect to TCP port\r\n");
        self.serial.write_str("  udp-bind <port> - Bind UDP socket\r\n");
        self.serial.write_str("  udp-send <fd> <ip> <port> <text> - Send UDP datagram\r\n");
        self.serial.write_str("  udp-recv <fd> - Receive UDP datagram\r\n");
        self.serial.write_str("  dhcp      - Acquire IP via DHCP\r\n");
        self.serial.write_str("  dhcp-server - Start DHCP server (requires static IP)\r\n");
        self.serial.write_str("  resolve <domain> - DNS resolve domain name\r\n");
        self.serial.write_str("  id        - Show current user/group IDs\r\n");
        self.serial.write_str("  whoami    - Show current username\r\n");
    }

    fn cmd_echo(&mut self, args: &[&str]) {
        for (i, arg) in args.iter().enumerate() {
            if arg.is_empty() { continue; }
            if i > 0 {
                self.serial.write_byte_serial(b' ');
            }
            self.serial.write_str(arg);
        }
        self.serial.write_str("\r\n");
    }

    fn cmd_clear(&mut self) {
        self.serial.write_str("\x1B[2J\x1B[H");
    }

    fn cmd_timer(&mut self) {
        let ticks = zenus_arch::interrupts::handler::get_timer_tick();
        self.serial.write_str("Timer ticks: ");
        self.serial.write_u64(ticks);
        self.serial.write_str("\r\n");
    }

    fn cmd_ps(&mut self) {
        self.serial.write_str("PID\tUID\tGID\tState\r\n");
        let tasks = zenus_sched::scheduler::list_tasks();
        for info in tasks.iter().flatten() {
            self.serial.write_u64(info.id);
            self.serial.write_str("\t");
            self.serial.write_u64(info.uid as u64);
            self.serial.write_str("\t");
            self.serial.write_u64(info.gid as u64);
            self.serial.write_str("\t");
            self.serial.write_str(info.state.to_str());
            if info.id == zenus_sched::scheduler::current_task_id() {
                self.serial.write_str(" (current)");
            }
            self.serial.write_str("\r\n");
        }
    }

    fn cmd_kill(&mut self, args: &[&str]) {
        let pid_str = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.serial.write_str("kill: missing pid\r\n");
                return;
            }
        };

        let pid = match pid_str.parse::<u64>() {
            Ok(p) => p,
            Err(_) => {
                self.serial.write_str("kill: invalid pid\r\n");
                return;
            }
        };

        if pid == 0 {
            self.serial.write_str("kill: cannot kill idle process\r\n");
            return;
        }

        let current_pid = zenus_sched::scheduler::current_task_id();
        if pid == current_pid {
            self.serial.write_str("kill: cannot kill the shell itself\r\n");
            return;
        }

        if zenus_sched::scheduler::kill_task(pid) {
            self.serial.write_str("killed: ");
            self.serial.write_u64(pid);
            self.serial.write_str("\r\n");
        } else {
            self.serial.write_str("kill: not found: ");
            self.serial.write_u64(pid);
            self.serial.write_str("\r\n");
        }
    }

    fn cmd_mkdir(&mut self, args: &[&str]) {
        let path = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.serial.write_str("mkdir: missing operand\r\n");
                return;
            }
        };

        if zenus_fs::vfs::create_dir(path) {
            self.serial.write_str("ok\r\n");
        } else {
            self.serial.write_str("mkdir: failed to create directory\r\n");
        }
    }

    fn cmd_rm(&mut self, args: &[&str]) {
        let path = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.serial.write_str("rm: missing operand\r\n");
                return;
            }
        };

        if zenus_fs::vfs::remove(path) {
            self.serial.write_str("ok\r\n");
        } else {
            self.serial.write_str("rm: failed to remove\r\n");
        }
    }

    fn cmd_touch(&mut self, args: &[&str]) {
        let path = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.serial.write_str("touch: missing operand\r\n");
                return;
            }
        };

        if zenus_fs::vfs::create_file(path) {
            self.serial.write_str("ok\r\n");
        } else {
            self.serial.write_str("touch: failed to create file\r\n");
        }
    }

    fn cmd_ifconfig(&mut self) {
        let count = zenus_net::nic::iface_count();
        for i in 0..count {
            if let Some(iface) = zenus_net::nic::get_iface(i) {
                self.serial.write_str("Interface ");
                self.serial.write_u64(i as u64);
                self.serial.write_str(":\r\n");
                self.serial.write_str("  MAC: ");
                for b in &iface.mac {
                    self.serial.write_hex(*b as u64);
                    self.serial.write_str(":");
                }
                self.serial.write_str("\r\n  IP: ");
                self.serial.write_u64(iface.ip[0] as u64);
                self.serial.write_str(".");
                self.serial.write_u64(iface.ip[1] as u64);
                self.serial.write_str(".");
                self.serial.write_u64(iface.ip[2] as u64);
                self.serial.write_str(".");
                self.serial.write_u64(iface.ip[3] as u64);
                self.serial.write_str("\r\n  Link: ");
                if iface.link_up {
                    self.serial.write_str("UP\r\n");
                } else {
                    self.serial.write_str("DOWN\r\n");
                }
            }
        }
    }

    fn cmd_meminfo(&mut self) {
        let free_head = zenus_mem::allocator::ALLOCATOR.free_head_addr();
        self.serial.write_str("Heap: 4MB free-list allocator\r\n");
        self.serial.write_str("  Free list head: 0x");
        self.serial.write_hex(free_head as u64);
        self.serial.write_str("\r\n");

        let fa = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
        self.serial.write_str("Physical frames:\r\n");
        self.serial.write_str("  Total: ");
        self.serial.write_u64(fa.total_memory() / 4096);
        self.serial.write_str(" frames (");
        self.serial.write_u64(fa.total_memory() / (1024*1024));
        self.serial.write_str(" MB)\r\n");
        self.serial.write_str("  Used:  ");
        self.serial.write_u64(fa.used_memory() / 4096);
        self.serial.write_str(" frames (");
        self.serial.write_u64(fa.used_memory() / (1024*1024));
        self.serial.write_str(" MB)\r\n");
        self.serial.write_str("  Free stack: ");
        self.serial.write_u64(fa.free_frames_count() as u64);
        self.serial.write_str(" frames\r\n");
    }

    fn cmd_reboot(&mut self) {
        self.serial.write_str("Rebooting...\r\n");
        zenus_arch::acpi::reboot_via_keyboard();
    }

    fn cmd_shutdown(&mut self) {
        self.serial.write_str("Shutting down...\r\n");
        zenus_arch::acpi::shutdown_via_acpi();
    }

    fn cmd_uname(&mut self) {
        self.serial.write_str("Zenus OS v0.1.0 x86_64\r\n");
    }

    fn cmd_dmesg(&mut self) {
        let snap = zenus_console::log::dmesg_snapshot();
        for i in 0..snap.count {
            let entry = &snap.entries[i];
            let len = core::cmp::min(entry.len as usize, entry.msg.len());
            let msg = core::str::from_utf8(&entry.msg[..len]).unwrap_or("");
            self.serial.write_str(entry.level.prefix());
            self.serial.write_str(" ");
            self.serial.write_str(msg);
            self.serial.write_str("\r\n");
        }
        if snap.count == 0 {
            self.serial.write_str("(no messages)\r\n");
        }
    }

    fn cmd_mount(&mut self) {
        self.serial.write_str("Mount points:\r\n");
        self.serial.write_str("  /       tmpfs (root)\r\n");
        self.serial.write_str("  /dev    devfs\r\n");
        if zenus_fs::vfs::open("/mnt").is_some() {
            self.serial.write_str("  /mnt    ext2 (if mounted)\r\n");
        }
        let (hits, misses) = zenus_fs::block_cache::bc_stats();
        self.serial.write_str("Block cache: ");
        self.serial.write_u64(hits);
        self.serial.write_str(" hits, ");
        self.serial.write_u64(misses);
        self.serial.write_str(" misses\r\n");
    }

    fn cmd_bcache(&mut self) {
        let (hits, misses) = zenus_fs::block_cache::bc_stats();
        self.serial.write_str("Block cache stats:\r\n");
        self.serial.write_str("  Hits:   ");
        self.serial.write_u64(hits);
        self.serial.write_str("\r\n  Misses: ");
        self.serial.write_u64(misses);
        let total = hits + misses;
        if total > 0 {
            self.serial.write_str("\r\n  Rate:   ");
            self.serial.write_u64(hits * 100 / total);
            self.serial.write_str("%\r\n");
        } else {
            self.serial.write_str("\r\n  (no I/O yet)\r\n");
        }
    }

    fn cmd_fsck(&mut self) {
        let result = zenus_fs::ext2_fsck::fsck(0);
        self.serial.write_str("fsck results:\r\n");
        if result.passed() {
            self.serial.write_str("  PASSED");
        } else {
            self.serial.write_str("  FAILED");
        }
        self.serial.write_str(" (");
        self.serial.write_u64(result.errors as u64);
        self.serial.write_str(" errors, ");
        self.serial.write_u64(result.warnings as u64);
        self.serial.write_str(" warnings)\r\n");
        for i in 0..result.count {
            let msg = &result.messages[i];
            let sev = match msg.severity {
                zenus_fs::ext2_fsck::FsckSeverity::Fatal => "FATAL",
                zenus_fs::ext2_fsck::FsckSeverity::Error => "ERROR",
                zenus_fs::ext2_fsck::FsckSeverity::Warning => " WARN",
                _ => " INFO",
            };
            self.serial.write_str("  [");
            self.serial.write_str(sev);
            self.serial.write_str("] ");
            self.serial.write_str(msg.msg);
            self.serial.write_str("\r\n");
        }
    }

    fn cmd_dhcp(&mut self) {
        self.serial.write_str("DHCP client starting...\r\n");
        let iface_idx = 1;
        if zenus_net::dhcp::dhcp_start(iface_idx) {
            self.serial.write_str("[ OK ] DHCP: address acquired\r\n");
            self.cmd_ifconfig();
        } else {
            self.serial.write_str("[FAIL] DHCP: no response\r\n");
        }
    }

    fn cmd_dhcp_server(&mut self, _args: &[&str]) {
        self.serial.write_str("DHCP server running on 10.0.2.100-10.0.2.115\r\n");
        let iface_idx = 1;
        let iface = match zenus_net::nic::get_iface(iface_idx) {
            Some(iface) => iface,
            None => {
                self.serial.write_str("[FAIL] No interface\r\n");
                return;
            }
        };
        if iface.ip == [0; 4] || iface.ip == [127, 0, 0, 1] {
            self.serial.write_str("[FAIL] Server needs a static IP (run `dhcp` first)\r\n");
            return;
        }
        self.serial.write_str("[ OK ] DHCP server ready on ");
        self.serial_write_ip(iface.ip);
        self.serial.write_str("\r\n");
        self.serial.write_str("Leases:\r\n");
        if zenus_net::dhcp_server::lease_count() == 0 {
            self.serial.write_str("  (none)\r\n");
        } else {
            zenus_net::dhcp_server::print_leases(&mut |s| self.serial.write_str(s));
        }
    }

    fn cmd_resolve(&mut self, args: &[&str]) {
        if args.len() < 1 {
            self.serial.write_str("Usage: resolve <domain>\r\n");
            return;
        }
        let dns_server = [10, 0, 2, 3];
        self.serial.write_str("Resolving ");
        self.serial.write_str(args[0]);
        self.serial.write_str(" via ");
        self.serial_write_ip(dns_server);
        self.serial.write_str("...\r\n");
        match zenus_net::dns::resolve(1, dns_server, args[0]) {
            Some(ip) => {
                self.serial.write_str("  -> ");
                self.serial_write_ip(ip);
                self.serial.write_str("\r\n");
            }
            None => {
                self.serial.write_str("  [FAIL] resolution failed\r\n");
            }
        }
    }

    fn cmd_id(&mut self) {
        let uid = zenus_sched::scheduler::current_uid();
        let euid = zenus_sched::scheduler::current_euid();
        let gid = zenus_sched::scheduler::current_gid();
        let egid = zenus_sched::scheduler::current_egid();
        self.serial.write_str("uid=");
        self.serial.write_u64(uid as u64);
        if euid != uid {
            self.serial.write_str(" euid=");
            self.serial.write_u64(euid as u64);
        }
        self.serial.write_str(" gid=");
        self.serial.write_u64(gid as u64);
        if egid != gid {
            self.serial.write_str(" egid=");
            self.serial.write_u64(egid as u64);
        }
        self.serial.write_str("\r\n");
    }

    fn cmd_whoami(&mut self) {
        self.serial.write_str("root\r\n");
    }

    fn cmd_chmod(&mut self, args: &[&str]) {
        if args.len() < 2 {
            self.serial.write_str("Usage: chmod <mode> <file>\r\n");
            return;
        }
        let mode_str = args[0];
        let mode = match usize::from_str_radix(mode_str, 8) {
            Ok(m) => m as u16,
            Err(_) => {
                self.serial.write_str("chmod: invalid mode\r\n");
                return;
            }
        };
        let path = args[1];
        match zenus_fs::vfs::open(path) {
            Some(node) => {
                if node.fs.chmod(node.inode, mode) {
                    self.serial.write_str("chmod: ok\r\n");
                } else {
                    self.serial.write_str("chmod: failed\r\n");
                }
            }
            None => {
                self.serial.write_str("chmod: ");
                self.serial.write_str(path);
                self.serial.write_str(": not found\r\n");
            }
        }
    }

    fn serial_write_ip(&mut self, ip: [u8; 4]) {
        self.serial.write_u64(ip[0] as u64);
        self.serial.write_str(".");
        self.serial.write_u64(ip[1] as u64);
        self.serial.write_str(".");
        self.serial.write_u64(ip[2] as u64);
        self.serial.write_str(".");
        self.serial.write_u64(ip[3] as u64);
    }

    fn cmd_journal_init(&mut self) {
        let dev_id = 0;
        let start_block = 3000u64;
        let num_blocks = 16u64;
        if zenus_fs::journal::journal_init(dev_id, start_block, num_blocks) {
            self.serial.write_str("Journal initialized on dev ");
            self.serial.write_u64(dev_id as u64);
            self.serial.write_str(" blocks ");
            self.serial.write_u64(start_block);
            self.serial.write_str("-");
            self.serial.write_u64(start_block + num_blocks - 1);
            self.serial.write_str("\r\n");
        } else {
            self.serial.write_str("Journal init failed\r\n");
        }
    }

    fn cmd_journal_test(&mut self) {
        self.serial.write_str("Journal test:\r\n");
        if !zenus_fs::journal::journal_begin() {
            self.serial.write_str("  [FAIL] journal_begin\r\n");
            return;
        }
        self.serial.write_str("  [ OK ] journal_begin\r\n");

        let test_msg1 = b"JOURNAL TEST BLOCK 0";
        let test_msg2 = b"JOURNAL TEST BLOCK 1";
        let test_msg3 = b"JOURNAL TEST BLOCK 2";
        for (i, msg) in [test_msg1, test_msg2, test_msg3].iter().enumerate() {
            let mut buf = [0u8; 512];
            let len = core::cmp::min(msg.len(), 512);
            buf[..len].copy_from_slice(&msg[..len]);
            if !zenus_fs::journal::journal_write(500 + i as u64, &buf) {
                self.serial.write_str("  [FAIL] journal_write block ");
                self.serial.write_u64(i as u64);
                self.serial.write_str("\r\n");
                return;
            }
            self.serial.write_str("  [ OK ] journal_write block ");
            self.serial.write_u64(500 + i as u64);
            self.serial.write_str("\r\n");
        }

        if !zenus_fs::journal::journal_commit() {
            self.serial.write_str("  [FAIL] journal_commit\r\n");
            return;
        }
        self.serial.write_str("  [ OK ] journal_commit\r\n");

        self.serial.write_str("Replaying journal...\r\n");
        if zenus_fs::journal::journal_replay(0, 3000) {
            self.serial.write_str("  [ OK ] replay (committed entries applied)\r\n");
        } else {
            self.serial.write_str("  [ OK ] replay (no uncommitted entries)\r\n");
        }

        self.serial.write_str("Verifying blocks 500-502...\r\n");
        zenus_fs::block_cache::bc_flush();
        for i in 0..3 {
            let mut buf = [0u8; 512];
            if zenus_fs::block_cache::bc_read(0, 500 + i, &mut buf) {
                let first = buf[0] as u64;
                let second = buf[1] as u64;
                let third = buf[2] as u64;
                self.serial.write_str("  Block ");
                self.serial.write_u64(500 + i);
                self.serial.write_str(": ");
                self.serial.write_u64(first);
                self.serial.write_byte_serial(b',');
                self.serial.write_u64(second);
                self.serial.write_byte_serial(b',');
                self.serial.write_u64(third);
                self.serial.write_str("\r\n");
            } else {
                self.serial.write_str("  Block ");
                self.serial.write_u64(500 + i);
                self.serial.write_str(": read failed\r\n");
            }
        }
        self.serial.write_str("Journal data blocks 3001-3003:\r\n");
        for i in 0..3 {
            let mut buf = [0u8; 512];
            if zenus_fs::block_cache::bc_read(0, 3001 + i, &mut buf) {
                let first = buf[0] as u64;
                self.serial.write_str("  Jnl ");
                self.serial.write_u64(3001 + i);
                self.serial.write_str(": ");
                self.serial.write_u64(first);
                self.serial.write_str("\r\n");
            }
        }
    }

    fn echo_server_poll() {
        zenus_net::socket::poll_all(1);
        unsafe {
            for i in 0..MAX_ECHO_LISTENS {
                if let Some(fd) = ECHO_LISTEN_FDS[i] {
                    while let Some(client_fd) = zenus_net::socket::accept(fd, 1) {
                        for j in 0..MAX_ECHO_CLIENTS {
                            if ECHO_CLIENT_FDS[j].is_none() {
                                ECHO_CLIENT_FDS[j] = Some(client_fd);
                                let mut s = SerialPort::new(0x3F8);
                                s.write_str("[ECHO] accepted fd ");
                                s.write_u64(client_fd as u64);
                                s.write_str("\n");
                                break;
                            }
                        }
                    }
                }
            }
            for i in 0..MAX_ECHO_CLIENTS {
                let fd = match ECHO_CLIENT_FDS[i] {
                    Some(fd) => fd,
                    None => continue,
                };
                let mut buf = [0u8; 1500];
                if let Some(len) = zenus_net::socket::recv(fd, &mut buf) {
                    if len > 0 {
                        zenus_net::socket::send(fd, &buf[..len], 1);
                    }
                }
                if !zenus_net::socket::is_connected(fd) {
                    ECHO_CLIENT_FDS[i] = None;
                }
            }
        }
    }

    fn echo_register_listen(fd: usize) -> bool {
        unsafe {
            for i in 0..MAX_ECHO_LISTENS {
                if ECHO_LISTEN_FDS[i].is_none() {
                    ECHO_LISTEN_FDS[i] = Some(fd);
                    return true;
                }
            }
        }
        false
    }

    fn cmd_tcp_listen(&mut self, args: &[&str]) {
        let port_str = args.iter().find(|a| !a.is_empty()).unwrap_or(&"7");
        let port: u16 = match port_str.parse() {
            Ok(p) => p,
            Err(_) => {
                self.serial.write_str("Invalid port\r\n");
                return;
            }
        };
        match zenus_net::tcp::listen(port) {
            Some(idx) => {
                self.serial.write_str("Listening on port ");
                self.serial.write_u64(port as u64);
                self.serial.write_str(" (conn ");
                self.serial.write_u64(idx as u64);
                self.serial.write_str(")\r\n");
            }
            None => {
                self.serial.write_str("Failed to listen (table full)\r\n");
            }
        }
    }

    fn cmd_tcp_status(&mut self) {
        self.serial.write_str("TCP connections:\r\n");
        self.serial.write_str("  #   State     Local     Remote    Port\r\n");
        for conn in 0..zenus_net::tcp::MAX_CONNS {
            let name = zenus_net::tcp::state_name(conn);
            if name != "NONE" {
                self.serial.write_str("  ");
                self.serial.write_u64(conn as u64);
                self.serial.write_str("  ");
                self.serial.write_str(name);
                self.serial.write_str("\r\n");
            }
        }
        self.serial.write_str("Total: ");
        self.serial.write_u64(zenus_net::tcp::connection_count() as u64);
        self.serial.write_str("\r\n");
    }

    fn cmd_tcp_send(&mut self, args: &[&str]) {
        if args.len() < 2 {
            self.serial.write_str("Usage: tcp-send <conn> <text>\r\n");
            return;
        }
        let conn: usize = match args[0].parse() {
            Ok(c) => c,
            Err(_) => {
                self.serial.write_str("Invalid connection number\r\n");
                return;
            }
        };
        let text = args[1..].join(" ");
        if !zenus_net::tcp::send_data(conn, text.as_bytes()) {
            self.serial.write_str("Send failed\r\n");
            return;
        }
        if !zenus_net::tcp::flush_tx(conn, 0) {
            self.serial.write_str("Flush failed\r\n");
            return;
        }
        self.serial.write_str("Sent\r\n");
    }

    fn cmd_tcp_echo(&mut self, args: &[&str]) {
        let port: u16 = args.first().and_then(|a| a.parse().ok()).unwrap_or(7);
        self.serial.write_str("Starting echo server on port ");
        self.serial.write_u64(port as u64);
        self.serial.write_str("...\r\n");
        let fd = match zenus_net::socket::socket(
            zenus_net::socket::AF_INET,
            zenus_net::socket::SOCK_STREAM,
            0,
        ) {
            Some(fd) => fd,
            None => {
                self.serial.write_str("Failed to create socket\r\n");
                return;
            }
        };
        if !zenus_net::socket::bind(fd, port) {
            self.serial.write_str("Failed to bind\r\n");
            return;
        }
        if !zenus_net::socket::listen(fd, 1) {
            self.serial.write_str("Failed to listen\r\n");
            return;
        }
        self.serial.write_str("Echo server started on port ");
        self.serial.write_u64(port as u64);
        self.serial.write_str(" (fd ");
        self.serial.write_u64(fd as u64);
        self.serial.write_str(")\r\n");
        if !Self::echo_register_listen(fd) {
            self.serial.write_str("Warning: echo fd table full\r\n");
        }
    }

    fn cmd_tcp_connect(&mut self, args: &[&str]) {
        if args.len() < 1 {
            self.serial.write_str("Usage: tcp-connect <port> [ip]\r\n");
            return;
        }
        let port: u16 = match args[0].parse() {
            Ok(p) => p,
            Err(_) => { self.serial.write_str("Invalid port\r\n"); return; }
        };
        let dst_ip = if args.len() >= 2 {
            let mut ip = [0u8; 4];
            let mut part = 0;
            for octet in args[1].split('.') {
                if part >= 4 { break; }
                ip[part] = match octet.parse() { Ok(n) => n, Err(_) => { self.serial.write_str("Invalid IP\r\n"); return; } };
                part += 1;
            }
            if part != 4 { self.serial.write_str("Invalid IP\r\n"); return; }
            ip
        } else {
            [10, 0, 2, 2]
        };
        let fd = match zenus_net::socket::socket(zenus_net::socket::AF_INET, zenus_net::socket::SOCK_STREAM, 0) {
            Some(fd) => fd,
            None => { self.serial.write_str("Failed to create socket\r\n"); return; }
        };
        self.serial.write_str("Connecting to ");
        self.serial_write_ip(dst_ip);
        self.serial.write_str(":");
        self.serial.write_u64(port as u64);
        self.serial.write_str(" (fd ");
        self.serial.write_u64(fd as u64);
        self.serial.write_str(")...\r\n");
        if zenus_net::socket::connect(fd, 1, dst_ip, port) {
            self.serial.write_str("[ OK ] Connected\r\n");
        } else {
            self.serial.write_str("[FAIL] Connection failed\r\n");
        }
    }

    fn cmd_udp_bind(&mut self, args: &[&str]) {
        if args.len() < 1 {
            self.serial.write_str("Usage: udp-bind <port>\r\n");
            return;
        }
        let port: u16 = match args[0].parse() {
            Ok(p) => p,
            Err(_) => { self.serial.write_str("Invalid port\r\n"); return; }
        };
        let fd = match zenus_net::socket::socket(zenus_net::socket::AF_INET, zenus_net::socket::SOCK_DGRAM, 0) {
            Some(fd) => fd,
            None => { self.serial.write_str("Failed to create socket\r\n"); return; }
        };
        if zenus_net::socket::bind(fd, port) {
            self.serial.write_str("UDP socket bound on port ");
            self.serial.write_u64(port as u64);
            self.serial.write_str(" (fd ");
            self.serial.write_u64(fd as u64);
            self.serial.write_str(")\r\n");
        } else {
            self.serial.write_str("Failed to bind\r\n");
        }
    }

    fn cmd_udp_send(&mut self, args: &[&str]) {
        if args.len() < 3 {
            self.serial.write_str("Usage: udp-send <fd> <ip> <port> <text>\r\n");
            return;
        }
        let fd: usize = match args[0].parse() { Ok(f) => f, Err(_) => { self.serial.write_str("Invalid fd\r\n"); return; } };
        let mut dst_ip = [0u8; 4];
        let mut part = 0;
        for octet in args[1].split('.') {
            if part >= 4 { break; }
            dst_ip[part] = match octet.parse() { Ok(n) => n, Err(_) => { self.serial.write_str("Invalid IP\r\n"); return; } };
            part += 1;
        }
        if part != 4 { self.serial.write_str("Invalid IP\r\n"); return; }
        let port: u16 = match args[2].parse() { Ok(p) => p, Err(_) => { self.serial.write_str("Invalid port\r\n"); return; } };
        let text = args[3..].join(" ");
        if zenus_net::socket::sendto(fd, text.as_bytes(), 1, dst_ip, port) {
            self.serial.write_str("Sent\r\n");
        } else {
            self.serial.write_str("Send failed\r\n");
        }
    }

    fn cmd_udp_recv(&mut self, args: &[&str]) {
        if args.len() < 1 {
            self.serial.write_str("Usage: udp-recv <fd>\r\n");
            return;
        }
        let fd: usize = match args[0].parse() { Ok(f) => f, Err(_) => { self.serial.write_str("Invalid fd\r\n"); return; } };
        let mut buf = [0u8; 1500];
        if let Some(len) = zenus_net::socket::recv(fd, &mut buf) {
            self.serial.write_str("Received: ");
            if let Ok(s) = core::str::from_utf8(&buf[..len]) {
                self.serial.write_str(s);
            } else {
                self.serial.write_hex(len as u64);
                self.serial.write_str(" bytes (non-utf8)\r\n");
            }
        } else {
            self.serial.write_str("No data\r\n");
        }
    }

    fn cmd_readdev(&mut self, args: &[&str]) {
        let path = args.iter().find(|a| !a.is_empty()).unwrap_or(&"/dev/sda");
        let node = zenus_fs::vfs::open(path);
        if node.is_none() {
            self.serial.write_str("readdev: device not found\r\n");
            return;
        }
        let node = node.unwrap();
        let mut buf = [0u8; 512];
        match node.fs.read(node.inode, 0, &mut buf) {
            Some(_) => {
                self.serial.write_str("Sector 0:\r\n");
                for i in 0..8 {
                    for j in 0..16 {
                        let val = buf[i * 16 + j];
                        self.serial.write_hex(val as u64);
                        self.serial.write_str(" ");
                    }
                    self.serial.write_str("  |");
                    for j in 0..16 {
                        let c = buf[i * 16 + j];
                        if c >= 32 && c <= 126 {
                            self.serial.write_byte_serial(c);
                        } else {
                            self.serial.write_byte_serial(b'.');
                        }
                    }
                    self.serial.write_str("|\r\n");
                }
                self.serial.write_str("(hexdump of first 128 bytes)\r\n");
            }
            None => self.serial.write_str("readdev: read failed\r\n"),
        }
    }

    fn cmd_ls(&mut self, args: &[&str]) {
        let long = args.iter().any(|a| *a == "-l");
        let path = args.iter().find(|a| !a.is_empty() && **a != "-l").unwrap_or(&"/");
        let path = if path.is_empty() { "/" } else { path };

        let entries = zenus_fs::vfs::read_dir(path);
        if entries.is_empty() {
            match zenus_fs::vfs::open(path) {
                Some(node) => {
                    let e = node.fs.read_dir(node.inode);
                    if e.is_empty() {
                        self.serial.write_str("(empty)\r\n");
                    } else {
                        for entry in e {
                            if long {
                                let stat = node.fs.stat(entry.inode);
                                self.serial.write_str(zenus_fs::vfs::perm_str(stat.mode));
                                self.serial.write_str(" ");
                                self.serial.write_u64(stat.uid as u64);
                                self.serial.write_str(":");
                                self.serial.write_u64(stat.gid as u64);
                                self.serial.write_str(" ");
                                self.serial.write_u64(stat.size);
                                self.serial.write_str(" ");
                            }
                            self.serial_write_dirent(entry.name, entry.file_type);
                        }
                        self.serial.write_str("\r\n");
                    }
                }
                None => {
                    self.serial.write_str("ls: ");
                    self.serial.write_str(path);
                    self.serial.write_str(": not found\r\n");
                }
            }
            return;
        }
        for entry in entries {
            if long {
                let node = match zenus_fs::vfs::open(path) {
                    Some(n) => n,
                    None => continue,
                };
                let stat = node.fs.stat(entry.inode);
                self.serial.write_str(zenus_fs::vfs::perm_str(stat.mode));
                self.serial.write_str(" ");
                self.serial.write_u64(stat.uid as u64);
                self.serial.write_str(":");
                self.serial.write_u64(stat.gid as u64);
                self.serial.write_str(" ");
                self.serial.write_u64(stat.size);
                self.serial.write_str(" ");
            }
            self.serial_write_dirent(entry.name, entry.file_type);
        }
        self.serial.write_str("\r\n");
    }

    fn serial_write_dirent(&mut self, name: &str, file_type: zenus_fs::vfs::FileType) {
        self.serial.write_str(name);
        if file_type == zenus_fs::vfs::FileType::Directory {
            self.serial.write_str("/");
        }
        self.serial.write_str("  ");
    }

    fn cmd_cat(&mut self, args: &[&str]) {
        let path = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.serial.write_str("cat: missing operand\r\n");
                return;
            }
        };

        match zenus_fs::vfs::open(path) {
            Some(node) => {
                let stat = node.fs.stat(node.inode);
                if stat.file_type == zenus_fs::vfs::FileType::Directory {
                    self.serial.write_str("cat: ");
                    self.serial.write_str(path);
                    self.serial.write_str(": Is a directory\r\n");
                    return;
                }
                let mut buf = [0u8; 512];
                let mut offset: u64 = 0;
                let mut last_byte: u8 = 0;
                loop {
                    match node.fs.read(node.inode, offset, &mut buf) {
                        Some(0) | None => break,
                        Some(n) => {
                            for i in 0..n {
                                let b = buf[i as usize];
                                if b == b'\n' {
                                    self.serial.write_byte_serial(b'\r');
                                }
                                self.serial.write_byte_serial(b);
                                last_byte = b;
                            }
                            offset += n as u64;
                        }
                    }
                }
                if offset > 0 && last_byte != b'\n' {
                    self.serial.write_str("\r\n");
                }
            }
            None => {
                self.serial.write_str("cat: ");
                self.serial.write_str(path);
                self.serial.write_str(": not found\r\n");
            }
        }
    }
}

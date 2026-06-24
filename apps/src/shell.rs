use zenus_console::serial::SerialPort;

const MAX_LINE: usize = 256;
const MAX_ARGS: usize = 16;
const PROMPT: &str = "zenus$ ";

const MAX_ECHO_LISTENS: usize = 8;
const MAX_ECHO_CLIENTS: usize = 16;

struct EchoState {
    listen_fds: [Option<usize>; MAX_ECHO_LISTENS],
    client_fds: [Option<usize>; MAX_ECHO_CLIENTS],
}

static ECHO_STATE: zenus_sync::spinlock::SpinLock<EchoState> = zenus_sync::spinlock::SpinLock::new(EchoState {
    listen_fds: [None; MAX_ECHO_LISTENS],
    client_fds: [None; MAX_ECHO_CLIENTS],
});

pub struct Shell {
    serial: SerialPort,
    hhdm_offset: u64,
}

impl Shell {
    pub fn new() -> Self {
        Shell {
            serial: SerialPort::new(0x3F8),
            hhdm_offset: zenus_arch::limine::hhdm_offset(),
        }
    }

    fn write_str(&mut self, s: &str) {
        self.serial.write_str_noirq(s);
        zenus_console::vga::write_str(s, self.hhdm_offset);
    }

    fn write_byte(&mut self, b: u8) {
        self.serial.write_str_noirq(core::str::from_utf8(&[b]).unwrap_or(""));
        let arr = [b];
        if let Ok(s) = core::str::from_utf8(&arr) {
            zenus_console::vga::write_str(s, self.hhdm_offset);
        }
    }

    fn write_vga_only(&mut self, b: u8) {
        let arr = [b];
        if let Ok(s) = core::str::from_utf8(&arr) {
            zenus_console::vga::write_str(s, self.hhdm_offset);
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
            if yield_count % 50 == 0 {
                zenus_arch::watchdog::watchdog_pet();
                zenus_sched::init::service_supervise();
            }
            self.write_str(PROMPT);
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
        let mut idle_count = 0u64;

        unsafe { POS = 0 };

        loop {
            let c = if self.serial.is_data_available() {
                let b = self.serial.read_byte_serial();
                Some(b)
            } else if zenus_arch::keyboard::is_key_available() {
                let b = zenus_arch::keyboard::read_key().unwrap_or(0);
                Some(b)
            } else {
                None
            };

            if let Some(c) = c {
                match c {
                    b'\r' | b'\n' => {
                        self.write_str("\r\n");
                        unsafe {
                            let s = core::str::from_utf8(&BUF[..POS]).unwrap_or("");
                            POS = 0;
                            return if s.is_empty() { None } else { Some(s) };
                        }
                    }
                    b'\x7F' | b'\x08' => {
                        if unsafe { POS > 0 } {
                            unsafe { POS -= 1 };
                            self.write_str("\x08 \x08");
                        }
                    }
                    0x20..=0x7E => {
                        unsafe {
                            if POS < MAX_LINE - 1 {
                                BUF[POS] = c;
                                POS += 1;
                                self.write_vga_only(c);
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                idle_count += 1;
                if idle_count % 10 == 0 {
                    zenus_net::nic::net_poll();
                    Self::echo_server_poll();
                }
                if !zenus_arch::keyboard::is_key_available() {
                    zenus_sched::scheduler::yield_now();
                }
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
            // Phase 3 commands
            "init-start" => self.cmd_init_start(),
            "init-shutdown" => self.cmd_init_shutdown(),
            "service-list" => self.cmd_service_list(),
            "service-start" => self.cmd_service_start(&parts[1..count]),
            "service-stop" => self.cmd_service_stop(&parts[1..count]),
            "service-restart" => self.cmd_service_restart(&parts[1..count]),
            "sysctl" => self.cmd_sysctl(&parts[1..count]),
            "pkg-install" => self.cmd_pkg_install(&parts[1..count]),
            "pkg-list" => self.cmd_pkg_list(),
            "pkg-remove" => self.cmd_pkg_remove(&parts[1..count]),
            "pkg-info" => self.cmd_pkg_info(&parts[1..count]),
            "watchdog-pet" => self.cmd_watchdog_pet(),
            "watchdog-status" => self.cmd_watchdog_status(),
            "crashdump" => self.cmd_crashdump(),
            "lockdep-status" => self.cmd_lockdep_status(),
            "syslog" => self.cmd_syslog(&parts[1..count]),
            "ssh-start" => self.cmd_ssh_start(),
            "ssh-status" => self.cmd_ssh_status(),
            "ns-info" => self.cmd_ns_info(),
            "ns-sethost" => self.cmd_ns_sethost(&parts[1..count]),
            "ns-clone" => self.cmd_ns_clone(&parts[1..count]),
            _ => {
                self.write_str("Unknown command: ");
                self.write_str(cmd);
                self.write_str("\r\n");
            }
        }
    }

    fn cmd_help(&mut self) {
        self.write_str("Commands:\r\n");
        self.write_str("  help   - Show this help\r\n");
        self.write_str("  echo   - Print text\r\n");
        self.write_str("  ls     - List directory\r\n");
        self.write_str("  ls -l  - List with permissions, owner, size\r\n");
        self.write_str("  chmod <mode> <file> - Change file permissions (octal)\r\n");
        self.write_str("Phase 3 Server Commands:\r\n");
        self.write_str("  init-start      - Start init system & services\r\n");
        self.write_str("  init-shutdown   - Shutdown all services\r\n");
        self.write_str("  service-list    - List registered services\r\n");
        self.write_str("  service-start <name> - Start a service\r\n");
        self.write_str("  service-stop <name>  - Stop a service\r\n");
        self.write_str("  service-restart <name> - Restart a service\r\n");
        self.write_str("  sysctl [name]   - Show/set kernel parameters\r\n");
        self.write_str("  sysctl <name>=<val> - Set kernel parameter\r\n");
        self.write_str("  pkg-install <path> - Install .zpk package\r\n");
        self.write_str("  pkg-list        - List installed packages\r\n");
        self.write_str("  pkg-remove <name> - Remove package\r\n");
        self.write_str("  pkg-info <name> - Show package info\r\n");
        self.write_str("  watchdog-pet    - Pet the watchdog timer\r\n");
        self.write_str("  watchdog-status - Show watchdog status\r\n");
        self.write_str("  crashdump       - Show crash dump info\r\n");
        self.write_str("  lockdep-status  - Show lockdep status\r\n");
        self.write_str("  syslog [n]      - Show last N syslog entries\r\n");
        self.write_str("  ssh-start       - Start SSH server service\r\n");
        self.write_str("  ssh-status      - Show SSH server status\r\n");
        self.write_str("  cat    - Show file contents\r\n");
        self.write_str("  clear  - Clear screen\r\n");
        self.write_str("  timer  - Show APIC timer tick count\r\n");
        self.write_str("  ps     - List processes\r\n");
        self.write_str("  kill   - Kill process\r\n");
        self.write_str("  mkdir  - Create directory\r\n");
        self.write_str("  rm     - Remove file/directory\r\n");
        self.write_str("  touch  - Create empty file\r\n");
        self.write_str("  ifconfig - Show network interfaces\r\n");
        self.write_str("  meminfo  - Show memory usage\r\n");
        self.write_str("  reboot   - Reboot the system\r\n");
        self.write_str("  shutdown - Shutdown the system\r\n");
        self.write_str("  uname    - Show kernel version info\r\n");
        self.write_str("  version  - Alias for uname\r\n");
        self.write_str("  dmesg    - Show kernel log buffer\r\n");
        self.write_str("  readdev  - Hexdump first sector of a block device\r\n");
        self.write_str("  tcp-listen <port> - Listen on TCP port\r\n");
        self.write_str("  tcp-status        - Show TCP connection table\r\n");
        self.write_str("  tcp-send <conn> <text> - Send data on connection\r\n");
        self.write_str("  tcp-echo  - Start echo server on port 7\r\n");
        self.write_str("  tcp-connect <port> [ip] - Connect to TCP port\r\n");
        self.write_str("  udp-bind <port> - Bind UDP socket\r\n");
        self.write_str("  udp-send <fd> <ip> <port> <text> - Send UDP datagram\r\n");
        self.write_str("  udp-recv <fd> - Receive UDP datagram\r\n");
        self.write_str("  dhcp      - Acquire IP via DHCP\r\n");
        self.write_str("  dhcp-server - Start DHCP server (requires static IP)\r\n");
        self.write_str("  resolve <domain> - DNS resolve domain name\r\n");
        self.write_str("  id        - Show current user/group IDs\r\n");
        self.write_str("  whoami    - Show current username\r\n");
        self.write_str("Namespace Commands:\r\n");
        self.write_str("  ns-info      - Show current namespace IDs and hostname\r\n");
        self.write_str("  ns-sethost   - Set hostname in current UTS namespace\r\n");
        self.write_str("  ns-clone [--uts] [--pid] [--mnt] - Clone into new namespace(s)\r\n");
    }

    fn cmd_echo(&mut self, args: &[&str]) {
        for (i, arg) in args.iter().enumerate() {
            if arg.is_empty() { continue; }
            if i > 0 {
                self.write_byte(b' ');
            }
            self.write_str(arg);
        }
        self.write_str("\r\n");
    }

    fn cmd_clear(&mut self) {
        self.write_str("\x1B[2J\x1B[H");
    }

    fn cmd_timer(&mut self) {
        let ticks = zenus_arch::interrupts::handler::get_timer_tick();
        self.write_str("Timer ticks: ");
        self.serial.write_u64(ticks);
        self.write_str("\r\n");
    }

    fn cmd_ps(&mut self) {
        self.write_str("PID\tUID\tGID\tState\tPidNS\tUtsNS\r\n");
        let tasks = zenus_sched::scheduler::list_tasks();
        for info in tasks.iter().flatten() {
            self.serial.write_u64(info.id);
            self.write_str("\t");
            self.serial.write_u64(info.uid as u64);
            self.write_str("\t");
            self.serial.write_u64(info.gid as u64);
            self.write_str("\t");
            self.write_str(info.state.to_str());
            self.write_str("\t");
            self.serial.write_u64(info.pid_ns as u64);
            self.write_str("\t");
            self.serial.write_u64(info.uts_ns as u64);
            if info.id == zenus_sched::scheduler::current_task_id() {
                self.write_str(" (current)");
            }
            self.write_str("\r\n");
        }
    }

    fn cmd_kill(&mut self, args: &[&str]) {
        let pid_str = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.write_str("kill: missing pid\r\n");
                return;
            }
        };

        let pid = match pid_str.parse::<u64>() {
            Ok(p) => p,
            Err(_) => {
                self.write_str("kill: invalid pid\r\n");
                return;
            }
        };

        if pid == 0 {
            self.write_str("kill: cannot kill idle process\r\n");
            return;
        }

        let current_pid = zenus_sched::scheduler::current_task_id();
        if pid == current_pid {
            self.write_str("kill: cannot kill the shell itself\r\n");
            return;
        }

        if zenus_sched::scheduler::kill_task(pid) {
            self.write_str("killed: ");
            self.serial.write_u64(pid);
            self.write_str("\r\n");
        } else {
            self.write_str("kill: not found: ");
            self.serial.write_u64(pid);
            self.write_str("\r\n");
        }
    }

    fn cmd_mkdir(&mut self, args: &[&str]) {
        let path = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.write_str("mkdir: missing operand\r\n");
                return;
            }
        };

        if zenus_fs::vfs::create_dir(path) {
            self.write_str("ok\r\n");
        } else {
            self.write_str("mkdir: failed to create directory\r\n");
        }
    }

    fn cmd_rm(&mut self, args: &[&str]) {
        let path = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.write_str("rm: missing operand\r\n");
                return;
            }
        };

        if zenus_fs::vfs::remove(path) {
            self.write_str("ok\r\n");
        } else {
            self.write_str("rm: failed to remove\r\n");
        }
    }

    fn cmd_touch(&mut self, args: &[&str]) {
        let path = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.write_str("touch: missing operand\r\n");
                return;
            }
        };

        if zenus_fs::vfs::create_file(path) {
            self.write_str("ok\r\n");
        } else {
            self.write_str("touch: failed to create file\r\n");
        }
    }

    fn cmd_ifconfig(&mut self) {
        let count = zenus_net::nic::iface_count();
        for i in 0..count {
            if let Some(iface) = zenus_net::nic::get_iface(i) {
                self.write_str("Interface ");
                self.serial.write_u64(i as u64);
                self.write_str(":\r\n");
                self.write_str("  MAC: ");
                for b in &iface.mac {
                    self.serial.write_hex(*b as u64);
                    self.write_str(":");
                }
                self.write_str("\r\n  IP: ");
                self.serial.write_u64(iface.ip[0] as u64);
                self.write_str(".");
                self.serial.write_u64(iface.ip[1] as u64);
                self.write_str(".");
                self.serial.write_u64(iface.ip[2] as u64);
                self.write_str(".");
                self.serial.write_u64(iface.ip[3] as u64);
                self.write_str("\r\n  Link: ");
                if iface.link_up {
                    self.write_str("UP\r\n");
                } else {
                    self.write_str("DOWN\r\n");
                }
            }
        }
    }

    fn cmd_meminfo(&mut self) {
        let free_head = zenus_mem::allocator::ALLOCATOR.free_head_addr();
        self.write_str("Heap: 4MB free-list allocator\r\n");
        self.write_str("  Free list head: 0x");
        self.serial.write_hex(free_head as u64);
        self.write_str("\r\n");

        let fa = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
        self.write_str("Physical frames:\r\n");
        self.write_str("  Total: ");
        self.serial.write_u64(fa.total_memory() / 4096);
        self.write_str(" frames (");
        self.serial.write_u64(fa.total_memory() / (1024*1024));
        self.write_str(" MB)\r\n");
        self.write_str("  Used:  ");
        self.serial.write_u64(fa.used_memory() / 4096);
        self.write_str(" frames (");
        self.serial.write_u64(fa.used_memory() / (1024*1024));
        self.write_str(" MB)\r\n");
        self.write_str("  Free stack: ");
        self.serial.write_u64(fa.free_frames_count() as u64);
        self.write_str(" frames\r\n");
    }

    fn cmd_reboot(&mut self) {
        self.write_str("Rebooting...\r\n");
        zenus_arch::acpi::reboot_via_keyboard();
    }

    fn cmd_shutdown(&mut self) {
        self.write_str("Shutting down...\r\n");
        zenus_arch::acpi::shutdown_via_acpi();
    }

    fn cmd_uname(&mut self) {
        let uts_ns = zenus_sched::scheduler::current_uts_ns();
        let hostname = zenus_ns::uts::get_hostname(uts_ns);
        let hlen = hostname.iter().position(|&b| b == 0).unwrap_or(64);
        self.write_str("Zenus OS v0.1.0 x86_64");
        if hlen > 0 {
            self.write_str(" (");
            if let Ok(h) = core::str::from_utf8(&hostname[..hlen]) {
                self.write_str(h);
            }
            self.write_str(")");
        }
        self.write_str("\r\n");
    }

    fn cmd_dmesg(&mut self) {
        let snap = zenus_console::log::dmesg_snapshot();
        for i in 0..snap.count {
            let entry = &snap.entries[i];
            let len = core::cmp::min(entry.len as usize, entry.msg.len());
            let msg = core::str::from_utf8(&entry.msg[..len]).unwrap_or("");
            self.write_str(entry.level.prefix());
            self.write_str(" ");
            self.write_str(msg);
            self.write_str("\r\n");
        }
        if snap.count == 0 {
            self.write_str("(no messages)\r\n");
        }
    }

    fn cmd_mount(&mut self) {
        let mnt_ns = zenus_sched::scheduler::current_mnt_ns();
        self.write_str("Mount points (ns ");
        self.serial.write_u64(mnt_ns as u64);
        self.write_str("):\r\n");
        self.write_str("  /       tmpfs (root)\r\n");
        if zenus_fs::vfs::open_in_ns(mnt_ns, "/dev").is_some() {
            self.write_str("  /dev    devfs\r\n");
        }
        if zenus_fs::vfs::open_in_ns(mnt_ns, "/mnt").is_some() {
            self.write_str("  /mnt    ext2 (if mounted)\r\n");
        }
        if zenus_fs::vfs::open_in_ns(mnt_ns, "/virtio").is_some() {
            self.write_str("  /virtio virtio-blk ext2\r\n");
        }
        if zenus_fs::vfs::open_in_ns(mnt_ns, "/initrd").is_some() {
            self.write_str("  /initrd initrd (tarfs)\r\n");
        }
        let (hits, misses) = zenus_fs::block_cache::bc_stats();
        self.write_str("Block cache: ");
        self.serial.write_u64(hits);
        self.write_str(" hits, ");
        self.serial.write_u64(misses);
        self.write_str(" misses\r\n");
    }

    fn cmd_bcache(&mut self) {
        let (hits, misses) = zenus_fs::block_cache::bc_stats();
        self.write_str("Block cache stats:\r\n");
        self.write_str("  Hits:   ");
        self.serial.write_u64(hits);
        self.write_str("\r\n  Misses: ");
        self.serial.write_u64(misses);
        let total = hits + misses;
        if total > 0 {
            self.write_str("\r\n  Rate:   ");
            self.serial.write_u64(hits * 100 / total);
            self.write_str("%\r\n");
        } else {
            self.write_str("\r\n  (no I/O yet)\r\n");
        }
    }

    fn cmd_fsck(&mut self) {
        let result = zenus_fs::ext2_fsck::fsck(0);
        self.write_str("fsck results:\r\n");
        if result.passed() {
            self.write_str("  PASSED");
        } else {
            self.write_str("  FAILED");
        }
        self.write_str(" (");
        self.serial.write_u64(result.errors as u64);
        self.write_str(" errors, ");
        self.serial.write_u64(result.warnings as u64);
        self.write_str(" warnings)\r\n");
        for i in 0..result.count {
            let msg = &result.messages[i];
            let sev = match msg.severity {
                zenus_fs::ext2_fsck::FsckSeverity::Fatal => "FATAL",
                zenus_fs::ext2_fsck::FsckSeverity::Error => "ERROR",
                zenus_fs::ext2_fsck::FsckSeverity::Warning => " WARN",
                _ => " INFO",
            };
            self.write_str("  [");
            self.write_str(sev);
            self.write_str("] ");
            self.write_str(msg.msg);
            self.write_str("\r\n");
        }
    }

    fn cmd_dhcp(&mut self) {
        self.write_str("DHCP client starting...\r\n");
        let iface_idx = 1;
        if zenus_net::dhcp::dhcp_start(iface_idx) {
            self.write_str("[ OK ] DHCP: address acquired\r\n");
            self.cmd_ifconfig();
        } else {
            self.write_str("[FAIL] DHCP: no response\r\n");
        }
    }

    fn cmd_dhcp_server(&mut self, _args: &[&str]) {
        self.write_str("DHCP server running on 10.0.2.100-10.0.2.115\r\n");
        let iface_idx = 1;
        let iface = match zenus_net::nic::get_iface(iface_idx) {
            Some(iface) => iface,
            None => {
                self.write_str("[FAIL] No interface\r\n");
                return;
            }
        };
        if iface.ip == [0; 4] || iface.ip == [127, 0, 0, 1] {
            self.write_str("[FAIL] Server needs a static IP (run `dhcp` first)\r\n");
            return;
        }
        self.write_str("[ OK ] DHCP server ready on ");
        self.serial_write_ip(iface.ip);
        self.write_str("\r\n");
        self.write_str("Leases:\r\n");
        if zenus_net::dhcp_server::lease_count() == 0 {
            self.write_str("  (none)\r\n");
        } else {
            zenus_net::dhcp_server::print_leases(&mut |s| self.write_str(s));
        }
    }

    fn cmd_resolve(&mut self, args: &[&str]) {
        if args.len() < 1 {
            self.write_str("Usage: resolve <domain>\r\n");
            return;
        }
        let dns_server = [10, 0, 2, 3];
        self.write_str("Resolving ");
        self.write_str(args[0]);
        self.write_str(" via ");
        self.serial_write_ip(dns_server);
        self.write_str("...\r\n");
        match zenus_net::dns::resolve(1, dns_server, args[0]) {
            Some(ip) => {
                self.write_str("  -> ");
                self.serial_write_ip(ip);
                self.write_str("\r\n");
            }
            None => {
                self.write_str("  [FAIL] resolution failed\r\n");
            }
        }
    }

    fn cmd_id(&mut self) {
        let uid = zenus_sched::scheduler::current_uid();
        let euid = zenus_sched::scheduler::current_euid();
        let gid = zenus_sched::scheduler::current_gid();
        let egid = zenus_sched::scheduler::current_egid();
        self.write_str("uid=");
        self.serial.write_u64(uid as u64);
        if euid != uid {
            self.write_str(" euid=");
            self.serial.write_u64(euid as u64);
        }
        self.write_str(" gid=");
        self.serial.write_u64(gid as u64);
        if egid != gid {
            self.write_str(" egid=");
            self.serial.write_u64(egid as u64);
        }
        self.write_str("\r\n");
    }

    fn cmd_whoami(&mut self) {
        self.write_str("root\r\n");
    }

    // --- Phase 3 commands ---

    fn cmd_init_start(&mut self) {
        if zenus_sched::init::init_system_start() {
            self.write_str("[ OK ] Init system started\r\n");
        } else {
            self.write_str("[INFO] Init system already running\r\n");
        }
    }

    fn cmd_init_shutdown(&mut self) {
        self.write_str("Shutting down init system...\r\n");
        zenus_sched::init::init_shutdown();
    }

    fn cmd_service_list(&mut self) {
        let services = zenus_sched::init::service_list();
        if services.is_empty() {
            self.write_str("No services registered\r\n");
            return;
        }
        self.write_str("Service          State     PID   Restarts\r\n");
        self.write_str("----------------------------------------\r\n");
        for (name, state, pid, restarts) in services {
            self.write_str(name);
            // Pad name
            let name_len = name.len();
            for _ in name_len..16 {
                self.write_byte(b' ');
            }
            let state_str = match state {
                zenus_sched::init::ServiceState::Running => "Running",
                zenus_sched::init::ServiceState::Stopped => "Stopped",
                zenus_sched::init::ServiceState::Crashed => "Crashed",
                zenus_sched::init::ServiceState::Disabled => "Disabled",
            };
            self.write_str(" ");
            self.write_str(state_str);
            // Pad state
            let state_len = state_str.len();
            for _ in state_len..10 {
                self.write_byte(b' ');
            }
            self.serial.write_u64(pid);
            self.write_str("   ");
            self.serial.write_u64(restarts as u64);
            self.write_str("\r\n");
        }
    }

    fn cmd_service_start(&mut self, args: &[&str]) {
        let name = match args.iter().find(|a| !a.is_empty()) {
            Some(n) => n,
            None => {
                self.write_str("Usage: service-start <name>\r\n");
                return;
            }
        };
        if zenus_sched::init::service_start(name) {
            self.write_str("Started: ");
            self.write_str(name);
            self.write_str("\r\n");
        } else {
            self.write_str("Failed to start: ");
            self.write_str(name);
            self.write_str("\r\n");
        }
    }

    fn cmd_service_stop(&mut self, args: &[&str]) {
        let name = match args.iter().find(|a| !a.is_empty()) {
            Some(n) => n,
            None => {
                self.write_str("Usage: service-stop <name>\r\n");
                return;
            }
        };
        if zenus_sched::init::service_stop(name) {
            self.write_str("Stopped: ");
            self.write_str(name);
            self.write_str("\r\n");
        } else {
            self.write_str("Failed to stop: ");
            self.write_str(name);
            self.write_str("\r\n");
        }
    }

    fn cmd_service_restart(&mut self, args: &[&str]) {
        let name = match args.iter().find(|a| !a.is_empty()) {
            Some(n) => n,
            None => {
                self.write_str("Usage: service-restart <name>\r\n");
                return;
            }
        };
        if zenus_sched::init::service_restart(name) {
            self.write_str("Restarted: ");
            self.write_str(name);
            self.write_str("\r\n");
        } else {
            self.write_str("Failed to restart: ");
            self.write_str(name);
            self.write_str("\r\n");
        }
    }

    fn cmd_sysctl(&mut self, args: &[&str]) {
        if args.len() < 1 {
            // List all sysctls
            let list = zenus_fs::sysctl::sysctl_list();
            self.write_str("Sysctl parameters:\r\n");
            for entry in list {
                self.write_str("  ");
                self.write_str(entry.name);
                self.write_str(" = ");
                match &entry.value {
                    zenus_fs::sysctl::SysctlValue::IntVal(v) => {
                        self.serial.write_i64(*v);
                    }
                    zenus_fs::sysctl::SysctlValue::UintVal(v) => {
                        self.serial.write_u64(*v);
                    }
                    zenus_fs::sysctl::SysctlValue::BoolVal(v) => {
                        self.write_str(if *v { "true" } else { "false" });
                    }
                    zenus_fs::sysctl::SysctlValue::StrVal(v) => {
                        self.write_str(v);
                    }
                }
                if entry.read_only {
                    self.write_str(" (read-only)");
                }
                self.write_str("\r\n");
            }
            return;
        }

        let arg = args[0];
        if let Some(eq_pos) = arg.find('=') {
            let name = &arg[..eq_pos];
            let val_str = &arg[eq_pos + 1..];
            let idx = match zenus_fs::sysctl::sysctl_find(name) {
                Some(i) => i,
                None => {
                    self.write_str("sysctl: not found: ");
                    self.write_str(name);
                    self.write_str("\r\n");
                    return;
                }
            };
            let entry = match zenus_fs::sysctl::sysctl_get(idx) {
                Some(e) => e,
                None => {
                    self.write_str("sysctl: error reading: ");
                    self.write_str(name);
                    self.write_str("\r\n");
                    return;
                }
            };
            let value = match entry.value {
                zenus_fs::sysctl::SysctlValue::IntVal(_) => {
                    let v: i64 = match val_str.parse() {
                        Ok(n) => n,
                        Err(_) => { self.write_str("sysctl: invalid integer\r\n"); return; }
                    };
                    zenus_fs::sysctl::SysctlValue::IntVal(v)
                }
                zenus_fs::sysctl::SysctlValue::UintVal(_) => {
                    let v: u64 = match val_str.parse() {
                        Ok(n) => n,
                        Err(_) => { self.write_str("sysctl: invalid unsigned integer\r\n"); return; }
                    };
                    zenus_fs::sysctl::SysctlValue::UintVal(v)
                }
                zenus_fs::sysctl::SysctlValue::BoolVal(_) => {
                    let v = val_str == "1" || val_str == "true" || val_str == "yes";
                    zenus_fs::sysctl::SysctlValue::BoolVal(v)
                }
                zenus_fs::sysctl::SysctlValue::StrVal(_) => {
                    self.write_str("sysctl: string values cannot be set via shell\r\n");
                    return;
                }
            };
            if zenus_fs::sysctl::sysctl_set(idx, value) {
                self.write_str("sysctl: ");
                self.write_str(name);
                self.write_str(" = ");
                self.write_str(val_str);
                self.write_str("\r\n");
            } else {
                self.write_str("sysctl: failed to set: ");
                self.write_str(name);
                self.write_str("\r\n");
            }
        } else {
            let idx = match zenus_fs::sysctl::sysctl_find(arg) {
                Some(i) => i,
                None => {
                    self.write_str("sysctl: not found: ");
                    self.write_str(arg);
                    self.write_str("\r\n");
                    return;
                }
            };
            let entry = match zenus_fs::sysctl::sysctl_get(idx) {
                Some(e) => e,
                None => {
                    self.write_str("sysctl: error reading\r\n");
                    return;
                }
            };
            self.write_str(entry.name);
            self.write_str(" = ");
            match &entry.value {
                zenus_fs::sysctl::SysctlValue::IntVal(v) => self.serial.write_i64(*v),
                zenus_fs::sysctl::SysctlValue::UintVal(v) => self.serial.write_u64(*v),
                zenus_fs::sysctl::SysctlValue::BoolVal(v) => self.write_str(if *v { "true" } else { "false" }),
                zenus_fs::sysctl::SysctlValue::StrVal(v) => self.write_str(v),
            }
            if entry.read_only {
                self.write_str(" (read-only)");
            }
            self.write_str("\r\n");
        }
    }

    fn cmd_pkg_install(&mut self, args: &[&str]) {
        let path = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.write_str("Usage: pkg-install <path>\r\n");
                return;
            }
        };
        let node = match zenus_fs::vfs::open(path) {
            Some(n) => n,
            None => {
                self.write_str("pkg-install: file not found: ");
                self.write_str(path);
                self.write_str("\r\n");
                return;
            }
        };
        let stat = node.fs.stat(node.inode);
        let size = stat.size as usize;
        if size == 0 || size > 65536 {
            self.write_str("pkg-install: invalid file size\r\n");
            return;
        }
        let mut buf = alloc::vec![0u8; size];
        if node.fs.read(node.inode, 0, &mut buf).is_none() {
            self.write_str("pkg-install: read failed\r\n");
            return;
        }
        if zenus_fs::pkg::pkg_install(&buf, 0) {
            self.write_str("pkg-install: installed successfully\r\n");
        } else {
            self.write_str("pkg-install: installation failed (invalid .zpk?)\r\n");
        }
    }

    fn cmd_pkg_list(&mut self) {
        let pkgs = zenus_fs::pkg::pkg_list();
        if pkgs.is_empty() {
            self.write_str("No packages installed\r\n");
            return;
        }
        self.write_str("Installed packages:\r\n");
        for pkg in pkgs {
            self.write_str("  ");
            self.write_str(&pkg.name);
            self.write_str(" v");
            self.write_str(&pkg.version);
            self.write_str(" (");
            self.serial.write_u64(pkg.file_count as u64);
            self.write_str(" files)\r\n");
        }
    }

    fn cmd_pkg_remove(&mut self, args: &[&str]) {
        let name = match args.iter().find(|a| !a.is_empty()) {
            Some(n) => n,
            None => {
                self.write_str("Usage: pkg-remove <name>\r\n");
                return;
            }
        };
        if zenus_fs::pkg::pkg_remove(name) {
            self.write_str("Removed: ");
            self.write_str(name);
            self.write_str("\r\n");
        } else {
            self.write_str("Package not found: ");
            self.write_str(name);
            self.write_str("\r\n");
        }
    }

    fn cmd_pkg_info(&mut self, args: &[&str]) {
        let name = match args.iter().find(|a| !a.is_empty()) {
            Some(n) => n,
            None => {
                self.write_str("Usage: pkg-info <name>\r\n");
                return;
            }
        };
        match zenus_fs::pkg::pkg_info(name) {
            Some(info) => {
                self.write_str("Package: ");
                self.write_str(&info.name);
                self.write_str("\r\nVersion: ");
                self.write_str(&info.version);
                self.write_str("\r\nFiles: ");
                self.serial.write_u64(info.file_count as u64);
                self.write_str("\r\n");
                for f in &info.files {
                    self.write_str("  ");
                    self.write_str(f);
                    self.write_str("\r\n");
                }
            }
            None => {
                self.write_str("Package not found: ");
                self.write_str(name);
                self.write_str("\r\n");
            }
        }
    }

    fn cmd_watchdog_pet(&mut self) {
        zenus_arch::watchdog::watchdog_pet();
        self.write_str("Watchdog petted\r\n");
    }

    fn cmd_watchdog_status(&mut self) {
        let active = zenus_arch::watchdog::watchdog_is_active();
        let remaining = zenus_arch::watchdog::watchdog_get_remaining();
        let timeout = zenus_arch::watchdog::watchdog_get_timeout();
        if active {
            self.write_str("Watchdog: ACTIVE\r\n");
        } else {
            self.write_str("Watchdog: INACTIVE\r\n");
        }
        self.write_str("Timeout: ");
        self.serial.write_u64(timeout as u64);
        self.write_str("s\r\nRemaining: ");
        self.serial.write_u64(remaining as u64);
        self.write_str("s\r\n");
    }

    fn cmd_crashdump(&mut self) {
        match zenus_arch::crash::crash_dump_get() {
            Some(dump) => {
                self.write_str("Crash dump available:\r\n");
                zenus_arch::crash::crash_dump_print(dump);
            }
            None => {
                self.write_str("No crash dump recorded\r\n");
            }
        }
    }

    fn cmd_lockdep_status(&mut self) {
        let status = zenus_sync::lockdep::lockdep_status();
        self.write_str("Lockdep status:\r\n");
        self.write_str("  Classes: ");
        self.serial.write_u64(status.class_count as u64);
        self.write_str("\r\n  Edges: ");
        self.serial.write_u64(status.edge_count as u64);
        self.write_str("\r\n  Violations: ");
        self.serial.write_u64(status.violations);
        self.write_str("\r\n");
        if status.class_count > 0 {
            self.write_str("  Lock classes:\r\n");
            for i in 0..status.class_count {
                self.write_str("    [");
                self.serial.write_u64(i as u64);
                self.write_str("] ");
                self.write_str(status.classes[i]);
                self.write_str("\r\n");
            }
        }
        if status.edge_count > 0 {
            self.write_str("  Dependencies:\r\n");
            for i in 0..status.edge_count {
                let (from, to) = status.edges[i];
                self.write_str("    ");
                self.write_str(status.classes[from]);
                self.write_str(" -> ");
                self.write_str(status.classes[to]);
                self.write_str("\r\n");
            }
        }
        if !zenus_sync::lockdep::lockdep_is_enabled() {
            self.write_str("  Lockdep is DISABLED\r\n");
        }
    }

    fn cmd_syslog(&mut self, args: &[&str]) {
        let count = args.first().and_then(|a| a.parse::<usize>().ok()).unwrap_or(20);
        let total = zenus_console::syslog::syslog_get_count();
        if total == 0 {
            self.write_str("(no syslog entries)\r\n");
            return;
        }
        let start = total.saturating_sub(count);
        self.write_str("Syslog (last ");
        self.serial.write_u64(count.min(total) as u64);
        self.write_str(" of ");
        self.serial.write_u64(total as u64);
        self.write_str("):\r\n");
        for i in start..total {
            if let Some(entry) = zenus_console::syslog::syslog_get(i) {
                use core::fmt::Write;
                let mut serial = SerialPort::new(0x3F8);
                let _ = write!(serial, "[{}] {}", entry.level.prefix(), zenus_console::syslog::syslog_msg_str(&entry));
                self.write_str("\r\n");
            }
        }
    }

    fn cmd_ssh_start(&mut self) {
        if zenus_net::ssh::SshServer::is_running() {
            self.write_str("SSH server is already running\r\n");
            return;
        }
        // SSH server is a registered service; start it via init
        if zenus_sched::init::service_start("ssh") {
            self.write_str("SSH server started\r\n");
        } else {
            self.write_str("Failed to start SSH server\r\n");
        }
    }

    fn cmd_ssh_status(&mut self) {
        if zenus_net::ssh::SshServer::is_running() {
            let conns = zenus_net::ssh::SshServer::connection_count();
            self.write_str("SSH server: RUNNING\r\n");
            self.write_str("Connections: ");
            self.serial.write_u64(conns as u64);
            self.write_str("\r\n");
        } else {
            self.write_str("SSH server: STOPPED\r\n");
        }
    }

    fn cmd_ns_info(&mut self) {
        let uts_ns = zenus_sched::scheduler::current_uts_ns();
        let pid_ns = zenus_sched::scheduler::current_pid_ns();
        let mnt_ns = zenus_sched::scheduler::current_mnt_ns();
        let local_pid = zenus_sched::scheduler::current_local_pid();
        let global_pid = zenus_sched::scheduler::current_task_id();
        let hostname = zenus_ns::uts::get_hostname(uts_ns);
        let hlen = hostname.iter().position(|&b| b == 0).unwrap_or(64);
        self.write_str("PID namespace: ");
        self.serial.write_u64(pid_ns as u64);
        self.write_str("\r\n");
        self.write_str("UTS namespace: ");
        self.serial.write_u64(uts_ns as u64);
        self.write_str("\r\n");
        self.write_str("MNT namespace: ");
        self.serial.write_u64(mnt_ns as u64);
        self.write_str("\r\n");
        self.write_str("Global PID:    ");
        self.serial.write_u64(global_pid);
        self.write_str("\r\n");
        self.write_str("Local PID:     ");
        self.serial.write_u64(local_pid);
        self.write_str("\r\n");
        self.write_str("Hostname:      ");
        if hlen > 0 {
            let h = core::str::from_utf8(&hostname[..hlen]).unwrap_or("?");
            self.write_str(h);
        }
        self.write_str("\r\n");
    }

    fn cmd_ns_sethost(&mut self, args: &[&str]) {
        if args.is_empty() || args[0].is_empty() {
            self.write_str("Usage: ns-sethost <hostname>\r\n");
            return;
        }
        let hostname = args[0];
        let uts_ns = zenus_sched::scheduler::current_uts_ns();
        if zenus_ns::uts::set_hostname(uts_ns, hostname.as_bytes()) {
            self.write_str("Hostname set to: ");
            self.write_str(hostname);
            self.write_str("\r\n");
        } else {
            self.write_str("Failed to set hostname\r\n");
        }
    }

    fn cmd_ns_clone(&mut self, args: &[&str]) {
        let mut flags = 0u64;
        if args.contains(&"uts") || args.contains(&"--uts") {
            flags |= zenus_ns::CLONE_NEWUTS;
        }
        if args.contains(&"pid") || args.contains(&"--pid") {
            flags |= zenus_ns::CLONE_NEWPID;
        }
        if args.contains(&"mnt") || args.contains(&"--mnt") {
            flags |= zenus_ns::CLONE_NEWNS;
        }
        self.write_str("Cloning with flags: 0x");
        self.serial.write_hex(flags);
        self.write_str("\r\n");

        let _ = zenus_sched::scheduler::clone_task(
            flags,
            0,
            65536,
            0,
            0,
            0,
            0x6000_0000_0000,
        );
        self.write_str("Clone attempted\r\n");
    }

    fn cmd_chmod(&mut self, args: &[&str]) {
        if args.len() < 2 {
            self.write_str("Usage: chmod <mode> <file>\r\n");
            return;
        }
        let mode_str = args[0];
        let mode = match usize::from_str_radix(mode_str, 8) {
            Ok(m) => m as u16,
            Err(_) => {
                self.write_str("chmod: invalid mode\r\n");
                return;
            }
        };
        let path = args[1];
        match zenus_fs::vfs::open(path) {
            Some(node) => {
                if node.fs.chmod(node.inode, mode) {
                    self.write_str("chmod: ok\r\n");
                } else {
                    self.write_str("chmod: failed\r\n");
                }
            }
            None => {
                self.write_str("chmod: ");
                self.write_str(path);
                self.write_str(": not found\r\n");
            }
        }
    }

    fn serial_write_ip(&mut self, ip: [u8; 4]) {
        self.serial.write_u64(ip[0] as u64);
        self.write_str(".");
        self.serial.write_u64(ip[1] as u64);
        self.write_str(".");
        self.serial.write_u64(ip[2] as u64);
        self.write_str(".");
        self.serial.write_u64(ip[3] as u64);
    }

    fn cmd_journal_init(&mut self) {
        let dev_id = 0;
        let start_block = 3000u64;
        let num_blocks = 16u64;
        if zenus_fs::journal::journal_init(dev_id, start_block, num_blocks) {
            self.write_str("Journal initialized on dev ");
            self.serial.write_u64(dev_id as u64);
            self.write_str(" blocks ");
            self.serial.write_u64(start_block);
            self.write_str("-");
            self.serial.write_u64(start_block + num_blocks - 1);
            self.write_str("\r\n");
        } else {
            self.write_str("Journal init failed\r\n");
        }
    }

    fn cmd_journal_test(&mut self) {
        self.write_str("Journal test:\r\n");
        if !zenus_fs::journal::journal_begin() {
            self.write_str("  [FAIL] journal_begin\r\n");
            return;
        }
        self.write_str("  [ OK ] journal_begin\r\n");

        let test_msg1 = b"JOURNAL TEST BLOCK 0";
        let test_msg2 = b"JOURNAL TEST BLOCK 1";
        let test_msg3 = b"JOURNAL TEST BLOCK 2";
        for (i, msg) in [test_msg1, test_msg2, test_msg3].iter().enumerate() {
            let mut buf = [0u8; 512];
            let len = core::cmp::min(msg.len(), 512);
            buf[..len].copy_from_slice(&msg[..len]);
            if !zenus_fs::journal::journal_write(500 + i as u64, &buf) {
                self.write_str("  [FAIL] journal_write block ");
                self.serial.write_u64(i as u64);
                self.write_str("\r\n");
                return;
            }
            self.write_str("  [ OK ] journal_write block ");
            self.serial.write_u64(500 + i as u64);
            self.write_str("\r\n");
        }

        if !zenus_fs::journal::journal_commit() {
            self.write_str("  [FAIL] journal_commit\r\n");
            return;
        }
        self.write_str("  [ OK ] journal_commit\r\n");

        self.write_str("Replaying journal...\r\n");
        if zenus_fs::journal::journal_replay(0, 3000) {
            self.write_str("  [ OK ] replay (committed entries applied)\r\n");
        } else {
            self.write_str("  [ OK ] replay (no uncommitted entries)\r\n");
        }

        self.write_str("Verifying blocks 500-502...\r\n");
        zenus_fs::block_cache::bc_flush();
        for i in 0..3 {
            let mut buf = [0u8; 512];
            if zenus_fs::block_cache::bc_read(0, 500 + i, &mut buf) {
                let first = buf[0] as u64;
                let second = buf[1] as u64;
                let third = buf[2] as u64;
                self.write_str("  Block ");
                self.serial.write_u64(500 + i);
                self.write_str(": ");
                self.serial.write_u64(first);
                self.write_byte(b',');
                self.serial.write_u64(second);
                self.write_byte(b',');
                self.serial.write_u64(third);
                self.write_str("\r\n");
            } else {
                self.write_str("  Block ");
                self.serial.write_u64(500 + i);
                self.write_str(": read failed\r\n");
            }
        }
        self.write_str("Journal data blocks 3001-3003:\r\n");
        for i in 0..3 {
            let mut buf = [0u8; 512];
            if zenus_fs::block_cache::bc_read(0, 3001 + i, &mut buf) {
                let first = buf[0] as u64;
                self.write_str("  Jnl ");
                self.serial.write_u64(3001 + i);
                self.write_str(": ");
                self.serial.write_u64(first);
                self.write_str("\r\n");
            }
        }
    }

    fn echo_server_poll() {
        zenus_net::socket::poll_all(1);
        let mut state = ECHO_STATE.lock();
        for i in 0..MAX_ECHO_LISTENS {
            if let Some(fd) = state.listen_fds[i] {
                while let Some(client_fd) = zenus_net::socket::accept(fd, 1) {
                    for j in 0..MAX_ECHO_CLIENTS {
                        if state.client_fds[j].is_none() {
                            state.client_fds[j] = Some(client_fd);
                            let s = SerialPort::new(0x3F8);
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
            let fd = match state.client_fds[i] {
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
                state.client_fds[i] = None;
            }
        }
    }

    fn echo_register_listen(fd: usize) -> bool {
        let mut state = ECHO_STATE.lock();
        for i in 0..MAX_ECHO_LISTENS {
            if state.listen_fds[i].is_none() {
                state.listen_fds[i] = Some(fd);
                return true;
            }
        }
        false
    }

    fn cmd_tcp_listen(&mut self, args: &[&str]) {
        let port_str = args.iter().find(|a| !a.is_empty()).unwrap_or(&"7");
        let port: u16 = match port_str.parse() {
            Ok(p) => p,
            Err(_) => {
                self.write_str("Invalid port\r\n");
                return;
            }
        };
        match zenus_net::tcp::listen(port) {
            Some(idx) => {
                self.write_str("Listening on port ");
                self.serial.write_u64(port as u64);
                self.write_str(" (conn ");
                self.serial.write_u64(idx as u64);
                self.write_str(")\r\n");
            }
            None => {
                self.write_str("Failed to listen (table full)\r\n");
            }
        }
    }

    fn cmd_tcp_status(&mut self) {
        self.write_str("TCP connections:\r\n");
        self.write_str("  #   State     Local     Remote    Port\r\n");
        for conn in 0..zenus_net::tcp::MAX_CONNS {
            let name = zenus_net::tcp::state_name(conn);
            if name != "NONE" {
                self.write_str("  ");
                self.serial.write_u64(conn as u64);
                self.write_str("  ");
                self.write_str(name);
                self.write_str("\r\n");
            }
        }
        self.write_str("Total: ");
        self.serial.write_u64(zenus_net::tcp::connection_count() as u64);
        self.write_str("\r\n");
    }

    fn cmd_tcp_send(&mut self, args: &[&str]) {
        if args.len() < 2 {
            self.write_str("Usage: tcp-send <conn> <text>\r\n");
            return;
        }
        let conn: usize = match args[0].parse() {
            Ok(c) => c,
            Err(_) => {
                self.write_str("Invalid connection number\r\n");
                return;
            }
        };
        let text = args[1..].join(" ");
        if !zenus_net::tcp::send_data(conn, text.as_bytes()) {
            self.write_str("Send failed\r\n");
            return;
        }
        if !zenus_net::tcp::flush_tx(conn, 0) {
            self.write_str("Flush failed\r\n");
            return;
        }
        self.write_str("Sent\r\n");
    }

    fn cmd_tcp_echo(&mut self, args: &[&str]) {
        let port: u16 = args.first().and_then(|a| a.parse().ok()).unwrap_or(7);
        self.write_str("Starting echo server on port ");
        self.serial.write_u64(port as u64);
        self.write_str("...\r\n");
        let fd = match zenus_net::socket::socket(
            zenus_net::socket::AF_INET,
            zenus_net::socket::SOCK_STREAM,
            0,
        ) {
            Some(fd) => fd,
            None => {
                self.write_str("Failed to create socket\r\n");
                return;
            }
        };
        if !zenus_net::socket::bind(fd, port) {
            self.write_str("Failed to bind\r\n");
            return;
        }
        if !zenus_net::socket::listen(fd, 1) {
            self.write_str("Failed to listen\r\n");
            return;
        }
        self.write_str("Echo server started on port ");
        self.serial.write_u64(port as u64);
        self.write_str(" (fd ");
        self.serial.write_u64(fd as u64);
        self.write_str(")\r\n");
        if !Self::echo_register_listen(fd) {
            self.write_str("Warning: echo fd table full\r\n");
        }
    }

    fn cmd_tcp_connect(&mut self, args: &[&str]) {
        if args.len() < 1 {
            self.write_str("Usage: tcp-connect <port> [ip]\r\n");
            return;
        }
        let port: u16 = match args[0].parse() {
            Ok(p) => p,
            Err(_) => { self.write_str("Invalid port\r\n"); return; }
        };
        let dst_ip = if args.len() >= 2 {
            let mut ip = [0u8; 4];
            let mut part = 0;
            for octet in args[1].split('.') {
                if part >= 4 { break; }
                ip[part] = match octet.parse() { Ok(n) => n, Err(_) => { self.write_str("Invalid IP\r\n"); return; } };
                part += 1;
            }
            if part != 4 { self.write_str("Invalid IP\r\n"); return; }
            ip
        } else {
            [10, 0, 2, 2]
        };
        let fd = match zenus_net::socket::socket(zenus_net::socket::AF_INET, zenus_net::socket::SOCK_STREAM, 0) {
            Some(fd) => fd,
            None => { self.write_str("Failed to create socket\r\n"); return; }
        };
        self.write_str("Connecting to ");
        self.serial_write_ip(dst_ip);
        self.write_str(":");
        self.serial.write_u64(port as u64);
        self.write_str(" (fd ");
        self.serial.write_u64(fd as u64);
        self.write_str(")...\r\n");
        if zenus_net::socket::connect(fd, 1, dst_ip, port) {
            self.write_str("[ OK ] Connected\r\n");
        } else {
            self.write_str("[FAIL] Connection failed\r\n");
        }
    }

    fn cmd_udp_bind(&mut self, args: &[&str]) {
        if args.len() < 1 {
            self.write_str("Usage: udp-bind <port>\r\n");
            return;
        }
        let port: u16 = match args[0].parse() {
            Ok(p) => p,
            Err(_) => { self.write_str("Invalid port\r\n"); return; }
        };
        let fd = match zenus_net::socket::socket(zenus_net::socket::AF_INET, zenus_net::socket::SOCK_DGRAM, 0) {
            Some(fd) => fd,
            None => { self.write_str("Failed to create socket\r\n"); return; }
        };
        if zenus_net::socket::bind(fd, port) {
            self.write_str("UDP socket bound on port ");
            self.serial.write_u64(port as u64);
            self.write_str(" (fd ");
            self.serial.write_u64(fd as u64);
            self.write_str(")\r\n");
        } else {
            self.write_str("Failed to bind\r\n");
        }
    }

    fn cmd_udp_send(&mut self, args: &[&str]) {
        if args.len() < 3 {
            self.write_str("Usage: udp-send <fd> <ip> <port> <text>\r\n");
            return;
        }
        let fd: usize = match args[0].parse() { Ok(f) => f, Err(_) => { self.write_str("Invalid fd\r\n"); return; } };
        let mut dst_ip = [0u8; 4];
        let mut part = 0;
        for octet in args[1].split('.') {
            if part >= 4 { break; }
            dst_ip[part] = match octet.parse() { Ok(n) => n, Err(_) => { self.write_str("Invalid IP\r\n"); return; } };
            part += 1;
        }
        if part != 4 { self.write_str("Invalid IP\r\n"); return; }
        let port: u16 = match args[2].parse() { Ok(p) => p, Err(_) => { self.write_str("Invalid port\r\n"); return; } };
        let text = args[3..].join(" ");
        if zenus_net::socket::sendto(fd, text.as_bytes(), 1, dst_ip, port) {
            self.write_str("Sent\r\n");
        } else {
            self.write_str("Send failed\r\n");
        }
    }

    fn cmd_udp_recv(&mut self, args: &[&str]) {
        if args.len() < 1 {
            self.write_str("Usage: udp-recv <fd>\r\n");
            return;
        }
        let fd: usize = match args[0].parse() { Ok(f) => f, Err(_) => { self.write_str("Invalid fd\r\n"); return; } };
        let mut buf = [0u8; 1500];
        if let Some(len) = zenus_net::socket::recv(fd, &mut buf) {
            self.write_str("Received: ");
            if let Ok(s) = core::str::from_utf8(&buf[..len]) {
                self.write_str(s);
            } else {
                self.serial.write_hex(len as u64);
                self.write_str(" bytes (non-utf8)\r\n");
            }
        } else {
            self.write_str("No data\r\n");
        }
    }

    fn cmd_readdev(&mut self, args: &[&str]) {
        let path = args.iter().find(|a| !a.is_empty()).unwrap_or(&"/dev/sda");
        let Some(node) = zenus_fs::vfs::open(path) else {
            self.write_str("readdev: device not found\r\n");
            return;
        };
        let mut buf = [0u8; 512];
        match node.fs.read(node.inode, 0, &mut buf) {
            Some(_) => {
                self.write_str("Sector 0:\r\n");
                for i in 0..8 {
                    for j in 0..16 {
                        let val = buf[i * 16 + j];
                        self.serial.write_hex(val as u64);
                        self.write_str(" ");
                    }
                    self.write_str("  |");
                    for j in 0..16 {
                        let c = buf[i * 16 + j];
                        if c >= 32 && c <= 126 {
                            self.write_byte(c);
                        } else {
                            self.write_byte(b'.');
                        }
                    }
                    self.write_str("|\r\n");
                }
                self.write_str("(hexdump of first 128 bytes)\r\n");
            }
            None => self.write_str("readdev: read failed\r\n"),
        }
    }

    fn cmd_ls(&mut self, args: &[&str]) {
        let long = args.iter().any(|a| *a == "-l");
        let path = args.iter().find(|a| !a.is_empty() && **a != "-l").unwrap_or(&"/");
        let path = if path.is_empty() { "/" } else { path };
        let mnt_ns = zenus_sched::scheduler::current_mnt_ns();

        let entries = zenus_fs::vfs::read_dir_in_ns(mnt_ns, path);
        if entries.is_empty() {
            match zenus_fs::vfs::open_in_ns(mnt_ns, path) {
                Some(node) => {
                    let e = node.fs.read_dir(node.inode);
                    if e.is_empty() {
                        self.write_str("(empty)\r\n");
                    } else {
                        for entry in e {
                            if long {
                                let stat = node.fs.stat(entry.inode);
                                self.serial.write_bytes(&zenus_fs::vfs::perm_str(stat.mode));
                                self.write_str(" ");
                                self.serial.write_u64(stat.uid as u64);
                                self.write_str(":");
                                self.serial.write_u64(stat.gid as u64);
                                self.write_str(" ");
                                self.serial.write_u64(stat.size);
                                self.write_str(" ");
                            }
                            self.serial_write_dirent(&entry.name, entry.file_type);
                        }
                        self.write_str("\r\n");
                    }
                }
                None => {
                    self.write_str("ls: ");
                    self.write_str(path);
                    self.write_str(": not found\r\n");
                }
            }
            return;
        }
        for entry in entries {
            if long {
                let node = match zenus_fs::vfs::open_in_ns(mnt_ns, path) {
                    Some(n) => n,
                    None => continue,
                };
                let stat = node.fs.stat(entry.inode);
                self.serial.write_bytes(&zenus_fs::vfs::perm_str(stat.mode));
                self.write_str(" ");
                self.serial.write_u64(stat.uid as u64);
                self.write_str(":");
                self.serial.write_u64(stat.gid as u64);
                self.write_str(" ");
                self.serial.write_u64(stat.size);
                self.write_str(" ");
            }
            self.serial_write_dirent(&entry.name, entry.file_type);
        }
        self.write_str("\r\n");
    }

    fn serial_write_dirent(&mut self, name: &str, file_type: zenus_fs::vfs::FileType) {

        self.write_str(name);
        if file_type == zenus_fs::vfs::FileType::Directory {
            self.write_str("/");
        }
        self.write_str("  ");
    }

    fn cmd_cat(&mut self, args: &[&str]) {
        let path = match args.iter().find(|a| !a.is_empty()) {
            Some(p) => p,
            None => {
                self.write_str("cat: missing operand\r\n");
                return;
            }
        };
        let mnt_ns = zenus_sched::scheduler::current_mnt_ns();

        match zenus_fs::vfs::open_in_ns(mnt_ns, path) {
            Some(node) => {
                let stat = node.fs.stat(node.inode);
                if stat.file_type == zenus_fs::vfs::FileType::Directory {
                    self.write_str("cat: ");
                    self.write_str(path);
                    self.write_str(": Is a directory\r\n");
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
                                    self.write_byte(b'\r');
                                }
                                self.write_byte(b);
                                last_byte = b;
                            }
                            offset += n as u64;
                        }
                    }
                }
                if offset > 0 && last_byte != b'\n' {
                    self.write_str("\r\n");
                }
            }
            None => {
                self.write_str("cat: ");
                self.write_str(path);
                self.write_str(": not found\r\n");
            }
        }
    }
}

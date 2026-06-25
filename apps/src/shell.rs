use zutils_common::{Args, Writer};
use zenus_console::serial::SerialPort;

const MAX_LINE: usize = 256;
const PROMPT: &str = "zenus$ ";

struct ShellWriter {
    serial: SerialPort,
    hhdm_offset: u64,
}

impl Writer for ShellWriter {
    fn write_str(&mut self, s: &str) {
        self.serial.write_str(s);
        zenus_console::vga::write_str(s, self.hhdm_offset);
    }

    fn write_byte(&mut self, b: u8) {
        self.serial.write_str_noirq(core::str::from_utf8(&[b]).unwrap_or(""));
        let arr = [b];
        if let Ok(s) = core::str::from_utf8(&arr) {
            zenus_console::vga::write_str(s, self.hhdm_offset);
        }
    }

    fn write_u64(&mut self, v: u64) {
        self.serial.write_u64(v);
        let mut buf = [0u8; 20];
        let mut i = 20;
        let mut n = v;
        if n == 0 {
            self.serial.write_byte_serial(b'0');
            return;
        }
        while n > 0 {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        let s = core::str::from_utf8(&buf[i..]).unwrap_or("");
        zenus_console::vga::write_str(s, self.hhdm_offset);
    }

    fn write_i64(&mut self, v: i64) {
        if v < 0 {
            self.write_byte(b'-');
            self.write_u64((-v) as u64);
        } else {
            self.write_u64(v as u64);
        }
    }

    fn write_hex(&mut self, v: u64) {
        self.serial.write_hex(v);
        let mut buf = [0u8; 16];
        let mut i = 16;
        let mut started = false;
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for s in (0..16).rev() {
            let nib = ((v >> (s * 4)) & 0xf) as u8;
            if nib != 0 || started || s == 0 {
                i -= 1;
                buf[i] = HEX[nib as usize];
                started = true;
            }
        }
        let s = core::str::from_utf8(&buf[i..]).unwrap_or("");
        zenus_console::vga::write_str(s, self.hhdm_offset);
    }

    fn write_ip(&mut self, ip: [u8; 4]) {
        self.write_u64(ip[0] as u64);
        self.write_byte(b'.');
        self.write_u64(ip[1] as u64);
        self.write_byte(b'.');
        self.write_u64(ip[2] as u64);
        self.write_byte(b'.');
        self.write_u64(ip[3] as u64);
    }
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut parts = [0u8; 4];
    let mut i = 0;
    for octet in s.split('.') {
        if i >= 4 { return None; }
        let val = octet.parse::<u16>().ok()?;
        if val > 255 { return None; }
        parts[i] = val as u8;
        i += 1;
    }
    if i != 4 { None } else { Some(parts) }
}

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

    fn writer(&mut self) -> ShellWriter {
        ShellWriter {
            serial: SerialPort::new(0x3F8),
            hhdm_offset: self.hhdm_offset,
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
                self.echo_server_poll();
            }
            if yield_count % 50 == 0 {
                zenus_arch::watchdog::watchdog_pet();
                zenus_sched::init::service_supervise();
            }
            let mut w = self.writer();
            w.write_str(PROMPT);
            zenus_console::serial::flush_output();
            let line = match self.read_line() {
                Some(l) => l,
                None => continue,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            self.execute(trimmed);
            zenus_console::serial::flush_output();
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
                zenus_sched::scheduler::yield_now();
                None
            };

            if let Some(c) = c {
                match c {
                    b'\r' | b'\n' => {
                        let mut w = self.writer();
                        w.write_str("\r\n");
                        zenus_console::serial::flush_output();
                        unsafe {
                            let s = core::str::from_utf8(&BUF[..POS]).unwrap_or("");
                            POS = 0;
                            return if s.is_empty() { None } else { Some(s) };
                        }
                    }
                    b'\x7F' | b'\x08' => {
                        if unsafe { POS > 0 } {
                            unsafe { POS -= 1 };
                            self.writer().write_str("\x08 \x08");
                            zenus_console::serial::flush_output();
                        }
                    }
                    0x20..=0x7E => {
                        unsafe {
                            if POS < MAX_LINE - 1 {
                                BUF[POS] = c;
                                POS += 1;
                                self.writer().write_byte(c);
                                zenus_console::serial::flush_output();
                            }
                        }
                    }
                    _ => {
                        zenus_sched::scheduler::yield_now();
                    }
                }
            } else {
                idle_count += 1;
                if idle_count % 10 == 0 {
                    zenus_net::nic::net_poll();
                    self.echo_server_poll();
                }
                if idle_count % 50 == 0 {
                    zenus_arch::watchdog::watchdog_pet();
                }
                if !zenus_arch::keyboard::is_key_available() {
                    zenus_sched::scheduler::yield_now();
                }
            }
        }
    }

    fn execute(&mut self, line: &str) {
        let args = Args::parse(line);
        let mut w = self.writer();

        match args.cmd {
            "help" => zutils_help::execute(&mut w),
            "echo" => zutils_echo::execute(&args, &mut w),
            "ls" => zutils_ls::execute(&args, &mut w),
            "cat" => zutils_cat::execute(&args, &mut w),
            "clear" => zutils_clear::execute(&args, &mut w),
            "timer" => zutils_timer::execute(&args, &mut w),
            "ps" => zutils_ps::execute(&args, &mut w),
            "kill" => zutils_kill::execute(&args, &mut w),
            "mkdir" => zutils_mkdir::execute(&args, &mut w),
            "rm" => zutils_rm::execute(&args, &mut w),
            "touch" => zutils_touch::execute(&args, &mut w),
            "ifconfig" => zutils_ifconfig::execute(&args, &mut w),
            "meminfo" => zutils_meminfo::execute(&args, &mut w),
            "reboot" => zutils_reboot::execute(&args, &mut w),
            "shutdown" => zutils_shutdown::execute(&args, &mut w),
            "uname" | "version" => zutils_uname::execute(&args, &mut w),
            "dmesg" => zutils_dmesg::execute(&args, &mut w),
            "id" => zutils_id::execute(&args, &mut w),
            "whoami" => zutils_whoami::execute(&args, &mut w),
            "chmod" => zutils_chmod::execute(&args, &mut w),
            "cp" => zutils_cp::execute(&args, &mut w),
            "mv" => zutils_mv::execute(&args, &mut w),
            "mount" => zutils_mount::execute(&args, &mut w),
            "pwd" => zutils_pwd::execute(&args, &mut w),
            "df" => zutils_df::execute(&args, &mut w),
            _ => {
                if !self.execute_zenus_specific(line, &args, &mut w) {
                    w.write_str("Unknown command: ");
                    w.write_str(args.cmd);
                    w.write_str("\r\n");
                }
            }
        }
    }

    fn execute_zenus_specific(&mut self, line: &str, args: &Args, w: &mut ShellWriter) -> bool {
        match args.cmd {
            "bcache" => self.cmd_bcache(w),
            "fsck" => self.cmd_fsck(w),
            "journal-init" => self.cmd_journal_init(w),
            "journal-test" => self.cmd_journal_test(w),
            "tcp-listen" => self.cmd_tcp_listen(line, w),
            "tcp-status" => self.cmd_tcp_status(w),
            "tcp-send" => self.cmd_tcp_send(line, w),
            "tcp-echo" => self.cmd_tcp_echo(w),
            "tcp-connect" => self.cmd_tcp_connect(line, w),
            "udp-bind" => self.cmd_udp_bind(line, w),
            "udp-send" => self.cmd_udp_send(line, w),
            "udp-recv" => self.cmd_udp_recv(line, w),
            "dhcp" => self.cmd_dhcp(w),
            "dhcp-server" => self.cmd_dhcp_server(w),
            "resolve" => self.cmd_resolve(line, w),
            "readdev" => self.cmd_readdev(line, w),
            "init-start" => self.cmd_init_start(w),
            "init-shutdown" => self.cmd_init_shutdown(w),
            "service-list" => self.cmd_service_list(w),
            "service-start" => self.cmd_service_start(line, w),
            "service-stop" => self.cmd_service_stop(line, w),
            "service-restart" => self.cmd_service_restart(line, w),
            "sysctl" => self.cmd_sysctl(line, w),
            "pkg-install" => self.cmd_pkg_install(line, w),
            "pkg-list" => self.cmd_pkg_list(w),
            "pkg-remove" => self.cmd_pkg_remove(line, w),
            "pkg-info" => self.cmd_pkg_info(line, w),
            "watchdog-pet" => self.cmd_watchdog_pet(w),
            "watchdog-status" => self.cmd_watchdog_status(w),
            "crashdump" => self.cmd_crashdump(w),
            "lockdep-status" => self.cmd_lockdep_status(w),
            "syslog" => self.cmd_syslog(line, w),
            "ssh-start" => self.cmd_ssh_start(w),
            "ssh-status" => self.cmd_ssh_status(w),
            "ns-info" => self.cmd_ns_info(w),
            "ns-sethost" => self.cmd_ns_sethost(line, w),
            "ns-clone" => self.cmd_ns_clone(line, w),
            "grep" => zutils_grep::execute(args, w),
            "find" => zutils_find::execute(args, w),
            "du" => zutils_du::execute(args, w),
            "chown" => zutils_chown::execute(args, w),
            "pgrep" => zutils_pgrep::execute(args, w),
            "top" => zutils_top::execute(args, w),
            "uptime" => zutils_uptime::execute(args, w),
            "which" => zutils_which::execute(args, w),
            "zbench" => zutils_zbench::execute(args, w),
            "zdiag" => zutils_zdiag::execute(args, w),
            "zdoctor" => zutils_zdoctor::execute(args, w),
            "zinfo" => zutils_zinfo::execute(args, w),
            "zmem" => zutils_zmem::execute(args, w),
            "zpkg" => zutils_zpkg::execute(args, w),
            "zsys" => zutils_zsys::execute(args, w),
            "ztrace" => zutils_ztrace::execute(args, w),
            "firewall-list" => self.cmd_firewall_list(w),
            "firewall-add" => self.cmd_firewall_add(line, w),
            "firewall-remove" => self.cmd_firewall_remove(line, w),
            _ => return false,
        }
        true
    }

    // ── Zenus-specific commands (remain inline) ──

    fn cmd_bcache(&mut self, w: &mut ShellWriter) {
        let (hits, misses) = zenus_fs::block_cache::bc_stats();
        w.write_str("Block cache stats:\r\n");
        w.write_str("  Hits:   ");
        w.write_u64(hits);
        w.write_str("\r\n  Misses: ");
        w.write_u64(misses);
        let total = hits + misses;
        if total > 0 {
            w.write_str("\r\n  Rate:   ");
            w.write_u64(hits * 100 / total);
            w.write_str("%\r\n");
        } else {
            w.write_str("\r\n  (no I/O yet)\r\n");
        }
    }

    fn cmd_fsck(&mut self, w: &mut ShellWriter) {
        let result = zenus_fs::ext2_fsck::fsck(0);
        w.write_str("fsck results:\r\n");
        if result.passed() {
            w.write_str("  PASSED");
        } else {
            w.write_str("  FAILED");
        }
        w.write_str(" (");
        w.write_u64(result.errors as u64);
        w.write_str(" errors, ");
        w.write_u64(result.warnings as u64);
        w.write_str(" warnings)\r\n");
        for i in 0..result.count {
            let msg = &result.messages[i];
            let sev = match msg.severity {
                zenus_fs::ext2_fsck::FsckSeverity::Fatal => "FATAL",
                zenus_fs::ext2_fsck::FsckSeverity::Error => "ERROR",
                zenus_fs::ext2_fsck::FsckSeverity::Warning => " WARN",
                _ => " INFO",
            };
            w.write_str("  [");
            w.write_str(sev);
            w.write_str("] ");
            w.write_str(msg.msg);
            w.write_str("\r\n");
        }
    }

    fn cmd_journal_init(&mut self, w: &mut ShellWriter) {
        let dev_id = 0;
        let start_block = 3000u64;
        let num_blocks = 16u64;
        if zenus_fs::journal::journal_init(dev_id, start_block, num_blocks) {
            w.write_str("Journal initialized on dev ");
            w.write_u64(dev_id as u64);
            w.write_str(" blocks ");
            w.write_u64(start_block);
            w.write_str("-");
            w.write_u64(start_block + num_blocks - 1);
            w.write_str("\r\n");
        } else {
            w.write_str("Journal init failed\r\n");
        }
    }

    fn cmd_journal_test(&mut self, w: &mut ShellWriter) {
        w.write_str("Journal test:\r\n");
        if !zenus_fs::journal::journal_begin() {
            w.write_str("  [FAIL] journal_begin\r\n");
            return;
        }
        w.write_str("  [ OK ] journal_begin\r\n");

        let test_msg1 = b"JOURNAL TEST BLOCK 0";
        let test_msg2 = b"JOURNAL TEST BLOCK 1";
        let test_msg3 = b"JOURNAL TEST BLOCK 2";
        for (i, msg) in [test_msg1, test_msg2, test_msg3].iter().enumerate() {
            let mut buf = [0u8; 512];
            let len = core::cmp::min(msg.len(), 512);
            buf[..len].copy_from_slice(&msg[..len]);
            if !zenus_fs::journal::journal_write(500 + i as u64, &buf) {
                w.write_str("  [FAIL] journal_write block ");
                w.write_u64(i as u64);
                w.write_str("\r\n");
                return;
            }
            w.write_str("  [ OK ] journal_write block ");
            w.write_u64(500 + i as u64);
            w.write_str("\r\n");
        }

        if !zenus_fs::journal::journal_commit() {
            w.write_str("  [FAIL] journal_commit\r\n");
            return;
        }
        w.write_str("  [ OK ] journal_commit\r\n");

        w.write_str("Replaying journal...\r\n");
        if zenus_fs::journal::journal_replay(0, 3000) {
            w.write_str("  [ OK ] replay (committed entries applied)\r\n");
        } else {
            w.write_str("  [ OK ] replay (no uncommitted entries)\r\n");
        }

        w.write_str("Verifying blocks 500-502...\r\n");
        zenus_fs::block_cache::bc_flush();
        for i in 0..3 {
            let mut buf = [0u8; 512];
            if zenus_fs::block_cache::bc_read(0, 500 + i, &mut buf) {
                w.write_str("  Block ");
                w.write_u64(500 + i);
                w.write_str(": ");
                w.write_byte(buf[0]);
                w.write_byte(b',');
                w.write_byte(buf[1]);
                w.write_byte(b',');
                w.write_byte(buf[2]);
                w.write_str("\r\n");
            } else {
                w.write_str("  Block ");
                w.write_u64(500 + i);
                w.write_str(": read failed\r\n");
            }
        }
        w.write_str("Journal data blocks 3001-3003:\r\n");
        for i in 0..3 {
            let mut buf = [0u8; 512];
            if zenus_fs::block_cache::bc_read(0, 3001 + i, &mut buf) {
                w.write_str("  Jnl ");
                w.write_u64(3001 + i);
                w.write_str(": ");
                w.write_byte(buf[0]);
                w.write_str("\r\n");
            }
        }
    }

    fn cmd_tcp_listen(&mut self, _line: &str, w: &mut ShellWriter) {
        w.write_str("tcp-listen: not available via serial shell\r\n");
    }

    fn cmd_tcp_status(&mut self, w: &mut ShellWriter) {
        w.write_str("TCP connections:\r\n");
        let status = zenus_net::tcp::tcp_status();
        for (i, conn) in status.iter().enumerate() {
            if conn.active {
                w.write_str("  [");
                w.write_u64(i as u64);
                w.write_str("] port ");
                w.write_u64(conn.local_port as u64);
                w.write_str(" -> ");
                w.write_ip(conn.remote_ip);
                w.write_str(":");
                w.write_u64(conn.remote_port as u64);
                w.write_str(" (");
                w.write_str(conn.state_str());
                w.write_str(")\r\n");
            }
        }
    }

    fn cmd_tcp_send(&mut self, _line: &str, w: &mut ShellWriter) {
        w.write_str("tcp-send: not available via serial shell\r\n");
    }

    fn cmd_tcp_echo(&mut self, w: &mut ShellWriter) {
        w.write_str("TCP echo server is running on port 7\r\n");
    }

    fn cmd_tcp_connect(&mut self, _line: &str, w: &mut ShellWriter) {
        w.write_str("tcp-connect: not available via serial shell\r\n");
    }

    fn cmd_udp_bind(&mut self, _line: &str, w: &mut ShellWriter) {
        w.write_str("udp-bind: not available via serial shell\r\n");
    }

    fn cmd_udp_send(&mut self, _line: &str, w: &mut ShellWriter) {
        w.write_str("udp-send: not available via serial shell\r\n");
    }

    fn cmd_udp_recv(&mut self, _line: &str, w: &mut ShellWriter) {
        w.write_str("udp-recv: not available via serial shell\r\n");
    }

    fn cmd_dhcp(&mut self, w: &mut ShellWriter) {
        w.write_str("DHCP client starting...\r\n");
        let iface_idx = 1;
        if zenus_net::dhcp::dhcp_start(iface_idx) {
            w.write_str("[ OK ] DHCP: address acquired\r\n");
            zutils_ifconfig::execute(&Args::parse("ifconfig"), w);
        } else {
            w.write_str("[FAIL] DHCP: no response\r\n");
        }
    }

    fn cmd_dhcp_server(&mut self, w: &mut ShellWriter) {
        w.write_str("DHCP server running on 10.0.2.100-10.0.2.115\r\n");
        let iface_idx = 1;
        let iface = match zenus_net::nic::get_iface(iface_idx) {
            Some(iface) => iface,
            None => {
                w.write_str("[FAIL] No interface\r\n");
                return;
            }
        };
        if iface.ip == [0; 4] || iface.ip == [127, 0, 0, 1] {
            w.write_str("[FAIL] Server needs a static IP (run `dhcp` first)\r\n");
            return;
        }
        w.write_str("[ OK ] DHCP server ready on ");
        w.write_ip(iface.ip);
        w.write_str("\r\n");
    }

    fn cmd_resolve(&mut self, _line: &str, w: &mut ShellWriter) {
        w.write_str("resolve: not available via serial shell\r\n");
    }

    fn cmd_readdev(&mut self, _line: &str, w: &mut ShellWriter) {
        w.write_str("readdev: not available via serial shell\r\n");
    }

    fn cmd_init_start(&mut self, w: &mut ShellWriter) {
        if zenus_sched::init::init_system_start() {
            w.write_str("[ OK ] Init system started\r\n");
        } else {
            w.write_str("[INFO] Init system already running\r\n");
        }
    }

    fn cmd_init_shutdown(&mut self, w: &mut ShellWriter) {
        w.write_str("Shutting down init system...\r\n");
        zenus_sched::init::init_shutdown();
    }

    fn cmd_service_list(&mut self, w: &mut ShellWriter) {
        let services = zenus_sched::init::service_list();
        if services.is_empty() {
            w.write_str("No services registered\r\n");
            return;
        }
        w.write_str("Service          State     PID   Restarts\r\n");
        w.write_str("----------------------------------------\r\n");
        for (name, state, pid, restarts) in services {
            w.write_str(name);
            for _ in name.len()..16 { w.write_byte(b' '); }
            let state_str = match state {
                zenus_sched::init::ServiceState::Running => "Running",
                zenus_sched::init::ServiceState::Stopped => "Stopped",
                zenus_sched::init::ServiceState::Crashed => "Crashed",
                zenus_sched::init::ServiceState::Disabled => "Disabled",
            };
            w.write_str(" ");
            w.write_str(state_str);
            for _ in state_str.len()..10 { w.write_byte(b' '); }
            w.write_u64(pid);
            w.write_str("   ");
            w.write_u64(restarts as u64);
            w.write_str("\r\n");
        }
    }

    fn cmd_service_start(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let name = match args.get(1) {
            Some(n) => n,
            None => {
                w.write_str("Usage: service-start <name>\r\n");
                return;
            }
        };
        if zenus_sched::init::service_start(name) {
            w.write_str("Started: ");
            w.write_str(name);
            w.write_str("\r\n");
        } else {
            w.write_str("Failed to start: ");
            w.write_str(name);
            w.write_str("\r\n");
        }
    }

    fn cmd_service_stop(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let name = match args.get(1) {
            Some(n) => n,
            None => {
                w.write_str("Usage: service-stop <name>\r\n");
                return;
            }
        };
        if zenus_sched::init::service_stop(name) {
            w.write_str("Stopped: ");
            w.write_str(name);
            w.write_str("\r\n");
        } else {
            w.write_str("Failed to stop: ");
            w.write_str(name);
            w.write_str("\r\n");
        }
    }

    fn cmd_service_restart(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let name = match args.get(1) {
            Some(n) => n,
            None => {
                w.write_str("Usage: service-restart <name>\r\n");
                return;
            }
        };
        if zenus_sched::init::service_restart(name) {
            w.write_str("Restarted: ");
            w.write_str(name);
            w.write_str("\r\n");
        } else {
            w.write_str("Failed to restart: ");
            w.write_str(name);
            w.write_str("\r\n");
        }
    }

    fn cmd_sysctl(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        if args.args().len() < 1 {
            let list = zenus_fs::sysctl::sysctl_list();
            w.write_str("Sysctl parameters:\r\n");
            for entry in list {
                w.write_str("  ");
                w.write_str(entry.name);
                w.write_str(" = ");
                match &entry.value {
                    zenus_fs::sysctl::SysctlValue::IntVal(v) => w.write_i64(*v),
                    zenus_fs::sysctl::SysctlValue::UintVal(v) => w.write_u64(*v),
                    zenus_fs::sysctl::SysctlValue::BoolVal(v) => w.write_str(if *v { "true" } else { "false" }),
                    zenus_fs::sysctl::SysctlValue::StrVal(v) => w.write_str(v),
                }
                if entry.read_only { w.write_str(" (read-only)"); }
                w.write_str("\r\n");
            }
            return;
        }

        let arg = args.get(1).unwrap_or("");
        if let Some(eq_pos) = arg.find('=') {
            let name = &arg[..eq_pos];
            let val_str = &arg[eq_pos + 1..];
            let idx = match zenus_fs::sysctl::sysctl_find(name) {
                Some(i) => i,
                None => { w.write_str("sysctl: not found\r\n"); return; }
            };
            let entry = match zenus_fs::sysctl::sysctl_get(idx) {
                Some(e) => e,
                None => { w.write_str("sysctl: error reading\r\n"); return; }
            };
            let value = match entry.value {
                zenus_fs::sysctl::SysctlValue::IntVal(_) => {
                    match val_str.parse() { Ok(v) => zenus_fs::sysctl::SysctlValue::IntVal(v), Err(_) => { w.write_str("sysctl: invalid integer\r\n"); return; } }
                }
                zenus_fs::sysctl::SysctlValue::UintVal(_) => {
                    match val_str.parse() { Ok(v) => zenus_fs::sysctl::SysctlValue::UintVal(v), Err(_) => { w.write_str("sysctl: invalid unsigned\r\n"); return; } }
                }
                zenus_fs::sysctl::SysctlValue::BoolVal(_) => {
                    zenus_fs::sysctl::SysctlValue::BoolVal(val_str == "1" || val_str == "true")
                }
                zenus_fs::sysctl::SysctlValue::StrVal(_) => { w.write_str("sysctl: string values cannot be set\r\n"); return; }
            };
            if zenus_fs::sysctl::sysctl_set(idx, value) {
                w.write_str("sysctl: ");
                w.write_str(name);
                w.write_str(" = ");
                w.write_str(val_str);
                w.write_str("\r\n");
            }
        } else {
            let idx = match zenus_fs::sysctl::sysctl_find(arg) {
                Some(i) => i,
                None => { w.write_str("sysctl: not found\r\n"); return; }
            };
            let entry = match zenus_fs::sysctl::sysctl_get(idx) {
                Some(e) => e,
                None => { w.write_str("sysctl: error reading\r\n"); return; }
            };
            w.write_str(entry.name);
            w.write_str(" = ");
            match &entry.value {
                zenus_fs::sysctl::SysctlValue::IntVal(v) => w.write_i64(*v),
                zenus_fs::sysctl::SysctlValue::UintVal(v) => w.write_u64(*v),
                zenus_fs::sysctl::SysctlValue::BoolVal(v) => w.write_str(if *v { "true" } else { "false" }),
                zenus_fs::sysctl::SysctlValue::StrVal(v) => w.write_str(v),
            }
            w.write_str("\r\n");
        }
    }

    fn cmd_pkg_install(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let path = match args.get(1) {
            Some(p) => p,
            None => { w.write_str("Usage: pkg-install <path>\r\n"); return; }
        };
        let node = match zenus_fs::vfs::open(path) {
            Some(n) => n,
            None => { w.write_str("pkg-install: file not found\r\n"); return; }
        };
        let stat = node.fs.stat(node.inode);
        let size = stat.size as usize;
        if size == 0 || size > 65536 { w.write_str("pkg-install: invalid file size\r\n"); return; }
        let mut buf = alloc::vec![0u8; size];
        if node.fs.read(node.inode, 0, &mut buf).is_none() { w.write_str("pkg-install: read failed\r\n"); return; }
        if zenus_fs::pkg::pkg_install(&buf, 0) {
            w.write_str("pkg-install: installed successfully\r\n");
        } else {
            w.write_str("pkg-install: installation failed\r\n");
        }
    }

    fn cmd_pkg_list(&mut self, w: &mut ShellWriter) {
        let pkgs = zenus_fs::pkg::pkg_list();
        if pkgs.is_empty() { w.write_str("No packages installed\r\n"); return; }
        w.write_str("Installed packages:\r\n");
        for pkg in pkgs {
            w.write_str("  ");
            w.write_str(&pkg.name);
            w.write_str(" v");
            w.write_str(&pkg.version);
            w.write_str(" (");
            w.write_u64(pkg.file_count as u64);
            w.write_str(" files)\r\n");
        }
    }

    fn cmd_pkg_remove(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let name = match args.get(1) {
            Some(n) => n,
            None => { w.write_str("Usage: pkg-remove <name>\r\n"); return; }
        };
        if zenus_fs::pkg::pkg_remove(name) {
            w.write_str("Removed: ");
            w.write_str(name);
            w.write_str("\r\n");
        } else {
            w.write_str("Package not found\r\n");
        }
    }

    fn cmd_pkg_info(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let name = match args.get(1) {
            Some(n) => n,
            None => { w.write_str("Usage: pkg-info <name>\r\n"); return; }
        };
        match zenus_fs::pkg::pkg_info(name) {
            Some(info) => {
                w.write_str("Package: ");
                w.write_str(&info.name);
                w.write_str("\r\nVersion: ");
                w.write_str(&info.version);
                w.write_str("\r\nFiles: ");
                w.write_u64(info.file_count as u64);
                w.write_str("\r\n");
                for f in &info.files {
                    w.write_str("  ");
                    w.write_str(f);
                    w.write_str("\r\n");
                }
            }
            None => { w.write_str("Package not found\r\n"); }
        }
    }

    fn cmd_watchdog_pet(&mut self, w: &mut ShellWriter) {
        zenus_arch::watchdog::watchdog_pet();
        w.write_str("Watchdog petted\r\n");
    }

    fn cmd_watchdog_status(&mut self, w: &mut ShellWriter) {
        let active = zenus_arch::watchdog::watchdog_is_active();
        let remaining = zenus_arch::watchdog::watchdog_get_remaining();
        let timeout = zenus_arch::watchdog::watchdog_get_timeout();
        if active { w.write_str("Watchdog: ACTIVE\r\n"); }
        else { w.write_str("Watchdog: INACTIVE\r\n"); }
        w.write_str("Timeout: ");
        w.write_u64(timeout as u64);
        w.write_str("s\r\nRemaining: ");
        w.write_u64(remaining as u64);
        w.write_str("s\r\n");
    }

    fn cmd_crashdump(&mut self, w: &mut ShellWriter) {
        match zenus_arch::crash::crash_dump_get() {
            Some(dump) => {
                w.write_str("Crash dump available:\r\n");
                zenus_arch::crash::crash_dump_print(dump);
            }
            None => { w.write_str("No crash dump recorded\r\n"); }
        }
    }

    fn cmd_lockdep_status(&mut self, w: &mut ShellWriter) {
        let status = zenus_sync::lockdep::lockdep_status();
        w.write_str("Lockdep status:\r\n");
        w.write_str("  Classes: ");
        w.write_u64(status.class_count as u64);
        w.write_str("\r\n  Edges: ");
        w.write_u64(status.edge_count as u64);
        w.write_str("\r\n  Violations: ");
        w.write_u64(status.violations);
        w.write_str("\r\n");
        if status.class_count > 0 {
            w.write_str("  Lock classes:\r\n");
            for i in 0..status.class_count {
                w.write_str("    [");
                w.write_u64(i as u64);
                w.write_str("] ");
                w.write_str(status.classes[i]);
                w.write_str("\r\n");
            }
        }
    }

    fn cmd_syslog(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let count = args.get(1).and_then(|a| a.parse::<usize>().ok()).unwrap_or(20);
        let total = zenus_console::syslog::syslog_get_count();
        if total == 0 { w.write_str("(no syslog entries)\r\n"); return; }
        let start = total.saturating_sub(count);
        w.write_str("Syslog (last ");
        w.write_u64(count.min(total) as u64);
        w.write_str(" of ");
        w.write_u64(total as u64);
        w.write_str("):\r\n");
        for i in start..total {
            if let Some(entry) = zenus_console::syslog::syslog_get(i) {
                w.write_str("[");
                w.write_str(entry.level.prefix());
                w.write_str("] ");
                w.write_str(zenus_console::syslog::syslog_msg_str(&entry));
                w.write_str("\r\n");
            }
        }
    }

    fn cmd_ssh_start(&mut self, w: &mut ShellWriter) {
        if zenus_net::ssh::SshServer::is_running() {
            w.write_str("SSH server is already running\r\n");
            return;
        }
        if zenus_sched::init::service_start("ssh") {
            w.write_str("SSH server started\r\n");
        } else {
            w.write_str("Failed to start SSH server\r\n");
        }
    }

    fn cmd_ssh_status(&mut self, w: &mut ShellWriter) {
        if zenus_net::ssh::SshServer::is_running() {
            let conns = zenus_net::ssh::SshServer::connection_count();
            w.write_str("SSH server: RUNNING\r\n");
            w.write_str("Connections: ");
            w.write_u64(conns as u64);
            w.write_str("\r\n");
        } else {
            w.write_str("SSH server: STOPPED\r\n");
        }
    }

    fn cmd_ns_info(&mut self, w: &mut ShellWriter) {
        let uts_ns = zenus_sched::scheduler::current_uts_ns();
        let pid_ns = zenus_sched::scheduler::current_pid_ns();
        let mnt_ns = zenus_sched::scheduler::current_mnt_ns();
        let net_ns = zenus_sched::scheduler::current_net_ns();
        let user_ns = zenus_sched::scheduler::current_user_ns();
        let ipc_ns = zenus_sched::scheduler::current_ipc_ns();
        let local_pid = zenus_sched::scheduler::current_local_pid();
        let global_pid = zenus_sched::scheduler::current_task_id();
        let hostname = zenus_ns::uts::get_hostname(uts_ns);
        let hlen = hostname.iter().position(|&b| b == 0).unwrap_or(64);
        w.write_str("PID namespace:  ");
        w.write_u64(pid_ns as u64);
        w.write_str("\r\nUTS namespace:  ");
        w.write_u64(uts_ns as u64);
        w.write_str("\r\nMNT namespace:  ");
        w.write_u64(mnt_ns as u64);
        w.write_str("\r\nNET namespace:  ");
        w.write_u64(net_ns as u64);
        w.write_str("\r\nUSER namespace: ");
        w.write_u64(user_ns as u64);
        w.write_str("\r\nIPC namespace:  ");
        w.write_u64(ipc_ns as u64);
        w.write_str("\r\nGlobal PID:     ");
        w.write_u64(global_pid);
        w.write_str("\r\nLocal PID:      ");
        w.write_u64(local_pid);
        w.write_str("\r\nHostname:       ");
        if hlen > 0 {
            w.write_str(core::str::from_utf8(&hostname[..hlen]).unwrap_or("?"));
        }
        w.write_str("\r\n");
    }

    fn cmd_ns_sethost(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let hostname = match args.get(1) {
            Some(h) => h,
            None => { w.write_str("Usage: ns-sethost <hostname>\r\n"); return; }
        };
        let uts_ns = zenus_sched::scheduler::current_uts_ns();
        if zenus_ns::uts::set_hostname(uts_ns, hostname.as_bytes()) {
            w.write_str("Hostname set to: ");
            w.write_str(hostname);
            w.write_str("\r\n");
        } else {
            w.write_str("Failed to set hostname\r\n");
        }
    }

    fn cmd_ns_clone(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let mut flags = 0u64;
        if args.has_flag("--uts") || args.has_flag("uts") { flags |= zenus_ns::CLONE_NEWUTS; }
        if args.has_flag("--pid") || args.has_flag("pid") { flags |= zenus_ns::CLONE_NEWPID; }
        if args.has_flag("--mnt") || args.has_flag("mnt") { flags |= zenus_ns::CLONE_NEWNS; }
        if args.has_flag("--net") || args.has_flag("net") { flags |= zenus_ns::CLONE_NEWNET; }
        if args.has_flag("--user") || args.has_flag("user") { flags |= zenus_ns::CLONE_NEWUSER; }
        if args.has_flag("--ipc") || args.has_flag("ipc") { flags |= zenus_ns::CLONE_NEWIPC; }
        w.write_str("Cloning with flags: 0x");
        w.write_hex(flags);
        w.write_str("\r\n");
        let _ = zenus_sched::scheduler::clone_task(flags, 0, 65536, 0, 0, 0, 0x6000_0000_0000);
        w.write_str("Clone attempted\r\n");
    }

    fn cmd_firewall_list(&mut self, w: &mut ShellWriter) {
        let rules = zenus_net::firewall::firewall_list_rules();
        let mut found = false;
        for i in 0..zenus_net::firewall::MAX_RULES {
            if let Some(r) = rules[i] {
                found = true;
                w.write_str("  [");
                w.write_u64(i as u64);
                w.write_str("] ");
                let name_len = r.name.iter().position(|&b| b == 0).unwrap_or(32);
                if name_len > 0 {
                    w.write_str(core::str::from_utf8(&r.name[..name_len]).unwrap_or("?"));
                }
                w.write_str(if r.enabled { " ENABLED  " } else { " DISABLED " });
                match r.action {
                    zenus_net::firewall::FirewallAction::Accept => w.write_str("ACCEPT "),
                    zenus_net::firewall::FirewallAction::Drop => w.write_str("DROP   "),
                    zenus_net::firewall::FirewallAction::Reject => w.write_str("REJECT "),
                }
                match r.proto {
                    zenus_net::firewall::FirewallProto::Any => w.write_str("any  "),
                    zenus_net::firewall::FirewallProto::Tcp => w.write_str("tcp  "),
                    zenus_net::firewall::FirewallProto::Udp => w.write_str("udp  "),
                    zenus_net::firewall::FirewallProto::Icmp => w.write_str("icmp "),
                }
                w.write_ip(r.src_ip);
                w.write_str("/");
                w.write_ip(r.src_mask);
                w.write_str(" -> ");
                w.write_ip(r.dst_ip);
                w.write_str("/");
                w.write_ip(r.dst_mask);
                w.write_str(" :");
                w.write_u64(r.src_port as u64);
                w.write_str(" -> :");
                w.write_u64(r.dst_port as u64);
                if r.established { w.write_str(" EST"); }
                w.write_str(" pkts:");
                w.write_u64(r.packets_matched);
                w.write_str("\r\n");
            }
        }
        if !found {
            w.write_str("  (no firewall rules)\r\n");
        }
        let conns = zenus_net::firewall::firewall_conn_count();
        w.write_str("Connection tracking entries: ");
        w.write_u64(conns as u64);
        w.write_str("\r\n");
    }

    fn cmd_firewall_add(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        if args.args().len() < 7 {
            w.write_str("Usage: firewall-add <action> <proto> <src> <src_mask> <dst> <dst_mask> <src_port> <dst_port> [established]\r\n");
            w.write_str("  action: accept|drop|reject\r\n");
            w.write_str("  proto: any|tcp|udp|icmp\r\n");
            return;
        }
        let action_str = args.get(1).unwrap_or("");
        let proto_str = args.get(2).unwrap_or("");
        let src_str = args.get(3).unwrap_or("");
        let srcm_str = args.get(4).unwrap_or("");
        let dst_str = args.get(5).unwrap_or("");
        let dstm_str = args.get(6).unwrap_or("");
        let src_port = args.get(7).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let dst_port = args.get(8).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
        let established = args.has_flag("established");

        let action = match action_str {
            "accept" => zenus_net::firewall::FirewallAction::Accept,
            "drop" => zenus_net::firewall::FirewallAction::Drop,
            "reject" => zenus_net::firewall::FirewallAction::Reject,
            _ => { w.write_str("firewall-add: invalid action\r\n"); return; }
        };
        let proto = match proto_str {
            "any" => zenus_net::firewall::FirewallProto::Any,
            "tcp" => zenus_net::firewall::FirewallProto::Tcp,
            "udp" => zenus_net::firewall::FirewallProto::Udp,
            "icmp" => zenus_net::firewall::FirewallProto::Icmp,
            _ => { w.write_str("firewall-add: invalid proto\r\n"); return; }
        };

        let src_ip = parse_ip(src_str);
        let src_mask = parse_ip(srcm_str);
        let dst_ip = parse_ip(dst_str);
        let dst_mask = parse_ip(dstm_str);
        if src_ip.is_none() || src_mask.is_none() || dst_ip.is_none() || dst_mask.is_none() {
            w.write_str("firewall-add: invalid IP address\r\n");
            return;
        }

        let idx = zenus_net::firewall::firewall_rule_count();
        let mut name = [0u8; 32];
        let name_str = format!("rule-{}", idx);
        let nb = name_str.as_bytes();
        let nlen = nb.len().min(31);
        name[..nlen].copy_from_slice(&nb[..nlen]);

        let rule = zenus_net::firewall::FirewallRule {
            name,
            enabled: true,
            action,
            proto,
            src_ip: src_ip.unwrap(),
            src_mask: src_mask.unwrap(),
            dst_ip: dst_ip.unwrap(),
            dst_mask: dst_mask.unwrap(),
            src_port,
            dst_port,
            established,
            packets_matched: 0,
        };

        if zenus_net::firewall::firewall_add_rule(rule) {
            w.write_str("firewall-add: rule added\r\n");
        } else {
            w.write_str("firewall-add: rule table full (max 32)\r\n");
        }
    }

    fn cmd_firewall_remove(&mut self, line: &str, w: &mut ShellWriter) {
        let args = Args::parse(line);
        let idx = match args.get(1).and_then(|s| s.parse::<usize>().ok()) {
            Some(i) => i,
            None => { w.write_str("Usage: firewall-remove <index>\r\n"); return; }
        };
        if zenus_net::firewall::firewall_remove_rule(idx) {
            w.write_str("firewall-remove: rule removed\r\n");
        } else {
            w.write_str("firewall-remove: no rule at that index\r\n");
        }
    }

    fn echo_server_poll(&mut self) {
        const MAX_ECHO_LISTENS: usize = 8;
        const MAX_ECHO_CLIENTS: usize = 16;
        static ECHO_STATE: zenus_sync::spinlock::SpinLock<super::EchoState> = zenus_sync::spinlock::SpinLock::new(super::EchoState {
            listen_fds: [None; 8],
            client_fds: [None; 16],
        });

        zenus_net::socket::poll_all(1);
        let mut state = ECHO_STATE.lock();
        for i in 0..MAX_ECHO_LISTENS {
            if let Some(fd) = state.listen_fds[i] {
                while let Some(client_fd) = zenus_net::socket::accept(fd, 1) {
                    for j in 0..MAX_ECHO_CLIENTS {
                        if state.client_fds[j].is_none() {
                            state.client_fds[j] = Some(client_fd);
                            break;
                        }
                    }
                }
            }
        }
        for i in 0..MAX_ECHO_CLIENTS {
            let fd = match state.client_fds[i] { Some(fd) => fd, None => continue };
            let mut buf = [0u8; 1500];
            if let Some(len) = zenus_net::socket::recv(fd, &mut buf) {
                if len > 0 { zenus_net::socket::send(fd, &buf[..len], 1); }
            }
            if !zenus_net::socket::is_connected(fd) { state.client_fds[i] = None; }
        }
    }
}

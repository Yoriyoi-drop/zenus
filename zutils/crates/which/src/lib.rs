#![no_std]

use zutils_common::{Args, Writer};

const KNOWN_COMMANDS: &[&str] = &[
    "help", "echo", "ls", "cat", "clear", "timer", "ps", "pgrep", "top", "uptime",
    "kill", "mkdir", "rm", "touch", "ifconfig", "meminfo", "reboot", "shutdown",
    "uname", "version", "dmesg", "id", "whoami", "chmod", "cp", "mv", "mount",
    "pwd", "df", "du", "chown", "grep", "find", "which",
    "bcache", "fsck", "journal-init", "journal-test",
    "tcp-listen", "tcp-status", "tcp-send", "tcp-echo", "tcp-connect",
    "udp-bind", "udp-send", "udp-recv",
    "dhcp", "dhcp-server", "resolve", "readdev",
    "init-start", "init-shutdown",
    "service-list", "service-start", "service-stop", "service-restart",
    "sysctl", "pkg-install", "pkg-list", "pkg-remove", "pkg-info",
    "watchdog-pet", "watchdog-status", "crashdump", "lockdep-status", "syslog",
    "ssh-start", "ssh-status", "ns-info", "ns-sethost", "ns-clone",
    "zbench", "zdiag", "zdoctor", "zinfo", "zmem", "zpkg", "zsys", "ztrace",
];

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let targets: Vec<&str> = args.args().iter().filter(|a| !a.is_empty()).copied().collect();
    if targets.is_empty() {
        w.write_str("Usage: which <command> [...]\r\n");
        return;
    }
    for target in targets {
        let mut found = false;
        for cmd in KNOWN_COMMANDS {
            if *cmd == target {
                w.write_str("/bin/");
                w.write_str(target);
                w.write_str("\r\n");
                found = true;
                break;
            }
        }
        if !found {
            w.write_str(target);
            w.write_str(": not found\r\n");
        }
    }
}

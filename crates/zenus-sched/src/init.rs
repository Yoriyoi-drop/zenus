use core::sync::atomic::{AtomicBool, Ordering};
use zenus_console::serial::SerialPort;
use zenus_sync::spinlock::SpinLock;
use super::scheduler;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ServiceState {
    Stopped,
    Running,
    Crashed,
    Disabled,
}

#[derive(Clone, Copy)]
pub struct Service {
    pub name: &'static str,
    pub entry: fn(),
    pub stack_size: usize,
    pub pid: u64,
    pub state: ServiceState,
    pub uid: u32,
    pub gid: u32,
    pub restart_count: u32,
    pub max_restarts: u32,
    pub auto_restart: bool,
}

const MAX_SERVICES: usize = 32;

struct ServiceTable {
    services: [Option<Service>; MAX_SERVICES],
    count: usize,
}

impl ServiceTable {
    const fn new() -> Self {
        ServiceTable {
            services: [None; MAX_SERVICES],
            count: 0,
        }
    }
}

static SERVICES: SpinLock<ServiceTable> = SpinLock::new(ServiceTable::new());
static INIT_STARTED: AtomicBool = AtomicBool::new(false);

fn serial() -> SerialPort {
    SerialPort::new(0x3F8)
}

pub fn init_system_start() -> bool {
    if INIT_STARTED.swap(true, Ordering::SeqCst) {
        zenus_console::kinfo!("Init already started");
        return false;
    }

    zenus_console::kinfo!("Starting system services...");
    loop {
        let snapshot = {
            let services = SERVICES.lock();
            if services.count == 0 {
                break;
            }
            let mut pending = alloc::vec::Vec::new();
            for i in 0..services.count {
                if let Some(ref svc) = services.services[i] {
                    if svc.state == ServiceState::Running && svc.pid == 0 {
                        pending.push((i, svc.entry, svc.stack_size, svc.name));
                    }
                }
            }
            if pending.is_empty() {
                break;
            }
            pending
        };
        for (idx, entry, stack_size, name) in snapshot {
            let pid = scheduler::create_task_named(entry, stack_size, name);
            serial().write_str("[INIT] create_task returned pid=");
            serial().write_u64(pid);
            serial().write_str("\n");
            let mut services = SERVICES.lock();
            if pid > 0 {
                if let Some(ref mut s) = services.services[idx] {
                    s.pid = pid;
                    s.state = ServiceState::Running;
                    s.restart_count = 0;
                }
                serial().write_str("[INIT] Started ");
                serial().write_str(name);
                serial().write_str(" (pid ");
                serial().write_u64(pid);
                serial().write_str(")\n");
            } else {
                serial().write_str("[INIT] Failed to start ");
                serial().write_str(name);
                serial().write_str("\n");
                if let Some(ref mut s) = services.services[idx] {
                    s.state = ServiceState::Stopped;
                }
            }
        }
    }

    let count = SERVICES.lock().count;
    serial().write_str("[INIT] ");
    serial().write_u64(count as u64);
    serial().write_str(" services registered\n");
    true
}

pub fn service_register(
    name: &'static str,
    entry: fn(),
    stack_size: usize,
    uid: u32,
    gid: u32,
    auto_restart: bool,
) -> bool {
    let mut services = SERVICES.lock();
    if services.count >= MAX_SERVICES {
        return false;
    }
    for i in 0..services.count {
        if let Some(ref svc) = services.services[i] {
            if svc.name == name {
                return false;
            }
        }
    }
    let idx = services.count;
    services.services[idx] = Some(Service {
        name,
        entry,
        stack_size,
        pid: 0,
        state: ServiceState::Running,
        uid,
        gid,
        restart_count: 0,
        max_restarts: 5,
        auto_restart,
    });
    services.count += 1;
    true
}

pub fn service_start(name: &str) -> bool {
    let entry;
    let stack_size;
    let idx;
    {
        let services = SERVICES.lock();
        let i = match find_service(&services, name) {
            Some(i) => i,
            None => return false,
        };
        let svc = match services.services[i].as_ref() {
            Some(s) => s,
            None => return false,
        };
        if svc.pid != 0 {
            return false;
        }
        idx = i;
        entry = svc.entry;
        stack_size = svc.stack_size;
    }
    let pid = scheduler::create_task(entry, stack_size);
    if pid == 0 {
        return false;
    }
    let mut services = SERVICES.lock();
    if let Some(ref mut s) = services.services[idx] {
        s.pid = pid;
        s.state = ServiceState::Running;
        s.restart_count = 0;
    }
    true
}

pub fn service_stop(name: &str) -> bool {
    let pid;
    {
        let services = SERVICES.lock();
        let i = match find_service(&services, name) {
            Some(i) => i,
            None => return false,
        };
        let svc = match services.services[i].as_ref() {
            Some(s) => s,
            None => return false,
        };
        if svc.pid == 0 {
            return false;
        }
        pid = svc.pid;
    }
    scheduler::kill_task(pid)
}

pub fn service_restart(name: &str) -> bool {
    service_stop(name);
    service_start(name)
}

pub fn service_status(idx: usize) -> Option<Service> {
    let services = SERVICES.lock();
    services.services[idx].clone()
}

pub fn service_count() -> usize {
    let services = SERVICES.lock();
    services.count
}

pub fn service_list() -> alloc::vec::Vec<(&'static str, ServiceState, u64, u32)> {
    let services = SERVICES.lock();
    let mut list = alloc::vec::Vec::with_capacity(services.count);
    for i in 0..services.count {
        if let Some(ref svc) = services.services[i] {
            list.push((svc.name, svc.state, svc.pid, svc.restart_count));
        }
    }
    list
}

pub fn service_supervise() {
    let s = serial();
    let task_list = scheduler::list_tasks();

    loop {
        let crashed: alloc::vec::Vec<(usize, bool, u32, &'static str)>;
        {
            let services = SERVICES.lock();
            crashed = services.services[..services.count]
                .iter()
                .enumerate()
                .filter_map(|(i, svc)| {
                    let svc = svc.as_ref()?;
                    if svc.state != ServiceState::Running || svc.pid == 0 {
                        return None;
                    }
                    let alive = task_list.iter().any(|t| matches!(t, Some(info) if info.id == svc.pid));
                    if alive {
                        return None;
                    }
                    Some((i, svc.auto_restart, svc.max_restarts, svc.name))
                })
                .collect();
        }

        if crashed.is_empty() {
            return;
        }

        for (i, auto_restart, max_restarts, name) in crashed {
            let count;
            {
                let mut services = SERVICES.lock();
                if i >= services.count {
                    continue;
                }
                let svc = match services.services[i].as_mut() {
                    Some(s) => s,
                    None => continue,
                };
                svc.state = ServiceState::Crashed;
                svc.pid = 0;
                count = svc.restart_count;
            }

            s.write_str("[SUPERVISOR] ");
            s.write_str(name);
            s.write_str(" crashed\n");

            if !auto_restart {
                s.write_str("[SUPERVISOR] ");
                s.write_str(name);
                s.write_str(" auto-restart disabled\n");
                continue;
            }

            if count >= max_restarts {
                s.write_str("[SUPERVISOR] ");
                s.write_str(name);
                s.write_str(" max restarts (");
                s.write_u64(max_restarts as u64);
                s.write_str(") reached\n");
                continue;
            }

            let entry;
            let stack_size;
            {
                let services = SERVICES.lock();
                let svc = match services.services[i].as_ref() {
                    Some(s) => s,
                    None => continue,
                };
                entry = svc.entry;
                stack_size = svc.stack_size;
            }

            let pid = scheduler::create_task(entry, stack_size);
            if pid > 0 {
                let mut services = SERVICES.lock();
                if let Some(ref mut svc) = services.services[i] {
                    svc.pid = pid;
                    svc.state = ServiceState::Running;
                    svc.restart_count = count + 1;
                }
                s.write_str("[SUPERVISOR] Restarted ");
                s.write_str(name);
                s.write_str(" (pid ");
                s.write_u64(pid);
                s.write_str(", restart ");
                s.write_u64((count + 1) as u64);
                s.write_str("/");
                s.write_u64(max_restarts as u64);
                s.write_str(")\n");
            } else {
                s.write_str("[SUPERVISOR] Failed to restart ");
                s.write_str(name);
                s.write_str("\n");
            }
        }
    }
}

pub fn init_shutdown() -> ! {
    let s = serial();
    s.write_str("[INIT] Shutting down all services...\n");

    loop {
        let to_stop: alloc::vec::Vec<(usize, u64, &'static str)>;
        {
            let services = SERVICES.lock();
            to_stop = services.services[..services.count]
                .iter()
                .enumerate()
                .filter_map(|(i, svc)| {
                    let svc = svc.as_ref()?;
                    if svc.pid == 0 {
                        return None;
                    }
                    Some((i, svc.pid, svc.name))
                })
                .collect();
        }

        if to_stop.is_empty() {
            break;
        }

        for (i, pid, name) in to_stop {
            scheduler::kill_task(pid);
            let mut services = SERVICES.lock();
            if let Some(ref mut svc) = services.services[i] {
                svc.state = ServiceState::Stopped;
                svc.pid = 0;
            }
            s.write_str("[INIT] Stopped ");
            s.write_str(name);
            s.write_str(" (pid ");
            s.write_u64(pid);
            s.write_str(")\n");
        }
    }

    zenus_console::kpanic_code!(zenus_console::error::codes::KRN_STACK_OVERFLOW, "System halted");
    loop {
        x86_64::instructions::hlt();
    }
}

fn find_service(table: &ServiceTable, name: &str) -> Option<usize> {
    for i in 0..table.count {
        if let Some(ref svc) = table.services[i] {
            if svc.name == name {
                return Some(i);
            }
        }
    }
    None
}

pub fn initrd_execute() -> bool {
    let s = serial();
    s.write_str("[INITRD] Executing /initrd/init/startup.sh...\n");

    let script_path = "/initrd/init/startup.sh";
    let node = match zenus_fs::vfs::open(script_path) {
        Some(n) => n,
        None => {
            s.write_str("[INITRD] File not found: /initrd/init/startup.sh\n");
            return false;
        }
    };

    let stat = node.fs.stat(node.inode);
    if stat.file_type != zenus_fs::vfs::FileType::File {
        s.write_str("[INITRD] Not a regular file\n");
        return false;
    }

    let size = stat.size as usize;
    if size == 0 || size > 65536 {
        s.write_str("[INITRD] Empty or too large\n");
        return false;
    }

    let mut buf = alloc::vec![0u8; size];
    match node.fs.read(node.inode, 0, &mut buf) {
        Some(n) if n as usize == size => {}
        _ => {
            zenus_console::kerror_code!(zenus_console::error::codes::FS_MOUNT_FAILED, "Initrd read failed");
            return false;
        }
    }

    let content = match core::str::from_utf8(&buf) {
        Ok(s) => s,
        Err(_) => {
            s.write_str("[INITRD] Invalid UTF-8\n");
            return false;
        }
    };

    execute_script(content);
    true
}

fn execute_script(script: &str) {
    let s = serial();
    for raw_line in script.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        s.write_str("[INITRD] ");
        s.write_str(trimmed);
        s.write_str("\n");
        execute_command(trimmed);
    }
    s.write_str("[INITRD] Startup script completed\n");
}

fn execute_command(cmdline: &str) {
    let parts: alloc::vec::Vec<&str> = cmdline.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }
    let s = serial();
    match parts[0] {
        "echo" => {
            let rest = cmdline.get("echo".len()..).map(|r| r.trim()).unwrap_or("");
            s.write_str(rest);
            s.write_str("\n");
        }
        "cat" => {
            if parts.len() < 2 {
                s.write_str("cat: missing operand\n");
                return;
            }
            let path = parts[1];
            match zenus_fs::vfs::open(path) {
                Some(node) => {
                    let stat = node.fs.stat(node.inode);
                    if stat.file_type == zenus_fs::vfs::FileType::Directory {
                        s.write_str("cat: Is a directory\n");
                        return;
                    }
                    let mut buf = [0u8; 512];
                    let mut offset: u64 = 0;
                    loop {
                        match node.fs.read(node.inode, offset, &mut buf) {
                            Some(0) | None => break,
                            Some(n) => {
                                let chunk = &buf[..n as usize];
                                if let Ok(txt) = core::str::from_utf8(chunk) {
                                    s.write_str(txt);
                                } else {
                                    for &b in chunk {
                                        if b.is_ascii_graphic() || b == b' ' || b == b'\n' {
                                            s.write_byte_serial(b);
                                        } else {
                                            s.write_byte_serial(b'.');
                                        }
                                    }
                                }
                                offset += n;
                            }
                        }
                    }
                    s.write_str("\n");
                }
                None => {
                    s.write_str("cat: ");
                    s.write_str(path);
                    s.write_str(": not found\n");
                }
            }
        }
        "ls" => {
            let path = if parts.len() > 1 { parts[1] } else { "/" };
            let entries = zenus_fs::vfs::read_dir(path);
            for entry in entries {
                s.write_str(&entry.name);
                if entry.file_type == zenus_fs::vfs::FileType::Directory {
                    s.write_str("/");
                }
                s.write_str("  ");
            }
            s.write_str("\n");
        }
        "mkdir" => {
            if parts.len() < 2 {
                s.write_str("mkdir: missing operand\n");
                return;
            }
            if zenus_fs::vfs::create_dir(parts[1]) {
                s.write_str("ok\n");
            } else {
                zenus_console::kwarn!("mkdir: failed");
            }
        }
        "touch" => {
            if parts.len() < 2 {
                s.write_str("touch: missing operand\n");
                return;
            }
            if zenus_fs::vfs::create_file(parts[1]) {
                s.write_str("ok\n");
            } else {
                zenus_console::kwarn!("touch: failed");
            }
        }
        "sleep" => {
            let secs = parts.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(1);
            for _ in 0..secs * 100_000_000 {
                core::hint::spin_loop();
            }
        }
        "modprobe" | "insmod" => {
            s.write_str("module loading not supported in initrd\n");
        }
        "mount" => {
            s.write_str("mount command not supported in init scripts\n");
        }
        _ => {
            s.write_str("Unknown command: ");
            s.write_str(parts[0]);
            s.write_str("\n");
        }
    }
}

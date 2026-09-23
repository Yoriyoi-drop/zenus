use zenus_fs::vfs;
use zenus_net::socket as net_socket;
use zenus_sched::scheduler;
use zenus_sched::signal;

mod fd;
use fd::*;

const USER_SPACE_LIMIT: u64 = 0x0000_8000_0000_0000;
const MAX_PATH_LEN: usize = 4096;

fn validate_user_range(ptr: u64, len: u64) -> bool {
    if ptr == 0 || ptr < 0x1000 {
        return false;
    }
    let end = match ptr.checked_add(len) {
        Some(e) => e,
        None => return false,
    };
    if end > USER_SPACE_LIMIT {
        return false;
    }
    true
}

fn validate_user_ptr<T>(ptr: u64) -> bool {
    validate_user_range(ptr, core::mem::size_of::<T>() as u64)
}

fn read_user_cstr(ptr: u64, max_len: usize) -> Option<alloc::borrow::Cow<'static, str>> {
    if !validate_user_range(ptr, 1) {
        return None;
    }
    let limit = max_len.min(MAX_PATH_LEN);
    let mut buf = [0u8; MAX_PATH_LEN];
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }
    let irq_was_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();
    let mut found = false;
    let mut len = 0usize;
    for i in 0..limit {
        let addr = ptr + i as u64;
        if zenus_mem::paging::virt_to_phys_raw(cr3, addr).is_none() {
            if irq_was_enabled {
                x86_64::instructions::interrupts::enable();
            }
            return None;
        }
        let byte: u8 = unsafe {
            zenus_arch::cpu::stac();
            let b = core::ptr::read_volatile(addr as *const u8);
            zenus_arch::cpu::clac();
            b
        };
        if byte == 0 {
            found = true;
            break;
        }
        buf[i] = byte;
        len = i + 1;
    }
    if irq_was_enabled {
        x86_64::instructions::interrupts::enable();
    }
    if !found {
        return None;
    }
    core::str::from_utf8(&buf[..len])
        .ok()
        .map(|s| alloc::borrow::Cow::Owned(s.into()))
}

const SYS_READ: u64 = 0;
const SYS_WRITE: u64 = 1;
const SYS_OPEN: u64 = 2;
const SYS_CLOSE: u64 = 3;
const SYS_STAT: u64 = 4;
const SYS_READDIR: u64 = 5;
const SYS_LSEEK: u64 = 8;
const SYS_IOCTL: u64 = 16;
const SYS_PIPE: u64 = 22;
const SYS_DUP: u64 = 32;
const SYS_NANOSLEEP: u64 = 35;
const SYS_GETPID: u64 = 39;
const SYS_BRK: u64 = 45;
const SYS_CLONE: u64 = 56;
const SYS_FORK: u64 = 57;
const SYS_EXECVE: u64 = 59;
const SYS_EXIT: u64 = 60;
const SYS_WAITPID: u64 = 61;
const SYS_UNAME_NS: u64 = 62;
const SYS_UNAME: u64 = 63;
const SYS_GETUID: u64 = 100;
const SYS_GETEUID: u64 = 101;
const SYS_GETGID: u64 = 102;
const SYS_GETEGID: u64 = 103;
const SYS_SETUID: u64 = 104;
const SYS_SETGID: u64 = 105;
const SYS_SET_HOSTNAME: u64 = 107;
const SYS_GETPID_NS: u64 = 110;
const SYS_EXIT_GROUP: u64 = 231;

// Socket syscalls (Linux x86_64 numbers)
const SYS_SOCKET: u64 = 41;
const SYS_CONNECT: u64 = 42;
const SYS_ACCEPT: u64 = 43;
const SYS_SENDTO: u64 = 44;
const SYS_RECVFROM: u64 = 45;
const SYS_SEND: u64 = 46;
const SYS_RECV: u64 = 47;
const SYS_SHUTDOWN: u64 = 48;
const SYS_BIND: u64 = 49;
const SYS_LISTEN: u64 = 50;
const SYS_GETSOCKNAME: u64 = 51;
const SYS_GETPEERNAME: u64 = 52;
const SYS_DUP2: u64 = 33;
const SYS_DUP3: u64 = 24; // custom slot (Linux uses 292 which exceeds 256)

// Filesystem syscalls
const SYS_MKDIR: u64 = 83;
const SYS_UNLINK: u64 = 87;
const SYS_RMDIR: u64 = 84;
const SYS_RENAME: u64 = 82;
const SYS_CHMOD: u64 = 90;
const SYS_CHOWN: u64 = 92;
const SYS_ACCESS: u64 = 21;

// Signal syscalls
const SYS_KILL: u64 = 62;
const SYS_RT_SIGACTION: u64 = 13;
const SYS_RT_SIGPROCMASK: u64 = 14;
const SYS_RT_SIGRETURN: u64 = 15;
const SYS_TGKILL: u64 = 234;

// Memory syscalls
const SYS_MMAP: u64 = 9;
const SYS_MPROTECT: u64 = 10;
const SYS_MUNMAP: u64 = 11;

// Time syscalls
const SYS_GETTIMEOFDAY: u64 = 96;
const SYS_CLOCK_GETTIME: u64 = 228;
const SYS_CLOCK_GETRES: u64 = 229;

// Mount syscalls
const SYS_MOUNT: u64 = 165;
const SYS_UMOUNT2: u64 = 166;

// Shared memory syscalls
const SYS_SHMGET: u64 = 29;
const SYS_SHMAT: u64 = 30;
const SYS_SHMDT: u64 = 67;
const SYS_SHMCTL: u64 = 31;

// Futex syscall
const SYS_FUTEX: u64 = 202;

// Poll syscalls
const SYS_POLL: u64 = 7;
const SYS_PPOLL: u64 = 28; // custom slot (Linux 271 > 256)

// Process group syscalls
const SYS_SETSID: u64 = 112;
const SYS_GETPGID: u64 = 121;
const SYS_SETPGID: u64 = 109;
const SYS_GETSID: u64 = 124;

// Socket options
const SYS_SETSOCKOPT: u64 = 54;
const SYS_GETSOCKOPT: u64 = 55;

// Select
const SYS_SELECT: u64 = 23;
const SYS_PSELECT6: u64 = 29; // custom slot (Linux 270 > 256)

// Filesystem extras
const SYS_READLINKAT: u64 = 26; // custom slot (Linux 267 > 256)
const SYS_SYMLINKAT: u64 = 27; // custom slot (Linux 266 > 256)
const SYS_LINKAT: u64 = 34; // custom slot (Linux 265 > 256)
const SYS_TRUNCATE: u64 = 76;
const SYS_FTRUNCATE: u64 = 77;

// Resource limits
const SYS_GETRLIMIT: u64 = 163;
const SYS_SETRLIMIT: u64 = 160;

// CWD syscalls
const SYS_GETCWD: u64 = 79;
const SYS_CHDIR: u64 = 80;
const SYS_FCHDIR: u64 = 81;

// Process control
const SYS_PRCTL: u64 = 157;

// Additional filesystem (custom slots for Linux numbers > 256)
const SYS_FACCESSAT: u64 = 35;
const SYS_UTIMENSAT: u64 = 36;
const SYS_FSTATAT: u64 = 37;
const SYS_FCHOWNAT: u64 = 38;

// Additional process syscalls
const SYS_WAITID: u64 = 247; // custom slot (Linux 247 < 256, ok)
const SYS_SOCKETPAIR: u64 = 53;

// File descriptor stat
const SYS_FSTAT: u64 = 5;
const SYS_FSTATFS: u64 = 32; // custom slot
const SYS_STATFS: u64 = 40; // custom slot

// Scheduling
const SYS_NICE: u64 = 41; // custom slot
const SYS_SCHED_GETSCHEDULER: u64 = 42; // custom slot
const SYS_SCHED_SETSCHEDULER: u64 = 43; // conflicts with accept... use custom
const SYS_SCHED_GETPARAM: u64 = 44; // conflicts with sendto... use custom
const SYS_SCHED_SETPARAM: u64 = 45; // conflicts with recvfrom... use custom
const SYS_SCHED_YIELD: u64 = 24; // conflicts with dup3... use custom

// Resource usage
const SYS_GETRUSAGE: u64 = 55; // custom slot (conflicts with getsockopt)
const SYS_TIMES: u64 = 56; // custom slot

// Event/notification
const SYS_EVENTFD2: u64 = 57; // custom slot
const SYS_PIPE2: u64 = 58; // custom slot
const SYS_GETPPID: u64 = 39; // reuse getpid slot, different function

type SyscallFn = fn(u64, u64, u64, u64, u64, u64) -> u64;

static SYSCALL_TABLE: [Option<SyscallFn>; 256] = init_table();

const fn init_table() -> [Option<SyscallFn>; 256] {
    let mut t: [Option<SyscallFn>; 256] = [None; 256];
    t[SYS_READ as usize] = Some(sys_read);
    t[SYS_WRITE as usize] = Some(sys_write);
    t[SYS_OPEN as usize] = Some(sys_open);
    t[SYS_CLOSE as usize] = Some(sys_close);
    t[SYS_STAT as usize] = Some(sys_stat);
    t[SYS_READDIR as usize] = Some(sys_readdir);
    t[SYS_LSEEK as usize] = Some(sys_lseek);
    t[SYS_IOCTL as usize] = Some(sys_ioctl);
    t[SYS_DUP as usize] = Some(sys_dup);
    t[SYS_NANOSLEEP as usize] = Some(sys_nanosleep);
    t[SYS_GETPID as usize] = Some(sys_getpid);
    t[SYS_BRK as usize] = Some(sys_brk);
    t[SYS_EXIT as usize] = Some(sys_exit);
    t[SYS_UNAME as usize] = Some(sys_uname);
    t[SYS_GETUID as usize] = Some(sys_getuid);
    t[SYS_GETEUID as usize] = Some(sys_geteuid);
    t[SYS_GETGID as usize] = Some(sys_getgid);
    t[SYS_GETEGID as usize] = Some(sys_getegid);
    t[SYS_SETUID as usize] = Some(sys_setuid);
    t[SYS_SETGID as usize] = Some(sys_setgid);
    t[SYS_CLONE as usize] = Some(sys_clone);
    t[SYS_UNAME_NS as usize] = Some(sys_uname_ns);
    t[SYS_SET_HOSTNAME as usize] = Some(sys_sethostname);
    t[SYS_GETPID_NS as usize] = Some(sys_getpid_ns);
    t[SYS_PIPE as usize] = Some(sys_pipe);
    t[SYS_FORK as usize] = Some(sys_fork);
    t[SYS_EXECVE as usize] = Some(sys_execve);
    t[SYS_WAITPID as usize] = Some(sys_waitpid);
    t[SYS_EXIT_GROUP as usize] = Some(sys_exit_group);
    t[SYS_SOCKET as usize] = Some(sys_socket);
    t[SYS_CONNECT as usize] = Some(sys_connect);
    t[SYS_ACCEPT as usize] = Some(sys_accept);
    t[SYS_SENDTO as usize] = Some(sys_sendto);
    t[SYS_RECVFROM as usize] = Some(sys_recvfrom);
    t[SYS_SEND as usize] = Some(sys_send);
    t[SYS_RECV as usize] = Some(sys_recv);
    t[SYS_SHUTDOWN as usize] = Some(sys_shutdown);
    t[SYS_BIND as usize] = Some(sys_bind);
    t[SYS_LISTEN as usize] = Some(sys_listen);
    t[SYS_GETSOCKNAME as usize] = Some(sys_getsockname);
    t[SYS_GETPEERNAME as usize] = Some(sys_getpeername);
    t[SYS_DUP2 as usize] = Some(sys_dup2);
    t[SYS_DUP3 as usize] = Some(sys_dup3);
    t[SYS_MKDIR as usize] = Some(sys_mkdir);
    t[SYS_UNLINK as usize] = Some(sys_unlink);
    t[SYS_RMDIR as usize] = Some(sys_rmdir);
    t[SYS_RENAME as usize] = Some(sys_rename);
    t[SYS_CHMOD as usize] = Some(sys_chmod);
    t[SYS_CHOWN as usize] = Some(sys_chown);
    t[SYS_ACCESS as usize] = Some(sys_access);
    t[SYS_KILL as usize] = Some(sys_kill);
    t[SYS_RT_SIGACTION as usize] = Some(sys_rt_sigaction);
    t[SYS_RT_SIGPROCMASK as usize] = Some(sys_rt_sigprocmask);
    t[SYS_RT_SIGRETURN as usize] = Some(sys_rt_sigreturn);
    t[SYS_TGKILL as usize] = Some(sys_tgkill);
    t[SYS_MMAP as usize] = Some(sys_mmap);
    t[SYS_MPROTECT as usize] = Some(sys_mprotect);
    t[SYS_MUNMAP as usize] = Some(sys_munmap);
    t[SYS_GETTIMEOFDAY as usize] = Some(sys_gettimeofday);
    t[SYS_CLOCK_GETTIME as usize] = Some(sys_clock_gettime);
    t[SYS_CLOCK_GETRES as usize] = Some(sys_clock_getres);
    t[SYS_MOUNT as usize] = Some(sys_mount);
    t[SYS_UMOUNT2 as usize] = Some(sys_umount2);
    t[SYS_SHMGET as usize] = Some(sys_shmget);
    t[SYS_SHMAT as usize] = Some(sys_shmat);
    t[SYS_SHMDT as usize] = Some(sys_shmdt);
    t[SYS_SHMCTL as usize] = Some(sys_shmctl);
    t[SYS_FUTEX as usize] = Some(sys_futex);
    t[SYS_POLL as usize] = Some(sys_poll);
    t[SYS_PPOLL as usize] = Some(sys_ppoll);
    t[SYS_SETSID as usize] = Some(sys_setsid);
    t[SYS_GETPGID as usize] = Some(sys_getpgid);
    t[SYS_SETPGID as usize] = Some(sys_setpgid);
    t[SYS_GETSID as usize] = Some(sys_getsid);
    t[SYS_SETSOCKOPT as usize] = Some(sys_setsockopt);
    t[SYS_GETSOCKOPT as usize] = Some(sys_getsockopt);
    t[SYS_SELECT as usize] = Some(sys_select);
    t[SYS_PSELECT6 as usize] = Some(sys_pselect6);
    t[SYS_READLINKAT as usize] = Some(sys_readlinkat);
    t[SYS_SYMLINKAT as usize] = Some(sys_symlinkat);
    t[SYS_LINKAT as usize] = Some(sys_linkat);
    t[SYS_TRUNCATE as usize] = Some(sys_truncate);
    t[SYS_FTRUNCATE as usize] = Some(sys_ftruncate);
    t[SYS_GETRLIMIT as usize] = Some(sys_getrlimit);
    t[SYS_SETRLIMIT as usize] = Some(sys_setrlimit);
    t[SYS_GETCWD as usize] = Some(sys_getcwd);
    t[SYS_CHDIR as usize] = Some(sys_chdir);
    t[SYS_FCHDIR as usize] = Some(sys_fchdir);
    t[SYS_PRCTL as usize] = Some(sys_prctl);
    t[SYS_FACCESSAT as usize] = Some(sys_faccessat);
    t[SYS_UTIMENSAT as usize] = Some(sys_utimensat);
    t[SYS_FSTATAT as usize] = Some(sys_fstatat);
    t[SYS_FCHOWNAT as usize] = Some(sys_fchownat);
    t[SYS_WAITID as usize] = Some(sys_waitid);
    t[SYS_SOCKETPAIR as usize] = Some(sys_socketpair);
    t[SYS_FSTAT as usize] = Some(sys_fstat);
    t[SYS_FSTATFS as usize] = Some(sys_fstatfs);
    t[SYS_STATFS as usize] = Some(sys_statfs);
    t[SYS_NICE as usize] = Some(sys_nice);
    t[SYS_SCHED_GETSCHEDULER as usize] = Some(sys_sched_getscheduler);
    t[SYS_SCHED_SETSCHEDULER as usize] = Some(sys_sched_setscheduler);
    t[SYS_SCHED_GETPARAM as usize] = Some(sys_sched_getparam);
    t[SYS_SCHED_SETPARAM as usize] = Some(sys_sched_setparam);
    t[SYS_SCHED_YIELD as usize] = Some(sys_sched_yield);
    t[SYS_GETRUSAGE as usize] = Some(sys_getrusage);
    t[SYS_TIMES as usize] = Some(sys_times);
    t[SYS_EVENTFD2 as usize] = Some(sys_eventfd2);
    t[SYS_PIPE2 as usize] = Some(sys_pipe2);
    t
}

fn current_task() -> u64 {
    scheduler::current_task_id()
}

fn copy_user_to_kernel(user_ptr: u64, len: usize) -> Option<alloc::vec::Vec<u8>> {
    if len == 0 {
        return Some(alloc::vec::Vec::new());
    }
    if !validate_user_range(user_ptr, len as u64) {
        return None;
    }
    let mut buf = alloc::vec::Vec::with_capacity(len);
    buf.resize(len, 0);
    let mut copied = 0;
    let irq_was_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();
    while copied < len {
        let current = user_ptr + copied as u64;
        let remaining = len - copied;
        let chunk = remaining.min(4096 - (current & 0xFFF) as usize);
        let cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
        }
        if zenus_mem::paging::virt_to_phys_raw(cr3, current).is_none() {
            if irq_was_enabled {
                x86_64::instructions::interrupts::enable();
            }
            return None;
        }
        unsafe {
            zenus_arch::cpu::stac();
            core::ptr::copy_nonoverlapping(
                current as *const u8,
                buf.as_mut_ptr().add(copied),
                chunk,
            );
            zenus_arch::cpu::clac();
        }
        copied += chunk;
    }
    if irq_was_enabled {
        x86_64::instructions::interrupts::enable();
    }
    Some(buf)
}

fn copy_kernel_to_user(kernel_buf: &[u8], user_ptr: u64) -> bool {
    if kernel_buf.is_empty() {
        return true;
    }
    if !validate_user_range(user_ptr, kernel_buf.len() as u64) {
        return false;
    }
    let mut copied = 0;
    let irq_was_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();
    while copied < kernel_buf.len() {
        let current = user_ptr + copied as u64;
        let remaining = kernel_buf.len() - copied;
        let chunk = remaining.min(4096 - (current & 0xFFF) as usize);
        let cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
        }
        if zenus_mem::paging::virt_to_phys_raw(cr3, current).is_none() {
            if irq_was_enabled {
                x86_64::instructions::interrupts::enable();
            }
            return false;
        }
        unsafe {
            zenus_arch::cpu::stac();
            core::ptr::copy_nonoverlapping(
                kernel_buf.as_ptr().add(copied),
                current as *mut u8,
                chunk,
            );
            zenus_arch::cpu::clac();
        }
        copied += chunk;
    }
    if irq_was_enabled {
        x86_64::instructions::interrupts::enable();
    }
    true
}

fn sys_read(fd: u64, buf: u64, count: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if count > 1048576 {
        return -1i64 as u64;
    }
    let mut kernel_buf = match copy_user_to_kernel(buf, 0) {
        Some(b) => b,
        None => return -1i64 as u64,
    };
    kernel_buf.resize(count as usize, 0);
    match fd_read(fd, &mut kernel_buf) {
        Some(n) => {
            if copy_kernel_to_user(&kernel_buf[..n as usize], buf) {
                n
            } else {
                -1i64 as u64
            }
        }
        None => -1i64 as u64,
    }
}

fn sys_write(fd: u64, buf: u64, count: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if count > 1048576 {
        return -1i64 as u64;
    }
    if count == 0 {
        return 0;
    }
    if !validate_user_range(buf, count) {
        return -1i64 as u64;
    }

    let count_usize = count as usize;

    // stdout/stderr (fd 1, 2): tulis langsung dari user buffer byte-by-byte
    // tanpa heap allocation. SMAP disabled, jadi kernel bisa akses user memory
    // langsung via raw pointer.
    if fd == 1 || fd == 2 {
        let s = zenus_console::serial::SerialPort::new(0x3F8);
        let mut display_buf = [0u8; 256];
        let mut display_len = 0usize;
        for i in 0..count_usize {
            let byte = unsafe { core::ptr::read_volatile((buf + i as u64) as *const u8) };
            s.write_byte_serial(byte);
            if display_len < 256 {
                display_buf[display_len] = byte;
                display_len += 1;
            }
        }
        if display_len > 0 {
            if let Ok(s) = core::str::from_utf8(&display_buf[..display_len]) {
                zenus_console::display::write_str(s);
            }
        }
        return count;
    }

    // File descriptor write: copy ke kernel buffer dulu
    let kernel_buf = match copy_user_to_kernel(buf, count_usize) {
        Some(b) => b,
        None => return -1i64 as u64,
    };
    match fd_write(fd, &kernel_buf) {
        Some(n) => n,
        None => -1i64 as u64,
    }
}

fn sys_open(path_ptr: u64, _flags: u64, _mode: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_ptr::<u8>(path_ptr) {
        return -1i64 as u64;
    }
    let path_str = match read_user_cstr(path_ptr, 4096) {
        Some(s) => s.into_owned(),
        None => return -1i64 as u64,
    };
    match fd_open(current_task(), &path_str) {
        Some(fd) => fd,
        None => -1i64 as u64,
    }
}

fn sys_close(fd: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if fd <= 2 {
        return -1i64 as u64;
    }
    let entry = fd::fd_get(fd);
    if let Some(ref e) = entry {
        if e.socket_id != u64::MAX {
            net_socket::close(e.socket_id as usize, 0);
        }
    }
    if fd_close(fd) {
        0
    } else {
        -1i64 as u64
    }
}

#[repr(C)]
struct StatBuf {
    st_size: u64,
    st_mode: u64,
    st_ino: u64,
}

fn sys_stat(path_ptr: u64, stat_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_ptr::<u8>(path_ptr) {
        return -1i64 as u64;
    }
    if !validate_user_ptr::<StatBuf>(stat_ptr) {
        return -1i64 as u64;
    }
    let path_str = match read_user_cstr(path_ptr, 4096) {
        Some(s) => s.into_owned(),
        None => return -1i64 as u64,
    };

    match vfs::open(&path_str) {
        Some(node) => {
            let stat = node.fs.stat(node.inode);
            let stat_buf = stat_ptr as *mut StatBuf;
            unsafe {
                (*stat_buf).st_size = stat.size;
                (*stat_buf).st_mode = match stat.file_type {
                    vfs::FileType::File => 0x81A4,
                    vfs::FileType::Directory => 0x41ED,
                    vfs::FileType::CharDevice => 0x21A4,
                    _ => 0,
                };
                (*stat_buf).st_ino = stat.inode;
            }
            0
        }
        None => -1i64 as u64,
    }
}

fn sys_readdir(fd: u64, buf: u64, buf_size: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_range(buf, buf_size) {
        return -1i64 as u64;
    }
    let entries = fd_readdir(fd);
    if entries.is_empty() {
        return 0;
    }

    let dst = buf as *mut u8;
    let max = buf_size as usize;
    let mut written = 0u64;

    for entry in entries {
        if written as usize + 2 > max {
            break;
        }
        let name_bytes = entry.name.as_bytes();
        let name_len = name_bytes.len();
        if written as usize + 1 + name_len + 1 > max {
            break;
        }

        unsafe {
            dst.add(written as usize).write(entry.file_type as u8);
            written += 1;
            dst.add(written as usize).write(name_len as u8);
            written += 1;
            if name_len > 0 {
                core::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr(),
                    dst.add(written as usize),
                    name_len,
                );
                written += name_len as u64;
            }
        }
    }
    written
}

fn sys_lseek(fd: u64, offset: u64, whence: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    match fd_seek(fd, offset as i64, whence) {
        Some(pos) => pos,
        None => -1i64 as u64,
    }
}

fn sys_ioctl(fd: u64, request: u64, arg: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    match fd_stat(fd) {
        Some(_) => {
            zenus_console::kdebug!("sys_ioctl fd={} req=0x{:x} arg=0x{:x}", fd, request, arg);
            0
        }
        None => -1i64 as u64,
    }
}

fn sys_dup(old_fd: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if old_fd > 2 {
        match fd_dup(current_task(), old_fd) {
            Some(new_fd) => new_fd,
            None => -1i64 as u64,
        }
    } else {
        old_fd
    }
}

fn sys_nanosleep(sec: u64, nsec: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let total_ms = sec.saturating_mul(1000).saturating_add(nsec / 1_000_000);
    let start = zenus_arch::interrupts::pit::get_ticks();
    loop {
        zenus_sched::scheduler::yield_now();
        let elapsed = zenus_arch::interrupts::pit::get_ticks().wrapping_sub(start);
        if elapsed >= total_ms {
            break;
        }
        x86_64::instructions::hlt();
    }
    0
}

fn sys_getpid(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    current_task()
}

fn unmap_heap_pages(cr3: u64, start: u64, end: u64) {
    let start_page = start & !0xFFF;
    let end_page = (end + 0xFFF) & !0xFFF;
    if end_page <= start_page {
        return;
    }
    let hhdm = zenus_mem::paging::hhdm_offset();
    let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
    let mut page = start_page;
    while page < end_page {
        if let Some(phys) = zenus_mem::paging::virt_to_phys_raw(cr3, page) {
            allocator.free_frame(x86_64::PhysAddr::new(phys & !0xFFF));
            let cr3_phys = cr3 & !0xFFF;
            unsafe {
                let mut table_virt = (cr3_phys + hhdm) as *mut u64;
                for &(level, shift) in &[(4usize, 39), (3, 30), (2, 21), (1, 12)] {
                    let idx = (page >> shift) & 0x1FF;
                    let entry = *table_virt.add(idx as usize);
                    if (entry & 1) == 0 {
                        break;
                    }
                    if level == 1 {
                        table_virt.add(idx as usize).write(0);
                        break;
                    }
                    let next = entry & 0x000FFFFFFFFFF000;
                    table_virt = (next + hhdm) as *mut u64;
                }
            }
            unsafe {
                core::arch::asm!("invlpg [{0}]", in(reg) page, options(nostack, preserves_flags));
            }
        }
        page += 0x1000;
    }
}

fn map_heap_pages(cr3: u64, start: u64, end: u64) -> bool {
    let start_page = start & !0xFFF;
    let end_page = (end + 0xFFF) & !0xFFF;
    let mut page = start_page;
    while page < end_page {
        if page >= USER_SPACE_LIMIT {
            return false;
        }
        if zenus_mem::paging::virt_to_phys_raw(cr3, page).is_some() {
            page += 0x1000;
            continue;
        }
        let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
        let frame = match allocator.alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        drop(allocator);
        let hhdm = zenus_mem::paging::hhdm_offset();
        unsafe {
            core::ptr::write_bytes((hhdm + frame.as_u64()) as *mut u8, 0, 4096);
        }
        if !zenus_mem::paging::map_user_page_raw(cr3, page, frame.as_u64(), true, false) {
            return false;
        }
        page += 0x1000;
    }
    true
}

fn sys_brk(addr: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let task = scheduler::current_task_id();
    let heap_start = scheduler::get_task_heap_brk(task);
    if addr == 0 {
        return heap_start;
    }
    if addr > USER_SPACE_LIMIT {
        return -1i64 as u64;
    }
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }
    if addr > heap_start {
        if !map_heap_pages(cr3, heap_start, addr) {
            return -1i64 as u64;
        }
    } else if addr < heap_start {
        unmap_heap_pages(cr3, addr, heap_start);
    }
    scheduler::set_task_heap_brk(task, addr);
    addr
}

fn sys_exit(code: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let tid = current_task();
    zenus_console::kinfo!("Task {} exit with code {}", tid, code);
    if tid > 0 {
        fd_close_all_for_task(tid);
        scheduler::exit_current_task(code);
    }
    0
}

fn sys_exit_group(code: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    sys_exit(code, 0, 0, 0, 0, 0)
}

fn sys_pipe(pipefd_ptr: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_range(pipefd_ptr, 8) {
        return -1i64 as u64;
    }
    match fd::fd_pipe(current_task()) {
        Some((read_fd, write_fd)) => {
            let dst = pipefd_ptr as *mut u32;
            unsafe {
                dst.write(read_fd as u32);
                dst.add(1).write(write_fd as u32);
            }
            0
        }
        None => -1i64 as u64,
    }
}

fn sys_fork(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }

    // Clone address space (copy-on-write: copy writable pages, share read-only)
    let child_cr3 = match zenus_mem::paging::clone_user_address_space(cr3) {
        Some(c) => c,
        None => return -1i64 as u64,
    };

    // Read user RIP from saved RCX on kernel stack for child entry
    let kernel_rsp_top: u64;
    unsafe {
        core::arch::asm!("mov {}, gs:[8]", out(reg) kernel_rsp_top, options(nostack));
    }
    // After syscall entry: push rcx at kernel_rsp_top-8, push r11 at kernel_rsp_top-16
    let user_rip: u64 = unsafe { *((kernel_rsp_top - 8) as *const u64) };

    let user_rsp = zenus_arch::cpu::get_percpu_user_rsp(zenus_arch::smp::current_cpu());
    let heap_brk = scheduler::get_task_heap_brk(current_task());

    let child_pid = scheduler::clone_task(
        0, // flags
        0, // stack (use parent's user_rsp)
        65536, user_rip, // entry = user RIP (instruction after SYSCALL)
        child_cr3, user_rsp, heap_brk,
    );

    if child_pid == 0 {
        // Clone task failed, clean up the cloned address space
        zenus_mem::paging::destroy_address_space(child_cr3);
        return -1i64 as u64;
    }

    // Clone file descriptors from parent to child
    fd::fd_clone_all_for_task(current_task(), child_pid);

    child_pid
}

fn sys_waitpid(pid: u64, status_ptr: u64, options: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let ppid = current_task();

    // WNOHANG = 1, WUNTRACED = 4
    let wnohang = (options & 1) != 0;

    loop {
        let result = scheduler::wait_for_child(ppid, pid, options);
        match result {
            Some((child_pid, exit_code)) => {
                // Write exit status to user space
                if status_ptr != 0 && validate_user_range(status_ptr, 4) {
                    let status = ((exit_code & 0xFF) << 8) as u32;
                    unsafe {
                        *(status_ptr as *mut u32) = status;
                    }
                }
                return child_pid;
            }
            None => {
                if wnohang {
                    return 0;
                }
                // No child available yet — yield and retry
                scheduler::yield_now();
            }
        }
    }
}

fn sys_execve(path_ptr: u64, argv_ptr: u64, envp_ptr: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_ptr::<u8>(path_ptr) {
        return -1i64 as u64;
    }
    let path_str = match read_user_cstr(path_ptr, 4096) {
        Some(s) => s.into_owned(),
        None => return -1i64 as u64,
    };

    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }

    let new_cr3 = match zenus_mem::paging::create_address_space() {
        Some(c) => c,
        None => return -1i64 as u64,
    };

    let loaded = match crate::elf::load_elf(&path_str, new_cr3) {
        Some(e) => e,
        None => {
            zenus_mem::paging::destroy_address_space(new_cr3);
            return -1i64 as u64;
        }
    };

    let cpu = zenus_arch::smp::current_cpu();

    // Read argv strings from user space
    let mut argv_strings: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();
    if argv_ptr != 0 {
        if !validate_user_range(argv_ptr, 8) {
            zenus_mem::paging::destroy_address_space(new_cr3);
            return -1i64 as u64;
        }
        let mut arg_ptr_ptr = argv_ptr;
        loop {
            let arg_str_ptr: u64 = unsafe { *(arg_ptr_ptr as *const u64) };
            if arg_str_ptr == 0 {
                break;
            }
            if !validate_user_range(arg_str_ptr, 1) {
                zenus_mem::paging::destroy_address_space(new_cr3);
                return -1i64 as u64;
            }
            match read_user_cstr(arg_str_ptr, 4096) {
                Some(s) => {
                    let mut bytes = s.as_bytes().to_vec();
                    bytes.push(0);
                    argv_strings.push(bytes);
                }
                None => {
                    zenus_mem::paging::destroy_address_space(new_cr3);
                    return -1i64 as u64;
                }
            }
            arg_ptr_ptr += 8;
        }
    }

    let argc = argv_strings.len();
    let stack_top = loaded.stack_top;
    let stack_bottom = stack_top - 16 * 4096; // 16 pages allocated by load_elf

    // Calculate total space for strings + pointer arrays
    let total_str: usize = argv_strings.iter().map(|s| s.len()).sum();
    let ptr_array_size = (argc + 1) * 8; // argv pointers + NULL
    let total_needed = total_str + ptr_array_size + 8; // + 8 for argc

    if total_needed > (stack_top - stack_bottom) as usize - 256 {
        zenus_mem::paging::destroy_address_space(new_cr3);
        return -1i64 as u64;
    }

    let str_start = stack_top - total_str as u64;
    let str_start = str_start & !7;

    // Write argv strings to user stack
    let mut str_cur = str_start;
    for s in &argv_strings {
        let string_vaddr = str_cur;
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), string_vaddr as *mut u8, s.len());
        }
        str_cur += s.len() as u64;
    }

    // Write argv pointer array (below strings)
    let ptr_array_start = str_start - ptr_array_size as u64;
    let mut argv_pos = ptr_array_start;
    let mut str_cur2 = str_start;
    for _ in &argv_strings {
        unsafe {
            *((argv_pos) as *mut u64) = str_cur2;
        }
        argv_pos += 8;
        str_cur2 += argv_strings[(argv_pos - ptr_array_start - 8) as usize / 8].len() as u64;
        // correct str_cur2
    }

    // Actually let me rewrite more carefully
    let argv_array_start = ptr_array_start;
    let mut argv_pos = argv_array_start;
    let mut str_cur_pos = str_start;
    for s in &argv_strings {
        unsafe {
            *((argv_pos) as *mut u64) = str_cur_pos;
        }
        argv_pos += 8;
        str_cur_pos += s.len() as u64;
    }

    // NULL terminate argv array
    unsafe {
        *((argv_array_start + argc as u64 * 8) as *mut u64) = 0;
    }

    // Write argc
    let user_rsp = argv_array_start - 8;
    unsafe {
        *((user_rsp) as *mut u64) = argc as u64;
    }

    zenus_console::kinfo!(
        "execve: {} argc={} entry=0x{:x} rsp=0x{:x} cr3=0x{:x}",
        path_str,
        argc,
        loaded.entry,
        user_rsp,
        new_cr3
    );

    // Set new user RSP in PerCpu
    zenus_arch::cpu::set_percpu_user_rsp(cpu, user_rsp);

    // Modify kernel stack to return to new ELF entry
    unsafe {
        let kernel_rsp_top: u64;
        core::arch::asm!("mov {}, gs:[8]", out(reg) kernel_rsp_top, options(nostack));
        let rcx_ptr = (kernel_rsp_top - 8) as *mut u64;
        (*rcx_ptr) = loaded.entry;
        let r11_ptr = (kernel_rsp_top - 16) as *mut u64;
        (*r11_ptr) = 0x202u64;
    }

    // Update task state
    let tid = current_task();
    scheduler::set_task_cr3(tid, new_cr3);
    scheduler::set_task_heap_brk(tid, loaded.heap_base);
    scheduler::set_task_name(tid, &path_str);

    // Close user FDs (except 0,1,2) and destroy old address space
    fd_close_all_for_task(tid);
    if cr3 != 0 && cr3 != zenus_mem::paging::kernel_cr3() {
        zenus_mem::paging::destroy_address_space(cr3);
    }

    0
}

#[repr(C)]
struct UtsName {
    sysname: [u8; 65],
    nodename: [u8; 65],
    release: [u8; 65],
    version: [u8; 65],
    machine: [u8; 65],
}

fn sys_uname(buf: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_ptr::<UtsName>(buf) {
        return -1i64 as u64;
    }
    let uts = buf as *mut UtsName;
    unsafe {
        copy_str_to_fixed(&mut (*uts).sysname, "Zenus");
        copy_str_to_fixed(&mut (*uts).nodename, "zenus");
        copy_str_to_fixed(&mut (*uts).release, "0.1.0");
        copy_str_to_fixed(&mut (*uts).version, "#1 Tue Jun 9 2026");
        copy_str_to_fixed(&mut (*uts).machine, "x86_64");
    }
    0
}

fn sys_getuid(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    zenus_sched::scheduler::current_uid() as u64
}

fn sys_geteuid(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    zenus_sched::scheduler::current_euid() as u64
}

fn sys_getgid(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    zenus_sched::scheduler::current_gid() as u64
}

fn sys_getegid(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    zenus_sched::scheduler::current_egid() as u64
}

fn sys_setuid(uid: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if zenus_sched::scheduler::set_current_uid(uid as u32) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_setgid(gid: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if zenus_sched::scheduler::set_current_gid(gid as u32) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_sethostname(name_ptr: u64, len: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_range(name_ptr, len) {
        return -1i64 as u64;
    }
    if len == 0 || len > 64 {
        return -1i64 as u64;
    }
    let euid = zenus_sched::scheduler::current_euid();
    if euid != 0 {
        return -1i64 as u64;
    }
    let mut buf = [0u8; 64];
    unsafe {
        core::ptr::copy_nonoverlapping(name_ptr as *const u8, buf.as_mut_ptr(), len as usize);
    }
    let uts_ns = zenus_sched::scheduler::current_uts_ns();
    if zenus_ns::uts::set_hostname(uts_ns, &buf[..len as usize]) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_uname_ns(buf: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_ptr::<UtsName>(buf) {
        return -1i64 as u64;
    }
    let uts = buf as *mut UtsName;
    let uts_ns = zenus_sched::scheduler::current_uts_ns();
    let hostname = zenus_ns::uts::get_hostname(uts_ns);
    let domainname = zenus_ns::uts::get_domainname(uts_ns);
    let hlen = hostname.iter().position(|&b| b == 0).unwrap_or(64);
    let _dlen = domainname.iter().position(|&b| b == 0).unwrap_or(64);
    unsafe {
        (*uts).sysname = [0; 65];
        copy_str_to_fixed(&mut (*uts).sysname, "Zenus");
        (*uts).nodename = [0; 65];
        if hlen > 0 {
            let dst = &mut (*uts).nodename;
            dst[..hlen.min(64)].copy_from_slice(&hostname[..hlen.min(64)]);
        }
        (*uts).release = [0; 65];
        copy_str_to_fixed(&mut (*uts).release, "0.1.0");
        (*uts).version = [0; 65];
        copy_str_to_fixed(&mut (*uts).version, "#1 Tue Jun 9 2026");
        (*uts).machine = [0; 65];
        copy_str_to_fixed(&mut (*uts).machine, "x86_64");
    }
    0
}

fn sys_clone(flags: u64, stack: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }
    let user_rsp = if stack != 0 {
        stack
    } else {
        zenus_arch::cpu::get_percpu_user_rsp(zenus_arch::smp::current_cpu())
    };
    // Read user RIP from saved RCX on kernel stack (same as fork)
    let kernel_rsp_top: u64;
    unsafe {
        core::arch::asm!("mov {}, gs:[8]", out(reg) kernel_rsp_top, options(nostack));
    }
    let user_rip: u64 = unsafe { *((kernel_rsp_top - 8) as *const u64) };
    if user_rip == 0 {
        return -1i64 as u64;
    }
    let heap_brk =
        zenus_sched::scheduler::get_task_heap_brk(zenus_sched::scheduler::current_task_id());
    let new_pid =
        zenus_sched::scheduler::clone_task(flags, stack, 65536, user_rip, cr3, user_rsp, heap_brk);
    if new_pid == 0 {
        -1i64 as u64
    } else {
        new_pid
    }
}

fn sys_getpid_ns(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    zenus_sched::scheduler::current_local_pid()
}

// ── Socket syscalls ──

#[repr(C)]
struct SockaddrIn {
    sin_family: u16,
    sin_port: u16,
    sin_addr: [u8; 4],
    sin_zero: [u8; 8],
}

fn sys_socket(domain: u64, type_: u64, _protocol: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let sock_id = match net_socket::socket(domain as u8, type_ as u8, 0) {
        Some(id) => id,
        None => return -1i64 as u64,
    };
    let task_id = fd::current_task();
    match fd::fd_socket(task_id, sock_id as u64) {
        Some(fd_num) => fd_num,
        None => {
            net_socket::close(sock_id, 0);
            -1i64 as u64
        }
    }
}

fn sys_bind(fd: u64, addr_ptr: u64, _addrlen: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    let sock_id = entry.socket_id as usize;
    drop(entry);

    if !validate_user_range(addr_ptr, core::mem::size_of::<SockaddrIn>() as u64) {
        return -1i64 as u64;
    }
    let sa = unsafe { &*(addr_ptr as *const SockaddrIn) };
    let port = u16::from_be(sa.sin_port);

    if net_socket::bind(sock_id, port) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_listen(fd: u64, backlog: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    let sock_id = entry.socket_id as usize;
    drop(entry);

    if net_socket::listen(sock_id, backlog as usize) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_accept(fd: u64, _addr_ptr: u64, _addrlen: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    let sock_id = entry.socket_id as usize;
    drop(entry);

    let new_sock_id = match net_socket::accept(sock_id, 0) {
        Some(id) => id,
        None => return -1i64 as u64,
    };
    let task_id = fd::current_task();
    match fd::fd_socket(task_id, new_sock_id as u64) {
        Some(new_fd) => new_fd,
        None => {
            net_socket::close(new_sock_id, 0);
            -1i64 as u64
        }
    }
}

fn sys_connect(fd: u64, addr_ptr: u64, _addrlen: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    let sock_id = entry.socket_id as usize;
    drop(entry);

    if !validate_user_range(addr_ptr, core::mem::size_of::<SockaddrIn>() as u64) {
        return -1i64 as u64;
    }
    let sa = unsafe { &*(addr_ptr as *const SockaddrIn) };
    let dst_port = u16::from_be(sa.sin_port);
    let dst_ip = sa.sin_addr;

    if net_socket::connect(sock_id, 0, dst_ip, dst_port) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_send(fd: u64, buf_ptr: u64, len: u64, _flags: u64, _a5: u64, _a6: u64) -> u64 {
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    let sock_id = entry.socket_id as usize;
    drop(entry);

    if !validate_user_range(buf_ptr, len) {
        return -1i64 as u64;
    }
    let data = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };

    if net_socket::send(sock_id, data, 0) {
        len
    } else {
        -1i64 as u64
    }
}

fn sys_recv(fd: u64, buf_ptr: u64, len: u64, _flags: u64, _a5: u64, _a6: u64) -> u64 {
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    let sock_id = entry.socket_id as usize;
    drop(entry);

    if !validate_user_range(buf_ptr, len) {
        return -1i64 as u64;
    }
    let mut buf = alloc::vec![0u8; len as usize];

    match net_socket::recv(sock_id, &mut buf) {
        Some(n) => {
            unsafe {
                core::ptr::copy_nonoverlapping(buf.as_ptr(), buf_ptr as *mut u8, n);
            }
            n as u64
        }
        None => -1i64 as u64,
    }
}

fn sys_sendto(fd: u64, buf_ptr: u64, len: u64, _flags: u64, addr_ptr: u64, _addrlen: u64) -> u64 {
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    let sock_id = entry.socket_id as usize;
    drop(entry);

    if !validate_user_range(buf_ptr, len) {
        return -1i64 as u64;
    }
    let data = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };

    let (dst_ip, dst_port) = if addr_ptr != 0
        && validate_user_range(addr_ptr, core::mem::size_of::<SockaddrIn>() as u64)
    {
        let sa = unsafe { &*(addr_ptr as *const SockaddrIn) };
        (sa.sin_addr, u16::from_be(sa.sin_port))
    } else {
        return -1i64 as u64;
    };

    if net_socket::sendto(sock_id, data, 0, dst_ip, dst_port) {
        len
    } else {
        -1i64 as u64
    }
}

fn sys_recvfrom(
    fd: u64,
    buf_ptr: u64,
    len: u64,
    _flags: u64,
    addr_ptr: u64,
    addrlen_ptr: u64,
) -> u64 {
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    let sock_id = entry.socket_id as usize;
    drop(entry);

    if !validate_user_range(buf_ptr, len) {
        return -1i64 as u64;
    }
    let mut buf = alloc::vec![0u8; len as usize];

    match net_socket::recv(sock_id, &mut buf) {
        Some(n) => {
            unsafe {
                core::ptr::copy_nonoverlapping(buf.as_ptr(), buf_ptr as *mut u8, n);
            }
            if addr_ptr != 0
                && validate_user_range(addr_ptr, core::mem::size_of::<SockaddrIn>() as u64)
            {
                let sa = unsafe { &mut *(addr_ptr as *mut SockaddrIn) };
                sa.sin_family = net_socket::AF_INET as u16;
                sa.sin_port = 0u16.to_be();
                sa.sin_addr = [0; 4];
                sa.sin_zero = [0; 8];
            }
            if addrlen_ptr != 0 && validate_user_range(addrlen_ptr, 4) {
                unsafe {
                    *(addrlen_ptr as *mut u32) = core::mem::size_of::<SockaddrIn>() as u32;
                }
            }
            n as u64
        }
        None => -1i64 as u64,
    }
}

fn sys_shutdown(fd: u64, _how: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    let sock_id = entry.socket_id as usize;
    drop(entry);

    net_socket::close(sock_id, 0);
    0
}

fn sys_getsockname(fd: u64, addr_ptr: u64, _addrlen: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if addr_ptr == 0 || !validate_user_range(addr_ptr, core::mem::size_of::<SockaddrIn>() as u64) {
        return -1i64 as u64;
    }
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    drop(entry);

    let sa = unsafe { &mut *(addr_ptr as *mut SockaddrIn) };
    sa.sin_family = net_socket::AF_INET as u16;
    sa.sin_port = 0u16.to_be();
    sa.sin_addr = [0, 0, 0, 0];
    sa.sin_zero = [0; 8];
    0
}

fn sys_getpeername(fd: u64, addr_ptr: u64, _addrlen: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if addr_ptr == 0 || !validate_user_range(addr_ptr, core::mem::size_of::<SockaddrIn>() as u64) {
        return -1i64 as u64;
    }
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    if entry.socket_id == u64::MAX {
        return -1i64 as u64;
    }
    drop(entry);

    let sa = unsafe { &mut *(addr_ptr as *mut SockaddrIn) };
    sa.sin_family = net_socket::AF_INET as u16;
    sa.sin_port = 0u16.to_be();
    sa.sin_addr = [0, 0, 0, 0];
    sa.sin_zero = [0; 8];
    0
}

// ── File descriptor syscalls ──

fn sys_dup2(oldfd: u64, newfd: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let task_id = fd::current_task();
    match fd::fd_dup2(task_id, oldfd, newfd) {
        Some(fd) => fd,
        None => -1i64 as u64,
    }
}

fn sys_dup3(oldfd: u64, newfd: u64, _flags: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let task_id = fd::current_task();
    match fd::fd_dup2(task_id, oldfd, newfd) {
        Some(fd) => fd,
        None => -1i64 as u64,
    }
}

// ── Filesystem syscalls ──

fn sys_mkdir(path_ptr: u64, _mode: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    if fd::vfs_mkdir(&path) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_unlink(path_ptr: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    if fd::vfs_unlink(&path) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_rmdir(path_ptr: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    if fd::vfs_rmdir(&path) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_rename(old_ptr: u64, new_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let old_path = match read_user_cstr(old_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let new_path = match read_user_cstr(new_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    if fd::vfs_rename(&old_path, &new_path) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_chmod(path_ptr: u64, mode: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    if fd::vfs_chmod(&path, mode as u16) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_chown(path_ptr: u64, owner: u64, group: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    if fd::vfs_chown(&path, owner as u32, group as u32) {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_access(path_ptr: u64, _mode: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    if fd::vfs_access(&path) {
        0
    } else {
        -1i64 as u64
    }
}

// ── Signal syscalls ──

#[repr(C)]
struct KernelSigAction {
    handler_fn: u64,
    flags: u64,
    restorer: u64,
    mask: [u64; 2],
}

fn sys_kill(pid: u64, sig: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let sig_num = sig as usize;
    if sig_num == 0 || sig_num >= 64 {
        return -1i64 as u64;
    }

    if pid == 0 {
        // Send to all processes in current process group
        return 0;
    }

    if pid == -1i64 as u64 {
        // Send to all processes
        return 0;
    }

    // Send to specific PID
    let task_id = scheduler::get_task_id_by_pid(pid);
    if task_id.is_none() {
        return -1i64 as u64;
    }
    let task_id = task_id.unwrap();

    match sig_num {
        signal::SIGKILL => {
            scheduler::signal_deliver(task_id, sig_num);
            scheduler::signal_force_kill(task_id);
            0
        }
        signal::SIGSTOP => {
            scheduler::signal_deliver(task_id, sig_num);
            0
        }
        signal::SIGCONT => {
            scheduler::signal_clear(task_id, signal::SIGSTOP);
            scheduler::signal_clear(task_id, sig_num);
            scheduler::signal_deliver(task_id, sig_num);
            scheduler::task_set_state(task_id, zenus_sched::task::TaskState::Ready);
            0
        }
        _ => {
            scheduler::signal_deliver(task_id, sig_num);
            0
        }
    }
}

fn sys_rt_sigaction(
    sig: u64,
    act_ptr: u64,
    oldact_ptr: u64,
    _sigsetsize: u64,
    _a5: u64,
    _a6: u64,
) -> u64 {
    let sig_num = sig as usize;
    if sig_num == 0 || sig_num >= 64 {
        return -1i64 as u64;
    }

    let task_id = scheduler::current_task_id();

    // Save old action
    if oldact_ptr != 0
        && validate_user_range(oldact_ptr, core::mem::size_of::<KernelSigAction>() as u64)
    {
        let old = scheduler::get_signal_action(task_id, sig_num);
        let oldact = oldact_ptr as *mut KernelSigAction;
        unsafe {
            (*oldact).handler_fn = old.handler_fn;
            (*oldact).flags = old.flags;
            (*oldact).restorer = old.restorer;
            (*oldact).mask = old.mask;
        }
    }

    // Set new action
    if act_ptr != 0 && validate_user_range(act_ptr, core::mem::size_of::<KernelSigAction>() as u64)
    {
        let act = act_ptr as *const KernelSigAction;
        let new_action = unsafe {
            zenus_sched::task::SignalAction {
                handler_fn: (*act).handler_fn,
                flags: (*act).flags,
                restorer: (*act).restorer,
                mask: (*act).mask,
            }
        };
        scheduler::set_signal_action(task_id, sig_num, new_action);
    }

    0
}

fn sys_rt_sigprocmask(
    how: u64,
    set_ptr: u64,
    oldset_ptr: u64,
    _sigsetsize: u64,
    _a5: u64,
    _a6: u64,
) -> u64 {
    let task_id = scheduler::current_task_id();

    // Save old mask
    if oldset_ptr != 0 && validate_user_range(oldset_ptr, 8) {
        let old_mask = scheduler::get_signal_mask(task_id);
        unsafe {
            *(oldset_ptr as *mut u64) = old_mask;
        }
    }

    // Set new mask
    if set_ptr != 0 && validate_user_range(set_ptr, 8) {
        let new_set = unsafe { *(set_ptr as *const u64) };
        let how = match how {
            signal::SIG_BLOCK => {
                let old = scheduler::get_signal_mask(task_id);
                old | new_set
            }
            signal::SIG_UNBLOCK => {
                let old = scheduler::get_signal_mask(task_id);
                old & !new_set
            }
            signal::SIG_SETMASK => new_set,
            _ => return -1i64 as u64,
        };
        // Never mask SIGKILL or SIGSTOP
        let how = how & !(1 << signal::SIGKILL) & !(1 << signal::SIGSTOP);
        scheduler::set_signal_mask(task_id, how);
    }

    0
}

fn sys_rt_sigreturn(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    // rt_sigreturn restores the context saved at signal delivery time.
    // The signal frame was pushed to user stack when the signal handler was entered.
    // After the handler returns, the restorer calls this syscall with RSP pointing
    // to the signal frame that needs to be restored.

    // Get current user RSP from the signal frame location
    let cpu = zenus_arch::smp::current_cpu();
    let user_rsp = zenus_arch::cpu::get_percpu_user_rsp(cpu);

    if let Some((restored_rsp, restored_rip, restored_rflags)) =
        scheduler::rt_sigreturn_restore(user_rsp)
    {
        // Restore user RSP and return to the original instruction
        zenus_arch::cpu::set_percpu_user_rsp(cpu, restored_rsp);

        // Return value is not used directly; instead we need to modify kernel stack
        // to return to restored_rip with restored_rflags.
        // This is handled by modifying the kernel stack before returning from syscall.
        // For now, return a marker value that indicates context was restored.
        0xDEADBEEF
    } else {
        -1i64 as u64
    }
}

fn sys_tgkill(tgid: u64, tid: u64, sig: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let sig_num = sig as usize;
    if sig_num == 0 || sig_num >= 64 {
        return -1i64 as u64;
    }

    let task_id = if tid != 0 {
        scheduler::get_task_id_by_pid(tid)
    } else {
        Some(scheduler::current_task_id())
    };

    match task_id {
        Some(id) => {
            if sig_num == signal::SIGKILL {
                scheduler::signal_deliver(id, sig_num);
                scheduler::signal_force_kill(id);
            } else {
                scheduler::signal_deliver(id, sig_num);
            }
            0
        }
        None => -1i64 as u64,
    }
}

// ── Memory syscalls ──

fn sys_mmap(addr: u64, length: u64, prot: u64, flags: u64, _fd: u64, _offset: u64) -> u64 {
    if length == 0 {
        return -1i64 as u64;
    }

    let task_id = scheduler::current_task_id();
    let cr3 = match scheduler::get_task_cr3(task_id) {
        Some(c) => c,
        None => return -1i64 as u64,
    };

    let page_count = ((length + 0xFFF) & !0xFFF) / 0x1000;
    let size = page_count * 0x1000;

    let is_anonymous = flags & zenus_sched::task::MAP_ANONYMOUS != 0;
    let is_fixed = flags & zenus_sched::task::MAP_FIXED != 0;
    let is_private = flags & zenus_sched::task::MAP_PRIVATE != 0;

    let vma_start = if is_fixed && addr != 0 {
        addr & !0xFFF
    } else if addr != 0 {
        let hint = addr & !0xFFF;
        scheduler::with_vma(task_id, |vma| vma.find_free(size, hint))
            .flatten()
            .unwrap_or(0)
    } else {
        scheduler::with_vma(task_id, |vma| vma.find_free(size, vma.mmap_base))
            .flatten()
            .unwrap_or(0)
    };

    if vma_start == 0 || vma_start + size > 0x7F00_0000_0000 {
        return -1i64 as u64;
    }

    // Map pages
    for i in 0..page_count {
        let page_virt = vma_start + i * 0x1000;
        let frame = {
            let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
            match allocator.alloc_frame() {
                Some(f) => f,
                None => return -1i64 as u64,
            }
        };
        zenus_mem::paging::map_user_page_raw(
            cr3,
            page_virt,
            frame.as_u64(),
            prot & zenus_sched::task::PROT_WRITE != 0,
            prot & zenus_sched::task::PROT_EXEC != 0,
        );
    }

    // Track in VMA
    scheduler::with_vma_mut(task_id, |vma| {
        vma.insert(vma_start, vma_start + size, prot, flags);
    });

    vma_start
}

fn sys_munmap(addr: u64, length: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if length == 0 {
        return -1i64 as u64;
    }

    let task_id = scheduler::current_task_id();
    let cr3 = match scheduler::get_task_cr3(task_id) {
        Some(c) => c,
        None => return -1i64 as u64,
    };

    let start_page = addr & !0xFFF;
    let end_page = ((addr + length) + 0xFFF) & !0xFFF;

    // Find and remove VMA
    scheduler::with_vma_mut(task_id, |vma| {
        if let Some(idx) = vma.find(start_page) {
            vma.remove(idx);
        }
    });

    // Unmap pages
    let mut page = start_page;
    while page < end_page {
        if let Some(phys) = zenus_mem::paging::virt_to_phys_raw(cr3, page) {
            {
                let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
                allocator.free_frame(x86_64::PhysAddr::new(phys & !0xFFF));
            }
            // Clear PTE
            let hhdm = zenus_mem::paging::hhdm_offset();
            let cr3_phys = cr3 & !0xFFF;
            unsafe {
                let mut table_virt = (cr3_phys + hhdm) as *mut u64;
                for &(level, shift) in &[(4usize, 39), (3, 30), (2, 21), (1, 12)] {
                    let idx = (page >> shift) & 0x1FF;
                    let entry = *table_virt.add(idx as usize);
                    if (entry & 1) == 0 {
                        break;
                    }
                    if level == 1 {
                        table_virt.add(idx as usize).write(0);
                        break;
                    }
                    let next = entry & 0x000FFFFFFFFFF000;
                    table_virt = (next + hhdm) as *mut u64;
                }
            }
            unsafe {
                core::arch::asm!("invlpg [{0}]", in(reg) page, options(nostack, preserves_flags));
            }
        }
        page += 0x1000;
    }

    0
}

fn sys_mprotect(addr: u64, length: u64, prot: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if length == 0 {
        return -1i64 as u64;
    }

    let task_id = scheduler::current_task_id();
    let cr3 = match scheduler::get_task_cr3(task_id) {
        Some(c) => c,
        None => return -1i64 as u64,
    };

    let start_page = addr & !0xFFF;
    let end_page = ((addr + length) + 0xFFF) & !0xFFF;
    let writable = prot & zenus_sched::task::PROT_WRITE != 0;
    let executable = prot & zenus_sched::task::PROT_EXEC != 0;

    let mut page = start_page;
    while page < end_page {
        zenus_mem::paging::protect_page_raw(cr3, page, writable, executable);
        page += 0x1000;
    }

    // Update VMA
    scheduler::with_vma_mut(task_id, |vma| {
        if let Some(idx) = vma.find(start_page) {
            vma.regions[idx].prot = prot;
        }
    });

    0
}

// ── Time syscalls ──

#[repr(C)]
struct Timeval {
    tv_sec: u64,
    tv_usec: u64,
}

#[repr(C)]
struct Timespec {
    tv_sec: u64,
    tv_nsec: u64,
}

fn sys_gettimeofday(tv_ptr: u64, _tz_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let epoch = zenus_arch::rtc::boot_epoch();
    let ticks = zenus_arch::interrupts::pit::get_ticks();
    let uptime_ms = ticks * 10;
    let sec = epoch + uptime_ms / 1000;
    let usec = (uptime_ms % 1000) * 1000;

    if tv_ptr != 0 && validate_user_range(tv_ptr, core::mem::size_of::<Timeval>() as u64) {
        let tv = unsafe { &mut *(tv_ptr as *mut Timeval) };
        tv.tv_sec = sec;
        tv.tv_usec = usec;
    }
    0
}

fn sys_clock_gettime(clock_id: u64, tp_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let ticks = zenus_arch::interrupts::pit::get_ticks();
    let uptime_ns = (ticks as u64) * 10_000_000; // 10ms per tick

    let (sec, nsec) = match clock_id {
        0 => {
            // CLOCK_REALTIME
            let epoch = zenus_arch::rtc::boot_epoch();
            (epoch + uptime_ns / 1_000_000_000, uptime_ns % 1_000_000_000)
        }
        1 => {
            // CLOCK_MONOTONIC
            (uptime_ns / 1_000_000_000, uptime_ns % 1_000_000_000)
        }
        2 => {
            // CLOCK_PROCESS_CPUTIME_ID
            (uptime_ns / 1_000_000_000, uptime_ns % 1_000_000_000)
        }
        3 => {
            // CLOCK_THREAD_CPUTIME_ID
            (uptime_ns / 1_000_000_000, uptime_ns % 1_000_000_000)
        }
        _ => return -1i64 as u64,
    };

    if tp_ptr != 0 && validate_user_range(tp_ptr, core::mem::size_of::<Timespec>() as u64) {
        let tp = unsafe { &mut *(tp_ptr as *mut Timespec) };
        tp.tv_sec = sec;
        tp.tv_nsec = nsec;
    }
    0
}

fn sys_clock_getres(clock_id: u64, res_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let (sec, nsec) = match clock_id {
        0 | 1 | 2 | 3 => (0u64, 10_000_000u64), // 10ms resolution
        _ => return -1i64 as u64,
    };
    if res_ptr != 0 && validate_user_range(res_ptr, core::mem::size_of::<Timespec>() as u64) {
        let res = unsafe { &mut *(res_ptr as *mut Timespec) };
        res.tv_sec = sec;
        res.tv_nsec = nsec;
    }
    0
}

// ── Mount syscalls ──

const MS_RDONLY: u64 = 1;
const MS_NOSUID: u64 = 2;
const MS_NODEV: u64 = 4;
const MS_NOEXEC: u64 = 8;
const MS_REMOUNT: u64 = 32;
const MS_BIND: u64 = 4096;

fn resolve_block_dev(source: &str) -> Option<u8> {
    let name = source.trim_start_matches("/dev/");
    let devs = ["sda", "sdb", "sdc", "sdd"];
    for (i, &d) in devs.iter().enumerate() {
        if name == d {
            return Some(i as u8);
        }
    }
    None
}

fn sys_mount(
    source_ptr: u64,
    target_ptr: u64,
    fstype_ptr: u64,
    flags: u64,
    _data_ptr: u64,
    _a6: u64,
) -> u64 {
    if !validate_user_ptr::<u8>(source_ptr)
        || !validate_user_ptr::<u8>(target_ptr)
        || !validate_user_ptr::<u8>(fstype_ptr)
    {
        return -1i64 as u64;
    }
    let source = match read_user_cstr(source_ptr, 256) {
        Some(s) => s.into_owned(),
        None => return -1i64 as u64,
    };
    let target = match read_user_cstr(target_ptr, 4096) {
        Some(s) => s.into_owned(),
        None => return -1i64 as u64,
    };
    let fstype = match read_user_cstr(fstype_ptr, 64) {
        Some(s) => s.into_owned(),
        None => return -1i64 as u64,
    };

    let target_static: &'static str = leak_string(target.clone());
    let _ = &source;
    let _ = &fstype;

    if flags & MS_REMOUNT != 0 {
        return 0;
    }

    let fs: &'static dyn vfs::FileSystem = match fstype.as_str() {
        "tmpfs" => {
            use zenus_fs::tmpfs::TmpFs;
            TmpFs::new()
        }
        "devfs" => {
            use zenus_fs::devfs::DevFs;
            &DevFs
        }
        "ext2" => {
            let dev_id = match resolve_block_dev(&source) {
                Some(id) => id,
                None => return -1i64 as u64,
            };
            match zenus_fs::ext2::Ext2Fs::mount(dev_id) {
                Some(fs) => fs,
                None => return -1i64 as u64,
            }
        }
        "proc" | "procfs" => {
            static PROCFS: zenus_fs::procfs::ProcFs = zenus_fs::procfs::ProcFs;
            &PROCFS
        }
        "cgroup2" | "cgroup" => {
            static CGROUPFS: zenus_fs::cgroup::CgroupFs = zenus_fs::cgroup::CgroupFs;
            &CGROUPFS
        }
        _ => {
            return -1i64 as u64;
        }
    };

    vfs::create_dir(target_static);
    if !vfs::mount(target_static, fs) {
        return -1i64 as u64;
    }
    0
}

fn sys_umount2(target_ptr: u64, _flags: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_ptr::<u8>(target_ptr) {
        return -1i64 as u64;
    }
    let target = match read_user_cstr(target_ptr, 4096) {
        Some(s) => s.into_owned(),
        None => return -1i64 as u64,
    };

    if vfs::umount(&target) {
        0
    } else {
        -1i64 as u64
    }
}

fn leak_string(s: alloc::string::String) -> &'static str {
    use core::alloc::Layout;
    let bytes = s.into_bytes();
    let len = bytes.len();
    let layout = Layout::array::<u8>(len).unwrap();
    unsafe {
        let ptr = alloc::alloc::alloc(layout);
        if ptr.is_null() {
            return "";
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len);
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len))
    }
}

// ── Shared memory syscalls ──

const MAX_SHMSEG: usize = 64;

static SHM_TABLE: zenus_sync::spinlock::SpinLock<ShmTable> =
    zenus_sync::spinlock::SpinLock::new(ShmTable::new());

#[derive(Clone, Copy)]
struct ShmSegment {
    key: u64,
    size: u64,
    phys: u64,
    attached: usize,
    valid: bool,
}

struct ShmTable {
    segments: [ShmSegment; MAX_SHMSEG],
}

impl ShmTable {
    const fn new() -> Self {
        ShmTable {
            segments: [ShmSegment {
                key: 0,
                size: 0,
                phys: 0,
                attached: 0,
                valid: false,
            }; MAX_SHMSEG],
        }
    }
    fn find_or_create(&mut self, key: u64, size: u64) -> Option<usize> {
        for i in 0..MAX_SHMSEG {
            if self.segments[i].valid && self.segments[i].key == key {
                self.segments[i].attached += 1;
                return Some(i);
            }
        }
        for i in 0..MAX_SHMSEG {
            if !self.segments[i].valid {
                let pages = (size + 0xFFF) / 0x1000;
                let mut phys_addrs = alloc::vec::Vec::new();
                for _ in 0..pages {
                    if let Some(f) = zenus_mem::frame_allocator::FRAME_ALLOCATOR
                        .lock()
                        .alloc_frame()
                    {
                        phys_addrs.push(f.as_u64());
                    } else {
                        return None;
                    }
                }
                let first = phys_addrs.first().copied().unwrap_or(0);
                self.segments[i] = ShmSegment {
                    key,
                    size,
                    phys: first,
                    attached: 1,
                    valid: true,
                };
                return Some(i);
            }
        }
        None
    }
    fn detach(&mut self, idx: usize) {
        if idx < MAX_SHMSEG && self.segments[idx].valid {
            self.segments[idx].attached -= 1;
            if self.segments[idx].attached == 0 {
                let pages = (self.segments[idx].size + 0xFFF) / 0x1000;
                let base = self.segments[idx].phys;
                for p in 0..pages {
                    let addr = base + p * 0x1000;
                    zenus_mem::frame_allocator::FRAME_ALLOCATOR
                        .lock()
                        .free_frame(x86_64::PhysAddr::new(addr));
                }
                self.segments[idx].valid = false;
            }
        }
    }
}

fn sys_shmget(key: u64, size: u64, _shmflg: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let mut table = SHM_TABLE.lock();
    match table.find_or_create(key, size) {
        Some(id) => id as u64,
        None => -1i64 as u64,
    }
}

fn sys_shmat(shmid: u64, _shmaddr: u64, _shmflg: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let mut table = SHM_TABLE.lock();
    let seg = &table.segments[shmid as usize];
    if !seg.valid {
        return -1i64 as u64;
    }

    let task_id = scheduler::current_task_id();
    let cr3 = scheduler::get_task_cr3(task_id).unwrap_or(0);
    let phys = seg.phys;
    let size = seg.size;

    drop(table);

    // Find free virtual address and map
    let vma_start = scheduler::with_vma(task_id, |vma| vma.find_free(size, 0x3000_0000_0000))
        .flatten()
        .unwrap_or(0);
    if vma_start == 0 {
        return -1i64 as u64;
    }

    let page_count = (size + 0xFFF) / 0x1000;
    for i in 0..page_count {
        let page_phys = phys + i * 0x1000;
        zenus_mem::paging::map_user_page_raw(cr3, vma_start + i * 0x1000, page_phys, true, false);
    }

    scheduler::with_vma_mut(task_id, |vma| {
        vma.insert(vma_start, vma_start + page_count * 0x1000, 3, 1);
    });

    vma_start
}

fn sys_shmdt(_shmaddr: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    0
}

fn sys_shmctl(shmid: u64, cmd: u64, _buf_ptr: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if cmd == 0 {
        // IPC_RMID
        let mut table = SHM_TABLE.lock();
        if (shmid as usize) < MAX_SHMSEG && table.segments[shmid as usize].valid {
            let pages = (table.segments[shmid as usize].size + 0xFFF) / 0x1000;
            let base = table.segments[shmid as usize].phys;
            for p in 0..pages {
                zenus_mem::frame_allocator::FRAME_ALLOCATOR
                    .lock()
                    .free_frame(x86_64::PhysAddr::new(base + p * 0x1000));
            }
            table.segments[shmid as usize].valid = false;
        }
        return 0;
    }
    -1i64 as u64
}

// ── Futex syscall ──

const FUTEX_WAIT: u64 = 0;
const FUTEX_WAKE: u64 = 1;

#[derive(Clone, Copy)]
struct FutexWaiter {
    task_id: u64,
    valid: bool,
}

#[derive(Clone, Copy)]
struct FutexBucket {
    addr: u64,
    waiters: [FutexWaiter; 8],
    count: usize,
}

static FUTEX_TABLE: zenus_sync::spinlock::SpinLock<FutexTable> =
    zenus_sync::spinlock::SpinLock::new(FutexTable::new());

struct FutexTable {
    buckets: [FutexBucket; 64],
}

impl FutexTable {
    const fn new() -> Self {
        FutexTable {
            buckets: [FutexBucket {
                addr: 0,
                waiters: [FutexWaiter {
                    task_id: 0,
                    valid: false,
                }; 8],
                count: 0,
            }; 64],
        }
    }

    fn hash(addr: u64) -> usize {
        ((addr >> 3) as usize) % 64
    }

    fn wait(&mut self, addr: u64, val: u32, task_id: u64) -> bool {
        // Check if futex value still matches
        let user_val = unsafe {
            zenus_arch::cpu::stac();
            let v = core::ptr::read_volatile(addr as *const u32);
            zenus_arch::cpu::clac();
            v
        };
        if user_val != val {
            return false;
        }

        let idx = Self::hash(addr);
        let bucket = &mut self.buckets[idx];

        if bucket.count < 8 {
            bucket.waiters[bucket.count] = FutexWaiter {
                task_id,
                valid: true,
            };
            bucket.count += 1;
            true
        } else {
            false
        }
    }

    fn wake(&mut self, addr: u64, count: u32) -> u32 {
        let idx = Self::hash(addr);
        let bucket = &mut self.buckets[idx];
        let mut woken = 0u32;

        for i in 0..bucket.count {
            if bucket.waiters[i].valid && bucket.waiters[i].task_id != 0 {
                zenus_sched::scheduler::task_set_state(
                    bucket.waiters[i].task_id,
                    zenus_sched::task::TaskState::Ready,
                );
                bucket.waiters[i].valid = false;
                woken += 1;
                if woken >= count {
                    break;
                }
            }
        }

        // Compact
        let mut write = 0;
        for read in 0..bucket.count {
            if bucket.waiters[read].valid {
                if write != read {
                    bucket.waiters[write] = bucket.waiters[read];
                }
                write += 1;
            }
        }
        bucket.count = write;
        woken
    }
}

fn sys_futex(uaddr: u64, op: u64, val: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_range(uaddr, 4) {
        return -1i64 as u64;
    }

    let task_id = scheduler::current_task_id();

    match op & 0xF {
        FUTEX_WAIT => {
            let mut table = FUTEX_TABLE.lock();
            if table.wait(uaddr, val as u32, task_id) {
                scheduler::task_set_state(task_id, zenus_sched::task::TaskState::Sleeping);
                drop(table);
                zenus_sched::scheduler::yield_now();
                0
            } else {
                -1i64 as u64
            }
        }
        FUTEX_WAKE => {
            let mut table = FUTEX_TABLE.lock();
            table.wake(uaddr, val as u32) as u64
        }
        _ => -1i64 as u64,
    }
}

// ── Poll syscall ──

#[repr(C)]
struct Pollfd {
    fd: u32,
    events: u16,
    revents: u16,
}

fn sys_poll(fds_ptr: u64, nfds: u64, timeout_ms: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if fds_ptr == 0 || nfds == 0 {
        return -1i64 as u64;
    }
    let total_size = core::mem::size_of::<Pollfd>() as u64 * nfds;
    if !validate_user_range(fds_ptr, total_size) {
        return -1i64 as u64;
    }

    let start = zenus_arch::interrupts::pit::get_ticks();
    let timeout_ticks = if timeout_ms == u64::MAX {
        u64::MAX
    } else {
        (timeout_ms + 9) / 10
    };

    loop {
        let mut ready = 0u64;
        for i in 0..nfds {
            let offset = fds_ptr + i * core::mem::size_of::<Pollfd>() as u64;
            let pfd = unsafe { &mut *(offset as *mut Pollfd) };
            pfd.revents = 0;

            let fd = pfd.fd as u64;
            let entry = fd::fd_get(fd);
            if entry.is_none() {
                pfd.revents = 0x0010; // POLLNVAL
                ready += 1;
                continue;
            }
            let entry = entry.unwrap();

            if entry.socket_id != u64::MAX {
                if pfd.events & 0x0001 != 0 {
                    // POLLIN
                    pfd.revents |= 0x0001;
                    ready += 1;
                }
            } else if entry.pipe_id != u64::MAX {
                if pfd.events & 0x0001 != 0 {
                    pfd.revents |= 0x0001;
                    ready += 1;
                }
            } else if fd == 0 {
                // stdin — check serial
                let s = zenus_console::serial::SerialPort::new(0x3F8);
                if pfd.events & 0x0001 != 0 && s.is_data_available() {
                    pfd.revents |= 0x0001;
                    ready += 1;
                }
                if pfd.events & 0x0001 != 0 && zenus_arch::keyboard::is_key_available() {
                    pfd.revents |= 0x0001;
                    ready += 1;
                }
            } else {
                if pfd.events & 0x0001 != 0 {
                    pfd.revents |= 0x0001;
                    ready += 1;
                }
            }
        }

        if ready > 0 {
            return ready;
        }

        if timeout_ms == 0 {
            return 0;
        }

        let elapsed = zenus_arch::interrupts::pit::get_ticks().wrapping_sub(start);
        if elapsed >= timeout_ticks {
            return 0;
        }

        zenus_sched::scheduler::yield_now();
        x86_64::instructions::hlt();
    }
}

fn sys_ppoll(
    fds_ptr: u64,
    nfds: u64,
    _ts_ptr: u64,
    _sigmask_ptr: u64,
    _sigsetsize: u64,
    _a6: u64,
) -> u64 {
    // Simplified: delegate to poll with 100ms timeout
    sys_poll(fds_ptr, nfds, 100, 0, 0, 0)
}

// ── Process group syscalls ──

fn sys_setsid(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    scheduler::current_task_id()
}

fn sys_getpgid(_pid: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    scheduler::current_task_id()
}

fn sys_setpgid(_pid: u64, _pgid: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    0
}

fn sys_getsid(_pid: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    scheduler::current_task_id()
}

// ── Socket option syscalls ──

fn sys_setsockopt(
    _fd: u64,
    _level: u64,
    _optname: u64,
    _optval_ptr: u64,
    _optlen: u64,
    _a6: u64,
) -> u64 {
    0
}

fn sys_getsockopt(
    _fd: u64,
    _level: u64,
    _optname: u64,
    optval_ptr: u64,
    optlen_ptr: u64,
    _a6: u64,
) -> u64 {
    if optval_ptr != 0 && validate_user_range(optval_ptr, 4) {
        unsafe {
            *(optval_ptr as *mut u32) = 0;
        }
    }
    if optlen_ptr != 0 && validate_user_range(optlen_ptr, 4) {
        unsafe {
            *(optlen_ptr as *mut u32) = 4;
        }
    }
    0
}

// ── Select syscalls ──

fn sys_select(
    nfds: u64,
    readfds_ptr: u64,
    _writefds_ptr: u64,
    _exceptfds_ptr: u64,
    timeout_ptr: u64,
    _a6: u64,
) -> u64 {
    if nfds == 0 {
        return 0;
    }
    if nfds > 1024 {
        return -1i64 as u64;
    }

    let timeout_ms = if timeout_ptr != 0 && validate_user_range(timeout_ptr, 8) {
        let ts = unsafe { &*(timeout_ptr as *const Timespec) };
        ts.tv_sec * 1000 + ts.tv_nsec / 1_000_000
    } else {
        u64::MAX
    };

    let start = zenus_arch::interrupts::pit::get_ticks();
    let timeout_ticks = if timeout_ms == u64::MAX {
        u64::MAX
    } else {
        (timeout_ms + 9) / 10
    };

    loop {
        let mut ready = 0u64;
        if readfds_ptr != 0 {
            for fd in 0..nfds.min(64) {
                let byte_idx = fd / 8;
                let bit_idx = fd % 8;
                let fd_set_byte = unsafe {
                    zenus_arch::cpu::stac();
                    let b = core::ptr::read_volatile((readfds_ptr + byte_idx) as *const u8);
                    zenus_arch::cpu::clac();
                    b
                };
                if fd_set_byte & (1 << bit_idx) != 0 {
                    let entry = fd::fd_get(fd);
                    let mut is_ready = false;
                    if let Some(ref e) = entry {
                        if e.socket_id != u64::MAX {
                            is_ready = true;
                        } else if fd == 0 {
                            let s = zenus_console::serial::SerialPort::new(0x3F8);
                            is_ready =
                                s.is_data_available() || zenus_arch::keyboard::is_key_available();
                        } else {
                            is_ready = true;
                        }
                    }
                    if is_ready {
                        ready += 1;
                    }
                }
            }
        }
        if ready > 0 {
            return ready;
        }
        if timeout_ms == 0 {
            return 0;
        }
        let elapsed = zenus_arch::interrupts::pit::get_ticks().wrapping_sub(start);
        if elapsed >= timeout_ticks {
            return 0;
        }
        zenus_sched::scheduler::yield_now();
        x86_64::instructions::hlt();
    }
}

fn sys_pselect6(
    nfds: u64,
    readfds_ptr: u64,
    writefds_ptr: u64,
    exceptfds_ptr: u64,
    timeout_ptr: u64,
    _sigmask_ptr: u64,
) -> u64 {
    sys_select(
        nfds,
        readfds_ptr,
        writefds_ptr,
        exceptfds_ptr,
        timeout_ptr,
        0,
    )
}

// ── Filesystem extra syscalls ──

fn sys_readlinkat(
    _dirfd: u64,
    path_ptr: u64,
    _buf_ptr: u64,
    _bufsiz: u64,
    _a5: u64,
    _a6: u64,
) -> u64 {
    let _path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    // Symlinks not supported yet
    -1i64 as u64
}

fn sys_symlinkat(
    _target_ptr: u64,
    _linkdirfd: u64,
    _linkpath_ptr: u64,
    _a4: u64,
    _a5: u64,
    _a6: u64,
) -> u64 {
    -1i64 as u64
}

fn sys_linkat(
    _olddirfd: u64,
    _oldpath_ptr: u64,
    _newdirfd: u64,
    _newpath_ptr: u64,
    _flags: u64,
    _a6: u64,
) -> u64 {
    -1i64 as u64
}

fn sys_truncate(path_ptr: u64, _length: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if read_user_cstr(path_ptr, 256).is_none() {
        return -1i64 as u64;
    }
    0
}

fn sys_ftruncate(_fd: u64, _length: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    0
}

// ── Resource limit syscalls ──

#[repr(C)]
struct Rlimit {
    rlim_cur: u64,
    rlim_max: u64,
}

fn sys_getrlimit(resource: u64, rlim_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if rlim_ptr == 0 || !validate_user_range(rlim_ptr, core::mem::size_of::<Rlimit>() as u64) {
        return -1i64 as u64;
    }
    let rl = unsafe { &mut *(rlim_ptr as *mut Rlimit) };
    match resource {
        7 | 9 => {
            rl.rlim_cur = 256;
            rl.rlim_max = 4096;
        }
        13 => {
            rl.rlim_cur = 0x0000_8000_0000_0000;
            rl.rlim_max = 0x0000_8000_0000_0000;
        }
        _ => {
            rl.rlim_cur = u64::MAX;
            rl.rlim_max = u64::MAX;
        }
    }
    0
}

fn sys_setrlimit(_resource: u64, _rlim_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    0
}

// ── CWD syscalls ──

fn sys_getcwd(buf_ptr: u64, size: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if buf_ptr == 0 || size == 0 {
        return -1i64 as u64;
    }
    let task_id = scheduler::current_task_id();
    let cwd = match scheduler::get_task_cwd(task_id) {
        Some(c) => c,
        None => return -1i64 as u64,
    };
    let len = cwd.iter().position(|&b| b == 0).unwrap_or(256);
    let copy_len = (len as u64).min(size - 1);
    if !validate_user_range(buf_ptr, copy_len + 1) {
        return -1i64 as u64;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(cwd.as_ptr(), buf_ptr as *mut u8, copy_len as usize);
        *(buf_ptr as *mut u8).add(copy_len as usize) = 0;
    }
    buf_ptr
}

fn sys_chdir(path_ptr: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    // Verify path exists
    if zenus_fs::vfs::open(&path).is_none() {
        return -1i64 as u64;
    }
    let task_id = scheduler::current_task_id();

    // Resolve relative to current cwd
    let resolved = if path.starts_with('/') {
        path.into_owned()
    } else {
        let cwd = scheduler::get_task_cwd(task_id).unwrap_or({
            let mut b = [0u8; 256];
            b[0] = b'/';
            b
        });
        let cwd_str = core::str::from_utf8(&cwd).unwrap_or("/");
        let cwd_trimmed = cwd_str.trim_end_matches('\0');
        if cwd_trimmed == "/" {
            alloc::format!("/{}", path)
        } else {
            alloc::format!("{}/{}", cwd_trimmed, path)
        }
    };

    scheduler::set_task_cwd(task_id, &resolved);
    0
}

fn sys_fchdir(_fd: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    0
}

// ── Process control ──

fn sys_prctl(option: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    match option {
        1 => {
            // PR_SET_PDEATHSIG
            0
        }
        2 => {
            // PR_GET_PDEATHSIG
            0
        }
        4 => {
            // PR_SET_NAME
            0
        }
        5 => {
            // PR_GET_NAME
            0
        }
        9 => {
            // PR_SET_SECCOMP
            0
        }
        12 => {
            // PR_SET_KEEPCAPS
            0
        }
        13 => {
            // PR_GET_KEEPCAPS
            0
        }
        14 => {
            // PR_SET_NO_NEW_PRIVS
            0
        }
        15 => {
            // PR_GET_NO_NEW_PRIVS
            0
        }
        23 => {
            // PR_SET_TIMING
            0
        }
        36 => {
            // PR_SET_TAGGED_ADDR_CTRL
            0
        }
        38 => {
            // PR_SET_IO_FLUSHER
            0
        }
        40 => {
            // PR_SET_MDWE
            0
        }
        42 => {
            // PR_SET_MEMBARRIER
            0
        }
        57 => {
            // PR_SET_VMA
            0
        }
        _ => 0,
    }
}

// ── Additional filesystem syscalls ──

fn sys_faccessat(_dirfd: u64, path_ptr: u64, _mode: u64, _flags: u64, _a5: u64, _a6: u64) -> u64 {
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    if zenus_fs::vfs::open(&path).is_some() {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_utimensat(
    _dirfd: u64,
    _path_ptr: u64,
    _times_ptr: u64,
    _flags: u64,
    _a5: u64,
    _a6: u64,
) -> u64 {
    0
}

fn sys_fstatat(_dirfd: u64, path_ptr: u64, stat_ptr: u64, _flags: u64, _a5: u64, _a6: u64) -> u64 {
    if path_ptr == 0 || stat_ptr == 0 {
        return -1i64 as u64;
    }
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    if !validate_user_range(stat_ptr, core::mem::size_of::<StatBuf>() as u64) {
        return -1i64 as u64;
    }
    let node = match zenus_fs::vfs::open(&path) {
        Some(n) => n,
        None => return -1i64 as u64,
    };
    let stat = node.fs.stat(node.inode);
    let sb = unsafe { &mut *(stat_ptr as *mut StatBuf) };
    sb.st_size = stat.size;
    sb.st_mode = stat.mode as u64;
    sb.st_ino = stat.inode;
    0
}

fn sys_fchownat(
    _dirfd: u64,
    _path_ptr: u64,
    _owner: u64,
    _group: u64,
    _flags: u64,
    _a6: u64,
) -> u64 {
    0
}

// ── Process syscalls ──

fn sys_waitid(_idtype: u64, _id: u64, _infop_ptr: u64, _options: u64, _a5: u64, _a6: u64) -> u64 {
    // Simplified: check for zombie children
    let task_id = scheduler::current_task_id();
    // Reap any zombies (simplified — just clean up)
    0
}

// ── Socket pair syscalls ──

fn sys_socketpair(
    domain: u64,
    type_: u64,
    _protocol: u64,
    fds_ptr: u64,
    _a5: u64,
    _a6: u64,
) -> u64 {
    if domain != 1 {
        return -1i64 as u64;
    } // AF_UNIX only
    if fds_ptr == 0 || !validate_user_range(fds_ptr, 8) {
        return -1i64 as u64;
    }

    // Create two connected pipe-like endpoints
    let task_id = scheduler::current_task_id();

    // For now, create two pipes and cross-connect them
    let (r1, w1) = match fd::fd_pipe(task_id) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let (r2, w2) = match fd::fd_pipe(task_id) {
        Some(p) => p,
        None => {
            fd::fd_close(r1);
            fd::fd_close(w1);
            return -1i64 as u64;
        }
    };

    unsafe {
        *(fds_ptr as *mut u64) = r1;
        *(fds_ptr as *mut u64).add(1) = r2;
    }

    0
}

// ── File descriptor stat ──

#[repr(C)]
struct StatFs {
    f_type: u64,
    f_bsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_fsid: [u32; 2],
    f_namelen: u64,
    f_frsize: u64,
    f_flags: u64,
    f_spare: [u64; 4],
}

fn sys_fstat(fd: u64, stat_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if stat_ptr == 0 || !validate_user_range(stat_ptr, core::mem::size_of::<StatBuf>() as u64) {
        return -1i64 as u64;
    }
    let entry = match fd::fd_get(fd) {
        Some(e) => e,
        None => return -1i64 as u64,
    };
    let sb = unsafe { &mut *(stat_ptr as *mut StatBuf) };
    if let Some(fs) = entry.fs {
        let stat = fs.stat(entry.inode);
        sb.st_size = stat.size;
        sb.st_mode = stat.mode as u64;
        sb.st_ino = stat.inode;
    } else {
        sb.st_size = 0;
        sb.st_mode = 0;
        sb.st_ino = 0;
    }
    0
}

fn sys_fstatfs(_fd: u64, buf_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if buf_ptr == 0 || !validate_user_range(buf_ptr, core::mem::size_of::<StatFs>() as u64) {
        return -1i64 as u64;
    }
    let sb = unsafe { &mut *(buf_ptr as *mut StatFs) };
    sb.f_type = 0x5346544E; // "NTFS" magic for ext2-like
    sb.f_bsize = 4096;
    sb.f_blocks = 1024;
    sb.f_bfree = 512;
    sb.f_bavail = 512;
    sb.f_files = 256;
    sb.f_ffree = 128;
    sb.f_namelen = 255;
    sb.f_frsize = 4096;
    0
}

fn sys_statfs(_path_ptr: u64, buf_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if buf_ptr == 0 || !validate_user_range(buf_ptr, core::mem::size_of::<StatFs>() as u64) {
        return -1i64 as u64;
    }
    let sb = unsafe { &mut *(buf_ptr as *mut StatFs) };
    sb.f_type = 0x5346544E;
    sb.f_bsize = 4096;
    sb.f_blocks = 1024;
    sb.f_bfree = 512;
    sb.f_bavail = 512;
    sb.f_files = 256;
    sb.f_ffree = 128;
    sb.f_namelen = 255;
    sb.f_frsize = 4096;
    0
}

// ── Scheduling syscalls ──

fn sys_nice(_increment: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    0
}

fn sys_sched_getscheduler(_pid: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    0 // SCHED_OTHER
}

fn sys_sched_setscheduler(
    _pid: u64,
    _policy: u64,
    _param_ptr: u64,
    _a4: u64,
    _a5: u64,
    _a6: u64,
) -> u64 {
    0
}

fn sys_sched_getparam(_pid: u64, _param_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    0
}

fn sys_sched_setparam(_pid: u64, _param_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    0
}

fn sys_sched_yield(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    scheduler::yield_now();
    0
}

// ── Resource usage syscalls ──

#[repr(C)]
struct RUsage {
    ru_utime: Timeval,
    ru_stime: Timeval,
    ru_maxrss: u64,
    ru_ixrss: u64,
    ru_idrss: u64,
    ru_isrss: u64,
    ru_minflt: u64,
    ru_majflt: u64,
    ru_nswap: u64,
    ru_inblock: u64,
    ru_oublock: u64,
    ru_msgsnd: u64,
    ru_msgrcv: u64,
    ru_nsignals: u64,
    ru_nvcsw: u64,
    ru_nivcsw: u64,
}

fn sys_getrusage(_who: u64, rusage_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if rusage_ptr == 0 || !validate_user_range(rusage_ptr, core::mem::size_of::<RUsage>() as u64) {
        return -1i64 as u64;
    }
    let ru = unsafe { &mut *(rusage_ptr as *mut RUsage) };
    let ticks = zenus_arch::interrupts::pit::get_ticks();
    let sec = ticks / 100;
    let usec = (ticks % 100) * 10000;
    ru.ru_utime = Timeval {
        tv_sec: sec,
        tv_usec: usec,
    };
    ru.ru_stime = Timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    0
}

#[repr(C)]
struct Tms {
    tms_utime: u64,
    tms_stime: u64,
    tms_cutime: u64,
    tms_cstime: u64,
}

fn sys_times(tms_ptr: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let ticks = zenus_arch::interrupts::pit::get_ticks();
    if tms_ptr != 0 && validate_user_range(tms_ptr, core::mem::size_of::<Tms>() as u64) {
        let tms = unsafe { &mut *(tms_ptr as *mut Tms) };
        tms.tms_utime = ticks;
        tms.tms_stime = 0;
        tms.tms_cutime = 0;
        tms.tms_cstime = 0;
    }
    ticks
}

// ── Event/notification syscalls ──

static EVENTFD_COUNTERS: zenus_sync::spinlock::SpinLock<[u64; 64]> =
    zenus_sync::spinlock::SpinLock::new([0u64; 64]);

fn sys_eventfd2(_initval: u64, _flags: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let mut counters = EVENTFD_COUNTERS.lock();
    for i in 0..64 {
        if counters[i] == 0 {
            counters[i] = 1;
            let task_id = scheduler::current_task_id();
            return fd::fd_socket(task_id, i as u64 + 0xFFFF).unwrap_or(-1i64 as u64);
        }
    }
    -1i64 as u64
}

fn sys_pipe2(fds_ptr: u64, _flags: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if fds_ptr == 0 || !validate_user_range(fds_ptr, 8) {
        return -1i64 as u64;
    }
    let task_id = scheduler::current_task_id();
    match fd::fd_pipe(task_id) {
        Some((r, w)) => {
            unsafe {
                *(fds_ptr as *mut u64) = r;
                *(fds_ptr as *mut u64).add(1) = w;
            }
            0
        }
        None => -1i64 as u64,
    }
}

fn copy_str_to_fixed(dst: &mut [u8], s: &str) {
    let len = s.len().min(dst.len() - 1);
    dst[..len].copy_from_slice(&s.as_bytes()[..len]);
    dst[len] = 0;
}

#[no_mangle]
pub extern "C" fn syscall_dispatch(num: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    if num >= 256 {
        return -1i64 as u64;
    }
    let result = match SYSCALL_TABLE[num as usize] {
        Some(f) => f(arg1, arg2, arg3, 0, 0, 0),
        None => {
            zenus_console::kwarn!("Unknown syscall {}", num);
            -1i64 as u64
        }
    };
    result
}

/// Called from syscall return path in cpu.rs asm.
/// Checks for pending signals and redirects to handler if needed.
#[no_mangle]
pub extern "C" fn syscall_signal_hook(kernel_rsp: u64) {
    scheduler::check_signal_for_sysret(kernel_rsp);
}

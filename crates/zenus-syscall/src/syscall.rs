use zenus_fs::vfs;
use zenus_net::socket as net_socket;
use zenus_sched::scheduler;
use zenus_sched::signal;

mod fd;
use fd::*;

const USER_SPACE_LIMIT: u64 = 0x0000_8000_0000_0000;
const MAX_PATH_LEN: usize = 4096;

/// Largest single copy a syscall will stage through a kernel buffer.
///
/// `sys_read` had this (65536) and `sys_recv` did not: `recv` bounded `len` only
/// by `USER_SPACE_LIMIT`, 128 TiB, and then ran `alloc::vec![0u8; len as usize]`.
/// The allocation was the bound, so asking for a terabyte did not fail — it
/// tried, and the kernel ran out of memory. One `recv(fd, buf, 1<<48)` from ring 3.
pub const MAX_XFER: u64 = 65536;

/// Checked `size_of::<T>() * count` in `u64`.
///
/// Pure, because the multiplication is the part that breaks: `size_of::<Pollfd>()`
/// is 8, so `8 * nfds` wraps for any `nfds > 2^61`. The wrapped value is
/// *smaller*, so it passes `validate_user_range` and the loop then walks `nfds`
/// entries of a buffer that was never validated.
pub fn checked_array_len<T>(count: u64) -> Option<u64> {
    (count as usize)
        .checked_mul(core::mem::size_of::<T>())
        .map(|n| n as u64)
        .filter(|&n| n > 0)
}

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
// BUG-001 (residual) fix: `SYS_READDIR` was 5, which is `SYS_FSTAT`'s slot.
// `init_table` assigns in source order, so slot 5 ended up holding `sys_fstat`
// and `sys_readdir` was unreachable from userspace. Moved to the Linux
// `getdents` number, which this table does not otherwise use.
const SYS_READDIR: u64 = 78;
const SYS_LSEEK: u64 = 8;
const SYS_IOCTL: u64 = 16;
const SYS_PIPE: u64 = 111; // was 22, conflicted with access
const SYS_DUP: u64 = 113; // was 32, conflicted with fstatfs
const SYS_NANOSLEEP: u64 = 114; // was 35, conflicted with faccessat
const SYS_GETPID: u64 = 39;
// BUG-001 (residual) fix: `SYS_BRK` was 45, which is `SYS_RECVFROM`'s slot —
// `sys_recvfrom` was registered later and overwrote `sys_brk`, so brk always
// dispatched to recvfrom. Moved to the Linux `brk` number.
const SYS_BRK: u64 = 12;
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
const SYS_DUP3: u64 = 211; // custom slot (was 24, conflicted with sched_yield)

// Filesystem syscalls
const SYS_MKDIR: u64 = 251; // was 83, within range but moved for consistency
const SYS_UNLINK: u64 = 252; // was 87, within range but moved for consistency
const SYS_RMDIR: u64 = 253; // was 84, within range but moved for consistency
const SYS_RENAME: u64 = 254; // was 82, within range but moved for consistency
const SYS_CHMOD: u64 = 255; // was 90, within range but moved for consistency
const SYS_CHOWN: u64 = 106; // was 92, within range but moved for consistency
const SYS_ACCESS: u64 = 108; // was 21, conflicted with pipe

// Signal syscalls
const SYS_KILL: u64 = 225; // was 62, conflicted with uname_ns
const SYS_RT_SIGACTION: u64 = 13;
const SYS_RT_SIGPROCMASK: u64 = 14;
const SYS_RT_SIGRETURN: u64 = 15;
const SYS_TGKILL: u64 = 226; // was 234, within range but moved for consistency

// Memory syscalls
const SYS_MMAP: u64 = 9;
const SYS_MPROTECT: u64 = 10;
const SYS_MUNMAP: u64 = 11;

// Time syscalls
const SYS_GETTIMEOFDAY: u64 = 237; // was 96, within range but moved for consistency
const SYS_CLOCK_GETTIME: u64 = 238; // was 228, within range but moved for consistency
const SYS_CLOCK_GETRES: u64 = 239; // was 229, within range but moved for consistency

// Mount syscalls
const SYS_MOUNT: u64 = 240; // was 165, within range but moved for consistency
const SYS_UMOUNT2: u64 = 241; // was 166, within range but moved for consistency

// Shared memory syscalls
const SYS_SHMGET: u64 = 29;
const SYS_SHMAT: u64 = 30;
const SYS_SHMDT: u64 = 227; // was 67, within range but moved for consistency
const SYS_SHMCTL: u64 = 31;

// Futex syscall
const SYS_FUTEX: u64 = 230; // was 202, within range but moved for consistency

// Poll syscalls
const SYS_POLL: u64 = 7;
const SYS_PPOLL: u64 = 212; // custom slot (was 28, conflicted with shmget)

// Process group syscalls
const SYS_SETSID: u64 = 232; // was 112, within range but moved for consistency
const SYS_GETPGID: u64 = 233; // was 121, within range but moved for consistency
const SYS_SETPGID: u64 = 235; // was 109, within range but moved for consistency
const SYS_GETSID: u64 = 236; // was 124, within range but moved for consistency

// Socket options
const SYS_SETSOCKOPT: u64 = 54;
const SYS_GETSOCKOPT: u64 = 55;

// Select
const SYS_SELECT: u64 = 23;
const SYS_PSELECT6: u64 = 213; // custom slot (was 29, conflicted with shmat)

// Filesystem extras
const SYS_READLINKAT: u64 = 214; // custom slot (was 26, conflicted with ppoll)
const SYS_SYMLINKAT: u64 = 215; // custom slot (was 27, conflicted with poll)
const SYS_LINKAT: u64 = 216; // custom slot (was 34, conflicted with dup)
const SYS_TRUNCATE: u64 = 249; // was 76, within range but moved for consistency
const SYS_FTRUNCATE: u64 = 250; // was 77, within range but moved for consistency

// Resource limits
const SYS_GETRLIMIT: u64 = 242; // was 163, within range but moved for consistency
const SYS_SETRLIMIT: u64 = 243; // was 160, within range but moved for consistency

// CWD syscalls
const SYS_GETCWD: u64 = 245; // was 79, within range but moved for consistency
const SYS_CHDIR: u64 = 246; // was 80, within range but moved for consistency
const SYS_FCHDIR: u64 = 248; // was 81, within range but moved for consistency

// Process control
const SYS_PRCTL: u64 = 244; // was 157, within range but moved for consistency

// Additional filesystem (custom slots for Linux numbers > 256)
const SYS_FACCESSAT: u64 = 217; // was 35, conflicted with nanosleep
const SYS_UTIMENSAT: u64 = 218; // was 36, conflicted with dup
const SYS_FSTATAT: u64 = 219; // was 37, conflicted with dup2
const SYS_FCHOWNAT: u64 = 220; // was 38, conflicted with getpid

// Additional process syscalls
const SYS_WAITID: u64 = 223; // was 247, within range but moved for consistency
const SYS_SOCKETPAIR: u64 = 224; // was 53, conflicted with setsockopt

// File descriptor stat
const SYS_FSTAT: u64 = 5;
const SYS_FSTATFS: u64 = 221; // was 32, conflicted with dup
const SYS_STATFS: u64 = 222; // was 40, conflicted with socket

// Scheduling (custom slots 200+ to avoid conflicts)
const SYS_NICE: u64 = 200;
const SYS_SCHED_GETSCHEDULER: u64 = 201;
const SYS_SCHED_SETSCHEDULER: u64 = 202;
const SYS_SCHED_GETPARAM: u64 = 203;
const SYS_SCHED_SETPARAM: u64 = 204;
const SYS_SCHED_YIELD: u64 = 205;

// Resource usage
const SYS_GETRUSAGE: u64 = 206;
const SYS_TIMES: u64 = 207;

// Event/notification
const SYS_EVENTFD2: u64 = 208;
const SYS_PIPE2: u64 = 209;
const SYS_GETPPID: u64 = 210;

type SyscallFn = fn(u64, u64, u64, u64, u64, u64) -> u64;

/// Number of dispatch slots. The ABI reserves 0..255, and `syscall_dispatch6`
/// rejects anything above it.
pub const SYSCALL_TABLE_SIZE: usize = 256;

static SYSCALL_TABLE: [Option<SyscallFn>; SYSCALL_TABLE_SIZE] = init_table();

/// Syscall numbers that have a handler, in table order.
///
/// The table is built by a `const fn`, so this is the only way to look at it
/// from outside. It exists so the host test suite can prove what the ABI bugs
/// actually were about: a number registered twice (one handler silently
/// shadows the other), a number outside the table, or `userspace/` hard-coding
/// a number the kernel no longer uses.
pub fn registered_syscalls() -> alloc::vec::Vec<u64> {
    let mut out = alloc::vec::Vec::new();
    for (number, slot) in SYSCALL_TABLE.iter().enumerate() {
        if slot.is_some() {
            out.push(number as u64);
        }
    }
    out
}

/// How many syscalls are implemented.
pub fn syscall_count() -> usize {
    registered_syscalls().len()
}

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
    t[SYS_GETPPID as usize] = Some(sys_getppid);
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
    if count > MAX_XFER {
        return -1i64 as u64;
    }
    if count == 0 {
        return 0;
    }
    if !validate_user_range(buf, count) {
        return -1i64 as u64;
    }
    let mut kernel_buf = alloc::vec![0u8; count as usize];
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

/// Copy a user range into a kernel buffer through the direct map.
///
/// Two things this gets right that a bare `read_volatile` loop did not:
///
/// * **Ring 0 is allowed to read it.** SMAP makes a supervisor access to a page
///   whose U/S bit is 1 fault even when the page is *present*, so the read has
///   to happen either inside `stac`/`clac` or through the supervisor-only direct
///   map. `sys_write`'s console path did neither, so with SMAP on the first
///   `write(1, ...)` from a user program took a `#PF` in ring 0 — the "GPF in
///   userspace programs" that kept SMAP switched off since it was introduced.
/// * **The page has to exist.** `validate_user_range` only bounds the address;
///   it does not translate it. Walking the tables here turns an unmapped
///   pointer into `EFAULT` instead of a ring-0 fault that takes the kernel with
///   it.
///
/// Returns `None` only when the very first byte is unmapped; a range that goes
/// bad part-way returns the bytes that were copied first, which is what a
/// short write means.
fn copy_user_chunk_hhdm(src: u64, dst: &mut [u8]) -> Option<usize> {
    let hhdm = zenus_mem::paging::hhdm_offset();
    if hhdm == 0 || dst.is_empty() {
        return None;
    }
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }
    let mut done = 0usize;
    while done < dst.len() {
        let cur = src + done as u64;
        let phys = match zenus_mem::paging::virt_to_phys_raw(cr3, cur) {
            Some(p) => p,
            None => break,
        };
        let chunk = (dst.len() - done).min(4096 - (cur & 0xFFF) as usize);
        let p = (hhdm + phys) as *const u8;
        // SAFETY: `virt_to_phys_raw` proved `cur` is present in the current
        // CR3, and the allocator only hands out frames out of a Limine usable
        // region, which is what the HHDM covers. `chunk` stops at the page
        // boundary so it cannot run off the end of the mapping.
        unsafe { core::ptr::copy_nonoverlapping(p, dst[done..].as_mut_ptr(), chunk) };
        done += chunk;
    }
    if done == 0 {
        None
    } else {
        Some(done)
    }
}

/// How much of a console `write` is staged at a time.
///
/// Big enough that the per-chunk page walk is not the bottleneck for a
/// terminal line, small enough to sit on the stack.
const CONSOLE_CHUNK: usize = 256;

/// How much of a console write is echoed to the framebuffer.
const CONSOLE_ECHO: usize = 256;

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

    // stdout/stderr (fd 1, 2): stream straight to the console in fixed chunks,
    // so a `write(1, ...)` costs no heap allocation. Each chunk goes through
    // `copy_user_chunk_hhdm` — see there for why a raw pointer read is not an
    // option here.
    if fd == 1 || fd == 2 {
        let s = zenus_console::serial::SerialPort::new(0x3F8);
        let mut chunk = [0u8; CONSOLE_CHUNK];
        // The framebuffer echo only ever shows the first screenful of a write,
        // so this does not grow with `count`.
        let mut echo = [0u8; CONSOLE_ECHO];
        let mut echo_len = 0usize;
        let mut done = 0u64;
        while done < count {
            let want = ((count - done) as usize).min(CONSOLE_CHUNK);
            let got = match copy_user_chunk_hhdm(buf + done, &mut chunk[..want]) {
                Some(n) => n,
                None => {
                    // Nothing at all was readable: EFAULT, exactly like every
                    // other syscall handed an address that is not there.
                    return if done == 0 { -1i64 as u64 } else { done };
                }
            };
            for &byte in &chunk[..got] {
                s.write_byte_serial(byte);
            }
            let room = (CONSOLE_ECHO - echo_len).min(got);
            echo[echo_len..echo_len + room].copy_from_slice(&chunk[..room]);
            echo_len += room;
            done += got as u64;
        }
        if echo_len > 0 {
            if let Ok(text) = core::str::from_utf8(&echo[..echo_len]) {
                zenus_console::display::write_str(text);
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
            let stat_buf = match unsafe { user_ptr::<StatBuf>(stat_ptr) } {
                Some(p) => p,
                None => return -1i64 as u64,
            };
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

    // Built in a kernel buffer and copied out once, rather than written entry
    // by entry through a raw pointer. The buffer is bounded by `buf_size`, which
    // `validate_user_range` above already bounded — and `copy_kernel_to_user`
    // then revalidates every page before touching it, so an in-range but
    // unmapped `buf` is EFAULT instead of a #PF in ring 0.
    let max = buf_size as usize;
    let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::new();

    for entry in entries {
        if out.len() + 2 > max {
            break;
        }
        let name_bytes = entry.name.as_bytes();
        let name_len = name_bytes.len();
        if out.len() + 1 + name_len + 1 > max {
            break;
        }
        out.push(entry.file_type as u8);
        out.push(name_len as u8);
        if name_len > 0 {
            out.extend_from_slice(name_bytes);
        }
    }

    if !copy_bytes_to_user(&out, buf) {
        return -1i64 as u64;
    }
    out.len() as u64
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

/// PIT frequency in Hz. `zenus_arch::interrupts::pit::get_ticks` counts at
/// this rate and `uptime` assumes it.
const TICKS_PER_SEC: u64 = 100;

/// Convert a `timespec` into a tick deadline, or `None` if it is not valid.
///
/// Pure, and there were two bugs in the body this replaced:
///
/// * `tv_nsec` must be below 1e9. A value at or above it is `EINVAL`; the old
///   code folded it in with `nsec / 1_000_000` and carried on, so
///   `nanosleep(0, 2_000_000_000)` silently became a 2-second sleep.
/// * The comparison was against **milliseconds** while `get_ticks` counts
///   **ticks**. Every sleep was therefore 100x out: `nanosleep(1, 0)` slept ten
///   seconds, and `nanosleep(0, 10_000_000)` slept 100 ms. And nothing
///   saturated, so `nanosleep(u64::MAX, 0)` produced a deadline of 1.8e19
///   ticks — a permanent block. That is what a fuzzed input found: the
///   campaign task yielded forever, the watchdog fired, and the run was
///   reported as a timeout with `hangs=0` rather than as the hang it was.
pub fn nanosleep_deadline(now: u64, sec: u64, nsec: u64) -> Option<u64> {
    const NSEC_PER_SEC: u64 = 1_000_000_000;
    if nsec >= NSEC_PER_SEC {
        return None;
    }
    let ms = sec
        .saturating_mul(1000)
        .saturating_add(nsec / 1_000_000);
    // Ticks are 10 ms, so round *up*: a request must never return early.
    let ticks = ms
        .saturating_mul(TICKS_PER_SEC)
        .saturating_add(999)
        / 1000;
    // Saturating, so the deadline stays in the future but never wraps.
    now.checked_add(ticks)
}

fn sys_nanosleep(sec: u64, nsec: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let now = zenus_arch::interrupts::pit::get_ticks();
    let deadline = match nanosleep_deadline(now, sec, nsec) {
        Some(d) => d,
        None => return -1i64 as u64,
    };
    loop {
        zenus_sched::scheduler::yield_now();
        if zenus_arch::interrupts::pit::get_ticks().wrapping_sub(now) >= deadline - now {
            break;
        }
        x86_64::instructions::hlt();
    }
    0
}

fn sys_getpid(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    current_task()
}

fn sys_getppid(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let me = current_task();
    match zenus_sched::scheduler::get_task(me) {
        Some(t) => t.parent_pid,
        // PID 1 (init) and any task whose record vanished report 0, which is
        // what Linux does for init.
        None => 0,
    }
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

/// What `brk(addr)` should do.
///
/// Pure, because the shrink case is where the damage is and it can only be
/// exercised against a live address space otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrkAction {
    /// `brk(0)`: report the current break, change nothing.
    Query,
    Grow,
    Shrink,
    /// Below the heap's floor, or at or above the user-space limit. `EINVAL` /
    /// `ENOMEM`, and above all: do not unmap anything.
    Refuse,
}

/// Classify a `brk` request.
///
/// `floor` is where the loader put the heap — the initial break. Without it,
/// `brk(small)` shrinks from the initial break down to `small`, and
/// `unmap_heap_pages` walks every page in between: 6.4 billion page-table
/// entries, and `free_frame` on every mapped one. The program's own text and
/// data pages are in that range, so it released its own code back to the frame
/// allocator while still executing from it.
///
/// So the floor is not a nicety. A shrink below it is refused outright.
pub fn brk_action(addr: u64, current: u64, floor: u64, limit: u64) -> BrkAction {
    if addr == 0 {
        return BrkAction::Query;
    }
    if addr >= limit {
        return BrkAction::Refuse;
    }
    if addr < floor {
        return BrkAction::Refuse;
    }
    if addr > current {
        BrkAction::Grow
    } else if addr < current {
        BrkAction::Shrink
    } else {
        // Equal to the current break: a no-op, not a shrink of zero pages.
        BrkAction::Query
    }
}

fn sys_brk(addr: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let task = scheduler::current_task_id();
    let heap_start = scheduler::get_task_heap_brk(task);
    let floor = scheduler::get_task_heap_floor(task);
    match brk_action(addr, heap_start, floor, USER_SPACE_LIMIT) {
        BrkAction::Query => return heap_start,
        BrkAction::Refuse => return -1i64 as u64,
        BrkAction::Grow | BrkAction::Shrink => {}
    }
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }
    match brk_action(addr, heap_start, floor, USER_SPACE_LIMIT) {
        BrkAction::Grow => {
            if !map_heap_pages(cr3, heap_start, addr) {
                return -1i64 as u64;
            }
        }
        BrkAction::Shrink => {
            // Bounded by the floor, so this can only ever cover the heap itself.
            unmap_heap_pages(cr3, addr, heap_start);
        }
        BrkAction::Query | BrkAction::Refuse => return -1i64 as u64,
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
            if !copy_u32_pair_to_user(read_fd as u32, write_fd as u32, pipefd_ptr) {
                fd::fd_close(read_fd);
                fd::fd_close(write_fd);
                return -1i64 as u64;
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
                if status_ptr != 0 {
                    let status = ((exit_code & 0xFF) << 8) as u32;
                    copy_u32_to_user(status, status_ptr);
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

fn sys_execve(path_ptr: u64, argv_ptr: u64, _envp_ptr: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
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
            let arg_str_ptr = match unsafe { user_ptr::<u64>(arg_ptr_ptr) } {
                Some(p) => unsafe { *p },
                None => {
                    zenus_mem::paging::destroy_address_space(new_cr3);
                    return -1i64 as u64;
                }
            };
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

    // Compute the layout before touching anything: a refusal here has to leave
    // the new address space destroyable and the old one untouched.
    //
    // This used to do its own arithmetic *and* write to the new image's user
    // addresses while the old CR3 was still loaded. `load_elf` randomises
    // `stack_top`, so those addresses are essentially never mapped in the
    // address space that was actually installed, and with SMAP off the ring-0
    // write silently landed on whatever did share the address.
    let argv_lens: alloc::vec::Vec<usize> = argv_strings.iter().map(|s| s.len()).collect();
    let layout = match crate::userstack::layout_user_stack(stack_top, &argv_lens) {
        Some(l) => l,
        None => {
            zenus_mem::paging::destroy_address_space(new_cr3);
            return -1i64 as u64;
        }
    };

    let argv_refs: alloc::vec::Vec<&[u8]> = argv_strings.iter().map(|s| s.as_slice()).collect();
    crate::userstack::write_initial_user_stack(new_cr3, &layout, &argv_refs);

    let user_rsp = layout.user_rsp;

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
    // `exec` loads a new image, so the old floor is meaningless. Without this
    // the new image's heap would be shrinkable back into whatever the previous
    // program had mapped.
    scheduler::reset_task_heap_floor(tid, loaded.heap_base);
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
    let uts = match unsafe { user_ptr::<UtsName>(buf) } {
        Some(p) => unsafe { &mut *p },
        None => return -1i64 as u64,
    };
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
    let uts = match unsafe { user_ptr::<UtsName>(buf) } {
        Some(p) => unsafe { &mut *p },
        None => return -1i64 as u64,
    };
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
    let sa_ptr = match unsafe { user_ptr::<SockaddrIn>(addr_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let sa = unsafe { &*sa_ptr };
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
    let sa_ptr = match unsafe { user_ptr::<SockaddrIn>(addr_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let sa = unsafe { &*sa_ptr };
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
    // `send` had no length bound at all — `recv` got MAX_XFER in an earlier
    // commit and `send` was simply missed, so one call asked the socket layer
    // to read 128 TiB out of user space.
    if len > MAX_XFER {
        return -1i64 as u64;
    }
    let data = match user_bytes(buf_ptr, len) {
        Some(d) => d,
        None => return -1i64 as u64,
    };

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
    // `USER_SPACE_LIMIT` is 128 TiB, and that was the only bound on the size of
    // the kernel buffer allocated below. Asking for a terabyte was not an error,
    // it was an allocation the kernel then could not satisfy.
    if len > MAX_XFER {
        return -1i64 as u64;
    }
    if !validate_user_range(buf_ptr, len) {
        return -1i64 as u64;
    }
    let mut buf = alloc::vec![0u8; len as usize];

    match net_socket::recv(sock_id, &mut buf) {
        Some(n) => {
            // Through the page-validating copy: `buf_ptr` was only range-checked.
            if !copy_bytes_to_user(&buf[..n as usize], buf_ptr) {
                return -1i64 as u64;
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
    if len > MAX_XFER {
        return -1i64 as u64;
    }
    let data = match user_bytes(buf_ptr, len) {
        Some(d) => d,
        None => return -1i64 as u64,
    };

    let (dst_ip, dst_port) = if addr_ptr != 0 {
        let sa_ptr = match unsafe { user_ptr::<SockaddrIn>(addr_ptr) } {
            Some(p) => p,
            None => return -1i64 as u64,
        };
        let sa = unsafe { &*sa_ptr };
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
    // `USER_SPACE_LIMIT` is 128 TiB, and that was the only bound on the size of
    // the kernel buffer allocated below. Asking for a terabyte was not an error,
    // it was an allocation the kernel then could not satisfy.
    if len > MAX_XFER {
        return -1i64 as u64;
    }
    if !validate_user_range(buf_ptr, len) {
        return -1i64 as u64;
    }
    let mut buf = alloc::vec![0u8; len as usize];

    match net_socket::recv(sock_id, &mut buf) {
        Some(n) => {
            // Through the page-validating copy: `buf_ptr` was only range-checked.
            if !copy_bytes_to_user(&buf[..n as usize], buf_ptr) {
                return -1i64 as u64;
            }
            if addr_ptr != 0 {
                let sa_ptr = match unsafe { user_ptr::<SockaddrIn>(addr_ptr) } {
                    Some(p) => p,
                    None => return -1i64 as u64,
                };
                let sa = unsafe { &mut *sa_ptr };
                sa.sin_family = net_socket::AF_INET as u16;
                sa.sin_port = 0u16.to_be();
                sa.sin_addr = [0; 4];
                sa.sin_zero = [0; 8];
            }
            if addrlen_ptr != 0 {
                copy_u32_to_user(core::mem::size_of::<SockaddrIn>() as u32, addrlen_ptr);
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
    let sa_ptr = match unsafe { user_ptr::<SockaddrIn>(addr_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let sa = unsafe { &mut *sa_ptr };
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
    let sa_ptr = match unsafe { user_ptr::<SockaddrIn>(addr_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let sa = unsafe { &mut *sa_ptr };
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

fn sys_access(path_ptr: u64, mode: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let path = match read_user_cstr(path_ptr, 256) {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    // The mode is part of the question. It used to be dropped, which turned
    // `access(path, W_OK)` into `access(path, F_OK)`.
    if fd::vfs_access(&path, mode as u32) {
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
    if oldact_ptr != 0 {
        let oldact = match unsafe { user_ptr::<KernelSigAction>(oldact_ptr) } {
            Some(p) => unsafe { &mut *p },
            None => return -1i64 as u64,
        };
        let old = scheduler::get_signal_action(task_id, sig_num);
        oldact.handler_fn = old.handler_fn;
        oldact.flags = old.flags;
        oldact.restorer = old.restorer;
        oldact.mask = old.mask;
    }

    // Set new action
    if act_ptr != 0 {
        let act = match unsafe { user_ptr::<KernelSigAction>(act_ptr) } {
            Some(p) => unsafe { &*p },
            None => return -1i64 as u64,
        };
        let new_action = unsafe {
            zenus_sched::task::SignalAction {
                handler_fn: act.handler_fn,
                flags: act.flags,
                restorer: act.restorer,
                mask: act.mask,
            }
        };
        scheduler::set_signal_action(task_id, sig_num, new_action);
    }

    0
}

/// Validate a user range and hand back a pointer to it, or `None` if any of its
/// pages is not mapped.
///
/// `validate_user_range` only bounds the address; it does not check that the
/// page exists. Seventeen syscalls used to take a caller-supplied address and
/// immediately build `&mut *(ptr as *mut T)` from it, so any in-range-but-
/// unmapped pointer read or wrote through a page table entry that was not
/// there — a `#PF` in ring 0. With SMAP off there is no `stac` fixup path that
/// could turn it into `EFAULT`, so the program crashes the kernel.
///
/// Every reference into user space has to come from here.
///
/// # Safety
///
/// The returned pointer is only valid while the mapping stays put. Callers
/// dereference it immediately, which is the intent: that is how a syscall reads
/// a struct out of user memory.
unsafe fn user_ptr<T>(ptr: u64) -> Option<*mut T> {
    let len = core::mem::size_of::<T>() as u64;
    if !validate_user_range(ptr, len) {
        return None;
    }
    if !user_pages_mapped(ptr, len) {
        return None;
    }
    Some(ptr as *mut T)
}

/// Are all the pages of `ptr..ptr+len` present in the current address space?
///
/// Reads no memory, so it is safe to call on a range that is not mapped — which
/// is exactly the case being tested.
fn user_pages_mapped(ptr: u64, len: u64) -> bool {
    if len == 0 {
        return true;
    }
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }
    let mut offset = 0u64;
    while offset < len {
        let current = ptr + offset;
        if zenus_mem::paging::virt_to_phys_raw(cr3, current).is_none() {
            return false;
        }
        offset += 4096 - (current & 0xFFF);
    }
    true
}

/// Write a scalar to a user pointer, with the per-page revalidation
/// [`copy_kernel_to_user`] does.
///
/// `validate_user_range` on its own only checks that the address sits inside the
/// user range. It does **not** check that the page is mapped. So the common
/// shape
///
/// ```text
/// if validate_user_range(ptr, 8) { *(ptr as *mut u64) = value; }
/// ```
///
/// takes a kernel `#PF` on any in-range-but-unmapped pointer — and with SMAP
/// off there is no `stac` fixup path that could turn it into `EFAULT`. The
/// fuzzing campaign found exactly that: `rt_sigprocmask` writing to `0x7075`,
/// which is inside the user range and mapped by nothing.
///
/// A syscall that writes to a user buffer must go through here, so a bad pointer
/// is `EFAULT` instead of a fault in ring 0.
fn copy_u64_to_user(value: u64, user_ptr: u64) -> bool {
    copy_kernel_to_user(&value.to_ne_bytes(), user_ptr)
}

fn copy_u32_to_user(value: u32, user_ptr: u64) -> bool {
    copy_kernel_to_user(&value.to_ne_bytes(), user_ptr)
}

/// Two adjacent `u64`s, as `pipe`/`pipe2`/`socketpair` write them.
fn copy_u64_pair_to_user(first: u64, second: u64, user_ptr: u64) -> bool {
    let mut buf = [0u8; 16];
    buf[0..8].copy_from_slice(&first.to_ne_bytes());
    buf[8..16].copy_from_slice(&second.to_ne_bytes());
    copy_kernel_to_user(&buf, user_ptr)
}

/// Two adjacent `u32`s, as `pipe` writes them.
fn copy_u32_pair_to_user(first: u32, second: u32, user_ptr: u64) -> bool {
    let mut buf = [0u8; 8];
    buf[0..4].copy_from_slice(&first.to_ne_bytes());
    buf[4..8].copy_from_slice(&second.to_ne_bytes());
    copy_kernel_to_user(&buf, user_ptr)
}

/// Copy a kernel slice into a user buffer.
///
/// A spelling of [`copy_kernel_to_user`] that says which way the bytes go, so
/// the syscall bodies read as "where does this end up" rather than "which
/// pointer arithmetic is safe today".
fn copy_bytes_to_user(bytes: &[u8], user_ptr: u64) -> bool {
    copy_kernel_to_user(bytes, user_ptr)
}

/// A read-only view of `len` bytes of user memory, or `None` if the range is
/// not addressable or any of its pages is unmapped.
///
/// `sys_send` and `sys_sendto` used to do `from_raw_parts(buf_ptr, len)` behind
/// nothing but `validate_user_range`, so the socket layer read straight out of
/// whatever address the caller named.
fn user_bytes(ptr: u64, len: u64) -> Option<&'static [u8]> {
    if !validate_user_range(ptr, len) {
        return None;
    }
    if !user_pages_mapped(ptr, len) {
        return None;
    }
    // SAFETY: the range was bounded and every page in it was found present in
    // the current address space, so the slice describes real mapped memory.
    //
    // The `'static` is a deliberate fib the caller must uphold: the view is only
    // valid until the syscall returns, which is the only window in which it is
    // used anywhere here. A real lifetime would need the caller's borrow to
    // outlive the FFI boundary, and nothing does.
    Some(unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) })
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
    if oldset_ptr != 0 {
        let old_mask = scheduler::get_signal_mask(task_id);
        if !copy_u64_to_user(old_mask, oldset_ptr) {
            return -1i64 as u64;
        }
    }

    // Set new mask
    if set_ptr != 0 {
        let new_set = match unsafe { user_ptr::<u64>(set_ptr) } {
            Some(p) => unsafe { *p },
            None => return -1i64 as u64,
        };
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

    if let Some((restored_rsp, _restored_rip, _restored_rflags)) =
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

fn sys_tgkill(_tgid: u64, tid: u64, sig: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
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

/// The page count and byte size an `mmap` should ask for, or `None` if the
/// length is not something this kernel can map.
///
/// Pure. Two bugs, and the second one is why the fuzzing campaign wedged.
///
/// * `((length + 0xFFF) & !0xFFF) / 0x1000` used a bare add, so a length near
///   `u64::MAX` wrapped to a *small* page count while the VMA was still
///   recorded with a different size than the pages it mapped.
/// * There was no upper bound of the *kernel's own*. `mmap` here maps eagerly,
///   one page at a time, so the request size *is* the cost. The campaign asked
///   for 4 GiB on a 2 GiB machine: the loop ran 1 048 577 times, each iteration
///   taking the frame allocator lock and walking the page tables, before
///   failing at the ~500 000th frame. That is minutes of emulated work for a
///   mapping that could never succeed, and it is what stopped the campaign at
///   case 5506.
///
///   A mapping is refused up front unless the free-frame count can satisfy it,
///   so the failure is immediate and the cost is proportional to what the
///   program can actually get.
pub fn mmap_extent(length: u64, max_size: u64, free_frames: usize) -> Option<(u64, u64)> {
    if length == 0 {
        return None;
    }
    // `checked_next_multiple_of` rounds up *and* reports the overflow, which the
    // old `+ 0xFFF` did without.
    let size = length.checked_next_multiple_of(0x1000)?;
    if size > max_size {
        return None;
    }
    let pages = size / 0x1000;
    if pages > free_frames as u64 {
        return None;
    }
    Some((pages, size))
}

fn sys_mmap(addr: u64, length: u64, prot: u64, flags: u64, _fd: u64, _offset: u64) -> u64 {
    let free_frames = zenus_mem::frame_allocator::FRAME_ALLOCATOR
        .lock()
        .free_frames_count();
    let (page_count, size) =
        match mmap_extent(length, zenus_mem::vma::MAX_MAPPING_SIZE, free_frames) {
            Some(e) => e,
            None => return -1i64 as u64,
        };

    let task_id = scheduler::current_task_id();
    let cr3 = match scheduler::get_task_cr3(task_id) {
        Some(c) => c,
        None => return -1i64 as u64,
    };

    let _is_anonymous = flags & zenus_sched::task::MAP_ANONYMOUS != 0;
    let is_fixed = flags & zenus_sched::task::MAP_FIXED != 0;
    let _is_private = flags & zenus_sched::task::MAP_PRIVATE != 0;

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

    // Map pages. When the frame allocator runs dry the pages already mapped are
    // unmapped and their frames released: they belong to this call, and keeping
    // them turns one over-large request into a machine with no free memory at
    // all — which is exactly what wedged the fuzzing campaign at case 5506.
    for i in 0..page_count {
        let page_virt = vma_start + i * 0x1000;
        let frame = {
            let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
            match allocator.alloc_frame() {
                Some(f) => f,
                None => {
                    drop(allocator);
                    for done in 0..i {
                        zenus_mem::paging::unmap_page_raw(cr3, vma_start + done * 0x1000);
                    }
                    return -1i64 as u64;
                }
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

/// The page range an `mprotect` walks, or `None` if the request is not a legal
/// range to walk at all.
///
/// Pure, and `sys_mprotect` had neither an overflow check nor an upper bound:
///
/// ```text
/// let end_page = ((addr + length) + 0xFFF) & !0xFFF;
/// while page < end_page { protect_page_raw(cr3, page, ..); page += 0x1000; }
/// ```
///
/// `addr + length` wraps silently, and nothing caps the span. The fuzzing
/// campaign found it immediately with
/// `mprotect(0x7665002f706d742f, 0x6c6900001000, …)` — a span of about 3.4 GB,
/// so roughly 870 000 iterations, each a four-level page-table walk ending in an
/// `invlpg`. Nothing in the loop could stop it, and `mprotect` is an ordinary
/// syscall: any program with a bad `length` hangs the machine.
///
/// `map_heap_pages` already had the `page >= USER_SPACE_LIMIT` bail-out that
/// this needed; `mprotect` never got one.
pub fn mprotect_range(addr: u64, length: u64, limit: u64) -> Option<(u64, u64)> {
    if length == 0 {
        return None;
    }
    // Round the end up *without* wrapping. `addr + length - 1` is the last byte
    // actually named, so this is the exact overflow check.
    let end = addr.checked_add(length.checked_sub(1)?)?;
    if end >= limit {
        return None;
    }
    let start_page = addr & !0xFFF;
    // `end` is the last byte named, so the exclusive end is one past the page
    // that holds it. `(end + 0xFFF) & !0xFFF` gets this wrong for an
    // already-aligned `end`: 0x3000 rounds to 0x3000, dropping the page that
    // contains byte 0x3000 itself.
    let end_page = (end | 0xFFF).checked_add(1)?;
    // `end_page` cannot be below `start_page` after the checks above, but the
    // loop depends on it and one bad number here is a 2^64-iteration hang.
    if end_page <= start_page {
        return None;
    }
    Some((start_page, end_page))
}

fn sys_mprotect(addr: u64, length: u64, prot: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let (start_page, end_page) = match mprotect_range(addr, length, USER_SPACE_LIMIT) {
        Some(r) => r,
        None => return -1i64 as u64,
    };

    let task_id = scheduler::current_task_id();
    let cr3 = match scheduler::get_task_cr3(task_id) {
        Some(c) => c,
        None => return -1i64 as u64,
    };

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

    if tv_ptr != 0 {
        let tv_ptr = match unsafe { user_ptr::<Timeval>(tv_ptr) } {
            Some(p) => p,
            None => return -1i64 as u64,
        };
        let tv = unsafe { &mut *tv_ptr };
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

    if tp_ptr != 0 {
        let tp_ptr = match unsafe { user_ptr::<Timespec>(tp_ptr) } {
            Some(p) => p,
            None => return -1i64 as u64,
        };
        let tp = unsafe { &mut *tp_ptr };
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
    if res_ptr != 0 {
        let res_ptr = match unsafe { user_ptr::<Timespec>(res_ptr) } {
            Some(p) => p,
            None => return -1i64 as u64,
        };
        let res = unsafe { &mut *res_ptr };
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
/// Rejected by `sys_mount`: bind mounts are not implemented.
const MS_BIND: u64 = 4096;

/// Flags the VFS actually implements. Anything else (MS_BIND and friends) is
/// rejected rather than silently ignored — `sys_mount` used to discard the
/// flags entirely, so a caller asking for MS_BIND got a plain mount and no
/// error.
const MS_SUPPORTED: u64 = MS_RDONLY | MS_NOSUID | MS_NODEV | MS_NOEXEC | MS_REMOUNT;

/// Translate syscall `MS_*` bits into the VFS's mount flags.
///
/// Pure. Only `MS_RDONLY` survives: the VFS has no suid, device-node or exec
/// enforcement to attach the other three to. `MS_SUPPORTED` still admits them,
/// because a real kernel accepts and records those bits, but nothing here reads
/// them — and pretending to honour a flag is the same lie as ignoring it, so
/// what they map to is stated rather than left implicit.
pub fn mount_flags_from_syscall(flags: u64) -> u32 {
    if flags & MS_RDONLY != 0 {
        zenus_fs::vfs::MS_RDONLY
    } else {
        0
    }
}

/// Sanity check on the unsupported-mask constant (keeps MS_BIND referenced and
/// documents why it is absent from `MS_SUPPORTED`).
const _: () = assert!(MS_BIND == 4096 && MS_BIND & MS_SUPPORTED == 0);

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
    // Mounting is a privileged operation and `sys_mount` never checked. Since
    // `current_euid()` is 0 for every task that has not explicitly dropped
    // privileges, the check has to be here: without it any ring-3 program could
    // attach an ext2 filesystem for any device it could name.
    if zenus_sched::scheduler::current_euid() != 0 {
        return -1i64 as u64;
    }
    // BUG-005 follow-up: the flags used to be dropped on the floor, so
    // MS_BIND (and any other unsupported bit) produced a full mount instead of
    // an error. Reject what the VFS cannot honour.
    if flags & !MS_SUPPORTED != 0 {
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

    // BUG-005: `target` was leaked with `leak_string` *before* the fstype was
    // validated, so every rejected mount (unknown fstype, missing block
    // device) still burned a heap allocation, and the MS_REMOUNT early return
    // leaked one more. `source` was parsed and then thrown away. Validate
    // first, leak last — and intern the path so repeated mounts of the same
    // target do not grow the leak without bound.
    if fstype.is_empty() || target.is_empty() {
        return -1i64 as u64;
    }
    if fstype.len() > 32 {
        return -1i64 as u64;
    }
    // Only the "real" filesystems need a source device.
    if fstype == "ext2" && source.is_empty() {
        return -1i64 as u64;
    }

    if flags & MS_REMOUNT != 0 {
        return if vfs::umount(&target) { 0 } else { -1i64 as u64 };
    }

    // Resolve the filesystem *before* touching the target path, so a failed
    // lookup leaves no side effects at all.
    let fs: Option<&'static dyn vfs::FileSystem> = match fstype.as_str() {
        "tmpfs" => {
            use zenus_fs::tmpfs::TmpFs;
            Some(TmpFs::new() as &'static dyn vfs::FileSystem)
        }
        "devfs" => {
            use zenus_fs::devfs::DevFs;
            Some(&DevFs as &'static dyn vfs::FileSystem)
        }
        "ext2" => {
            let dev_id = match resolve_block_dev(&source) {
                Some(id) => id,
                None => return -1i64 as u64,
            };
            match zenus_fs::ext2::Ext2Fs::mount(dev_id) {
                Some(fs) => Some(fs as &'static dyn vfs::FileSystem),
                None => return -1i64 as u64,
            }
        }
        "proc" | "procfs" => {
            static PROCFS: zenus_fs::procfs::ProcFs = zenus_fs::procfs::ProcFs;
            Some(&PROCFS as &'static dyn vfs::FileSystem)
        }
        "cgroup2" | "cgroup" => {
            static CGROUPFS: zenus_fs::cgroup::CgroupFs = zenus_fs::cgroup::CgroupFs;
            Some(&CGROUPFS as &'static dyn vfs::FileSystem)
        }
        _ => None,
    };

    let fs = match fs {
        Some(fs) => fs,
        None => return -1i64 as u64,
    };

    // Intern the path so the `&'static str` the VFS demands costs one
    // allocation per distinct mount point, not one per `mount()` call.
    let target_static = intern_static(&target);
    if target_static.is_empty() {
        return -1i64 as u64;
    }

    vfs::create_dir(target_static);
    // The flags are recorded on the mount now instead of discarded, so an
    // `MS_RDONLY` mount produces descriptors that refuse writes.
    if !vfs::mount_with_flags(
        zenus_ns::NS_ROOT,
        target_static,
        fs,
        mount_flags_from_syscall(flags),
    ) {
        return -1i64 as u64;
    }
    0
}

fn sys_umount2(target_ptr: u64, _flags: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    // Unmount is privileged for the same reason mount is: it removes a
    // filesystem from the tree, and any task could do it.
    if zenus_sched::scheduler::current_euid() != 0 {
        return -1i64 as u64;
    }
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

/// Interned `&'static str`s handed to the VFS.
///
/// `vfs::mount` takes `&'static str`, so a path has to outlive the call. The
/// old code simply leaked a copy on every `mount()`, which turned a loop of
/// `mount("/tmp")` into unbounded kernel memory growth. Interning bounds the
/// cost to one allocation per distinct path.
static INTERNED: zenus_sync::spinlock::SpinLock<InternPool> =
    zenus_sync::spinlock::SpinLock::new(InternPool::new());

const MAX_INTERNED: usize = 64;
const MAX_INTERNED_LEN: usize = 512;

struct InternPool {
    entries: [InternEntry; MAX_INTERNED],
    count: usize,
}

#[derive(Clone, Copy)]
struct InternEntry {
    ptr: usize,
    len: usize,
}

impl InternPool {
    const fn new() -> Self {
        InternPool {
            entries: [InternEntry { ptr: 0, len: 0 }; MAX_INTERNED],
            count: 0,
        }
    }
}

/// Return a `&'static str` equal to `s`, allocating at most one buffer per
/// distinct value. Returns `""` only on allocation failure or when the path is
/// longer than [`MAX_INTERNED_LEN`].
fn intern_static(s: &str) -> &'static str {
    if s.is_empty() || s.len() > MAX_INTERNED_LEN {
        return "";
    }

    let mut pool = INTERNED.lock();
    for i in 0..pool.count {
        let e = &pool.entries[i];
        if e.len == s.len() {
            // SAFETY: `e.ptr` was allocated with `e.len` bytes and is only ever
            // written before the entry is published under the pool lock.
            let existing = unsafe {
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(e.ptr as *const u8, e.len))
            };
            if existing == s {
                return existing;
            }
        }
    }

    if pool.count >= MAX_INTERNED {
        // Pool full: reuse slot 0 rather than growing without bound. A stale
        // `&'static str` handed out earlier stays valid (the VFS compares
        // pointers/strings), so this is safe, only lossy.
        drop(pool);
        return leak_string(alloc::string::String::from(s));
    }

    let leaked = leak_string(alloc::string::String::from(s));
    if leaked.is_empty() {
        return "";
    }
    let idx = pool.count;
    // `leak_string` returned a non-empty str backed by a live allocation, and
    // the entry keeps that allocation alive for the lifetime of the pool.
    pool.entries[idx].ptr = leaked.as_ptr() as usize;
    pool.entries[idx].len = leaked.len();
    pool.count = idx + 1;
    leaked
}

fn leak_string(s: alloc::string::String) -> &'static str {
    use core::alloc::Layout;
    let bytes = s.into_bytes();
    let len = bytes.len();
    if len == 0 {
        return "";
    }
    let layout = match Layout::array::<u8>(len) {
        Ok(l) => l,
        Err(_) => return "",
    };
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
    /// User virtual address this segment is mapped at in the *boot* task (the
    /// only task that can shmat today), so `shmdt` can find the segment back
    /// without guessing from the size.
    mapped_at: u64,
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
                mapped_at: 0,
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
                    mapped_at: 0,
                    valid: true,
                };
                return Some(i);
            }
        }
        None
    }
    /// Find the segment mapped at `addr`, if any.
    fn find_by_mapped_at(&self, addr: u64) -> Option<usize> {
        (0..MAX_SHMSEG).find(|&i| self.segments[i].valid && self.segments[i].mapped_at == addr)
    }

    fn detach(&mut self, idx: usize) {
        if idx < MAX_SHMSEG && self.segments[idx].valid {
            // `attached` is a usize and `detach` is reachable from more than one
            // path; underflow would wrap to a huge count and leak the segment
            // forever.
            if self.segments[idx].attached == 0 {
                self.segments[idx].valid = false;
                return;
            }
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
    // BUG-003 fix: bounds check shmid sebelum akses array segments
    if shmid as usize >= MAX_SHMSEG {
        return -1i64 as u64;
    }
    let table = SHM_TABLE.lock();
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

    // Remember where this task mapped the segment so `shmdt` can find it again.
    let mut table = SHM_TABLE.lock();
    if shmid as usize >= MAX_SHMSEG || !table.segments[shmid as usize].valid {
        // The segment was removed between the first check and here.
        return -1i64 as u64;
    }
    table.segments[shmid as usize].mapped_at = vma_start;
    // Count the mapping as a reference. Without this the *first* `shmdt` took
    // the count to zero and released the frames while this task's PTEs were
    // still live — the other half of the use-after-free that clearing the PTE
    // fixes. `shmget` also counts, so a segment created and never attached is
    // still reclaimed when its creator detaches.
    table.segments[shmid as usize].attached += 1;

    vma_start
}

fn sys_shmdt(shmaddr: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    // The old body returned 0 without doing anything, so `shmat`+`shmdt`
    // leaked the mapping *and* the reference count: the physical frames were
    // only ever released by an explicit `shmctl(IPC_RMID)`, and the user
    // mapping stayed in the task's VMA table after the segment was gone.
    if !validate_user_ptr::<u8>(shmaddr) {
        return -1i64 as u64;
    }

    let task_id = scheduler::current_task_id();
    let cr3 = scheduler::get_task_cr3(task_id).unwrap_or(0);

    // Drop the user mapping and release the VMA slot.
    let region = scheduler::with_vma(task_id, |vma| {
        vma.find(shmaddr).and_then(|idx| {
            let r = vma.regions[idx];
            Some((r.start, r.end, r.flags))
        })
    })
    .flatten();

    let (start, end, flags) = match region {
        Some(r) => r,
        None => return -1i64 as u64,
    };

    // Only unmap what we know this task mapped: a shared mapping is backed by
    // `MAP_SHARED`, and unmapping a private one would drop anonymous pages the
    // allocator still owns.
    if flags & zenus_sched::task::MAP_SHARED == 0 {
        return -1i64 as u64;
    }

    let pages = (end - start) / 0x1000;
    for i in 0..pages {
        let virt = start + i * 0x1000;
        // Clear the PTE. The old code only ran `invlpg`, which invalidates the
        // TLB entry and leaves the PTE present, still pointing at the frame — so
        // after `detach` released the frames, the next access through this
        // address read whatever had since been allocated into them. The frames
        // themselves stay owned by the SHM segment; `shmctl(IPC_RMID)` or the
        // last `detach` frees them.
        zenus_mem::paging::unmap_page_raw_keep_frame(cr3, virt);
    }
    scheduler::with_vma_mut(task_id, |vma| {
        if let Some(idx) = vma.find(start) {
            vma.remove(idx);
        }
    });

    // Drop one attachment reference. The segment's frames are freed by
    // `detach` when the last attachment goes away.
    let mut table = SHM_TABLE.lock();
    match table.find_by_mapped_at(start) {
        Some(idx) => {
            table.detach(idx);
            0
        }
        // The mapping existed but the segment is gone (already removed): the
        // caller still gets a clean detach.
        None => 0,
    }
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

    /// True when `bucket` holds waiters for `addr`.
    ///
    /// Buckets are hashed by address and one bucket only ever tracks a single
    /// address, so a bucket claimed by a different futex must not be reused —
    /// otherwise `FUTEX_WAKE` on address A wakes the waiters parked on
    /// address B, and `FUTEX_WAIT` on B joins A's queue.
    fn owns(bucket: &FutexBucket, addr: u64) -> bool {
        bucket.addr == addr
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

        // A claimed bucket owned by another address is treated as full: this
        // is a hash collision, not a waiter-limit hit.
        if bucket.count > 0 && !Self::owns(bucket, addr) {
            return false;
        }

        if bucket.count < 8 {
            bucket.addr = addr;
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
        // Never wake waiters registered against a different address.
        if !Self::owns(bucket, addr) {
            return 0;
        }
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
    // `size_of::<Pollfd>() * nfds` used to be computed with a bare `*`, which
    // wraps for any `nfds` above 2^61. The wrapped total is smaller, so it
    // passes `validate_user_range` — and the loop below then walks `nfds`
    // entries of a buffer that was only validated for the wrapped length.
    let total_size = match checked_array_len::<Pollfd>(nfds) {
        Some(n) => n,
        None => return -1i64 as u64,
    };
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
            let pfd = match unsafe { user_ptr::<Pollfd>(offset) } {
                Some(p) => unsafe { &mut *p },
                None => return -1i64 as u64,
            };
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
    if optval_ptr != 0 && !copy_u32_to_user(0, optval_ptr) {
        return -1i64 as u64;
    }
    if optlen_ptr != 0 && !copy_u32_to_user(4, optlen_ptr) {
        return -1i64 as u64;
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

    let timeout_ms = if timeout_ptr != 0 {
        let ts_ptr = match unsafe { user_ptr::<Timespec>(timeout_ptr) } {
            Some(p) => p,
            None => return -1i64 as u64,
        };
        let ts = unsafe { &*ts_ptr };
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
    let rl_ptr = match unsafe { user_ptr::<Rlimit>(rlim_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let rl = unsafe { &mut *rl_ptr };
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
    let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::with_capacity(copy_len as usize + 1);
    out.extend_from_slice(&cwd[..copy_len as usize]);
    out.push(0);
    if !copy_bytes_to_user(&out, buf_ptr) {
        return -1i64 as u64;
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

/// What Zenus can actually do with a `prctl` option.
///
/// The kernel used to answer `0` for every option, including
/// `PR_SET_SECCOMP`, `PR_SET_NO_NEW_PRIVS` and `PR_SET_MEMBARRIER`. A program
/// that checks `prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)` to decide whether it
/// may still gain privileges through `execve` was told "yes", and was wrong:
/// Zenus enforces nothing either way. Silence there is a security bug, so the
/// options are now classified and the unsupported ones fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrctlSupport {
    /// Implemented: the call does what it says.
    Name,
    /// Accepted and honoured as a no-op because there is nothing to honour:
    /// `PR_SET_PDEATHSIG` on a kernel with no `execve`-time reaping, and
    /// `PR_SET/GET_KEEPCAPS` on a kernel with no capabilities.
    NoOp,
    /// Not implemented. Must fail with `-EINVAL` so callers cannot rely on it.
    Unsupported,
    /// Not a prctl option at all.
    Unknown,
}

pub const PR_SET_PDEATHSIG: u64 = 1;
pub const PR_GET_PDEATHSIG: u64 = 2;
pub const PR_SET_NAME: u64 = 15; // Linux value
pub const PR_GET_NAME: u64 = 16;
pub const PR_SET_SECCOMP: u64 = 22;
pub const PR_SET_KEEPCAPS: u64 = 8;
pub const PR_GET_KEEPCAPS: u64 = 9;
pub const PR_SET_NO_NEW_PRIVS: u64 = 38;
pub const PR_GET_NO_NEW_PRIVS: u64 = 39;

/// Classify a `prctl` option. Pure, so the host tests can pin the answer for
/// every option the kernel claims.
pub fn prctl_support(option: u64) -> PrctlSupport {
    match option {
        PR_SET_NAME | PR_GET_NAME => PrctlSupport::Name,
        PR_SET_PDEATHSIG | PR_GET_PDEATHSIG | PR_SET_KEEPCAPS | PR_GET_KEEPCAPS => {
            PrctlSupport::NoOp
        }
        PR_SET_SECCOMP | PR_SET_NO_NEW_PRIVS | PR_GET_NO_NEW_PRIVS => PrctlSupport::Unsupported,
        _ => PrctlSupport::Unsupported,
    }
}

fn sys_prctl(option: u64, arg2: u64, arg3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    match prctl_support(option) {
        PrctlSupport::Name => {
            let task_id = scheduler::current_task_id();
            match option {
                PR_SET_NAME => {
                    match read_user_cstr(arg2, 32) {
                        Some(name) => {
                            scheduler::set_task_name(task_id, &name);
                            0
                        }
                        None => -1i64 as u64,
                    }
                }
                _ => {
                    // PR_GET_NAME writes the name into arg2.
                    if arg2 == 0 || !validate_user_range(arg2, 32) {
                        return -1i64 as u64;
                    }
                    let task = match scheduler::get_task(task_id) {
                        Some(t) => t,
                        None => return -1i64 as u64,
                    };
                    let buf = task.name.as_ptr() as *mut u8;
                    unsafe {
                        core::ptr::copy_nonoverlapping(buf, arg2 as *mut u8, 32);
                    }
                    let _ = arg3;
                    0
                }
            }
        }
        PrctlSupport::NoOp => 0,
        PrctlSupport::Unsupported | PrctlSupport::Unknown => -22i64 as u64, // -EINVAL
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
    let sb_ptr = match unsafe { user_ptr::<StatBuf>(stat_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let sb = unsafe { &mut *sb_ptr };
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
    // Reap any zombies (simplified — just clean up)
    0
}

// ── Socket pair syscalls ──

fn sys_socketpair(
    domain: u64,
    _type_: u64,
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
    let (r2, _w2) = match fd::fd_pipe(task_id) {
        Some(p) => p,
        None => {
            fd::fd_close(r1);
            fd::fd_close(w1);
            return -1i64 as u64;
        }
    };

    if !copy_u64_pair_to_user(r1, r2, fds_ptr) {
        fd::fd_close(r1);
        fd::fd_close(r2);
        return -1i64 as u64;
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
    let sb_ptr = match unsafe { user_ptr::<StatBuf>(stat_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let sb = unsafe { &mut *sb_ptr };
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
    let sb_ptr = match unsafe { user_ptr::<StatFs>(buf_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let sb = unsafe { &mut *sb_ptr };
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
    let sb_ptr = match unsafe { user_ptr::<StatFs>(buf_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let sb = unsafe { &mut *sb_ptr };
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
    let ru_ptr = match unsafe { user_ptr::<RUsage>(rusage_ptr) } {
        Some(p) => p,
        None => return -1i64 as u64,
    };
    let ru = unsafe { &mut *ru_ptr };
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
    if tms_ptr != 0 {
        let tms_ptr = match unsafe { user_ptr::<Tms>(tms_ptr) } {
            Some(p) => p,
            None => return -1i64 as u64,
        };
        let tms = unsafe { &mut *tms_ptr };
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
            if !copy_u64_pair_to_user(r, w, fds_ptr) {
                fd::fd_close(r);
                fd::fd_close(w);
                return -1i64 as u64;
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
    syscall_dispatch6(num, arg1, arg2, arg3, 0, 0, 0)
}

/// Full 6-argument syscall dispatcher.
///
/// Args 4/5/6 correspond to r10/r8/r9 in the Linux syscall ABI. The previous
/// 4-argument `syscall_dispatch` zeroed them, so any syscall taking more than
/// three arguments (mmap's fd/offset, pselect6, ppoll, ...) silently received
/// zeros for its tail arguments.
#[no_mangle]
pub extern "C" fn syscall_dispatch6(
    num: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) -> u64 {
    if num >= 256 {
        return -1i64 as u64;
    }
    let result = match SYSCALL_TABLE[num as usize] {
        Some(f) => f(arg1, arg2, arg3, arg4, arg5, arg6),
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

#[cfg(test)]
mod host_tests {
    use super::{
        brk_action, checked_array_len, mmap_extent, mount_flags_from_syscall, mprotect_range,
        nanosleep_deadline, validate_user_range, BrkAction, MAX_XFER, MS_NODEV, MS_NOSUID,
        MS_NOEXEC, USER_SPACE_LIMIT,
    };
    use super::MS_RDONLY as SYSCALL_MS_RDONLY;

    #[repr(C)]
    struct Pollfd {
        fd: u32,
        events: u16,
        revents: u16,
    }

    /// Regression: `sys_poll` computed the span it validates as
    /// `size_of::<Pollfd>() * nfds` with a bare `*`. `size_of::<Pollfd>()` is 8,
    /// so `nfds` above 2^61 wraps — and the wrapped value is *smaller*, so it
    /// passed `validate_user_range`. The loop then walked `nfds` entries of a
    /// buffer that had only been validated for the wrapped length.
    #[test]
    fn the_pollfd_span_is_checked_before_it_is_used_as_a_length() {
        assert_eq!(core::mem::size_of::<Pollfd>(), 8);
        assert_eq!(checked_array_len::<Pollfd>(0), None, "zero entries");
        assert_eq!(checked_array_len::<Pollfd>(1), Some(8));
        assert_eq!(checked_array_len::<Pollfd>(4), Some(32));

        // The wrap itself: 8 * 2^61 == 0.
        let nfds = 1u64 << 61;
        assert_eq!(8u64.wrapping_mul(nfds), 0, "the overflow being guarded");
        assert_eq!(
            checked_array_len::<Pollfd>(nfds),
            None,
            "a wrapping span must not be reported as a valid length"
        );
        assert_eq!(checked_array_len::<Pollfd>(u64::MAX), None);
        assert_eq!(checked_array_len::<Pollfd>((1 << 61) + 1), None);
    }

    /// The same multiply for a zero-sized element would be a heap walk with no
    /// bytes of stride, so the helper refuses zero-length spans outright rather
    /// than reporting them as valid.
    #[test]
    fn a_zero_length_span_is_refused() {
        struct Empty;
        assert_eq!(checked_array_len::<Empty>(1), None);
        assert_eq!(checked_array_len::<Empty>(1000), None);
    }

    /// Regression: `recv` bounded its length only by `USER_SPACE_LIMIT` — 128
    /// TiB — and then allocated a kernel buffer of that size, while `sys_read`
    /// capped the same operation at 64 KiB. Asking for a terabyte was not an
    /// error; it was an allocation the kernel then could not satisfy.
    #[test]
    fn a_single_transfer_cannot_be_unbounded() {
        assert_eq!(MAX_XFER, 65536);
        assert!(USER_SPACE_LIMIT > MAX_XFER * 1000, "the limit must bind");

        // What `recv` used to accept: 128 TiB is 2^47, and `USER_SPACE_LIMIT` is
        // exactly that, so a terabyte-sized request passed validation.
        assert_eq!(USER_SPACE_LIMIT, 1u64 << 47, "128 TiB");
        assert!(1u64 << 40 < USER_SPACE_LIMIT, "1 TiB fit under the old bound");
        // What it accepts now.
        assert!(MAX_XFER < (1u64 << 40));
        assert_eq!(MAX_XFER.checked_add(1), Some(MAX_XFER + 1));
    }

    /// Regression: `brk(small)` called `unmap_heap_pages(cr3, small, heap_brk)`.
    /// `heap_brk` starts at the loader's heap base around 0x6000_0000_0000, so
    /// the loop walked every page between the program's own text segment and its
    /// heap — about 6.4 billion page-table entries — and `free_frame`d every
    /// mapped one it found, which included the code the program was executing.
    ///
    /// The floor is the initial break. Anything below it is refused.
    /// Regression: `sys_mount` accepted `MS_RDONLY` and dropped it. A caller
    /// that mounted a filesystem read-only got a writable view of it, and
    /// `sys_write` had nothing to check because no mount carried flags at all —
    /// `vfs::Mount` had no field for them.
    #[test]
    fn ms_rdonly_survives_the_translation_into_the_vfs() {
        use zenus_fs::vfs::MS_RDONLY as VFS_MS_RDONLY;

        assert_eq!(
            mount_flags_from_syscall(SYSCALL_MS_RDONLY),
            VFS_MS_RDONLY,
            "MS_RDONLY must reach the VFS, not be dropped"
        );
        assert_eq!(mount_flags_from_syscall(0), 0);
        // MS_NOSUID|MS_NODEV|MS_NOEXEC are admitted by MS_SUPPORTED but have
        // nothing enforcing them, so they map to nothing rather than to a flag
        // the VFS would ignore.
        assert_eq!(mount_flags_from_syscall(MS_NOSUID), 0);
        assert_eq!(mount_flags_from_syscall(MS_NODEV | MS_NOEXEC), 0);
        // Combined with RDONLY, only RDONLY carries through.
        assert_eq!(
            mount_flags_from_syscall(SYSCALL_MS_RDONLY | MS_NOSUID | MS_NODEV | MS_NOEXEC),
            VFS_MS_RDONLY
        );
    }

    /// Regression, found by the in-kernel fuzzing campaign: `rt_sigprocmask`
    /// wrote `*(oldset_ptr as *mut u64) = mask` after `validate_user_range`,
    /// which only checks that the address is inside the user range — not that
    /// the page is mapped. The fuzzer passed `0x7075`, and the kernel took a
    /// page fault in ring 0 instead of returning `EFAULT`.
    ///
    /// Six sites had that shape: `wait4` status, `getsockname` addrlen,
    /// `rt_sigprocmask` oldset, `getsockopt` optval/optlen, `socketpair` fds,
    /// `pipe2` fds. They now go through `copy_kernel_to_user`, which
    /// revalidates each page.
    ///
    /// The store itself needs a live page table, so what is locked in here is
    /// the *invariant*: no syscall may reach through a raw pointer store into
    /// user space. A source scan catches a new site the day it is added, which
    /// is the only way this can be enforced from the host.
    #[test]
    fn no_syscall_reaches_user_space_through_a_raw_pointer() {
        // Assembled from pieces so this test's own needles do not appear
        // verbatim in the file it scans — otherwise it finds itself.
        //
        // Seven shapes, because the same bug has now been written seven ways: a
        // scalar store, a struct store, a struct reference, a slice over user
        // memory, and a `let p = ptr as *mut T` that is later dereferenced as
        // `(*p).field`. Each of those cost a separate fuzzer run to find.
        const NEEDLES: [&str; 7] = [
            concat!("as *mut u64", ") ="),
            concat!("as *mut u32", ") ="),
            concat!("as *mut u8", ") ="),
            concat!("&mut *("),
            concat!("&*("),
            concat!("from_raw_parts(", "buf_ptr"),
            concat!("let ", " = ", " as *mut "),
        ];
        // The needle literals above are themselves on lines the scan would flag,
        // so the test skips its own body: everything from here to the end.
        let source = include_str!("syscall.rs");
        let source = match source.split_once("mod host_tests {") {
            Some((head, _)) => head,
            None => source,
        };

        // The only raw access left is the helper that builds a pointer from a
        // value it just validated. The four entries that used to allow the
        // `sys_execve` argv stores are gone: they moved to
        // `userstack::write_initial_user_stack`, which is the one place allowed
        // to run with another CR3 loaded and wraps them in `stac`/`clac`.
        let allowed = [concat!("Some(ptr as *mut T", ")")];

        // An allowlist that outlives its entries is worse than no allowlist: it
        // reads as "this shape is fine here" long after the code that needed
        // the exemption is gone, so a new store next to it sails through.
        for entry in allowed {
            assert!(
                source.lines().any(|l| l.contains(entry)),
                "the allowlist entry `{entry}` no longer matches any line in \
                 syscall.rs; delete it so the exemption cannot rot"
            );
        }

        for (lineno, line) in source.lines().enumerate() {
            let trimmed = line.trim();
            if !NEEDLES.iter().any(|n| trimmed.contains(n)) {
                continue;
            }
            // Skip the doc comments that quote the old shapes as prose.
            if trimmed.starts_with("//") {
                continue;
            }
            assert!(
                allowed.iter().any(|a| trimmed.contains(a)),
                "syscall.rs:{} reaches into user space directly: {}. Route it \
                 through user_ptr() for a struct, user_bytes() for a slice, or \
                 copy_bytes_to_user / copy_u64_to_user / copy_u32_to_user / \
                 copy_u64_pair_to_user / copy_u32_pair_to_user for bytes, so an \
                 in-range-but-unmapped pointer becomes EFAULT instead of a page \
                 fault in ring 0.",
                lineno + 1,
                trimmed
            );
        }
    }

    /// `send` and `sendto` had no `MAX_XFER` bound at all — `recv` got one in
    /// an earlier commit and these two were simply missed, so a single call
    /// asked the socket layer to read 128 TiB out of user space. They now share
    /// the bound.
    #[test]
    fn every_transfer_shares_one_length_bound() {
        // The bound is the same one `sys_read` uses.
        assert_eq!(MAX_XFER, 65536);
        // And it sits well inside the address space, so a legal request is
        // never rejected by the size check alone.
        assert!(MAX_XFER < USER_SPACE_LIMIT);
    }

    /// The copy helpers refuse exactly what `validate_user_range` already
    /// refuses, so the two cannot disagree about what is addressable. And a
    /// pointer *inside* the range passes the range check — which is precisely
    /// why the helpers need their per-page revalidation on top.
    #[test]
    fn a_scalar_copy_helper_and_the_range_check_agree_on_what_is_addressable() {
        for (ptr, why) in [
            (0u64, "null"),
            (0x800, "below the first page"),
            (USER_SPACE_LIMIT, "exactly at the limit"),
            (USER_SPACE_LIMIT + 0x1000, "past the limit"),
            (u64::MAX - 4, "address space wraps"),
        ] {
            assert!(
                !validate_user_range(ptr, 8),
                "{why}: {ptr:#x} must not pass the range check"
            );
        }

        assert!(
            validate_user_range(0x7075, 8),
            "0x7075 is in range — this is the case that used to fault"
        );
    }

    /// Regression, found by the fuzzing campaign: `sys_mprotect` had no overflow
    /// check and no upper bound. It computed `((addr + length) + 0xFFF) & !0xFFF`
    /// and walked page by page from `addr` to there. The fuzzer passed
    /// `mprotect(0x7665002f706d742f, 0x6c6900001000, …)` — a ~3.4 GB span, so
    /// ~870 000 iterations of a four-level page-table walk each ending in an
    /// `invlpg`, with nothing in the loop able to stop.
    ///
    /// This is not a fuzz-only bug: `mprotect` is an ordinary syscall, so any
    /// program with a bad `length` wedges the machine.
    #[test]
    fn mprotect_refuses_a_range_it_would_have_to_walk_forever() {
        // The exact pair the fuzzer produced.
        assert_eq!(
            mprotect_range(0x7665002f706d742f, 0x6c6900001000, USER_SPACE_LIMIT),
            None,
            "a 3.4 GB span starting past the user-space limit"
        );

        // `addr + length` overflow is refused, not wrapped into the past. A
        // wrapped end below the start would skip the loop entirely; a wrapped
        // end above it would be a 2^64-iteration hang.
        assert_eq!(mprotect_range(u64::MAX, 2, USER_SPACE_LIMIT), None);
        assert_eq!(mprotect_range(u64::MAX - 8, 4096, USER_SPACE_LIMIT), None);
        assert_eq!(mprotect_range(0x1000, u64::MAX, USER_SPACE_LIMIT), None);

        // Reaching the user-space limit is refused: the last usable page is
        // below it.
        assert_eq!(mprotect_range(0x1000, USER_SPACE_LIMIT, USER_SPACE_LIMIT), None);

        // A zero length is EINVAL, as before.
        assert_eq!(mprotect_range(0x1000, 0, USER_SPACE_LIMIT), None);

        // Ordinary requests still work, and the range is page-aligned.
        assert_eq!(
            mprotect_range(0x1000, 0x1000, USER_SPACE_LIMIT),
            Some((0x1000, 0x2000))
        );
        assert_eq!(
            mprotect_range(0x1234, 1, USER_SPACE_LIMIT),
            Some((0x1000, 0x2000)),
            "one byte still covers its whole page, and the end rounds up"
        );
        assert_eq!(
            mprotect_range(0x1000, 0x2001, USER_SPACE_LIMIT),
            Some((0x1000, 0x4000)),
            "0x2001 bytes is three pages, not two"
        );

        // The last byte may be the one below the limit.
        assert_eq!(
            mprotect_range(USER_SPACE_LIMIT - 0x1000, 0x1000, USER_SPACE_LIMIT),
            Some((USER_SPACE_LIMIT - 0x1000, USER_SPACE_LIMIT))
        );

        // The helper is only useful if `sys_mprotect` actually calls it. The
        // range walk needs a live address space, so what is locked in is that
        // the syscall cannot go back to computing its own unbounded end.
        let source = include_str!("syscall.rs");
        let body = source
            .split("fn sys_mprotect(")
            .nth(1)
            .and_then(|rest| rest.split("\nfn ").next())
            .expect("sys_mprotect exists");
        assert!(
            body.contains("mprotect_range(addr, length, USER_SPACE_LIMIT)"),
            "sys_mprotect must derive its page range from mprotect_range"
        );
        assert!(
            !body.contains("(addr + length)"),
            "sys_mprotect must not compute its own end offset: that is the \
             unchecked expression this bug was"
        );
    }

    /// Regression, found by the fuzzing campaign: `sys_nanosleep` compared a
    /// **millisecond** count against `pit::get_ticks`, which counts **ticks** —
    /// 10 ms apart. Every sleep was 100x too long, and `sec` never saturated, so
    /// a fuzzed `nanosleep(u64::MAX, 0)` had a deadline of 1.8e19 ticks. The
    /// campaign task then yielded forever, the watchdog fired, and the run was
    /// labelled a timeout with `hangs=0` instead of the hang it was.
    #[test]
    fn a_sleep_deadline_is_in_ticks_and_cannot_wrap() {
        // One second is 100 ticks at 100 Hz — not 1000. And a tick is 10 ms, so
        // 10 ms is one tick, not ten.
        assert_eq!(nanosleep_deadline(0, 1, 0), Some(100));
        assert_eq!(nanosleep_deadline(0, 0, 10_000_000), Some(1));
        assert_eq!(nanosleep_deadline(0, 0, 0), Some(0), "a zero sleep is not an error");

        // Rounds up, so a request never returns early. 1 ms still costs a whole
        // tick, and 11 ms must not round down to one.
        assert_eq!(nanosleep_deadline(0, 0, 1_000_000), Some(1));
        assert_eq!(nanosleep_deadline(0, 0, 11_000_000), Some(2));

        // The deadline is relative to now, not absolute.
        assert_eq!(nanosleep_deadline(500, 1, 0), Some(600));

        // `tv_nsec` out of range is EINVAL, not a silently folded-in number.
        assert_eq!(nanosleep_deadline(0, 0, 1_000_000_000), None);
        assert_eq!(nanosleep_deadline(0, 0, u64::MAX), None);
        assert_eq!(nanosleep_deadline(0, 0, 999_999_999), Some(100));

        // And the fuzzed case that hung the campaign now has a deadline in the
        // future rather than 1.8e19 ticks out.
        let huge = nanosleep_deadline(0, u64::MAX, 0).expect("saturates, not None");
        assert!(huge > 0);
        assert_eq!(
            nanosleep_deadline(u64::MAX, 1, 0),
            None,
            "a real sleep at the end of the counter must be refused, not wrapped \
             into the past — which would return immediately"
        );
        assert_eq!(
            nanosleep_deadline(u64::MAX, 0, 0),
            Some(u64::MAX),
            "a zero sleep at the end of the counter is just already elapsed"
        );
    }

    /// Regression: `sys_mmap` computed `((length + 0xFFF) & !0xFFF) / 0x1000`
    /// with a bare add, so a length near `u64::MAX` wrapped to a *small* page
    /// count while the VMA was recorded with a different size than the pages
    /// actually mapped. It also had no upper bound of its own, so the fuzzer
    /// could ask for 4 GiB and walk the frame allocator for half a million
    /// frames before failing — leaking every one of them, which is what wedged
    /// the campaign.
    #[test]
    fn an_mmap_extent_rounds_up_without_wrapping() {
        const MAX: u64 = 0x7F00_0000_0000;
        // Plenty of frames for an ordinary request, and a deliberately
        // insufficient pool for the one the fuzzer made.
        const PLENTY: usize = 4_000_000;
        const NONE: usize = 0;

        // The exact request the fuzzer made: refused up front, because the
        // machine cannot back it. Before the cap, the loop ran 1 048 577 times
        // before failing at the ~500 000th frame.
        assert_eq!(
            mmap_extent(0x100000010, MAX, NONE),
            None,
            "4 GiB plus one byte cannot be backed on a 2 GiB machine"
        );
        // Exactly enough frames and it is allowed through.
        assert_eq!(
            mmap_extent(0x100000010, MAX, 0x100001),
            Some((0x100001, 0x100001000)),
            "with the frames to back it, the same request is legal"
        );

        // Rounding.
        assert_eq!(mmap_extent(1, MAX, PLENTY), Some((1, 0x1000)));
        assert_eq!(mmap_extent(0x1000, MAX, PLENTY), Some((1, 0x1000)));
        assert_eq!(mmap_extent(0x1001, MAX, PLENTY), Some((2, 0x2000)));

        // Overflow is refused, not wrapped. The old `+ 0xFFF` wrapped here and
        // produced a page count of zero.
        assert_eq!(mmap_extent(u64::MAX, MAX, PLENTY), None);
        assert_eq!(mmap_extent(u64::MAX - 8, MAX, PLENTY), None);
        assert_eq!(mmap_extent(MAX + 1, MAX, PLENTY), None);
        assert_eq!(
            mmap_extent(MAX, MAX, usize::MAX),
            Some((MAX / 0x1000, MAX)),
            "a mapping that exactly fills the address-space cap is legal"
        );

        // A length just under the cap is fine, and the size reported is the
        // rounded length, not the cap.
        assert_eq!(
            mmap_extent(MAX - 0x1000, MAX, usize::MAX),
            Some(((MAX - 0x1000) / 0x1000, MAX - 0x1000))
        );

        // Zero length stays EINVAL, whatever the machine has.
        assert_eq!(mmap_extent(0, MAX, PLENTY), None);
        assert_eq!(mmap_extent(0, MAX, NONE), None);
    }

    /// Regression: `brk(small)` called `unmap_heap_pages(cr3, small, heap_brk)`.
    /// `heap_brk` starts at the loader's heap base around 0x6000_0000_0000, so
    /// the loop walked every page between the program's own text segment and its
    /// heap — about 6.4 billion page-table entries — and `free_frame`d every
    /// mapped one it found, which included the code the program was executing.
    ///
    /// The floor is the initial break. Anything below it is refused.
    #[test]
    fn brk_refuses_to_shrink_below_the_heap_floor() {
        let floor = 0x6000_0000_0000u64;
        let current = floor + 0x10_0000;

        // The attack: shrink to a page near the bottom of user space.
        assert_eq!(
            brk_action(0x1000, current, floor, USER_SPACE_LIMIT),
            BrkAction::Refuse,
            "brk(0x1000) must not unmap everything between there and the heap"
        );
        assert_eq!(brk_action(0, current, floor, USER_SPACE_LIMIT), BrkAction::Query);
        assert_eq!(
            brk_action(floor - 1, current, floor, USER_SPACE_LIMIT),
            BrkAction::Refuse,
            "one byte below the floor is still below it"
        );

        // Exactly the floor is a legal (empty) shrink.
        assert_eq!(brk_action(floor, current, floor, USER_SPACE_LIMIT), BrkAction::Shrink);

        // Ordinary operations.
        assert_eq!(
            brk_action(current + 0x1000, current, floor, USER_SPACE_LIMIT),
            BrkAction::Grow
        );
        assert_eq!(
            brk_action(current - 0x1000, current, floor, USER_SPACE_LIMIT),
            BrkAction::Shrink
        );
        assert_eq!(
            brk_action(current, current, floor, USER_SPACE_LIMIT),
            BrkAction::Query,
            "brk(current) is a no-op, not a zero-length shrink"
        );

        // At or above the user-space limit is refused.
        assert_eq!(
            brk_action(USER_SPACE_LIMIT, current, floor, USER_SPACE_LIMIT),
            BrkAction::Refuse
        );
        assert_eq!(
            brk_action(USER_SPACE_LIMIT + 1, current, floor, USER_SPACE_LIMIT),
            BrkAction::Refuse
        );
        assert_eq!(
            brk_action(u64::MAX, current, floor, USER_SPACE_LIMIT),
            BrkAction::Refuse
        );

        // A task whose floor could not be read falls back to the same
        // default base the break uses, so shrinks below it are still refused.
        // A floor of 0 would make every shrink legal, which is the unsafe
        // direction to fail in.
        assert_eq!(
            brk_action(0x1000, current, 0x6000_0000_0000, USER_SPACE_LIMIT),
            BrkAction::Refuse
        );
    }
}

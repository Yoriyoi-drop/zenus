#![no_std]

//! Minimal userspace libc for Zenus.
//!
//! Provides thin wrappers around Zenus syscalls so userspace programs can be
//! written in Rust without making raw syscall instructions directly.


// ── Syscall numbers

pub const SYS_READ: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_STAT: u64 = 4;
pub const SYS_FSTAT: u64 = 5;
pub const SYS_LSTAT: u64 = 6;
pub const SYS_POLL: u64 = 7;
pub const SYS_LSEEK: u64 = 8;
pub const SYS_MMAP: u64 = 9;
pub const SYS_MPROTECT: u64 = 10;
pub const SYS_MUNMAP: u64 = 11;
pub const SYS_BRK: u64 = 12;
pub const SYS_RT_SIGACTION: u64 = 13;
pub const SYS_RT_SIGPROCMASK: u64 = 14;
pub const SYS_RT_SIGRETURN: u64 = 15;
pub const SYS_IOCTL: u64 = 16;
pub const SYS_PREAD64: u64 = 17;
pub const SYS_PWRITE64: u64 = 18;
pub const SYS_READV: u64 = 19;
pub const SYS_WRITEV: u64 = 20;
pub const SYS_ACCESS: u64 = 21;
pub const SYS_PIPE: u64 = 22;
pub const SYS_SELECT: u64 = 23;
pub const SYS_SCHED_YIELD: u64 = 24;
pub const SYS_MREMAP: u64 = 25;
pub const SYS_MSYNC: u64 = 26;
pub const SYS_MINCORE: u64 = 27;
pub const SYS_MADVISE: u64 = 28;
pub const SYS_SHMGET: u64 = 29;
pub const SYS_SHMAT: u64 = 30;
pub const SYS_SHMCTL: u64 = 31;
pub const SYS_DUP: u64 = 32;
pub const SYS_DUP2: u64 = 33;
pub const SYS_PAUSE: u64 = 34;
pub const SYS_NANOSLEEP: u64 = 35;
pub const SYS_GETITIMER: u64 = 36;
pub const SYS_ALARM: u64 = 37;
pub const SYS_SETITIMER: u64 = 38;
pub const SYS_GETPID: u64 = 39;
pub const SYS_SENDFILE: u64 = 40;
pub const SYS_SOCKET: u64 = 41;
pub const SYS_CONNECT: u64 = 42;
pub const SYS_ACCEPT: u64 = 43;
pub const SYS_SENDTO: u64 = 44;
pub const SYS_RECVFROM: u64 = 45;
pub const SYS_SENDMSG: u64 = 46;
pub const SYS_RECVMSG: u64 = 47;
pub const SYS_SHUTDOWN: u64 = 48;
pub const SYS_BIND: u64 = 49;
pub const SYS_LISTEN: u64 = 50;
pub const SYS_GETSOCKNAME: u64 = 51;
pub const SYS_GETPEERNAME: u64 = 52;
pub const SYS_SOCKETPAIR: u64 = 53;
pub const SYS_SETSOCKOPT: u64 = 54;
pub const SYS_GETSOCKOPT: u64 = 55;
pub const SYS_CLONE: u64 = 56;
pub const SYS_FORK: u64 = 57;
pub const SYS_VFORK: u64 = 58;
pub const SYS_EXECVE: u64 = 59;
pub const SYS_EXIT: u64 = 60;
pub const SYS_WAIT4: u64 = 61;
pub const SYS_KILL: u64 = 62;
pub const SYS_UNAME: u64 = 63;
pub const SYS_FCNTL: u64 = 64;
pub const SYS_FLOCK: u64 = 65;
pub const SYS_FSYNC: u64 = 66;
pub const SYS_FDATASYNC: u64 = 67;
pub const SYS_TRUNCATE: u64 = 68;
pub const SYS_FTRUNCATE: u64 = 69;
pub const SYS_GETDENTS: u64 = 70;
pub const SYS_GETCWD: u64 = 71;
pub const SYS_CHDIR: u64 = 72;
pub const SYS_FCHDIR: u64 = 73;
pub const SYS_RENAME: u64 = 74;
pub const SYS_MKDIR: u64 = 75;
pub const SYS_RMDIR: u64 = 76;
pub const SYS_CREAT: u64 = 77;
pub const SYS_LINK: u64 = 78;
pub const SYS_UNLINK: u64 = 79;
pub const SYS_SYMLINK: u64 = 80;
pub const SYS_READLINK: u64 = 81;
pub const SYS_CHMOD: u64 = 82;
pub const SYS_FCHMOD: u64 = 83;
pub const SYS_CHOWN: u64 = 84;
pub const SYS_FCHOWN: u64 = 85;
pub const SYS_LCHOWN: u64 = 86;
pub const SYS_UMASK: u64 = 87;
pub const SYS_GETTIMEOFDAY: u64 = 88;
pub const SYS_GETRLIMIT: u64 = 89;
pub const SYS_GETRUSAGE: u64 = 90;
pub const SYS_SYSINFO: u64 = 91;
pub const SYS_TIMES: u64 = 92;
pub const SYS_PTRACE: u64 = 93;
pub const SYS_GETUID: u64 = 94;
pub const SYS_SYSLOG: u64 = 95;
pub const SYS_GETGID: u64 = 96;
pub const SYS_SETUID: u64 = 97;
pub const SYS_SETGID: u64 = 98;
pub const SYS_GETEUID: u64 = 99;
pub const SYS_GETEGID: u64 = 100;
pub const SYS_SETPGID: u64 = 101;
pub const SYS_GETPPID: u64 = 102;
pub const SYS_GETPGRP: u64 = 103;
pub const SYS_SETSID: u64 = 104;
pub const SYS_SETREUID: u64 = 105;
pub const SYS_SETREGID: u64 = 106;
pub const SYS_GETGROUPS: u64 = 107;
pub const SYS_SETGROUPS: u64 = 108;
pub const SYS_SETRESUID: u64 = 109;
pub const SYS_GETRESUID: u64 = 110;
pub const SYS_SETRESGID: u64 = 111;
pub const SYS_GETRESGID: u64 = 112;
pub const SYS_GETPGID: u64 = 113;
pub const SYS_SETFSUID: u64 = 114;
pub const SYS_SETFSGID: u64 = 115;
pub const SYS_GETSID: u64 = 116;
pub const SYS_CAPGET: u64 = 117;
pub const SYS_CAPSET: u64 = 118;
pub const SYS_RT_SIGPENDING: u64 = 119;
pub const SYS_RT_SIGTIMEDWAIT: u64 = 120;
pub const SYS_RT_SIGQUEUEINFO: u64 = 121;
pub const SYS_RT_SIGSUSPEND: u64 = 122;
pub const SYS_SETLISTXATTR: u64 = 123;
pub const SYS_LSETLISTXATTR: u64 = 124;
pub const SYS_FSETLISTXATTR: u64 = 125;
pub const SYS_GETXATTR: u64 = 126;
pub const SYS_LGETXATTR: u64 = 127;
pub const SYS_FGETXATTR: u64 = 128;
pub const SYS_LISTXATTR: u64 = 129;
pub const SYS_LLISTXATTR: u64 = 130;
pub const SYS_FLISTXATTR: u64 = 131;
pub const SYS_REMOVEXATTR: u64 = 132;
pub const SYS_LREMOVEXATTR: u64 = 133;
pub const SYS_FREMOVEXATTR: u64 = 134;
pub const SYS_TKILL: u64 = 135;
pub const SYS_TIME: u64 = 136;
pub const SYS_FUTEX: u64 = 137;
pub const SYS_SCHED_SETAFFINITY: u64 = 138;
pub const SYS_SCHED_GETAFFINITY: u64 = 139;
pub const SYS_SET_TID_ADDRESS: u64 = 140;
pub const SYS_SIGALTSTACK: u64 = 141;
pub const SYS_UTIME: u64 = 142;
pub const SYS_MKNOD: u64 = 143;
pub const SYS_USELIB: u64 = 144;
pub const SYS_PROFILE: u64 = 145;
pub const SYS_VSERVER: u64 = 146;
pub const SYS_DCOOKIE: u64 = 147;
pub const SYS_SYNCFS: u64 = 148;
pub const SYS_GETPRIORITY: u64 = 149;
pub const SYS_SETPRIORITY: u64 = 150;
pub const SYS_RENAMEAT: u64 = 151;
pub const SYS_SYNC: u64 = 152;
pub const SYS_MOUNT: u64 = 153;
pub const SYS_UMOUNT2: u64 = 154;
pub const SYS_SWAPON: u64 = 155;
pub const SYS_SWAPOFF: u64 = 156;
pub const SYS_REBOOT: u64 = 157;
pub const SYS_SETHOSTNAME: u64 = 158;
pub const SYS_SETDOMAINNAME: u64 = 159;
pub const SYS_IOPL: u64 = 160;
pub const SYS_IOPERM: u64 = 161;
pub const SYS_CREATE_MODULE: u64 = 162;
pub const SYS_INIT_MODULE: u64 = 163;
pub const SYS_DELETE_MODULE: u64 = 164;
pub const SYS_GET_KERNEL_SYMS: u64 = 165;
pub const SYS_QUERY_MODULE: u64 = 166;
pub const SYS_QUERYCTL: u64 = 167;
pub const SYS_SYSCTL: u64 = 168;
pub const SYS_SIGRETURN: u64 = 169;
pub const SYS_PRCTL: u64 = 244;

// ── Errno ─────────────────────────────────────────────────────────────────

pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const ESRCH: i32 = 3;
pub const EINTR: i32 = 4;
pub const EIO: i32 = 5;
pub const ENXIO: i32 = 6;
pub const E2BIG: i32 = 7;
pub const ENOEXEC: i32 = 8;
pub const EBADF: i32 = 9;
pub const ECHILD: i32 = 10;
pub const EDEADLK: i32 = 11;
pub const ENOMEM: i32 = 12;
pub const EACCES: i32 = 13;
pub const EFAULT: i32 = 14;
pub const ENOTBLK: i32 = 15;
pub const EBUSY: i32 = 16;
pub const EEXIST: i32 = 17;
pub const EXDEV: i32 = 18;
pub const ENODEV: i32 = 19;
pub const ENOTDIR: i32 = 20;
pub const EISDIR: i32 = 21;
pub const EINVAL: i32 = 22;
pub const ENFILE: i32 = 23;
pub const EMFILE: i32 = 24;
pub const ENOTTY: i32 = 25;
pub const ENOSPC: i32 = 28;
pub const ESPIPE: i32 = 29;
pub const EROFS: i32 = 30;
pub const EMLINK: i32 = 31;
pub const EPIPE: i32 = 32;

// ── Raw syscall ────────────────────────────────────────────────────────────

#[cfg(target_os = "none")]
#[inline(always)]
unsafe fn syscall6(num: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") num,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        in("r8") a5,
        in("r9") a6,
        lateout("rax") ret,
        options(nostack),
    );
    ret
}

#[cfg(target_os = "none")]
#[inline(always)]
unsafe fn syscall0(num: u64) -> i64 {
    syscall6(num, 0, 0, 0, 0, 0, 0)
}

#[cfg(not(target_os = "none"))]
#[inline(always)]
unsafe fn syscall0(_num: u64) -> i64 {
    -1
}

#[cfg(target_os = "none")]
#[inline(always)]
unsafe fn syscall1(num: u64, a1: u64) -> i64 {
    syscall6(num, a1, 0, 0, 0, 0, 0)
}

#[cfg(not(target_os = "none"))]
#[inline(always)]
unsafe fn syscall1(_num: u64, _a1: u64) -> i64 {
    -1
}

#[cfg(target_os = "none")]
#[inline(always)]
unsafe fn syscall2(num: u64, a1: u64, a2: u64) -> i64 {
    syscall6(num, a1, a2, 0, 0, 0, 0)
}

#[cfg(not(target_os = "none"))]
#[inline(always)]
unsafe fn syscall2(_num: u64, _a1: u64, _a2: u64) -> i64 {
    -1
}

#[cfg(target_os = "none")]
#[inline(always)]
unsafe fn syscall3(num: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    syscall6(num, a1, a2, a3, 0, 0, 0)
}

#[cfg(not(target_os = "none"))]
#[inline(always)]
unsafe fn syscall3(_num: u64, _a1: u64, _a2: u64, _a3: u64) -> i64 {
    -1
}

#[cfg(target_os = "none")]
#[inline(always)]
unsafe fn syscall4(num: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    syscall6(num, a1, a2, a3, a4, 0, 0)
}

// ── Result type ────────────────────────────────────────────────────────────

type LibcResult<T> = core::result::Result<T, i32>;

fn ret_result(ret: i64) -> LibcResult<i64> {
    if ret >= 0 {
        Ok(ret)
    } else {
        Err(-ret as i32)
    }
}

// ── Process control ────────────────────────────────────────────────────────

pub fn exit(status: i32) -> ! {
    unsafe { syscall1(SYS_EXIT, status as u64) };
    unreachable!()
}

pub fn fork() -> LibcResult<PidT> {
    unsafe { ret_result(syscall0(SYS_FORK)).map(|r| r as PidT) }
}

pub fn execve(path: *const u8, argv: *const *const u8, envp: *const *const u8) -> LibcResult<isize> {
    unsafe { ret_result(syscall3(SYS_EXECVE, path as u64, argv as u64, envp as u64)).map(|r| r as isize) }
}

pub fn waitpid(pid: PidT, stat_loc: *mut i32, options: i32) -> LibcResult<PidT> {
    unsafe { ret_result(syscall3(SYS_WAIT4, pid as u64, stat_loc as u64, options as u64)).map(|r| r as PidT) }
}

pub fn kill(pid: PidT, sig: i32) -> LibcResult<isize> {
    unsafe { ret_result(syscall2(SYS_KILL, pid as u64, sig as u64)).map(|r| r as isize) }
}

pub fn getpid() -> PidT {
    unsafe { syscall0(SYS_GETPID) as PidT }
}

pub fn getppid() -> PidT {
    unsafe { syscall0(SYS_GETPPID) as PidT }
}

// ── File operations ────────────────────────────────────────────────────────

pub fn open(path: *const u8, flags: i32, mode: i32) -> LibcResult<FdT> {
    unsafe { ret_result(syscall3(SYS_OPEN, path as u64, flags as u64, mode as u64)).map(|r| r as FdT) }
}

pub fn close(fd: FdT) -> LibcResult<isize> {
    unsafe { ret_result(syscall1(SYS_CLOSE, fd as u64)).map(|r| r as isize) }
}

pub fn read(fd: FdT, buf: *mut u8, count: usize) -> LibcResult<isize> {
    unsafe { ret_result(syscall3(SYS_READ, fd as u64, buf as u64, count as u64)).map(|r| r as isize) }
}

pub fn write(fd: FdT, buf: *const u8, count: usize) -> LibcResult<isize> {
    unsafe { ret_result(syscall3(SYS_WRITE, fd as u64, buf as u64, count as u64)).map(|r| r as isize) }
}

pub fn lseek(fd: FdT, offset: i64, whence: i32) -> LibcResult<isize> {
    unsafe { ret_result(syscall3(SYS_LSEEK, fd as u64, offset as u64, whence as u64)).map(|r| r as isize) }
}

pub fn pipe(pipefd: *mut i32) -> LibcResult<isize> {
    unsafe { ret_result(syscall1(SYS_PIPE, pipefd as u64)).map(|r| r as isize) }
}

pub fn dup(oldfd: FdT) -> LibcResult<FdT> {
    unsafe { ret_result(syscall1(SYS_DUP, oldfd as u64)).map(|r| r as FdT) }
}

pub fn dup2(oldfd: FdT, newfd: FdT) -> LibcResult<FdT> {
    unsafe { ret_result(syscall2(SYS_DUP2, oldfd as u64, newfd as u64)).map(|r| r as FdT) }
}

// ── Types ──────────────────────────────────────────────────────────────────

pub type PidT = i32;
pub type FdT = i32;
pub type SsizeT = isize;
pub type SizeT = usize;
pub type OffT = i64;
pub type ModeT = u32;
pub type DevT = u64;

pub const O_RDONLY: i32 = 0;
pub const O_WRONLY: i32 = 1;
pub const O_RDWR: i32 = 2;
pub const O_CREAT: i32 = 64;
pub const O_EXCL: i32 = 128;
pub const O_NOCTTY: i32 = 256;
pub const O_TRUNC: i32 = 512;
pub const O_APPEND: i32 = 1024;

pub const STDIN: FdT = 0;
pub const STDOUT: FdT = 1;
pub const STDERR: FdT = 2;

#[cfg(test)]
mod tests {
    #[test]
    fn errno_constants_are_positive() {
        assert!(super::EPERM > 0);
        assert!(super::ENOENT > 0);
        assert!(super::EINVAL > 0);
        assert!(super::ENOMEM > 0);
    }

    #[test]
    fn syscall_numbers_match_kernel_range() {
        assert!(super::SYS_READ < 256);
        assert!(super::SYS_WRITE < 256);
        assert!(super::SYS_EXECVE < 256);
        assert!(super::SYS_EXIT < 256);
        assert!(super::SYS_FORK < 256);
        assert!(super::SYS_PIPE < 256);
        assert!(super::SYS_WAIT4 < 256);
        assert!(super::SYS_KILL < 256);
    }
}

use core::fmt::Write;
use zenus_console::serial::SerialPort;
use zenus_fs::vfs;
use zenus_sched::scheduler;

mod fd;
use fd::*;

const USER_SPACE_LIMIT: u64 = 0x0000_8000_0000_0000;

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
    // Check that the pages are actually mapped in the current page table
    let start_page = ptr & !0xFFF;
    let end_page = ((end + 0xFFF) & !0xFFF).min(USER_SPACE_LIMIT);
    let mut page = start_page;
    while page < end_page {
        let cr3: u64;
        unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags)); }
        if zenus_mem::paging::virt_to_phys_raw(cr3, page).is_none() {
            return false;
        }
        page += 0x1000;
    }
    true
}

fn validate_user_ptr<T>(ptr: u64) -> bool {
    validate_user_range(ptr, core::mem::size_of::<T>() as u64)
}

const SYS_READ: u64 = 0;
const SYS_WRITE: u64 = 1;
const SYS_OPEN: u64 = 2;
const SYS_CLOSE: u64 = 3;
const SYS_STAT: u64 = 4;
const SYS_READDIR: u64 = 5;
const SYS_LSEEK: u64 = 8;
const SYS_IOCTL: u64 = 16;
const SYS_DUP: u64 = 32;
const SYS_NANOSLEEP: u64 = 35;
const SYS_GETPID: u64 = 39;
const SYS_BRK: u64 = 45;
const SYS_EXIT: u64 = 60;
const SYS_UNAME: u64 = 63;
const SYS_GETUID: u64 = 100;
const SYS_GETEUID: u64 = 101;
const SYS_GETGID: u64 = 102;
const SYS_GETEGID: u64 = 103;
const SYS_SETUID: u64 = 104;
const SYS_SETGID: u64 = 105;

type SyscallFn = fn(u64, u64, u64, u64, u64, u64) -> u64;

static SYSCALL_TABLE: [Option<SyscallFn>; 128] = init_table();

const fn init_table() -> [Option<SyscallFn>; 128] {
    let mut t: [Option<SyscallFn>; 128] = [None; 128];
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
    t
}

fn current_task() -> u64 {
    scheduler::current_task_id()
}

fn copy_user_to_kernel(user_ptr: u64, len: usize) -> Option<alloc::vec::Vec<u8>> {
    if len == 0 { return Some(alloc::vec::Vec::new()); }
    if !validate_user_range(user_ptr, len as u64) { return None; }
    let mut buf = alloc::vec::Vec::with_capacity(len);
    buf.resize(len, 0);
    // Copy per-page with re-validation to minimize TOCTOU window
    let mut copied = 0;
    while copied < len {
        let current = user_ptr + copied as u64;
        let remaining = len - copied;
        let chunk = remaining.min(4096 - (current & 0xFFF) as usize);
        let cr3: u64;
        unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags)); }
        if zenus_mem::paging::virt_to_phys_raw(cr3, current).is_none() {
            return None;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                current as *const u8,
                buf.as_mut_ptr().add(copied),
                chunk,
            );
        }
        copied += chunk;
    }
    Some(buf)
}

fn copy_kernel_to_user(kernel_buf: &[u8], user_ptr: u64) -> bool {
    if kernel_buf.is_empty() { return true; }
    if !validate_user_range(user_ptr, kernel_buf.len() as u64) { return false; }
    let mut copied = 0;
    while copied < kernel_buf.len() {
        let current = user_ptr + copied as u64;
        let remaining = kernel_buf.len() - copied;
        let chunk = remaining.min(4096 - (current & 0xFFF) as usize);
        let cr3: u64;
        unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags)); }
        if zenus_mem::paging::virt_to_phys_raw(cr3, current).is_none() {
            return false;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                kernel_buf.as_ptr().add(copied),
                current as *mut u8,
                chunk,
            );
        }
        copied += chunk;
    }
    true
}

fn sys_read(fd: u64, buf: u64, count: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if count > 1048576 { return -1i64 as u64; }
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
    if count > 1048576 { return -1i64 as u64; }
    let kernel_buf = match copy_user_to_kernel(buf, count as usize) {
        Some(b) => b,
        None => return -1i64 as u64,
    };
    match fd_write(fd, &kernel_buf) {
        Some(n) => n,
        None => -1i64 as u64,
    }
}

fn sys_open(path_ptr: u64, _flags: u64, _mode: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_ptr::<u8>(path_ptr) { return -1i64 as u64; }
    let path = unsafe { core::ffi::CStr::from_ptr(path_ptr as *const i8) };
    let path_str = match path.to_str() {
        Ok(s) => s,
        Err(_) => return -1i64 as u64,
    };
    match fd_open(current_task(), path_str) {
        Some(fd) => fd,
        None => -1i64 as u64,
    }
}

fn sys_close(fd: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if fd <= 2 { return -1i64 as u64; }
    if fd_close(fd) { 0 } else { -1i64 as u64 }
}

#[repr(C)]
struct StatBuf {
    st_size: u64,
    st_mode: u64,
    st_ino: u64,
}

fn sys_stat(path_ptr: u64, stat_ptr: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if !validate_user_ptr::<u8>(path_ptr) { return -1i64 as u64; }
    if !validate_user_ptr::<StatBuf>(stat_ptr) { return -1i64 as u64; }
    let path = unsafe { core::ffi::CStr::from_ptr(path_ptr as *const i8) };
    let path_str = match path.to_str() {
        Ok(s) => s,
        Err(_) => return -1i64 as u64,
    };

    match vfs::open(path_str) {
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
    if !validate_user_range(buf, buf_size) { return -1i64 as u64; }
    let entries = fd_readdir(fd);
    if entries.is_empty() { return 0; }

    let dst = buf as *mut u8;
    let max = buf_size as usize;
    let mut written = 0u64;

    for entry in entries {
        if written as usize + 2 > max { break; }
        let name_bytes = entry.name.as_bytes();
        let name_len = name_bytes.len();
        if written as usize + 1 + name_len + 1 > max { break; }

        unsafe {
            dst.add(written as usize).write(entry.file_type as u8);
            written += 1;
            dst.add(written as usize).write(name_len as u8);
            written += 1;
            if name_len > 0 {
                core::ptr::copy_nonoverlapping(name_bytes.as_ptr(), dst.add(written as usize), name_len);
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
            let mut s = SerialPort::new(0x3F8);
            let _ = write!(s, "[ioctl] fd={} req=0x{:x} arg=0x{:x}\n", fd, request, arg);
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
        old_fd // stdio stays the same
    }
}

fn sys_nanosleep(sec: u64, nsec: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let mut s = SerialPort::new(0x3F8);
    let _ = write!(s, "[sys] nanosleep {}s {}ns\n", sec, nsec);
    let total_ms = sec.saturating_mul(1000).saturating_add(nsec / 1_000_000);
    let start = zenus_arch::interrupts::pit::get_ticks();
    loop {
        zenus_sched::scheduler::yield_now();
        let elapsed = zenus_arch::interrupts::pit::get_ticks().wrapping_sub(start);
        if elapsed >= total_ms {
            break;
        }
    }
    0
}

fn sys_getpid(_a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    current_task()
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
    if addr < heap_start || addr > USER_SPACE_LIMIT {
        return -1i64 as u64;
    }
    let cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags)); }
    if !map_heap_pages(cr3, heap_start, addr) {
        return -1i64 as u64;
    }
    scheduler::set_task_heap_brk(task, addr);
    addr
}

fn sys_exit(_fd: u64, _buf: u64, _count: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    let tid = current_task();
    let mut s = SerialPort::new(0x3F8);
    let _ = write!(s, "[sys] task {} exit\n", tid);
    if tid > 0 {
        fd_close_all_for_task(tid);
        scheduler::task_exit();
    }
    loop { unsafe { core::arch::asm!("hlt", options(nostack, preserves_flags)); } }
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
    if !validate_user_ptr::<UtsName>(buf) { return -1i64 as u64; }
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
    if zenus_sched::scheduler::set_current_uid(uid as u32) { 0 } else { -1i64 as u64 }
}

fn sys_setgid(gid: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    if zenus_sched::scheduler::set_current_gid(gid as u32) { 0 } else { -1i64 as u64 }
}

fn copy_str_to_fixed(dst: &mut [u8], s: &str) {
    let len = s.len().min(dst.len() - 1);
    dst[..len].copy_from_slice(&s.as_bytes()[..len]);
    dst[len] = 0;
}

#[no_mangle]
pub extern "C" fn syscall_dispatch(
    num: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
) -> u64 {
    if num >= 128 { return -1i64 as u64; }
    match SYSCALL_TABLE[num as usize] {
        Some(f) => f(arg1, arg2, arg3, 0, 0, 0),
        None => {
            let mut s = SerialPort::new(0x3F8);
            let _ = write!(s, "[sys] unknown syscall {}\n", num);
            -1i64 as u64
        }
    }
}

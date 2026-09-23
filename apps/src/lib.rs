#![no_std]
#![no_main]

extern crate alloc;

use core::panic::PanicInfo;

use zenus_arch::cpu;

#[cfg(feature = "testing")]
mod test_runner;

fn ata_read0(lba: u64, buf: &mut [u8]) -> bool {
    zenus_arch::ata::read_sectors(0, lba, 1, buf)
}
fn ata_write0(lba: u64, buf: &[u8]) -> bool {
    zenus_arch::ata::write_sectors(0, lba, 1, buf)
}
fn ata_read1(lba: u64, buf: &mut [u8]) -> bool {
    zenus_arch::ata::read_sectors(1, lba, 1, buf)
}
fn ata_write1(lba: u64, buf: &[u8]) -> bool {
    zenus_arch::ata::write_sectors(1, lba, 1, buf)
}
fn ata_read2(lba: u64, buf: &mut [u8]) -> bool {
    zenus_arch::ata::read_sectors(2, lba, 1, buf)
}
fn ata_write2(lba: u64, buf: &[u8]) -> bool {
    zenus_arch::ata::write_sectors(2, lba, 1, buf)
}
fn ata_read3(lba: u64, buf: &mut [u8]) -> bool {
    zenus_arch::ata::read_sectors(3, lba, 1, buf)
}
fn ata_write3(lba: u64, buf: &[u8]) -> bool {
    zenus_arch::ata::write_sectors(3, lba, 1, buf)
}

// ── procfs data generators ──

fn gen_proc_cpuinfo() -> alloc::string::String {
    use core::fmt::Write;
    let cpu = zenus_arch::smp::cpu_count();
    let mut s = alloc::string::String::new();
    for i in 0..cpu {
        let _ = write!(s, "processor\t: {}\nvendor_id\t: Zenus\ncpu family\t: 1\nmodel\t\t: 1\nmodel name\t: Zenus OS x86_64\ncpu MHz\t\t: 2500.000\nphysical id\t: 0\ncore id\t\t: {}\ncpu cores\t: {}\n\n", i, i, cpu);
    }
    s
}

fn gen_proc_meminfo() -> alloc::string::String {
    use core::fmt::Write;
    use zenus_mem::frame_allocator::FRAME_ALLOCATOR;
    let fa = FRAME_ALLOCATOR.lock();
    let total_kb = fa.total_memory() / 1024;
    let used_kb = fa.used_memory() / 1024;
    let free_kb = if total_kb > used_kb {
        total_kb - used_kb
    } else {
        0
    };
    drop(fa);
    let mut s = alloc::string::String::new();
    let _ = write!(s, "MemTotal:       {} kB\nMemFree:        {} kB\nMemAvailable:   {} kB\nBuffers:        0 kB\nCached:         0 kB\nSwapTotal:      0 kB\nSwapFree:       0 kB\nActive:         {} kB\nInactive:       0 kB\n", total_kb, free_kb, free_kb, used_kb);
    s
}

fn gen_proc_uptime() -> alloc::string::String {
    use core::fmt::Write;
    let ticks = zenus_sched::scheduler::uptime_ticks();
    let secs = ticks / 100;
    let mut s = alloc::string::String::new();
    let _ = write!(s, "{}.{} {}.{}\n", secs, 0u64, secs / 2, 0u64);
    s
}

fn gen_proc_stat() -> alloc::string::String {
    use core::fmt::Write;
    let ticks = zenus_sched::scheduler::uptime_ticks();
    let cpu_user = ticks / 3;
    let cpu_nice = ticks / 10;
    let cpu_system = ticks / 3;
    let cpu_idle = ticks / 3;
    let mut s = alloc::string::String::new();
    let _ = write!(s, "cpu  {} {} {} {} 0 0 0 0 0 0\nintr 0\nctxt 0\nbtime {}\nprocesses {}\nprocs_running 1\nprocs_blocked 0\n", cpu_user, cpu_nice, cpu_system, cpu_idle, 1700000000u64, zenus_sched::scheduler::task_count());
    s
}

fn gen_proc_loadavg() -> alloc::string::String {
    use core::fmt::Write;
    let running = zenus_sched::scheduler::task_count();
    let mut s = alloc::string::String::new();
    let _ = write!(s, "0.00 0.00 0.00 {}/{} 0\n", running, running);
    s
}

fn gen_proc_task_count() -> u64 {
    zenus_sched::scheduler::task_count()
}

extern crate zenus_syscall;

#[used]
#[link_section = ".limine_reqs"]
static _FORCE_LIMINE: [u64; 0] = [];
use zenus_arch::interrupts;
use zenus_arch::smp;
use zenus_console::serial::SerialPort;
use zenus_fs::vfs::FileSystem as _;
use zenus_mem::paging;

mod shell;

use zenus_mem::frame_allocator;
use zenus_mem::frame_allocator::MemoryRegion;
use zenus_sched::scheduler;

struct EchoState {
    listen_fds: [Option<usize>; 8],
    client_fds: [Option<usize>; 16],
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    if let Some(loc) = info.location() {
        zenus_console::kpanic_code!(
            zenus_console::error::codes::KRN_UNHANDLED_EXCEPTION,
            "Panic at {}:{} — {}",
            loc.file(),
            loc.line(),
            info.message()
        );
    } else {
        zenus_console::kpanic_code!(
            zenus_console::error::codes::KRN_UNHANDLED_EXCEPTION,
            "Panic: {}",
            info.message()
        );
    }
}

/// Execute a userspace ELF binary at boot, before the shell starts.
/// Opens the file from VFS, loads it as ELF, creates a user task,
/// and waits for it to complete. This is the same flow as `run`
/// command but runs before the interactive shell.
/// Returns true if execution was successful, false otherwise.
fn boot_run_userspace(path: &str) -> bool {
    use zenus_fs::vfs;
    use zenus_mem::paging;

    zenus_console::kinfo!("boot-run: opening {}", path);

    let node = match vfs::open(path) {
        Some(n) => n,
        None => {
            zenus_console::kinfo!("boot-run: file not found: {}", path);
            return false;
        }
    };

    let stat = node.fs.stat(node.inode);
    zenus_console::kinfo!("boot-run: file size={}", stat.size);

    if stat.size < 64 || stat.size > 1024 * 1024 {
        zenus_console::kinfo!("boot-run: invalid file size");
        return false;
    }

    let size = stat.size as usize;
    let mut data = alloc::vec![0u8; size];
    if node.fs.read(node.inode, 0, &mut data).is_none() {
        zenus_console::kinfo!("boot-run: read failed");
        return false;
    }

    zenus_console::kinfo!("boot-run: creating address space");

    let new_cr3 = match paging::create_address_space() {
        Some(c) => c,
        None => {
            zenus_console::kinfo!("boot-run: failed to create address space");
            return false;
        }
    };

    zenus_console::kinfo!("boot-run: loading ELF");

    let loaded = match zenus_syscall::elf::load_elf_raw(&data, new_cr3) {
        Some(e) => e,
        None => {
            paging::destroy_address_space(new_cr3);
            zenus_console::kinfo!("boot-run: invalid ELF: {}", path);
            return false;
        }
    };

    zenus_console::kinfo!(
        "boot-run: ELF loaded entry=0x{:x} stack_top=0x{:x}",
        loaded.entry,
        loaded.stack_top
    );

    // Set up argv on user stack by writing DIRECTLY to the physical page
    // via HHDM. The physical address of the last stack page is returned by
    // load_elf_raw as stack_first_phys. Compute offsets within that page.
    // This avoids CR3 switching + TLB / SMAP issues entirely.
    let user_stack_top = loaded.stack_top;
    let hhdm = paging::hhdm_offset();
    let stack_phys = loaded.stack_first_phys;

    let path_bytes = path.as_bytes();
    let path_len = path_bytes.len();
    let str_pos = user_stack_top - path_len as u64 - 1;
    let ptr_area = (str_pos - 16) & !7u64;
    let user_rsp = ptr_area - 8;

    // Compute physical addresses: base physical + offset within page
    let phys_str = stack_phys + (str_pos & 0xFFF);
    let phys_ptr = stack_phys + (ptr_area & 0xFFF);
    let phys_rsp = stack_phys + (user_rsp & 0xFFF);

    // Write path string at str_pos
    let dst_str = (hhdm + phys_str) as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(path_bytes.as_ptr(), dst_str, path_len);
        *dst_str.add(path_len) = 0; // null terminator
    }

    // Write argv[0] = str_pos, argv[1] = NULL at ptr_area
    let dst_ptr = (hhdm + phys_ptr) as *mut u64;
    unsafe {
        *dst_ptr = str_pos; // argv[0] = pointer to path string
        *dst_ptr.add(1) = 0; // argv[1] = NULL
    }

    // Write argc = 1 at user_rsp
    let dst_rsp = (hhdm + phys_rsp) as *mut u64;
    unsafe {
        *dst_rsp = 1;
    }

    // Safe operations after SMAP bypass + CR3 restore
    let stack_size = 65536;
    let pid = scheduler::create_user_task(
        loaded.entry,
        stack_size,
        user_rsp,
        new_cr3,
        loaded.heap_base,
    );

    if pid == 0 {
        paging::destroy_address_space(new_cr3);
        zenus_console::kinfo!("boot-run: failed to create task");
        return false;
    }

    zenus_console::kinfo!("boot-run: PID {} created, waiting...", pid);

    // Wait for child using wait_for_child() with ZOMBIE_LIST.
    // exit_current_task() stores a zombie record, so wait_for_child()
    // can find it. This is the same mechanism used by the shell `run`
    // command (verified working).
    let ppid = scheduler::current_task_id();
    for _ in 0u64..2000u64 {
        scheduler::yield_now();
        if let Some((cpid, exit_code)) = scheduler::wait_for_child(ppid, pid, 1) {
            zenus_console::kinfo!("boot-run: PID {} exited with code {}", cpid, exit_code);
            zenus_console::serial::flush_output_blocking();
            scheduler::reap_task(cpid);
            return true;
        }
    }

    zenus_console::kinfo!("boot-run: TIMEOUT waiting for PID {}", pid);
    zenus_console::serial::flush_output_blocking();
    scheduler::signal_force_kill(pid);
    scheduler::reap_task(pid);
    return false;
}

fn shell_task() {
    // Skip userspace test programs for now, go straight to interactive shell
    let mut shell = shell::Shell::new();
    shell.run();
}

#[cfg(not(feature = "testing"))]
#[no_mangle]
pub extern "C" fn entry() -> ! {
    SerialPort::init();
    zenus_console::log::dmesg_init();

    // Write initial boot message and flush — ensures serial port works even
    // if we crash later. This bypasses the buffer for absolute reliability.
    // Boot marker — single character to confirm UART works
    zenus_console::serial::uart_write_byte_emergency(b'Z');

    if zenus_arch::limine::MEMMAP_REQUEST.response.is_null() {
        zenus_console::kpanic_code!(
            zenus_console::error::codes::KRN_PANIC_INVALID_MEM,
            "MEMMAP_REQUEST response is null"
        );
    }
    if zenus_arch::limine::HHDM_REQUEST.response.is_null() {
        zenus_console::kpanic_code!(
            zenus_console::error::codes::KRN_PANIC_INVALID_MEM,
            "HHDM_REQUEST response is null"
        );
    }
    let hhdm_offset = unsafe {
        let hhdm_resp: &zenus_arch::limine::LimineHhdmResponse =
            &*zenus_arch::limine::HHDM_REQUEST.response.as_ptr();
        hhdm_resp.offset
    };
    zenus_arch::limine::store_hhdm_offset(hhdm_offset);

    let memmap_resp: &zenus_arch::limine::LimineMemmapResponse =
        unsafe { &*zenus_arch::limine::MEMMAP_REQUEST.response.as_ptr() };
    let entry_ptrs = memmap_resp
        .entries
        .as_ptr::<*mut zenus_arch::limine::LimineMemmapEntry>();
    let mut regions: [MemoryRegion; 64] = [MemoryRegion {
        base: 0,
        length: 0,
        kind: 0,
    }; 64];
    let mut region_count = 0;
    let total = core::cmp::min(memmap_resp.entry_count as usize, 64);
    unsafe {
        for i in 0..total {
            let entry_ptr = *entry_ptrs.add(i);
            if entry_ptr.is_null() {
                continue;
            }
            let entry = &*entry_ptr;
            regions[i] = MemoryRegion {
                base: entry.base,
                length: entry.length,
                kind: entry.kind,
            };
            region_count += 1;
        }
    }
    let mem_regions = &regions[..region_count];

    cpu::init();
    frame_allocator::global_init(mem_regions);
    paging::init(hhdm_offset);
    frame_allocator::reserve_boot_stack(hhdm_offset);
    interrupts::init();
    // Fix page table permissions (clear U/S bit at all levels for all
    // present PML4 entries 0-511), then enable SMEP + SMAP.
    // NOTE: Requires QEMU `-cpu max` or a CPU with SMEP/SMAP support.
    //
    // DISABLED (2026-07-19): SMAP causes GPF in userspace programs (args/pipe_test)
    // because write to user stack via boot_run_userspace with stac does not
    // reliably write the expected values. Root cause suspected in PML4 U/S interaction
    // between ensure_kernel_pages_supervisor and create_address_space / map_user_page_raw.
    // Enable after fixing: uncomment the two lines below.
    //zenus_mem::paging::ensure_kernel_pages_supervisor();
    //cpu::enable_smep_smap();
    zenus_console::vga::init(hhdm_offset);

    // Initialize framebuffer console if available (UEFI/GOP boot)
    if let Some((fb_phys, fb_width, fb_height, fb_bpp, fb_pitch)) =
        zenus_arch::limine::framebuffer_info()
    {
        let fb_virt = fb_phys + hhdm_offset;
        let fb_info = zenus_console::fb::Framebuffer {
            addr: fb_virt,
            width: fb_width,
            height: fb_height,
            bpp: fb_bpp,
            pitch: fb_pitch,
        };
        zenus_console::fb::init(&fb_info);
    }

    let apic_base_raw = unsafe { cpu::read_msr(0x1B) };
    let apic_base = apic_base_raw & 0xFFFFF000;
    interrupts::apic::init_with_virt(apic_base + hhdm_offset);
    // Use PIT → PIC → ExtINT for timer interrupts (stable, tested).
    // PIT IRQ0 and IRQ1 stay unmasked (PIC mask 0xFC from remap_pic).
    // PIC EOI is sent in schedule_tick to prevent interrupt flooding.
    interrupts::pit::init();
    interrupts::apic::enable_pic_lint0();
    zenus_arch::rtc::init();
    zenus_arch::rtc::cache_boot_epoch();
    zenus_arch::random::init_rng();

    // AMAN: interrupts tetap disabled selama boot. Timer ISR menyebabkan
    // race condition karena schedule_tick() mengakses spinlock yang sama
    // dengan init code. Kita enable interrupts nanti setelah shell task
    // dibuat dan init selesai. Output di-flush manual dengan flush_output_blocking().

    zenus_arch::keyboard::init();
    // Register serial IRQ (UART interrupt-driven I/O) for KVM compatibility.
    // With in-kernel irqchip, KVM doesn't process host stdin during HLT.
    // UART interrupts force a VM exit, allowing QEMU to read stdin.
    // Route IRQ4 (COM1) through IOAPIC → vector 36, then enable IER bit 0.
    if zenus_arch::interrupts::ioapic::is_initialized() {
        let apic_id = zenus_arch::interrupts::apic::current_apic_id() as u8;
        if zenus_arch::interrupts::ioapic::route_irq(4, 36u8, apic_id) {
            zenus_console::kinfo!("Serial IRQ4 -> vector 36 (IOAPIC)");
        } else {
            zenus_console::kwarn!("Serial IRQ4 -> IOAPIC FAILED");
        }
    }
    zenus_console::serial::enable_serial_interrupts();
    scheduler::init();
    zenus_console::serial::flush_output_blocking();

    zenus_fs::vfs::init();
    zenus_console::serial::flush_output_blocking();
    zenus_fs::vfs::create_dir("/dev");
    let devfs: &dyn zenus_fs::vfs::FileSystem = &zenus_fs::devfs::DevFs;
    zenus_fs::vfs::mount("/dev", devfs);
    zenus_fs::vfs::create_dir("/tmp");
    zenus_fs::vfs::create_dir("/proc");
    static PROCFS: zenus_fs::procfs::ProcFs = zenus_fs::procfs::ProcFs;
    zenus_fs::vfs::mount("/proc", &PROCFS);
    zenus_fs::vfs::create_dir("/sys");
    zenus_fs::vfs::create_dir("/sys/fs");
    zenus_fs::vfs::create_dir("/sys/fs/cgroup");
    static CGROUP2: zenus_fs::cgroup::CgroupFs = zenus_fs::cgroup::CgroupFs;
    zenus_fs::vfs::mount("/sys/fs/cgroup", &CGROUP2);

    if !zenus_arch::limine::MODULE_REQUEST.response.is_null() {
        unsafe {
            let mod_resp: &zenus_arch::limine::LimineModuleResponse =
                &*zenus_arch::limine::MODULE_REQUEST.response.as_ptr();
            if mod_resp.module_count > 0 {
                let mod_ptrs = mod_resp
                    .modules
                    .as_ptr::<*mut zenus_arch::limine::LimineFile>();
                let module = &**mod_ptrs;
                let initrd_virt = module.address.0;
                let _mod_data =
                    core::slice::from_raw_parts(initrd_virt as *const u8, module.size as usize);
                if let Some(tarfs) = zenus_fs::tarfs::TarFs::load(initrd_virt, module.size) {
                    zenus_fs::vfs::mount("/initrd", tarfs);
                }
            }
        }
    }

    zenus_arch::crash::crash_dump_init();
    zenus_console::syslog::syslog_init();
    zenus_fs::sysctl::sysctl_init();
    zenus_fs::pkg::pkg_init();

    // Register procfs data sources
    zenus_fs::procfs::register_cpuinfo(gen_proc_cpuinfo);
    zenus_fs::procfs::register_meminfo(gen_proc_meminfo);
    zenus_fs::procfs::register_uptime(gen_proc_uptime);
    zenus_fs::procfs::register_stat(gen_proc_stat);
    zenus_fs::procfs::register_loadavg(gen_proc_loadavg);
    zenus_fs::procfs::register_task_count(gen_proc_task_count);

    zenus_ns::uts::init();
    zenus_ns::pid::init();
    zenus_ns::mnt::init();
    zenus_ns::net::init();
    zenus_ns::user::init();
    zenus_ns::ipc::init();
    zenus_console::serial::flush_output_blocking();

    #[cfg(not(feature = "testing"))]
    {
        // 12. PCI
        zenus_arch::pci::init();
        zenus_console::serial::flush_output_blocking();

        // 13. Virtio
        unsafe {
            zenus_virtio::init();
        }
        zenus_console::serial::flush_output_blocking();

        // 14. ATA
        zenus_arch::ata::init();
        {
            let count = zenus_arch::ata::device_count();
            let names = ["sda", "sdb", "sdc", "sdd"];
            for i in 0..count.min(4) {
                if let Some(dev) = zenus_arch::ata::get_device(i) {
                    let name = names[i];
                    match i {
                        0 => {
                            zenus_fs::devfs::register_block_device(
                                name,
                                zenus_fs::devfs::BlockDeviceOps {
                                    read: ata_read0,
                                    write: ata_write0,
                                    size: dev.lba_sectors * 512,
                                },
                            );
                        }
                        1 => {
                            zenus_fs::devfs::register_block_device(
                                name,
                                zenus_fs::devfs::BlockDeviceOps {
                                    read: ata_read1,
                                    write: ata_write1,
                                    size: dev.lba_sectors * 512,
                                },
                            );
                        }
                        2 => {
                            zenus_fs::devfs::register_block_device(
                                name,
                                zenus_fs::devfs::BlockDeviceOps {
                                    read: ata_read2,
                                    write: ata_write2,
                                    size: dev.lba_sectors * 512,
                                },
                            );
                        }
                        3 => {
                            zenus_fs::devfs::register_block_device(
                                name,
                                zenus_fs::devfs::BlockDeviceOps {
                                    read: ata_read3,
                                    write: ata_write3,
                                    size: dev.lba_sectors * 512,
                                },
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
        zenus_console::serial::flush_output_blocking();

        // 15. Ext2 mount
        if zenus_arch::ata::device_count() > 0 {
            zenus_fs::vfs::create_dir("/mnt");
            if let Some(ext2_fs) = zenus_fs::ext2::Ext2Fs::mount(0) {
                zenus_fs::vfs::mount("/mnt", ext2_fs);
            }
        }
        if zenus_fs::devfs::block_device_count() > zenus_arch::ata::device_count() {
            zenus_fs::vfs::create_dir("/virtio");
            let ata_count = zenus_arch::ata::device_count() as u8;
            if let Some(ext2_fs) = zenus_fs::ext2::Ext2Fs::mount(ata_count) {
                zenus_fs::vfs::mount("/virtio", ext2_fs);
            }
        }

        // 16. Network
        zenus_net::nic::init();
        if let Some(_idx) = zenus_net::tcp::listen(7) {}
        zenus_sync::lockdep::lockdep_init();
        zenus_arch::watchdog::watchdog_init(zenus_arch::watchdog::WatchdogType::Software, 30);

        if zenus_arch::ata::device_count() > 0 {
            if !zenus_fs::journal::journal_replay(0, 3000) {}
            if zenus_fs::journal::journal_init(0, 3000, 16) {}
        }

        // 17. SMP
        smp::init();
        zenus_arch::smp::set_ap_idle_fn(zenus_sched::scheduler::ap_idle);
        smp::wake_aps();

        // Drain any bytes that arrived during boot (before UART FIFO stabilized)
        zenus_console::serial::drain_boot_input();

        // Start system services (if any registered)
        zenus_sched::init::init_system_start();

        // Boot complete
        zenus_console::kinfo!("Zenus OS booted");
        zenus_console::serial::flush_output_blocking();

        // Create shell as a scheduled task for preemptive multitasking.
        // The scheduler will manage the shell via idle → yield → shell
        // cycle, enabling timer-based preemption.
        let _shell_tid = scheduler::create_task_named(shell_task, 65536, "shell");
        zenus_console::kinfo!("Shell PID={}", _shell_tid);

        return run_after_init();
    }
}

#[cfg(feature = "testing")]
#[no_mangle]
pub extern "C" fn entry() -> ! {
    // Reuse the same initialization as non-testing, then run tests
    SerialPort::init();
    zenus_console::log::dmesg_init();
    zenus_console::serial::uart_write_byte_emergency(b'Z');

    if zenus_arch::limine::MEMMAP_REQUEST.response.is_null() {
        zenus_console::kpanic_code!(
            zenus_console::error::codes::KRN_PANIC_INVALID_MEM,
            "MEMMAP_REQUEST response is null"
        );
    }
    if zenus_arch::limine::HHDM_REQUEST.response.is_null() {
        zenus_console::kpanic_code!(
            zenus_console::error::codes::KRN_PANIC_INVALID_MEM,
            "HHDM_REQUEST response is null"
        );
    }
    let hhdm_offset = unsafe {
        let hhdm_resp: &zenus_arch::limine::LimineHhdmResponse =
            &*zenus_arch::limine::HHDM_REQUEST.response.as_ptr();
        hhdm_resp.offset
    };
    zenus_arch::limine::store_hhdm_offset(hhdm_offset);

    let memmap_resp: &zenus_arch::limine::LimineMemmapResponse =
        unsafe { &*zenus_arch::limine::MEMMAP_REQUEST.response.as_ptr() };
    let entry_ptrs = memmap_resp
        .entries
        .as_ptr::<*mut zenus_arch::limine::LimineMemmapEntry>();
    let mut regions: [MemoryRegion; 64] = [MemoryRegion {
        base: 0,
        length: 0,
        kind: 0,
    }; 64];
    let mut region_count = 0;
    let total = core::cmp::min(memmap_resp.entry_count as usize, 64);
    unsafe {
        for i in 0..total {
            let entry_ptr = *entry_ptrs.add(i);
            if entry_ptr.is_null() {
                continue;
            }
            let entry = &*entry_ptr;
            regions[i] = MemoryRegion {
                base: entry.base,
                length: entry.length,
                kind: entry.kind,
            };
            region_count += 1;
        }
    }
    let mem_regions = &regions[..region_count];

    cpu::init();
    frame_allocator::global_init(mem_regions);
    paging::init(hhdm_offset);
    frame_allocator::reserve_boot_stack(hhdm_offset);
    interrupts::init();
    zenus_console::vga::init(hhdm_offset);
    zenus_arch::rtc::init();
    zenus_arch::random::init_rng();
    zenus_arch::keyboard::init();
    if zenus_arch::interrupts::ioapic::is_initialized() {
        let apic_id = zenus_arch::interrupts::apic::current_apic_id() as u8;
        if zenus_arch::interrupts::ioapic::route_irq(4, 36u8, apic_id) {
            zenus_console::kinfo!("Serial IRQ4 -> vector 36 (IOAPIC)");
        } else {
            zenus_console::kwarn!("Serial IRQ4 -> IOAPIC FAILED");
        }
    }
    zenus_console::serial::enable_serial_interrupts();
    scheduler::init();
    zenus_console::serial::flush_output_blocking();

    zenus_fs::vfs::init();
    zenus_console::serial::flush_output_blocking();
    zenus_fs::vfs::create_dir("/dev");
    let devfs: &dyn zenus_fs::vfs::FileSystem = &zenus_fs::devfs::DevFs;
    zenus_fs::vfs::mount("/dev", devfs);
    zenus_fs::vfs::create_dir("/tmp");
    zenus_fs::vfs::create_dir("/proc");
    static PROCFS: zenus_fs::procfs::ProcFs = zenus_fs::procfs::ProcFs;
    zenus_fs::vfs::mount("/proc", &PROCFS);

    zenus_arch::crash::crash_dump_init();
    zenus_console::syslog::syslog_init();
    zenus_fs::sysctl::sysctl_init();
    zenus_fs::pkg::pkg_init();

    zenus_ns::uts::init();
    zenus_ns::pid::init();
    zenus_ns::mnt::init();
    zenus_ns::net::init();
    zenus_ns::user::init();
    zenus_ns::ipc::init();
    zenus_console::serial::flush_output_blocking();

    // Register procfs data sources
    zenus_fs::procfs::register_cpuinfo(gen_proc_cpuinfo);
    zenus_fs::procfs::register_meminfo(gen_proc_meminfo);
    zenus_fs::procfs::register_uptime(gen_proc_uptime);
    zenus_fs::procfs::register_stat(gen_proc_stat);
    zenus_fs::procfs::register_loadavg(gen_proc_loadavg);
    zenus_fs::procfs::register_task_count(gen_proc_task_count);

    // Now run tests
    use zenus_console::serial::SerialPort;
    let mut test_serial = SerialPort::new(0x3F8);
    test_runner::run_tests(&mut test_serial);
    loop {
        x86_64::instructions::hlt();
    }
}

// Only the non-testing `entry()` calls this. The testing build has its own
// `entry()` which runs `test_runner` directly, so this function must not
// reference `test_runner` when the feature is off — `cfg!()` is a runtime
// boolean and would still typecheck (and link) the testing branch.
#[cfg(not(feature = "testing"))]
fn run_after_init() -> ! {
    loop {
        scheduler::idle();
    }
}

// Test builds: `entry()` falls through to here only if it returns, which it
// does not (it hlt-loops after running tests). Kept as a safety net.
#[cfg(feature = "testing")]
#[allow(dead_code)]
fn run_after_init() -> ! {
    use zenus_console::serial::SerialPort;
    let mut test_serial = SerialPort::new(0x3F8);
    test_runner::run_tests(&mut test_serial);
    loop {
        x86_64::instructions::hlt();
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    entry()
}

#![no_std]
#![no_main]

extern crate alloc;

use core::panic::PanicInfo;

use zenus_arch::cpu;

#[cfg(feature = "testing")]
mod test_runner;

fn ata_read0(lba: u64, buf: &mut [u8]) -> bool { zenus_arch::ata::read_sectors(0, lba, 1, buf) }
fn ata_write0(lba: u64, buf: &[u8]) -> bool { zenus_arch::ata::write_sectors(0, lba, 1, buf) }
fn ata_read1(lba: u64, buf: &mut [u8]) -> bool { zenus_arch::ata::read_sectors(1, lba, 1, buf) }
fn ata_write1(lba: u64, buf: &[u8]) -> bool { zenus_arch::ata::write_sectors(1, lba, 1, buf) }
fn ata_read2(lba: u64, buf: &mut [u8]) -> bool { zenus_arch::ata::read_sectors(2, lba, 1, buf) }
fn ata_write2(lba: u64, buf: &[u8]) -> bool { zenus_arch::ata::write_sectors(2, lba, 1, buf) }
fn ata_read3(lba: u64, buf: &mut [u8]) -> bool { zenus_arch::ata::read_sectors(3, lba, 1, buf) }
fn ata_write3(lba: u64, buf: &[u8]) -> bool { zenus_arch::ata::write_sectors(3, lba, 1, buf) }

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

struct EchoState {
    listen_fds: [Option<usize>; 8],
    client_fds: [Option<usize>; 16],
}

use zenus_mem::frame_allocator;
use zenus_mem::frame_allocator::MemoryRegion;
use zenus_sched::scheduler;

fn rdtsc() -> u64 {
    unsafe {
        let lo: u32; let hi: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, preserves_flags));
        ((hi as u64) << 32) | lo as u64
    }
}

fn boot_stage(serial: &mut SerialPort, label: &str, tsc_start: u64) -> u64 {
    let now = rdtsc();
    let elapsed = now - tsc_start;
    serial.write_str("  [");
    serial.write_str(label);
    serial.write_str("] ");
    serial.write_u64(elapsed / 1_000_000);
    serial.write_str("M\n");
    now
}

macro_rules! both {
    ($serial:expr, $hhdm:expr, $msg:expr) => {{
        $serial.write_str($msg);
        #[cfg(not(feature = "smp"))]
        zenus_console::vga::write_str($msg, $hhdm);
        let trimmed = $msg.trim_end_matches('\n');
        if !trimmed.is_empty() {
            zenus_console::log::dmesg_push(zenus_console::log::LogLevel::Info, trimmed);
        }
    }};
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut serial = SerialPort::new(0x3F8);
    serial.write_str("!!! KERNEL PANIC !!!\n");
    use core::fmt::Write;
    let _ = write!(serial, "{}", info.message());
    if let Some(loc) = info.location() {
        serial.write_str("File: ");
        serial.write_str(loc.file());
        serial.write_str(":");
        serial.write_u64(loc.line() as u64);
        serial.write_str("\n");
    }
    serial.write_str("[PANIC] Attempting reboot...\n");
    zenus_arch::acpi::reboot_via_keyboard();
}

fn shell_task() {
    let mut shell = shell::Shell::new();
    shell.run();
}

fn ssh_service_task() {
}

#[no_mangle]
pub extern "C" fn entry() -> ! {
    unsafe {
        core::arch::asm!("out 0xe9, al", in("al") b'Z');
    }
    SerialPort::init();
    zenus_console::log::dmesg_init();
    zenus_console::log::dmesg_push(zenus_console::log::LogLevel::Info, "Zenus boot started");
    let mut serial = SerialPort::new(0x3F8);
    let _t0 = rdtsc();

    // 1. Parse Limine boot info
    let s = rdtsc();
    zenus_arch::limine::BootInfo::get();
    let _ = boot_stage(&mut serial, "limine", s);

    if zenus_arch::limine::MEMMAP_REQUEST.response.is_null() {
        serial.write_str("[FATAL] MEMMAP response is NULL - bootloader did not fill requests\n");
        loop { x86_64::instructions::hlt(); }
    }
    if zenus_arch::limine::HHDM_REQUEST.response.is_null() {
        serial.write_str("[FATAL] HHDM response is NULL - cannot proceed without higher half\n");
        loop { x86_64::instructions::hlt(); }
    }
    let hhdm_offset = unsafe {
        let hhdm_resp: &zenus_arch::limine::LimineHhdmResponse =
            &*zenus_arch::limine::HHDM_REQUEST.response.as_ptr();
        hhdm_resp.offset
    };
    zenus_arch::limine::store_hhdm_offset(hhdm_offset);

    zenus_console::vga::init(hhdm_offset);
    zenus_console::vga::write_str("\n", hhdm_offset);

    // 2. Memory map
    let s = rdtsc();
    let memmap_resp: &zenus_arch::limine::LimineMemmapResponse =
        unsafe { &*zenus_arch::limine::MEMMAP_REQUEST.response.as_ptr() };
    let entry_ptrs = memmap_resp.entries.as_ptr::<*mut zenus_arch::limine::LimineMemmapEntry>();
    let mut regions: [MemoryRegion; 64] = [MemoryRegion { base: 0, length: 0, kind: 0 }; 64];
    let mut region_count = 0;
    let total = core::cmp::min(memmap_resp.entry_count as usize, 64);
    unsafe {
        for i in 0..total {
            let entry_ptr = *entry_ptrs.add(i);
            if entry_ptr.is_null() { continue; }
            let entry = &*entry_ptr;
            regions[i] = MemoryRegion {
                base: entry.base,
                length: entry.length,
                kind: entry.kind,
            };
            region_count += 1;
        }
    }
    let _ = boot_stage(&mut serial, "memmap", s);
    let mem_regions = &regions[..region_count];

    // 3. CPU features
    let s = rdtsc();
    cpu::init();
    let _ = boot_stage(&mut serial, "cpu", s);

    // 4. Frame allocator & paging
    let s = rdtsc();
    frame_allocator::global_init(mem_regions);
    {
        let allocator = frame_allocator::FRAME_ALLOCATOR.lock();
        let s2 = SerialPort::new(0x3F8);
        s2.write_str("[OK] Memory: ");
        s2.write_u64(allocator.total_memory() / (1024 * 1024));
        s2.write_str(" MB total\n");
        paging::init(hhdm_offset);
    }
    frame_allocator::reserve_boot_stack(hhdm_offset);
    let _ = boot_stage(&mut serial, "paging", s);

    // 5. Interrupts
    let s = rdtsc();
    interrupts::init();
    let _ = boot_stage(&mut serial, "idt", s);

    // 6. APIC + PIT + timers
    let s = rdtsc();
    let apic_base_raw = unsafe { cpu::read_msr(0x1B) };
    let apic_base = apic_base_raw & 0xFFFFF000;
    interrupts::apic::init_with_virt(apic_base + hhdm_offset);
    interrupts::apic::enable_pic_lint0();
    interrupts::pit::init();
    zenus_arch::rtc::init();
    zenus_arch::random::init_rng();
    let _ = boot_stage(&mut serial, "apic+pit", s);

    // 7. Enable interrupts
    let s = rdtsc();
    x86_64::instructions::interrupts::enable();
    let _ = boot_stage(&mut serial, "sti", s);

    // 8. Keyboard
    let s = rdtsc();
    zenus_arch::keyboard::init();
    let _ = boot_stage(&mut serial, "kbd", s);

    // 9. Scheduler
    let s = rdtsc();
    scheduler::init();
    let _ = boot_stage(&mut serial, "sched", s);

    let test_msg = "Hello via syscall!\n";
    let _ret = unsafe {
        extern "C" {
            fn syscall_dispatch(num: u64, a1: u64, a2: u64, a3: u64) -> u64;
        }
        syscall_dispatch(1, 1, test_msg.as_ptr() as u64, test_msg.len() as u64)
    };

    // 10. Filesystem
    let s = rdtsc();
    zenus_fs::vfs::init();
    zenus_fs::vfs::create_dir("/dev");
    let devfs: &dyn zenus_fs::vfs::FileSystem = &zenus_fs::devfs::DevFs;
    zenus_fs::vfs::mount("/dev", devfs);
    zenus_fs::vfs::create_dir("/tmp");
    let _ = boot_stage(&mut serial, "vfs", s);

    // 11. Initrd
    let s = rdtsc();
    if zenus_arch::limine::MODULE_REQUEST.response.is_null() {
        serial.write_str("[WARN] No initrd module loaded\n");
    } else {
        unsafe {
            let mod_resp: &zenus_arch::limine::LimineModuleResponse =
                &*zenus_arch::limine::MODULE_REQUEST.response.as_ptr();
            if mod_resp.module_count > 0 {
                let mod_ptrs = mod_resp.modules.as_ptr::<*mut zenus_arch::limine::LimineFile>();
                let module = &**mod_ptrs;
                let initrd_virt = module.address.0;
                let _mod_data = core::slice::from_raw_parts(
                    initrd_virt as *const u8,
                    module.size as usize,
                );
                if let Some(tarfs) = zenus_fs::tarfs::TarFs::load(initrd_virt, module.size) {
                    zenus_fs::vfs::mount("/initrd", tarfs);
                }
            }
        }
    }
    let _ = boot_stage(&mut serial, "initrd", s);

    // 12. PCI
    let s = rdtsc();
    zenus_arch::pci::init();
    let _ = boot_stage(&mut serial, "pci", s);

    // 13. Virtio
    let s = rdtsc();
    unsafe { zenus_virtio::init(); }
    let _ = boot_stage(&mut serial, "virtio", s);

    // 14. ATA
    let s = rdtsc();
    zenus_arch::ata::init();
    {
        let count = zenus_arch::ata::device_count();
        let names = ["sda", "sdb", "sdc", "sdd"];
        for i in 0..count.min(4) {
            if let Some(dev) = zenus_arch::ata::get_device(i) {
                let name = names[i];
                match i {
                    0 => { zenus_fs::devfs::register_block_device(name, zenus_fs::devfs::BlockDeviceOps {
                        read: ata_read0, write: ata_write0, size: dev.lba_sectors * 512,
                    }); }
                    1 => { zenus_fs::devfs::register_block_device(name, zenus_fs::devfs::BlockDeviceOps {
                        read: ata_read1, write: ata_write1, size: dev.lba_sectors * 512,
                    }); }
                    2 => { zenus_fs::devfs::register_block_device(name, zenus_fs::devfs::BlockDeviceOps {
                        read: ata_read2, write: ata_write2, size: dev.lba_sectors * 512,
                    }); }
                    3 => { zenus_fs::devfs::register_block_device(name, zenus_fs::devfs::BlockDeviceOps {
                        read: ata_read3, write: ata_write3, size: dev.lba_sectors * 512,
                    }); }
                    _ => {}
                }
            }
        }
    }
    let _ = boot_stage(&mut serial, "ata", s);

    // 15. Misc subsystems (crash, syslog, sysctl, pkg, ns)
    let s = rdtsc();
    zenus_arch::crash::crash_dump_init();
    zenus_console::syslog::syslog_init();
    zenus_fs::sysctl::sysctl_init();
    zenus_fs::pkg::pkg_init();
    zenus_ns::uts::init();
    zenus_ns::pid::init();
    let _ = boot_stage(&mut serial, "subsys", s);

    // 16. Ext2 mount
    let s = rdtsc();
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
    let _ = boot_stage(&mut serial, "ext2", s);

    #[cfg(feature = "testing")]
    {
        test_runner::run_tests(&mut serial);
        serial.write_str("[TEST] Test mode complete. Halting.\n");
        loop { x86_64::instructions::hlt(); }
    }

    #[cfg(not(feature = "testing"))]
    {
        // 17. Network
        let s = rdtsc();
        zenus_net::nic::init();
        if let Some(_idx) = zenus_net::tcp::listen(7) {
        }
        let _ = boot_stage(&mut serial, "net", s);

        zenus_sync::lockdep::lockdep_init();
        zenus_arch::watchdog::watchdog_init(zenus_arch::watchdog::WatchdogType::Software, 30);

        if zenus_arch::ata::device_count() > 0 {
            if !zenus_fs::journal::journal_replay(0, 3000) {
            }
            if zenus_fs::journal::journal_init(0, 3000, 16) {
            }
        }

        // 18. SMP
        let s = rdtsc();
        smp::init();
        zenus_arch::smp::set_ap_idle_fn(zenus_sched::scheduler::ap_idle);
        smp::wake_aps();
        let _ = boot_stage(&mut serial, "smp", s);

        // 19. Shell task
        let shell_tid = scheduler::create_task(shell_task, 65536);

        zenus_sched::init::init_system_start();

        let total_ms = rdtsc() - _t0;
        serial.write_str("\n[BENCH] Boot stages above (TSC ticks / 1M)\n");
        serial.write_str("[BENCH] Total boot: ");
        serial.write_u64(total_ms / 1_000_000);
        serial.write_str("M ticks\n");

        serial.write_str("========================================\n");
        serial.write_str("  Zenus OS siap! Server mode aktif.\n");
        serial.write_str("========================================\n");
        serial.write_str("[PIT] PIT-based scheduling active (~100 Hz)\n");
        zenus_console::serial::flush_output();

        scheduler::idle();
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    entry()
}

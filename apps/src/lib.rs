#![no_std]
#![no_main]

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

// Force cargo to link zenus-syscall objects into the final binary
extern crate zenus_syscall;

// Force keep limine requests section from being GC'd
#[used]
#[link_section = ".limine_reqs"]
static _FORCE_LIMINE: [u64; 0] = [];
use zenus_arch::interrupts;
use zenus_arch::smp;
use zenus_console::serial::SerialPort;
use zenus_fs::vfs::FileSystem as _;
use zenus_mem::paging;

mod shell;
mod user;
use zenus_mem::frame_allocator;
use zenus_mem::frame_allocator::MemoryRegion;
use zenus_sched::scheduler;

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
    loop {
        x86_64::instructions::hlt();
    }
}

fn shell_task() {
    let mut sh = shell::Shell::new();
    sh.run();
}

#[no_mangle]
pub extern "C" fn entry() -> ! {
    // Early debug: write to Bochs/QEMU debug port
    unsafe {
        core::arch::asm!("out 0xe9, al", in("al") b'Z');
    }
    SerialPort::init();
    zenus_console::log::dmesg_init();
    zenus_console::log::dmesg_push(zenus_console::log::LogLevel::Info, "Zenus boot started");
    let mut serial = SerialPort::new(0x3F8);
    serial.write_str("\n");
    serial.write_str("========================================\n");
    serial.write_str("         Zenus OS v0.1.0               \n");
    serial.write_str("         Stable Server Kernel           \n");
    serial.write_str("========================================\n");

    // 1. Parse Limine boot info
    zenus_arch::limine::BootInfo::get();
    serial.write_str("[OK] Boot protocol initialized\n");
    zenus_console::log::dmesg_push(zenus_console::log::LogLevel::Info, "[OK] Boot protocol initialized");

    // Verify critical responses
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

    // Initialize VGA text mode output
    zenus_console::vga::init(hhdm_offset);
    zenus_console::vga::write_str("\n", hhdm_offset);
    zenus_console::vga::write_str("========================================\n", hhdm_offset);
    zenus_console::vga::write_str("         Zenus OS v0.1.0               \n", hhdm_offset);
    zenus_console::vga::write_str("         Stable Server Kernel           \n", hhdm_offset);
    zenus_console::vga::write_str("========================================\n", hhdm_offset);

    // 2. Initialize memory management
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
    both!(serial, hhdm_offset, "[OK] Memory map acquired\n");

    let mem_regions = &regions[..region_count];

    // 3. Initialize CPU features
    cpu::init();
    both!(serial, hhdm_offset, "[OK] CPU features initialized\n");

    // 4. Initialize global frame allocator & paging
    frame_allocator::global_init(mem_regions);
    {
        let allocator = frame_allocator::FRAME_ALLOCATOR.lock();
        let s2 = SerialPort::new(0x3F8);
        s2.write_str("[OK] Memory: ");
        s2.write_u64(allocator.total_memory() / (1024 * 1024));
        s2.write_str(" MB total\n");
        paging::init(hhdm_offset);
    }
    // Reserve boot stack pages so the frame allocator doesn't hand them out
    frame_allocator::reserve_boot_stack(hhdm_offset);
    both!(serial, hhdm_offset, "[OK] Paging initialized\n");

    // 5. Set up interrupt handling
    interrupts::init();
    both!(serial, hhdm_offset, "[OK] Interrupts initialized\n");

    // 6. Initialize APIC and timers
    let apic_base_raw = unsafe { cpu::read_msr(0x1B) };
    let apic_base = apic_base_raw & 0xFFFFF000;
    serial.write_str("[APIC] IA32_APIC_BASE raw: 0x");
    serial.write_hex(apic_base_raw);
    serial.write_str(" flags: 0x");
    serial.write_hex(apic_base_raw & 0xFFF);
    serial.write_str(" EN=");
    serial.write_byte_serial(if (apic_base_raw & (1 << 11)) != 0 { b'1' } else { b'0' });
    serial.write_str(" BSP=");
    serial.write_byte_serial(if (apic_base_raw & (1 << 8)) != 0 { b'1' } else { b'0' });
    serial.write_str("\n");
    interrupts::apic::init_with_virt(apic_base + hhdm_offset);
    interrupts::pit::init();
    zenus_arch::rtc::init();
    zenus_arch::random::init_rng();
    both!(serial, hhdm_offset, "[OK] RNG initialized\n");
    both!(serial, hhdm_offset, "[OK] APIC & timers initialized\n");

    // 7. Enable interrupts
    x86_64::instructions::interrupts::enable();
    both!(serial, hhdm_offset, "[OK] Interrupts enabled\n");

    // 7b. Initialize PS/2 keyboard driver
    zenus_arch::keyboard::init();
    both!(serial, hhdm_offset, "[OK] PS/2 Keyboard driver initialized\n");

    // 8. Initialize scheduler
    scheduler::init();

    // Test syscall: SYS_WRITE(1, msg, len) via direct extern C call
    let test_msg = "Hello via syscall!\n";
    let _ret = unsafe {
        extern "C" {
            fn syscall_dispatch(num: u64, a1: u64, a2: u64, a3: u64) -> u64;
        }
        syscall_dispatch(1, 1, test_msg.as_ptr() as u64, test_msg.len() as u64)
    };
    both!(serial, hhdm_offset, "[OK] Syscall handler initialized\n");

    // 9. Initialize filesystem (tmpfs root + devfs at /dev)
    zenus_fs::vfs::init();
    zenus_fs::vfs::create_dir("/dev");
    let devfs: &dyn zenus_fs::vfs::FileSystem = &zenus_fs::devfs::DevFs;
    zenus_fs::vfs::mount("/dev", devfs);
    // Create /tmp for temporary files
    zenus_fs::vfs::create_dir("/tmp");
    both!(serial, hhdm_offset, "[OK] Filesystem initialized (tmpfs + devfs)\n");

    // 9b. Load initrd (if available)
    if zenus_arch::limine::MODULE_REQUEST.response.is_null() {
        both!(serial, hhdm_offset, "[WARN] No initrd module loaded\n");
    } else {
        unsafe {
            let mod_resp: &zenus_arch::limine::LimineModuleResponse =
                &*zenus_arch::limine::MODULE_REQUEST.response.as_ptr();
            if mod_resp.module_count > 0 {
                let mod_ptrs = mod_resp.modules.as_ptr::<*mut zenus_arch::limine::LimineFile>();
                let module = &**mod_ptrs;
                serial.write_str("[INITRD] Loading initrd: ");
                serial.write_u64(module.size);
                serial.write_str(" bytes at ");
                serial.write_hex(module.address.0);
                serial.write_str("\n");

                let initrd_virt = module.address.0;
                let _mod_data = core::slice::from_raw_parts(
                    initrd_virt as *const u8,
                    module.size as usize,
                );

                both!(serial, hhdm_offset, "[INITRD] Parsing archive...\n");
                if let Some(tarfs) = zenus_fs::tarfs::TarFs::load(initrd_virt, module.size) {
                    both!(serial, hhdm_offset, "[OK] Initrd loaded\n");
                    zenus_fs::vfs::mount("/initrd", tarfs);
                    both!(serial, hhdm_offset, "[INITRD] Mounted at /initrd\n");

                    {
                        let mut buf = [0u8; 64];
                        if let Some(n) = tarfs.read(3, 0, &mut buf) {
                            let txt = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?");
                            both!(serial, hhdm_offset, "[INITRD] hello.txt: ");
                            both!(serial, hhdm_offset, txt);
                        }
                    }
                } else {
                    both!(serial, hhdm_offset, "[WARN] Failed to parse initrd\n");
                }
            } else {
                both!(serial, hhdm_offset, "[WARN] No initrd modules\n");
            }
        }
    }

    // 10. PCI bus scan
    zenus_arch::pci::init();

    // 10a. Initialize ATA/IDE drives (disk storage)
    zenus_arch::ata::init();
    // Register ATA drives as block devices in devfs
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

    // 10b. Try to mount ext2 filesystem on first ATA drive
    if zenus_arch::ata::device_count() > 0 {
        both!(serial, hhdm_offset, "[EXT2] Trying to mount /mnt...\n");
        zenus_fs::vfs::create_dir("/mnt");
        if let Some(ext2_fs) = zenus_fs::ext2::Ext2Fs::mount(0) {
            zenus_fs::vfs::mount("/mnt", ext2_fs);
            both!(serial, hhdm_offset, "[OK] ext2 filesystem mounted at /mnt\n");
        } else {
            both!(serial, hhdm_offset, "[EXT2] No ext2 filesystem found on /dev/sda\n");
        }
    }

    // 10c. Run unit tests before APIC timer (no preemption)
    #[cfg(feature = "testing")]
    {
        test_runner::run_tests(&mut serial);
        serial.write_str("[TEST] Test mode complete. Halting.\n");
        loop { x86_64::instructions::hlt(); }
    }

    // Normal boot (non-testing)
    #[cfg(not(feature = "testing"))]
    {
        // 11. Initialize networking
        zenus_net::nic::init();
        both!(serial, hhdm_offset, "[OK] Network initialized\n");

        // 11a. Start TCP echo server on port 7
        if let Some(_idx) = zenus_net::tcp::listen(7) {
            both!(serial, hhdm_offset, "[OK] TCP echo server on port 7\n");
        }

        // 11b. Initialize journal on device 0 at blocks 3000-3015
        if !zenus_fs::journal::journal_replay(0, 3000) {
            both!(serial, hhdm_offset, "[WARN] Journal replay failed\n");
        }
        if zenus_fs::journal::journal_init(0, 3000, 16) {
            both!(serial, hhdm_offset, "[OK] Journal initialized (blocks 3000-3015)\n");
        } else {
            both!(serial, hhdm_offset, "[WARN] Journal init failed\n");
        }

        // 11. Detect CPUs via Limine MP response
        smp::init();
        both!(serial, hhdm_offset, "[OK] SMP initialized\n");

        // 11b. Set AP idle function (SMP scheduler) and wake Application Processors
        zenus_arch::smp::set_ap_idle_fn(zenus_sched::scheduler::ap_idle);
        smp::wake_aps();

        // 12. Spawn shell as a real scheduler task FIRST (before timer starts)
        let _shell_tid = scheduler::create_task(shell_task, 65536);

        both!(serial, hhdm_offset, "[OK] Shell task spawned\n");

        // 13a. Spawn user-mode demo task with proper isolation (timer NOT yet running)
        let _cr3 = paging::create_address_space();
        let _user_tid = user::spawn_user();

        // Print banner BEFORE starting the APIC timer (VGA scroll uses function pointers
        // that may be corrupted if the timer interrupt fires during VGA operations).
        both!(serial, hhdm_offset, "========================================\n");
        both!(serial, hhdm_offset, "  Zenus OS siap! Server mode aktif.\n");
        both!(serial, hhdm_offset, "========================================\n");

        // 13. Start APIC timer for preemptive multitasking AFTER boot init is fully complete.
        serial.write_str("[TMR] Timer starting, entering idle\n");
        interrupts::apic::init_timer(48);

        // 14. Become idle — let the scheduler run the shell task
        scheduler::idle();
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    entry()
}

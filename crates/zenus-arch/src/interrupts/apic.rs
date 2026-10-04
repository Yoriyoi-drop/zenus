use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub static LAPIC_VIRT_BASE: AtomicU64 = AtomicU64::new(0);
static X2APIC_MODE: AtomicBool = AtomicBool::new(false);

fn lapic_base() -> *mut u32 {
    LAPIC_VIRT_BASE.load(Ordering::Relaxed) as *mut u32
}

fn lapic_read(reg: u32) -> u32 {
    if X2APIC_MODE.load(Ordering::Relaxed) {
        // x2APIC: use MSR-based access. MSR = 0x800 + (reg >> 4)
        let msr = 0x800u32 + (reg >> 4);
        unsafe { crate::cpu::read_msr(msr as u32) as u32 }
    } else {
        // xAPIC: use memory-mapped access
        unsafe {
            let addr = (lapic_base() as usize).wrapping_add(reg as usize);
            (addr as *const u32).read_volatile()
        }
    }
}

fn lapic_write(reg: u32, val: u32) {
    if X2APIC_MODE.load(Ordering::Relaxed) {
        // x2APIC: use MSR-based access. MSR = 0x800 + (reg >> 4)
        let msr = 0x800u32 + (reg >> 4);
        unsafe {
            crate::cpu::write_msr(msr as u32, val as u64);
        }
    } else {
        // xAPIC: use memory-mapped access
        unsafe {
            let addr = (lapic_base() as usize).wrapping_add(reg as usize);
            (addr as *mut u32).write_volatile(val);
        }
    }
}

pub fn init_with_virt(virt: u64) {
    LAPIC_VIRT_BASE.store(virt, Ordering::Relaxed);
    remap_pic();
    enable_lapic();
}

pub fn init_ap(virt: u64) {
    LAPIC_VIRT_BASE.store(virt, Ordering::Relaxed);
    enable_lapic();
}

pub fn current_apic_id() -> u32 {
    if X2APIC_MODE.load(Ordering::Relaxed) {
        // x2APIC: IA32_X2APIC_APIC_ID (MSR 0x802) holds the FULL APIC ID
        // in bits [31:0]. It is NOT the xAPIC MMIO layout (id in [31:24]),
        // so the previous `>> 24` returned 0 for every CPU (ids 0..3).
        // Consequence: smp::current_cpu() reported 0 on all APs → every
        // AP raced on AP_GDT[0] → `mov ss, 0x10` #GP → IDT=0 → #DF →
        // triple fault while wake_aps() waited ("Waiting for APs...").
        (unsafe { crate::cpu::read_msr(0x802) }) as u32
    } else {
        lapic_read(0x20) >> 24
    }
}

/// Probe: do the x2APIC MSRs actually work?
///
/// QEMU's TCG backend does NOT implement the x2APIC MSR range at all:
/// `helper_rdmsr()`'s `default:` arm returns `val = 0` for every
/// unimplemented MSR and `helper_wrmsr()`'s `default:` arm is a silent
/// no-op (`/* XXX: exception? */ break;`). So once Limine sets
/// IA32_APIC_BASE.EXTD=1, every x2APIC access "succeeds" but reads 0 and
/// writes vanish — APIC ID always 0 (all CPUs collide in smp::current_cpu),
/// SVR/LVT/ICR/EOI all dead, timer ticks freeze after the first EOI.
///
/// Probe with a write/read of the logical destination register (LDR,
/// MSR 0x80D) — harmless, and restored to 0 afterwards. Returns true only
/// if the value survives the round-trip.
fn x2apic_msrs_work() -> bool {
    const LDR: u32 = 0x80D;
    const PROBE: u64 = 0x0200_0000;
    unsafe {
        crate::cpu::write_msr(LDR, PROBE);
        let back = crate::cpu::read_msr(LDR);
        crate::cpu::write_msr(LDR, 0);
        back == PROBE
    }
}

fn enable_lapic() {
    // IA32_APIC_BASE: bit 10 = EN, bit 11 = EXTD (x2APIC enable)
    let base_raw = unsafe { crate::cpu::read_msr(0x1B) };
    let mut x2apic = (base_raw & (1 << 11)) != 0;
    let apic_was_disabled = (base_raw & (1 << 10)) == 0;

    if x2apic && !x2apic_msrs_work() {
        // EXTD claims x2APIC but the MSRs are dead (QEMU TCG). Drop back to
        // xAPIC: clear EXTD and verify. On real Intel hardware EXTD is
        // one-way (SDM: cannot be cleared until reset) — but there the MSR
        // probe above passes, so we never reach this branch.
        unsafe { crate::cpu::write_msr(0x1B, base_raw & !(1 << 11)) };
        let after = unsafe { crate::cpu::read_msr(0x1B) };
        if (after & (1 << 11)) != 0 {
            zenus_console::kpanic_code!(
                zenus_console::error::codes::DRV_INIT_FAILED,
                "x2APIC MSRs dead and EXTD stuck — LAPIC unusable (base={:#x})",
                after
            );
        }
        x2apic = false;
        zenus_console::kwarn!(
            "x2APIC MSRs non-functional (QEMU TCG) — fell back to xAPIC MMIO"
        );
    }

    if x2apic {
        zenus_console::kinfo!("x2APIC mode detected (EXTD=1) — using MSR-based APIC access");
        X2APIC_MODE.store(true, Ordering::Relaxed);
    } else {
        zenus_console::kinfo!("xAPIC mode (EXTD=0) — using memory-mapped APIC access");
        X2APIC_MODE.store(false, Ordering::Relaxed);
    }

    if apic_was_disabled {
        zenus_console::kinfo!("Enabling APIC (IA32_APIC_BASE.EN)");
        // Just enable APIC, don't touch x2APIC bit (may be locked)
        unsafe {
            crate::cpu::write_msr(0x1B, base_raw | (1 << 10));
        }
    }

    let val = lapic_read(0xF0);
    let apic_id = current_apic_id();
    zenus_console::kinfo!("APIC SVR={:#x} APIC ID={:#x}", val, apic_id);
    if X2APIC_MODE.load(Ordering::Relaxed) {
        zenus_console::kinfo!("x2APIC mode (EXTD=1) — using MSR access");
    }
    // Keep APIC enabled, set spurious vector to 39 (our handler)
    lapic_write(0xF0, (val | 0x100) & !0xFF | 39);
    let svr2 = lapic_read(0xF0);
    zenus_console::kinfo!("APIC SVR after enable={:#x}", svr2);
    // Mask all LVT entries (APs call this too — keep LINT0 masked for them)
    if !X2APIC_MODE.load(Ordering::Relaxed) {
        // CMCI MSR (0x82F) may not be accessible in x2APIC mode on some CPUs (KVM).
        // Skip it to avoid #GP.
        lapic_write(0x2F0, 0x0100FF); // CMCI: masked
    }
    lapic_write(0x320, 0x00010000); // Timer: masked
    lapic_write(0x330, 0x0100FF); // Thermal: masked
    lapic_write(0x340, 0x0100FF); // Performance Counter: masked
    lapic_write(0x350, 0x0100FF); // LINT0: masked by default; enable_tick_source() arms the LAPIC timer
    lapic_write(0x360, 0x0100FF); // LINT1: masked (bit 16), vector 0xFF
    lapic_write(0x370, 0x0100FF); // Error: masked
    lapic_write(0x380, 0); // Timer initial count = 0 (no fire)
}

/// Enable LINT0 in ExtINT mode to accept PIC interrupts.
/// Only call on BSP; APs keep LINT0 masked.
/// In x2APIC mode, ExtINT delivery is NOT supported (Intel SDM: invalid).
/// We route PIT via IOAPIC or keep PIC interrupt disabled instead.
/// Arm the tick source.
///
/// The PIT → 8259 → LINT0/ExtINT chain is fragile: both 8259s and the LAPIC
/// latch "in service" bits that only an EOI clears, and a stray interrupt
/// (QEMU raises IRQ7 on its own, an EOI issued by the wrong CPU re-aims a
/// line) leaves one of those bits set. After that the tick stream stops dead
/// while the CPU sits in `hlt` with interrupts enabled — the machine looked
/// hung with no fault anywhere.
///
/// The LAPIC timer has no such state: it is a per-CPU one-shot/periodic
/// counter feeding vector 32 directly, so nothing external can block it.
/// Keep LINT0 masked (spurious) so the PIT can no longer inject IRQ0 at all.
pub fn enable_tick_source(vector: u8) {
    // Divider 1 => the counter runs at the APIC bus clock. Calibrate that
    // clock against the PIT instead of assuming 100 MHz: on QEMU TCG the
    // emulated LAPIC clock is NOT 100 MHz (measured ~624 MHz here), so the
    // hardcoded TICK_COUNT produced ~624 preemptions/second while the log,
    // TIME_SLICE and `uptime` (pit::tick() is driven from this ISR) all
    // assumed 100 Hz — uptime ran 6x fast and every task was rescheduled
    // 6x more often than intended.
    let mut count = calibrate_lapic_count_per_pit_period();
    if count == 0 {
        // No sane PIT reference — fall back to the legacy assumption.
        zenus_console::kwarn!(
            "LAPIC calibration failed — falling back to TICK_COUNT={}",
            TICK_COUNT
        );
        count = TICK_COUNT;
    }
    lapic_write(0x3E0, 0xB);
    lapic_write(0x380, 0);
    lapic_write(0x380, count);
    lapic_write(0x320, vector as u32 | 0x20000); // periodic, unmasked
    // LINT0: masked (bit 16), spurious vector 0xFF — the PIT can no longer inject IRQ0.
    lapic_write(0x350, 0x0001_00FF);
    zenus_console::kinfo!(
        "tick source: LAPIC timer (100 Hz), LINT0 masked (calibrated count={})",
        count
    );
}

/// Measure how many LAPIC timer counts elapse in one PIT channel-0 period.
///
/// The PIT runs off the fixed 1193182 Hz crystal, so it is the reference:
/// reprogram channel 0 to 100 Hz (the value `pit::init()` uses anyway),
/// arm a *masked* full-scale LAPIC one-shot, poll the PIT countdown until
/// `PERIODS` periods have wrapped, and divide. The LAPIC stays masked the
/// whole time so no interrupt can be raised while measuring (this runs with
/// interrupts off during boot).
///
/// Returns 0 when the PIT never advances (stuck/broken reference) so the
/// caller can fall back to the legacy constant.
fn calibrate_lapic_count_per_pit_period() -> u32 {
    const PIT_DIVISOR: u16 = 11931; // 100 Hz — same as pit::init()
    const PERIODS: u32 = 10; // measure over 10 x 10 ms = 100 ms

    unsafe fn latch_pit_ch0() -> u16 {
        // Latch command for channel 0, then read the latched count (lo, hi).
        core::arch::asm!("out 0x43, al", in("al") 0x00u8, options(nostack, preserves_flags));
        let lo: u8;
        let hi: u8;
        core::arch::asm!("in al, dx", out("al") lo, in("dx") 0x40u16, options(nostack, preserves_flags));
        core::arch::asm!("in al, dx", out("al") hi, in("dx") 0x40u16, options(nostack, preserves_flags));
        ((hi as u16) << 8) | lo as u16
    }

    unsafe {
        // Channel 0, lo/hi access, mode 3 (rate generator), PIT crystal.
        core::arch::asm!("out 0x43, al", in("al") 0x36u8, options(nostack, preserves_flags));
        core::arch::asm!("out 0x40, al", in("al") (PIT_DIVISOR & 0xFF) as u8, options(nostack, preserves_flags));
        core::arch::asm!("out 0x40, al", in("al") (PIT_DIVISOR >> 8) as u8, options(nostack, preserves_flags));
    }

    lapic_write(0x3E0, 0xB); // divide by 1
    lapic_write(0x320, 0x0001_0000); // LVT timer MASKED: no IRQ while measuring
    lapic_write(0x380, 0);
    lapic_write(0x380, 0xFFFF_FFFF);

    let mut prev = unsafe { latch_pit_ch0() };
    let mut wraps = 0u32;
    let mut spins = 0u64;
    while wraps < PERIODS {
        let cur = unsafe { latch_pit_ch0() };
        // Counting down: a value larger than the previous one means the
        // counter reloaded, i.e. one full PIT period elapsed.
        if cur > prev {
            wraps += 1;
        }
        prev = cur;
        spins += 1;
        if spins > 2_000_000_000 {
            zenus_console::kinfo!("LAPIC calibration: PIT did not tick (spins={})", spins);
            return 0; // PIT is not ticking
        }
    }

    // Current Count register is 0x390 (0x380 is the INITIAL count, which
    // still holds 0xFFFF_FFFF — reading it made elapsed come out as 0 and
    // silently disabled the calibration).
    let elapsed = 0xFFFF_FFFFu32.wrapping_sub(lapic_read(0x390));
    zenus_console::kinfo!(
        "LAPIC calibration: elapsed={} counts over {} PIT periods (laps={})",
        elapsed,
        PERIODS,
        spins
    );
    if elapsed == 0 {
        return 0;
    }
    let per_period = elapsed / PERIODS;
    // Reject nonsense (PIT reprogrammed by something else, LAPIC dead).
    if per_period < 1_000 || per_period > 4_000_000_000 {
        zenus_console::kinfo!("LAPIC calibration: rejecting per_period={}", per_period);
        return 0;
    }
    per_period
}

/// Fallback LAPIC reload for a missing/broken PIT reference: assumes a
/// 100 MHz APIC bus clock. NOT true on QEMU TCG — only used when
/// [`calibrate_lapic_count_per_pit_period`] returns 0.
const TICK_COUNT: u32 = 1_000_000;

pub fn lapic_read_reg(reg: u32) -> u32 {
    lapic_read(reg)
}

pub fn eoi() {
    lapic_write(0xB0, 0);
}

pub fn lapic_write_reg(reg: u32, val: u32) {
    lapic_write(reg, val);
}

#[no_mangle]
pub extern "C" fn apic_timer_eoi() {
    lapic_write(0xB0, 0);
}

pub fn init_timer(vector: u8) {
    lapic_write(0x3E0, 0xB); // divide by 1
                             // Use a count large enough that the timer NEVER fires during the ISR.
                             // On QEMU KVM the APIC timer runs at TSC frequency (~2 GHz), so each
                             // tick is 50 μs with INITCNT=100_000 — shorter than the ISR execution
                             // time. This causes a nested timer interrupt between popfq and jmp rax
                             // in the ISR return path, corrupting the target task's saved RIP.
                             // With 50_000_000 ticks: 25 ms at 2 GHz, 500 ms at 100 MHz.
                             // TIME_SLICE=5 → every task runs for ~125 ms, which is still snappy.
    lapic_write(0x380, 1_000_000);
    lapic_write(0x320, vector as u32 | 0x20000); // periodic mode, unmasked
}

pub fn init_timer_ap(vector: u8) {
    lapic_write(0x3E0, 0xB);
    lapic_write(0x380, 0);
    lapic_write(0x320, 0x00010000);
    lapic_write(0x380, 100_000);
    // Keep timer MASKED initially — BSP will broadcast IPI to start AP timers
    // after all APs have signaled readiness.
    lapic_write(0x320, vector as u32 | 0x20000 | 0x10000);
}

pub fn send_ipi(cpu_id: u8, vector: u8) {
    let icr = (cpu_id as u32) << 24 | vector as u32;
    lapic_write(0x300, icr);
    // Wait for delivery
    while (lapic_read(0x300) & (1 << 12)) != 0 {
        core::hint::spin_loop();
    }
}

fn remap_pic() {
    unsafe {
        // Master PIC
        core::arch::asm!("out 0x20, al", in("al") 0x11u8);
        core::arch::asm!("out 0x21, al", in("al") 0x20u8);
        core::arch::asm!("out 0x21, al", in("al") 0x04u8);
        core::arch::asm!("out 0x21, al", in("al") 0x01u8);

        // Slave PIC
        core::arch::asm!("out 0xA0, al", in("al") 0x11u8);
        core::arch::asm!("out 0xA1, al", in("al") 0x28u8);
        core::arch::asm!("out 0xA1, al", in("al") 0x02u8);
        core::arch::asm!("out 0xA1, al", in("al") 0x01u8);

        // Keep IRQ0 (PIT) and IRQ1 (keyboard) unmasked; mask everything else
        core::arch::asm!("out 0x21, al", in("al") 0xFCu8);
        core::arch::asm!("out 0xA1, al", in("al") 0xFFu8);
    }
}

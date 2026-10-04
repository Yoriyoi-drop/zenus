//! In-kernel driver for the `make fuzz-*` targets.
//!
//! Each `entry()` build runs exactly one fuzzing mode and then shuts the VM
//! down, so the Makefile target can collect the serial output and exit with a
//! meaningful status:
//!
//! ```text
//! [FUZZ] SUMMARY cases=2000 crashes=3 hangs=0 new_paths=17 corpus=41 edges=...
//! [FUZZ] EXIT code=1        ← non-zero => crashes were found
//! ```
//!
//! Two things make this safe to run unattended:
//!
//!   * **Fault containment** — `zenus_arch::fuzz_guard` records a CPU exception
//!     raised inside a test case and resumes the campaign instead of panicking.
//!   * **A dedicated campaign task** — fuzzed syscalls include blocking ones
//!     (`futex(WAIT)`, `waitpid`, a `read` on stdin), which would park the boot
//!     task forever. The campaign therefore runs as its own task and the boot
//!     task only watches a deadline; when it expires the run is aborted and
//!     the VM powers off with a `TIMEOUT` verdict instead of hanging forever.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use zenus_console::serial::SerialPort;

/// Set by the campaign task when it finished normally.
static CAMPAIGN_DONE: AtomicBool = AtomicBool::new(false);
/// Wall-clock budget for a campaign, in PIT ticks (100 Hz) plus seconds.
const TIMEOUT_SECS: u64 = 120;

fn flush() {
    zenus_console::serial::flush_output_blocking();
}

fn print_banner(mode: zenus_fuzz::Mode, cases: u64) {
    flush();
    zenus_console::serial::uart_write_byte_emergency(b'\n');
    zenus_console::kinfo!("[FUZZ] === Zenus Fuzzing Campaign: {} ===", mode.name());
    zenus_console::kinfo!("[FUZZ] cases={} seed=0xDEADBEEF timeout={}s", cases, TIMEOUT_SECS);
    flush();
}

/// Power the VM off. QEMU understands the ACPI S5 request through the PM
/// control block; the out ports are tried first so the target works without a
/// properly wired ACPI table.
fn poweroff() -> ! {
    flush();
    // SAFETY: plain port writes to the well-known QEMU/Bochs power-off ports.
    unsafe {
        use core::arch::asm;
        // 0x604 / 0xB004: QEMU ACPI shutdown (16-bit S5 request).
        asm!("out dx, ax", in("dx") 0x604u16, in("ax") 0x2000u16);
        asm!("out dx, ax", in("dx") 0xB004u16, in("ax") 0x2000u16);
        // 0x4004: Bochs legacy.
        asm!("out dx, ax", in("dx") 0x4004u16, in("ax") 0x3400u16);
    }
    flush();
    zenus_arch::acpi::shutdown_via_acpi()
}

fn emit_exit(code: u32) {
    flush();
    let s = SerialPort::new(0x3F8);
    s.write_str("[FUZZ] EXIT code=");
    s.write_u64(code as u64);
    s.write_str("\n");
    flush();
}

/// Campaign verdict.
fn finish(stats: &zenus_fuzz::FuzzStats) -> ! {
    zenus_fuzz::print_stats();
    zenus_fuzz::print_report();

    let unrecovered = zenus_arch::fuzz_guard::UNRECOVERED.load(Ordering::Relaxed);
    let code: u32 = if unrecovered > 0 {
        // A fault containment could not unwind: the campaign result is not
        // trustworthy, so do not report "clean".
        2
    } else if stats.crashes > 0 {
        1
    } else {
        0
    };
    emit_exit(code);
    poweroff()
}

/// Report an aborted campaign (watchdog fired) and power off.
fn abort(reason: &str) -> ! {
    zenus_console::kinfo!(
        "[FUZZ] TIMEOUT reason={} cases={} subsystem={}",
        reason,
        zenus_fuzz::CASE_COUNTER.load(Ordering::Relaxed),
        zenus_fuzz::CURRENT_SUBSYSTEM.load(Ordering::Relaxed)
    );
    zenus_fuzz::print_report();
    emit_exit(3);
    poweroff()
}

/// Body of the campaign task.
fn campaign_task() {
    zenus_console::kinfo!("[FUZZ] campaign task entered, tid={}", zenus_sched::scheduler::current_task_id());
    let mode = mode_from_u8(MODE.load(Ordering::Relaxed));
    let cases = CASES.load(Ordering::Relaxed);
    let stats = zenus_fuzz::run_campaign(mode, cases, 0xDEAD_BEEF);
    CAMPAIGN_DONE.store(true, Ordering::Release);
    finish(&stats)
}

static MODE: AtomicU8 = AtomicU8::new(0);
static CASES: AtomicU64 = AtomicU64::new(0);

fn mode_from_u8(v: u8) -> zenus_fuzz::Mode {
    match v {
        1 => zenus_fuzz::Mode::Coverage,
        2 => zenus_fuzz::Mode::Stress,
        _ => zenus_fuzz::Mode::Smoke,
    }
}

/// Run `mode` in a dedicated task and wait for it, aborting on deadline.
///
/// Never returns.
pub fn run_and_exit(mode: zenus_fuzz::Mode, cases: u64) -> ! {
    print_banner(mode, cases);
    MODE.store(
        match mode {
            zenus_fuzz::Mode::Smoke => 0,
            zenus_fuzz::Mode::Coverage => 1,
            zenus_fuzz::Mode::Stress => 2,
            zenus_fuzz::Mode::Regression => 0,
        },
        Ordering::Release,
    );
    CASES.store(cases, Ordering::Release);

    zenus_arch::interrupts::pit::init();
    zenus_arch::fuzz_guard::reset();

    let entry: fn() = campaign_task;
    let tid = zenus_sched::scheduler::create_task_named(entry, 1 << 18, "fuzz");
    if tid == 0 {
        zenus_console::kerror!("[FUZZ] could not create the campaign task");
        emit_exit(2);
        poweroff()
    }
    zenus_console::kinfo!("[FUZZ] campaign task pid={}", tid);
    flush();

    BOOT_TICKS.store(zenus_arch::interrupts::pit::get_ticks(), Ordering::Release);
    // The watchdog runs as the idle task, on the idle stack, exactly like
    // `scheduler::idle()` does for the shell. A bare `yield_now()` from the boot
    // task leaves the boot task's frame on the boot stack and the switch never
    // completes, so the campaign task would never run.
    zenus_sched::scheduler::idle_until(watchdog);

    // Only reached when the watchdog fired.
    abort("watchdog")
}

/// Watchdog predicate: true once the deadline expired or the campaign task
/// finished without publishing a verdict.
fn watchdog() -> bool {
    if CAMPAIGN_DONE.load(Ordering::Acquire) {
        return true;
    }

    let start = BOOT_TICKS.load(Ordering::Relaxed);
    let elapsed = zenus_arch::interrupts::pit::get_ticks().wrapping_sub(start);
    if elapsed >= TIMEOUT_SECS * 100 {
        return true;
    }

    // Progress heartbeat from the boot side, so a hung case still shows how
    // far the campaign got.
    let ticks = elapsed / 100;
    if ticks != LAST_REPORT.load(Ordering::Relaxed) {
        LAST_REPORT.store(ticks, Ordering::Relaxed);
        zenus_console::kinfo!(
            "[FUZZ] waiting cases={} elapsed={}s",
            zenus_fuzz::CASE_COUNTER.load(Ordering::Relaxed),
            ticks
        );
        flush();
    }
    false
}

/// Timestamp the watchdog measures from.
static BOOT_TICKS: AtomicU64 = AtomicU64::new(0);

static LAST_REPORT: AtomicU64 = AtomicU64::new(0);

/// `make fuzz-regression`: replay the recorded crash corpus.
pub fn run_regression_and_exit() -> ! {
    print_banner(zenus_fuzz::Mode::Regression, 0);
    zenus_arch::interrupts::pit::init();
    zenus_arch::fuzz_guard::reset();
    zenus_fuzz::init();

    let recorded = zenus_fuzz::crash::get_crash_count();
    zenus_console::kinfo!("[FUZZ] replaying {} recorded crashes", recorded);
    flush();

    let still_bad = zenus_fuzz::run_regression(1);
    zenus_fuzz::print_stats();
    zenus_fuzz::print_report();

    flush();
    let s = SerialPort::new(0x3F8);
    s.write_str("[FUZZ] EXIT code=");
    s.write_u64(if still_bad == 0 { 0 } else { 1 });
    s.write_str("\n");
    flush();

    poweroff()
}

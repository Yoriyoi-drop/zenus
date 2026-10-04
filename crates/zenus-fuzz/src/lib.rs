#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


extern crate alloc;

pub mod corpus;
pub mod mutator;
pub mod coverage;
pub mod syscall_fuzz;
pub mod memory_fuzz;
pub mod device_fuzz;
pub mod filesystem_fuzz;
pub mod shell_fuzz;
pub mod crash;
pub mod minimizer;
pub mod snapshot;
pub mod scheduler_fuzz;

use alloc::vec::Vec;

use zenus_sync::spinlock::SpinLock;

/// Global fuzzing statistics
pub static FUZZ_STATS: SpinLock<FuzzStats> = SpinLock::new(FuzzStats::new());

/// Index of the subsystem the current test case belongs to (0..6), plus one
/// past the end for "none". Written by the campaign so an aborted run can say
/// where it stopped.
pub static CURRENT_SUBSYSTEM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);
/// Number of cases started so far.
pub static CASE_COUNTER: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Copy, Default)]
pub struct FuzzStats {
    pub total_cases: u64,
    pub crashes: u64,
    pub hangs: u64,
    pub new_paths: u64,
    pub corpus_size: u64,
    pub coverage_edges: u64,
    pub start_time: u64,
}

impl FuzzStats {
    pub const fn new() -> Self {
        FuzzStats {
            total_cases: 0,
            crashes: 0,
            hangs: 0,
            new_paths: 0,
            corpus_size: 0,
            coverage_edges: 0,
            start_time: 0,
        }
    }
}

/// Campaign mode, as described in the "Mode Operasi" table of the fuzzing doc.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// 100 – 10.000 cases, every commit.
    Smoke,
    /// 10⁵ – 10⁶ cases, looking for new paths.
    Coverage,
    /// 10⁶+ cases, overnight.
    Stress,
    /// Replay known crashes only.
    Regression,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Smoke => "smoke",
            Mode::Coverage => "coverage",
            Mode::Stress => "stress",
            Mode::Regression => "regression",
        }
    }

    /// Default case budget per mode.
    pub fn default_cases(self) -> u64 {
        match self {
            Mode::Smoke => 2_000,
            Mode::Coverage => 50_000,
            Mode::Stress => 1_000_000,
            Mode::Regression => 0,
        }
    }

    /// Probability (in percent) that a corpus input is mutated before running.
    pub fn mutation_ratio(self) -> u64 {
        match self {
            Mode::Smoke => 50,
            Mode::Coverage => 80,
            Mode::Stress => 90,
            Mode::Regression => 0,
        }
    }
}

/// Initialize fuzzing subsystem
pub fn init() {
    let mut stats = FUZZ_STATS.lock();
    stats.start_time = zenus_arch::interrupts::pit::get_ticks();
    drop(stats);
    corpus::init();
    coverage::init();
    crash::clear();
    snapshot::init();
    zenus_console::kinfo!("[FUZZ] Zenus Fuzzing Framework initialized");
}

/// Run one test case under fault containment and return its verdict.
///
/// This function is the *only* safe place to arm the checkpoint. A contained
/// fault resumes at the instruction right after the capture below, which is
/// inside this function: on resume all general registers are zeroed, so the
/// code after that point must not depend on anything the compiler kept in a
/// register across the risky call. Everything needed is re-read from statics
/// and from this function's own stack frame, so the resumed path can simply
/// report the fault and return to the campaign loop through the normal return
/// address.
///
/// `#[inline(never)]` is load-bearing: the recovery point has to be inside this
/// function, not in the middle of the campaign loop.
#[inline(never)]
pub fn run_case(input: &[u8]) -> CaseVerdict {
    // Arm first, publish the recovery point second: `arm` is inlined into a
    // handful of stores, so a recovery that landed on it would re-arm and
    // re-run the failing case forever.
    zenus_arch::fuzz_guard::arm(0, 0);
    let rip: u64;
    let rsp: u64;
    unsafe {
        core::arch::asm!(
            "lea {rip}, [rip]",
            "mov {rsp}, rsp",
            rip = out(reg) rip,
            rsp = out(reg) rsp,
            options(nostack, preserves_flags),
        );
    }
    zenus_arch::fuzz_guard::set_point(rsp, rip);

    if zenus_arch::fuzz_guard::take_resumed() {
        // We are the recovery: the input's registers are gone, so do not touch
        // them — just report and go home through the normal return.
        zenus_arch::fuzz_guard::disarm();
        return CaseVerdict::Faulted;
    }

    let result = execute_input(input);
    let faulted = zenus_arch::fuzz_guard::take_fault();
    zenus_arch::fuzz_guard::disarm();
    if faulted {
        // The guard caught something the syscall-level checks did not.
        crash::record_fault(crash::SUBSYS_SYSCALL, input);
        return CaseVerdict::Faulted;
    }
    match result {
        FuzzResult::Crash => CaseVerdict::Crash,
        _ => CaseVerdict::Ok(result),
    }
}

/// Outcome of a single [`run_case`].
#[derive(Clone, Copy, PartialEq)]
pub enum CaseVerdict {
    /// The case completed; carries the subsystem's own verdict.
    Ok(FuzzResult),
    /// The case reported a bug (sanitiser-style marker).
    Crash,
    /// The case raised a CPU exception that containment unwound.
    Faulted,
}

/// Run one fuzzing iteration
pub fn run_iteration() -> FuzzResult {
    let input = corpus::get_next_input();
    let result = execute_input(&input);
    process_result(input, result)
}

#[derive(Clone, Copy, PartialEq)]
pub enum FuzzResult {
    Normal,
    NewPath,
    Crash,
    Hang,
}

/// Dispatch an input to the subsystem encoded in its first byte.
fn execute_input(input: &[u8]) -> FuzzResult {
    // Determine fuzzing mode based on input header
    if input.is_empty() {
        return FuzzResult::Normal;
    }

    coverage::reset_delta();
    CURRENT_SUBSYSTEM.store(input[0] % 6, core::sync::atomic::Ordering::Relaxed);
    CASE_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

    let result = match input[0] % 6 {
        0 => syscall_fuzz::execute(&input[1..]),
        1 => memory_fuzz::execute(&input[1..]),
        2 => device_fuzz::execute(&input[1..]),
        3 => filesystem_fuzz::execute(&input[1..]),
        4 => shell_fuzz::execute(&input[1..]),
        5 => scheduler_fuzz::execute(&input[1..]),
        _ => FuzzResult::Normal,
    };

    // Coverage decides "new path": a case that executed without crashing but
    // reached an edge never seen before is corpus-worthy. Without this the
    // corpus only ever grows from crashes and coverage stays flat.
    if result == FuzzResult::Normal && coverage::grew() {
        return FuzzResult::NewPath;
    }

    result
}

fn process_result(input: Vec<u8>, result: FuzzResult) -> FuzzResult {
    let mut stats = FUZZ_STATS.lock();
    stats.total_cases += 1;

    match result {
        FuzzResult::NewPath => {
            stats.new_paths += 1;
            corpus::add_input(input);
        }
        FuzzResult::Crash => {
            stats.crashes += 1;
            // Minimisation is quadratic in the worst case; keep it for the
            // small reproducers and store the raw input for the large ones.
            let minimized = if input.len() <= 256 {
                minimizer::minimize(&input)
            } else {
                input.clone()
            };
            crash::record_crash(&minimized);
        }
        FuzzResult::Hang => {
            stats.hangs += 1;
            crash::record_hang(&input);
        }
        FuzzResult::Normal => {}
    }

    stats.corpus_size = corpus::size();
    stats.coverage_edges = coverage::edge_count();
    drop(stats);

    result
}

/// Deterministic per-case PRNG so a campaign is reproducible from its seed.
fn next_rand(state: &mut u64) -> u64 {
    // xorshift64*
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// Run a campaign.
///
/// Returns the number of cases executed. In `Regression` mode the known crash
/// inputs are replayed instead of the mutator/corpus loop.
pub fn run_campaign(mode: Mode, cases: u64, seed: u64) -> FuzzStats {
    init();
    corpus::seed_defaults();
    coverage::reset_delta();

    let mut rng = if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed };
    let ratio = mode.mutation_ratio();

    zenus_console::kinfo!(
        "[FUZZ] campaign start mode={} cases={} mutation={}%",
        mode.name(),
        cases,
        ratio
    );

    let mut executed = 0u64;
    while executed < cases {
        let base = corpus::get_next_input();
        let input = if !base.is_empty() && next_rand(&mut rng) % 100 < ratio {
            let idx = (next_rand(&mut rng) as usize) % mutator::STRATEGIES.len();
            mutator::mutate(&base, mutator::STRATEGIES[idx])
        } else {
            base
        };

        // `run_case` owns the checkpoint: a fault inside the case unwinds back
        // into *it*, not into this loop, so the loop's state is never touched
        // by a fault.
        match run_case(&input) {
            CaseVerdict::Ok(result) => process_result(input, result),
            CaseVerdict::Crash => process_result(input, FuzzResult::Crash),
            CaseVerdict::Faulted => process_result(input, FuzzResult::Crash),
        };
        executed += 1;

        // Heartbeat: keeps the serial output alive if the campaign wedges and
        // gives the Makefile targets a progress line to watch.
        if executed % 500 == 0 {
            let stats = *FUZZ_STATS.lock();
            zenus_console::kinfo!(
                "[FUZZ] progress cases={} crashes={} edges={} corpus={}",
                stats.total_cases,
                stats.crashes,
                stats.coverage_edges,
                stats.corpus_size
            );
        }
    }

    let stats = *FUZZ_STATS.lock();
    zenus_console::kinfo!("[FUZZ] campaign done mode={}", mode.name());
    stats
}

/// Replay every crash currently in the crash log and report whether each one
/// still reproduces. Used by `make fuzz-regression`.
pub fn run_regression(replays: u64) -> u64 {
    init();
    let mut reproduced = 0u64;
    let total = crash::get_crash_count();
    for i in 0..total {
        let record = match crash::get_crash(i) {
            Some(r) => r,
            None => continue,
        };
        let before = zenus_arch::fuzz_guard::FAULTS.load(core::sync::atomic::Ordering::Relaxed);
        for _ in 0..replays.max(1) {
            if record.subsystem == crash::SUBSYS_SYSCALL && record.input.len() >= 49 {
                let id = record.input[0] as u64;
                syscall_fuzz::execute_direct(id, parse_replay_args(&record.input));
            }
        }
        let after = zenus_arch::fuzz_guard::FAULTS.load(core::sync::atomic::Ordering::Relaxed);
        let fixed = after == before;
        if !fixed {
            reproduced += 1;
        }
        zenus_console::kinfo!(
            "[FUZZ] REGRESSION ZENUS-FUZZ-{:06} {}",
            record.id,
            if fixed { "FIXED" } else { "STILL-REPRODUCES" }
        );
    }
    reproduced
}

fn parse_replay_args(input: &[u8]) -> [u64; 6] {
    let mut args = [0u64; 6];
    for i in 0..6 {
        let off = 1 + i * 8;
        if off + 8 <= input.len() {
            let mut b = [0u8; 8];
            b.copy_from_slice(&input[off..off + 8]);
            args[i] = u64::from_le_bytes(b);
        }
    }
    args
}

/// Print fuzzing statistics
pub fn print_stats() {
    let stats = *FUZZ_STATS.lock();
    let elapsed = zenus_arch::interrupts::pit::get_ticks().wrapping_sub(stats.start_time);
    let secs = elapsed / 100;
    let cps = if secs > 0 { stats.total_cases / secs } else { 0 };

    zenus_console::kinfo!("[FUZZ] === Zenus Fuzz Stats ===");
    zenus_console::kinfo!("[FUZZ] total cases: {}", stats.total_cases);
    zenus_console::kinfo!("[FUZZ] crashes: {}", stats.crashes);
    zenus_console::kinfo!("[FUZZ] hangs: {}", stats.hangs);
    zenus_console::kinfo!("[FUZZ] new paths: {}", stats.new_paths);
    zenus_console::kinfo!("[FUZZ] corpus size: {}", stats.corpus_size);
    zenus_console::kinfo!("[FUZZ] coverage edges: {}", stats.coverage_edges);
    zenus_console::kinfo!("[FUZZ] exec/sec: {}", cps);
    zenus_console::kinfo!("[FUZZ] =========================");
}

/// Print the machine-readable campaign summary the Makefile targets parse.
pub fn print_report() {
    let stats = *FUZZ_STATS.lock();
    zenus_console::kinfo!(
        "[FUZZ] SUMMARY cases={} crashes={} hangs={} new_paths={} corpus={} edges={} faults={} unrecovered={}",
        stats.total_cases,
        stats.crashes,
        stats.hangs,
        stats.new_paths,
        stats.corpus_size,
        stats.coverage_edges,
        zenus_arch::fuzz_guard::FAULTS.load(core::sync::atomic::Ordering::Relaxed),
        zenus_arch::fuzz_guard::UNRECOVERED.load(core::sync::atomic::Ordering::Relaxed),
    );
    crash::dump_crashes();
}

/// Reset all fuzzing state between campaigns.
pub fn reset() {
    corpus::clear();
    coverage::init();
    crash::clear();
    snapshot::reset();
    zenus_arch::fuzz_guard::reset();
    let mut stats = FUZZ_STATS.lock();
    *stats = FuzzStats::new();
    stats.start_time = zenus_arch::interrupts::pit::get_ticks();
}

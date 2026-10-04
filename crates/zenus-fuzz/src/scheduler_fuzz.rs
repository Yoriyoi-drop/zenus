use alloc::vec::Vec;

use crate::coverage;
use crate::FuzzResult;

/// Scheduler operation types
#[derive(Clone, Copy)]
enum SchedOp {
    Yield,
    Sleep,
    Fork,
    Clone,
    Exit,
    Wait,
    Kill,
    Nice,
}

/// Fuzzing input format for scheduler:
/// [op: u8] [arg0: u64] [arg1: u64] [arg2: u64]
pub fn execute(input: &[u8]) -> FuzzResult {
    if input.len() < 25 {
        return FuzzResult::Normal;
    }

    let op = match input[0] % 8 {
        0 => SchedOp::Yield,
        1 => SchedOp::Sleep,
        2 => SchedOp::Fork,
        3 => SchedOp::Clone,
        4 => SchedOp::Exit,
        5 => SchedOp::Wait,
        6 => SchedOp::Kill,
        _ => SchedOp::Nice,
    };

    let arg0 = u64::from_le_bytes([
        input[1], input[2], input[3], input[4],
        input[5], input[6], input[7], input[8],
    ]);
    let arg1 = u64::from_le_bytes([
        input[9], input[10], input[11], input[12],
        input[13], input[14], input[15], input[16],
    ]);
    let arg2 = u64::from_le_bytes([
        input[17], input[18], input[19], input[20],
        input[21], input[22], input[23], input[24],
    ]);
    let _ = arg2;

    coverage::record_edge((op as u8 as u64).wrapping_mul(1000));

    match op {
        SchedOp::Yield => fuzz_yield(),
        SchedOp::Sleep => fuzz_sleep(arg0),
        SchedOp::Fork => fuzz_fork(),
        SchedOp::Clone => fuzz_clone(arg0, arg1),
        SchedOp::Exit => fuzz_exit(arg0),
        SchedOp::Wait => fuzz_wait(arg0, arg1),
        SchedOp::Kill => fuzz_kill(arg0, arg1),
        SchedOp::Nice => fuzz_nice(arg0),
    }
}

fn fuzz_yield() -> FuzzResult {
    // Test scheduler yield
    zenus_sched::scheduler::yield_now();
    FuzzResult::Normal
}

fn fuzz_sleep(duration_ms: u64) -> FuzzResult {
    if duration_ms > 10000 {
        return FuzzResult::Normal;
    }

    // PIT ticks at 100 Hz, so `get_ticks()` counts *centiseconds*. The old
    // comparison `elapsed >= duration_ms` therefore slept 100x too long
    // (and with `hlt()` in the loop it stalled the whole campaign on a fuzzed
    // duration instead of the intended few milliseconds).
    const TICKS_PER_SEC: u64 = 100;
    let target_ticks = duration_ms * TICKS_PER_SEC / 1000;

    let start = zenus_arch::interrupts::pit::get_ticks();
    loop {
        let elapsed = zenus_arch::interrupts::pit::get_ticks().wrapping_sub(start);
        if elapsed >= target_ticks {
            break;
        }
        zenus_sched::scheduler::yield_now();
        x86_64::instructions::hlt();
    }

    FuzzResult::Normal
}

fn fuzz_fork() -> FuzzResult {
    // Test fork with various scenarios
    // Note: Actual fork is dangerous in fuzzing, so we just test the interface
    FuzzResult::Normal
}

fn fuzz_clone(flags: u64, stack: u64) -> FuzzResult {
    // Test clone with various flags
    if flags > 0xFF {
        return FuzzResult::Normal;
    }

    // Check for invalid stack pointer
    if stack != 0 && (stack < 0x1000 || stack > 0x0000800000000000) {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

fn fuzz_exit(code: u64) -> FuzzResult {
    // Test exit with various codes
    if code > 0xFF {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

fn fuzz_wait(pid: u64, options: u64) -> FuzzResult {
    // Test wait with various PIDs
    if pid > 10000 {
        return FuzzResult::Normal;
    }

    // Check for invalid options
    if options > 0xFF {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

fn fuzz_kill(pid: u64, sig: u64) -> FuzzResult {
    // Test kill with various signals
    if sig == 0 || sig >= 64 {
        return FuzzResult::Normal;
    }

    if pid > 10000 {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

fn fuzz_nice(increment: u64) -> FuzzResult {
    // Test nice with various increments
    if increment > 40 {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

/// Scheduler seeds: every op crossed with the interesting pid / signal /
/// duration values, including the ones the guards above reject so the guards
/// themselves get exercised.
pub fn generate_seeds() -> Vec<Vec<u8>> {
    let args: [u64; 8] = [0, 1, 2, 0xFF, 0x100, 64, 1000, u64::MAX];
    let mut seeds = Vec::new();
    for op in 0..8u8 {
        for &a0 in &args {
            for &a1 in args.iter().take(4) {
                let mut seed = Vec::with_capacity(25);
                seed.push(op);
                seed.extend_from_slice(&a0.to_le_bytes());
                seed.extend_from_slice(&a1.to_le_bytes());
                seed.extend_from_slice(&0u64.to_le_bytes());
                seeds.push(seed);
            }
        }
    }
    seeds
}

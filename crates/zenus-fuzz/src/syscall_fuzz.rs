use alloc::vec::Vec;

use crate::coverage;
use crate::crash;
use crate::FuzzResult;

/// Syscall number of the case currently executing, published before the
/// dispatch.
///
/// A fuzzed syscall can block forever, and nothing in the fault containment
/// catches that — `fuzz_guard` resumes faults, not parks. When the watchdog
/// fires, `CURRENT_CASE` says *which* case wedged; this says which syscall it
/// was calling, which is what actually identifies the bug.
pub static CURRENT_SYSCALL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Arguments of the case currently executing, for the same reason.
pub static CURRENT_ARGS: [core::sync::atomic::AtomicU64; 6] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
];

/// Fuzzing input format for syscalls:
/// [syscall_id: u8] [arg0..arg5: u64 each] [memory_region: bytes]
pub fn execute(input: &[u8]) -> FuzzResult {
    if input.len() < 7 {
        return FuzzResult::Normal;
    }

    let syscall_id = input[0] as u64;
    if is_task_terminating(syscall_id) {
        // exit/exit_group would tear down the campaign task itself and the
        // campaign would never report a result. Reachable on purpose through
        // `execute_direct` for regression replays.
        return FuzzResult::Normal;
    }
    let args = parse_args(&input[1..]);

    // Published before the dispatch, so the watchdog can name the syscall that
    // wedged the campaign. `fuzz_guard` contains faults; nothing contains a
    // block, so this is the only trail a hang leaves.
    CURRENT_SYSCALL.store(syscall_id, core::sync::atomic::Ordering::Release);
    for (slot, value) in CURRENT_ARGS.iter().zip(args.iter()) {
        slot.store(*value, core::sync::atomic::Ordering::Release);
    }

    // Record coverage for this syscall path
    coverage::record_edge(syscall_id.wrapping_mul(31));

    // The case runs inside a fuzz checkpoint: a fault raised by the fuzzed
    // arguments is a *result*, not a system failure, so the campaign survives
    // and records it instead of taking the whole machine down with it.
    // The campaign already wraps this in a fault checkpoint
    // (`zenus_fuzz::execute_input`), and a contained fault resumes *around*
    // this call — so simply return without touching the dispatcher.
    let result = zenus_syscall::syscall::syscall_dispatch6(
        syscall_id,
        args[0],
        args[1],
        args[2],
        args[3],
        args[4],
        args[5],
    );

    // Sanity-marker returns: some syscall paths return these as sentinels.
    if result == 0xDEADBEEF || result == 0xCAFEBABE {
        crash::classify_crash(syscall_id, &args, result);
        return FuzzResult::Crash;
    }

    coverage::record_edge(0x8000_0000_0000_0000 | (result & 0xFFFF));

    FuzzResult::Normal
}

fn parse_args(input: &[u8]) -> [u64; 6] {
    let mut args = [0u64; 6];
    for i in 0..6 {
        let offset = i * 8;
        if offset + 8 <= input.len() {
            let mut b = [0u8; 8];
            b.copy_from_slice(&input[offset..offset + 8]);
            args[i] = u64::from_le_bytes(b);
        }
    }
    args
}

/// Syscall IDs registered in the dispatch table that terminate the calling
/// task. Fuzzing them from the campaign task would end the campaign (and, if
/// the task is the init task, the machine), so they are only reachable through
/// the explicit `syscall_fuzz::execute_direct` entry point.
fn is_task_terminating(id: u64) -> bool {
    matches!(id, 60 | 231)
}

/// Execute a syscall number directly, bypassing the campaign guard.
///
/// Used by the regression driver to replay a specific syscall/argument tuple
/// with full fault containment.
pub fn execute_direct(syscall_id: u64, args: [u64; 6]) -> u64 {
    zenus_syscall::syscall::syscall_dispatch6(
        syscall_id,
        args[0],
        args[1],
        args[2],
        args[3],
        args[4],
        args[5],
    )
}

/// Generate syscall fuzzing seeds
pub fn generate_seeds() -> Vec<Vec<u8>> {
    let mut seeds = Vec::new();

    // Syscall IDs worth trying first: the read/write/open/mmap core plus the
    // boundary values the documentation calls out (0, last valid, one past
    // the end of the table, and a random high byte).
    let syscall_ids: &[u64] = &[
        0,   // read
        1,   // write
        2,   // open
        3,   // close
        5,   // fstat
        6,   // readdir
        9,   // mmap
        11,  // munmap
        12,  // brk
        45,  // brk (custom slot)
        60,  // exit
        255, // last valid table slot
        256, // one past the end
    ];

    for &id in syscall_ids {
        let mut seed = Vec::with_capacity(49);
        seed.push(id as u8);
        for _ in 0..48 {
            seed.push(0);
        }
        seeds.push(seed);
    }

    // Pointer / length mutation seeds for the memory-touching syscalls.
    for &id in &[0u64, 1, 2, 9] {
        for &ptr in &[
            0u64,                  // NULL
            0x1000,                // first page
            0x8000_0000_0000,      // user-space boundary
            0xFFFF_8000_0000_0000, // kernel address
            u64::MAX,              // wrap-around
        ] {
            let mut seed = Vec::with_capacity(49);
            seed.push(id as u8);
            seed.extend_from_slice(&ptr.to_le_bytes());
            seed.extend_from_slice(&0x1000u64.to_le_bytes());
            for _ in 0..24 {
                seed.push(0);
            }
            seeds.push(seed);
        }
    }

    seeds
}

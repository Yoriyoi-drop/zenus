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
    if is_blocking_syscall(syscall_id) {
        // futex/waitpid/poll/select/ppoll can block indefinitely with no
        // timeout. Skip them in the campaign loop; they remain reachable
        // through `execute_direct` for regression replays.
        return FuzzResult::Normal;
    }
    let mut args = parse_args(&input[1..]);

    // Never fuzz fd=0 (stdin) on blocking reads: `fd_read(0, …)` calls
    // `read_byte_serial()` which busy-waits forever when no serial data is
    // available, wedging the whole campaign. Redirect fd 0 to fd 1 (stdout,
    // write-only — reads return -1 immediately instead of blocking).
    if syscall_id == 0 && args[0] == 0 {
        args[0] = 1;
    }

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
        syscall_id, args[0], args[1], args[2], args[3], args[4], args[5],
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

/// Syscall IDs that can block indefinitely with no timeout. Fuzzing them from
/// the campaign task would wedge the campaign until the watchdog fires, wasting
/// the entire per-case budget. They are skipped here but remain reachable
/// through `execute_direct` for regression replays.
fn is_blocking_syscall(id: u64) -> bool {
    id == 7   // poll: can block until timeout/fd ready
        || id == 23  // select: can block until timeout/fd ready
        || id == 61  // waitpid: can block until child exits
        || id == 212 // ppoll: can block until timeout/fd ready
        || id == 230 // futex(FUTEX_WAIT): can block until woken
}

/// Execute a syscall number directly, bypassing the campaign guard.
///
/// Used by the regression driver to replay a specific syscall/argument tuple
/// with full fault containment.
pub fn execute_direct(syscall_id: u64, args: [u64; 6]) -> u64 {
    zenus_syscall::syscall::syscall_dispatch6(
        syscall_id, args[0], args[1], args[2], args[3], args[4], args[5],
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

#[cfg(test)]
mod host_tests {
    use super::*;

    #[test]
    fn is_blocking_syscall_identifies_blocking_syscalls() {
        assert!(is_blocking_syscall(7));   // poll
        assert!(is_blocking_syscall(23));  // select
        assert!(is_blocking_syscall(61));  // waitpid
        assert!(is_blocking_syscall(212)); // ppoll
        assert!(is_blocking_syscall(230)); // futex
    }

    #[test]
    fn is_blocking_syscall_rejects_non_blocking_syscalls() {
        assert!(!is_blocking_syscall(0));   // read
        assert!(!is_blocking_syscall(1));   // write
        assert!(!is_blocking_syscall(2));   // open
        assert!(!is_blocking_syscall(9));   // mmap
        assert!(!is_blocking_syscall(60));  // exit
        assert!(!is_blocking_syscall(255)); // last table slot
        assert!(!is_blocking_syscall(256)); // past end
    }

    #[test]
    fn is_task_terminating_identifies_exit_syscalls() {
        assert!(is_task_terminating(60));  // exit
        assert!(is_task_terminating(231)); // exit_group
    }

    #[test]
    fn is_task_terminating_rejects_non_terminating_syscalls() {
        assert!(!is_task_terminating(0));   // read
        assert!(!is_task_terminating(1));   // write
        assert!(!is_task_terminating(230)); // futex
    }

    #[test]
    fn parse_args_reads_le64_values() {
        let input = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                     0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80];
        let args = parse_args(&input);
        assert_eq!(args[0], 0x08070605_04030201);
        assert_eq!(args[1], 0x80706050_40302010);
        assert_eq!(args[2], 0);
        assert_eq!(args[3], 0);
        assert_eq!(args[4], 0);
        assert_eq!(args[5], 0);
    }

    #[test]
    fn parse_args_handles_short_input() {
        let input = [0x42u8; 3]; // only 3 bytes
        let args = parse_args(&input);
        assert_eq!(args[0], 0);
        assert_eq!(args[1], 0);
        assert_eq!(args[2], 0);
        assert_eq!(args[3], 0);
        assert_eq!(args[4], 0);
        assert_eq!(args[5], 0);
    }

    #[test]
    fn generate_seeds_are_non_empty() {
        let seeds = generate_seeds();
        assert!(!seeds.is_empty());
        for seed in &seeds {
            assert!(!seed.is_empty());
        }
    }

    #[test]
    fn generate_seeds_include_blocking_and_terminating_ids() {
        // Regression: seeds must not include blocking or task-terminating
        // syscall IDs that would wedge the campaign.
        let seeds = generate_seeds();
        for seed in &seeds {
            let id = seed[0] as u64;
            assert!(!is_blocking_syscall(id),
                "seed uses blocking syscall {id}");
            assert!(!is_task_terminating(id),
                "seed uses task-terminating syscall {id}");
        }
    }
}

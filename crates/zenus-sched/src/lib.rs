#![no_std]
#![allow(static_mut_refs)]
#![allow(bad_asm_style)]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

extern crate alloc;

pub mod init;
pub mod scheduler;
pub mod signal;
pub mod task;

/// Host-side unit tests (`cargo test --workspace`).
///
/// The scheduler itself needs the APIC, the frame allocator and real stacks,
/// so only its pure layout arithmetic is covered here — but that arithmetic is
/// exactly where the frame-clobbering bugs lived, so it is pinned by tests
/// rather than by a comment.
#[cfg(test)]
mod host_tests {
    use crate::scheduler::{
        frame_base, stack_size_is_valid, DEFAULT_HEAP_BRK, FRAME_BYTES, ISR_DESCENT,
        MIN_TASK_STACK, STACK_GUARD,
    };

    /// A kernel task — one made by `create_task_named`, which is *every*
    /// kernel task including init and the fuzzing campaign task — must still
    /// have a heap floor.
    ///
    /// `sys_brk` consults the floor, and `create_task_named` set neither
    /// `heap_brk` nor `heap_floor`, so both read as 0. `get_task_heap_brk`
    /// already treated a stored 0 as "use the default break" — but
    /// `get_task_heap_floor` returned the stored 0 verbatim, so the floor check
    /// was inert for every kernel task and `brk(0x61706e69)` shrank from a page
    /// near the bottom of user space all the way up to 0x6000_0000_0000.
    ///
    /// Creating a task is not host-safe — `create_task_named` reads CR8 and
    /// allocates a real stack — so what is checked here is that the field is
    /// assigned, and that the accessors cannot hand back a zero floor.
    #[test]
    fn a_kernel_task_gets_a_heap_floor_too() {
        let source = include_str!("scheduler.rs");
        let body = source
            .split_once("pub fn create_task_named(")
            .and_then(|(_, rest)| rest.split("\npub fn ").next())
            .expect("create_task_named exists");
        assert!(
            body.contains("task.heap_brk = DEFAULT_HEAP_BRK;"),
            "create_task_named must give a kernel task a break"
        );
        assert!(
            body.contains("task.heap_floor = DEFAULT_HEAP_BRK;"),
            "create_task_named must give a kernel task a floor — without one, \\
             sys_brk's floor check is inert for every kernel task"
        );

        // The accessor must treat a stored 0 as "never initialised" rather than
        // as a floor, because a floor of 0 makes every shrink legal.
        let accessor = source
            .split_once("pub fn get_task_heap_floor(")
            .and_then(|(_, rest)| rest.split("\npub fn ").next())
            .expect("get_task_heap_floor exists");
        assert!(
            accessor.contains("if task.heap_floor == 0"),
            "a stored zero floor must fall back to the default break, not be \\
             returned verbatim"
        );

        // And the exact call the fuzzing campaign made has to be below it.
        assert!(
            0x6170_6e69u64 < DEFAULT_HEAP_BRK,
            "the fuzzed brk address must land below the default floor, which is \\
             the whole point of the check"
        );
    }

    /// Regression: `create_user_task` built its frame at `stack_top`, exactly
    /// where `TSS.RSP0` points, so the first timer tick pushed the CPU frame
    /// straight over the task's saved user context.
    #[test]
    fn task_frame_cannot_be_reached_by_the_timer_isr() {
        let stack_top = 0x0000_7FFF_0000_0000u64;
        let base = frame_base(stack_top);

        // The frame sits at the bottom edge of the reserved area...
        assert_eq!(base, stack_top - STACK_GUARD);
        // ...and the ISR, which descends from the top, must not reach it.
        assert!(
            base + FRAME_BYTES <= stack_top - ISR_DESCENT,
            "frame [{}, {}) overlaps the {ISR_DESCENT} bytes the ISR uses",
            base,
            base + FRAME_BYTES
        );
    }

    /// The guard is only meaningful while it dwarfs the ISR's descent plus the
    /// frame; shrinking STACK_GUARD silently re-opens the clobbering bug.
    #[test]
    fn guard_is_large_enough_for_frame_plus_isr() {
        assert!(
            STACK_GUARD >= ISR_DESCENT + FRAME_BYTES,
            "STACK_GUARD {STACK_GUARD} < ISR {ISR_DESCENT} + frame {FRAME_BYTES}"
        );
    }

    /// Regression: `stack_top - STACK_GUARD` underflows for a small stack,
    /// which made the constructors write the frame near 2^64.
    #[test]
    fn too_small_task_stacks_are_rejected() {
        assert!(!stack_size_is_valid(0));
        assert!(!stack_size_is_valid(1024));
        assert!(!stack_size_is_valid(STACK_GUARD), "exactly the guard is too small");
        assert!(!stack_size_is_valid(STACK_GUARD + 1));
        assert!(stack_size_is_valid(MIN_TASK_STACK));
        assert!(stack_size_is_valid(65536), "the shell's stack size");
        assert!(stack_size_is_valid(1 << 18), "the fuzz runner's stack size");
    }

    /// Regression: an absurd `stack_size` passed the lower bound, then
    /// `stack_base + stack_size` wrapped and `frame_base()` produced a wild
    /// address. `init::service_register` forwards caller-supplied sizes.
    #[test]
    fn absurd_task_stacks_are_rejected() {
        assert!(!stack_size_is_valid(u64::MAX));
        assert!(!stack_size_is_valid(isize::MAX as u64 + 1));
        assert!(!stack_size_is_valid(u64::MAX - STACK_GUARD));
        assert!(stack_size_is_valid(1 << 20));
        assert!(stack_size_is_valid(isize::MAX as u64));
    }

    /// Every accepted size must leave room for the frame *after* the guard is
    /// taken off the top — the property that makes `frame_base` safe.
    #[test]
    fn accepted_stacks_leave_room_for_the_frame() {
        for size in [MIN_TASK_STACK, 65536, 1 << 18, 1 << 20] {
            assert!(stack_size_is_valid(size));
            let usable = size - STACK_GUARD;
            assert!(
                usable >= FRAME_BYTES,
                "{size} byte stack leaves only {usable} usable bytes"
            );
            // No wrap in `stack_base + stack_size` for any real base either.
            assert!(size <= isize::MAX as u64);
        }
    }
}

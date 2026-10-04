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
        frame_base, stack_size_is_valid, FRAME_BYTES, ISR_DESCENT, MIN_TASK_STACK, STACK_GUARD,
    };

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

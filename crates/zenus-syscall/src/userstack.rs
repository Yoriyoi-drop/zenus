//! The initial user stack: where `argv`, the `argv` pointer array and `argc`
//! go on the 16 pages `elf::load_elf` just mapped, and how to write them.
//!
//! This exists as its own module because two callers were doing it differently
//! and both were wrong.
//!
//! `sys_execve` wrote the new image's argv while the **old** CR3 was still
//! loaded. `stack_top` is randomised per image
//! (`elf::load_elf` calls `get_random_page_aligned`), so those user addresses
//! are almost never mapped in the address space that was actually loaded — the
//! writes went to a page table walk that either faulted in ring 0 or, worse,
//! landed on whatever happened to share the address.
//!
//! The shell's `run` command did switch to the new CR3, but left interrupts on
//! for the whole window. A timer tick landing between the two `set_cr3` calls
//! ran the ISR on the half-built user address space, and the first page fault
//! the ISR took (touching an unmapped guard page while unwinding) was taken
//! with SMAP active, so the fault handler could not read the stack it was
//! faulting on. That is a triple fault, i.e. a reboot with no message.
//!
//! Both paths also disagreed about the layout: the shell packed the strings
//! from the top of the stack downwards, `sys_execve` grew them upwards from
//! `stack_top - total`, so a program run from the shell and the same program
//! run through `execve` saw argv at different addresses.
//!
//! The layout is pure arithmetic over `stack_top` and the argument lengths, so
//! it is split out and host-tested; only [`write_initial_user_stack`] touches
//! hardware.

/// One `elf::load_elf` stack: 16 pages, `stack_pages` in `elf.rs`.
pub const USER_STACK_PAGES: u64 = 16;
pub const USER_STACK_BYTES: u64 = USER_STACK_PAGES * 4096;

/// Slack kept below the layout so the first `push` in `_start` has somewhere to
/// go. The shell's `run` used an ad-hoc `64000` byte budget for this.
const STACK_HEADROOM: u64 = 512;

/// Where each piece of the initial stack goes.
///
/// All fields are virtual addresses in the *new* address space. `str_pos` is
/// the lowest byte of `argv[0]`'s string; the strings run upwards from there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserStackLayout {
    pub str_pos: u64,
    /// `argv[0..argc]`, then a NULL, 8-byte aligned.
    pub ptr_array_start: u64,
    /// The value `argc` is stored at, and the initial `%rsp`.
    pub user_rsp: u64,
    /// Every byte the layout occupies, measured from `user_rsp` up to the top.
    pub bytes_used: u64,
}

/// Work out where `argv`, the pointer array and `argc` go.
///
/// `argv_lens[i]` is the length of argument `i` **excluding** its NUL; this
/// function adds the terminators, because every caller has the same idea of
/// where they are and got it wrong in different ways once already.
///
/// Returns `None` when the layout would not fit in the 16 pages `load_elf`
/// mapped, or when the arithmetic would wrap. Callers must not fall back to a
/// truncated layout — the program's argv has to match its `argc`, and a
/// half-written pointer array is worse than a failed `execve`.
pub fn layout_user_stack(stack_top: u64, argv_lens: &[usize]) -> Option<UserStackLayout> {
    // `checked_add` all the way down. The strings sit *below* `stack_top`, so
    // an unchecked `stack_top - total` underflows into the kernel half for a
    // short stack with a long argv, and the writes then land wherever the wrap
    // put them.
    let mut str_pos = stack_top;
    for &len in argv_lens {
        str_pos = str_pos.checked_sub(len as u64)?.checked_sub(1)?;
    }

    // Pointer array plus its NULL terminator, 8-byte aligned. The mask is
    // applied *after* the subtraction and must not be preceded by extra slack:
    // `(x - 8) & !7` and `(x - 8 - 7) & !7` differ by a whole word for every
    // `x` that is not congruent to 7 mod 8, which is how the two old
    // implementations ended up with different layouts.
    let ptr_array_size = (argv_lens.len() as u64).checked_add(1)?.checked_mul(8)?;
    let ptr_array_start = str_pos.checked_sub(ptr_array_size)? & !7u64;

    let user_rsp = ptr_array_start.checked_sub(8)?;

    // `user_rsp` must stay inside the stack `load_elf` actually mapped, which
    // is the 16 pages ending at `stack_top`.
    let headroom_used = stack_top.checked_sub(user_rsp)?;
    if headroom_used + STACK_HEADROOM > USER_STACK_BYTES {
        return None;
    }

    Some(UserStackLayout {
        str_pos,
        ptr_array_start,
        user_rsp,
        bytes_used: headroom_used,
    })
}

/// Write `argv` and `argc` into the address space `cr3` describes.
///
/// This is the only place allowed to run with another CR3 loaded, so it keeps
/// the window as small as it can:
///
/// * interrupts off, restored to whatever they were,
/// * `stac` around the writes so SMAP cannot fault the supervisor access,
/// * `clac` and the old CR3 back before interrupts are re-enabled, so no ISR
///   ever observes a half-built user stack or a supervisor access window left
///   open on a user page.
pub fn write_initial_user_stack(cr3: u64, layout: &UserStackLayout, argv: &[&[u8]]) {
    let irq_was_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();

    let old_cr3 = zenus_mem::paging::kernel_cr3();
    unsafe {
        zenus_mem::paging::set_cr3(cr3);
        zenus_arch::cpu::stac();

        // Strings, packed from `str_pos` upwards. Each is NUL-terminated; the
        // terminator is the byte `argv[i]`'s length does not cover.
        let mut cur = layout.str_pos;
        for arg in argv {
            let len = arg.len();
            core::ptr::copy_nonoverlapping(arg.as_ptr(), cur as *mut u8, len);
            *((cur + len as u64) as *mut u8) = 0;
            cur += len as u64 + 1;
        }

        // Pointer array, then the NULL that ends argv.
        let mut ptr_pos = layout.ptr_array_start;
        let mut str_cur = layout.str_pos;
        for arg in argv {
            *((ptr_pos) as *mut u64) = str_cur;
            ptr_pos += 8;
            str_cur += arg.len() as u64 + 1;
        }
        *((ptr_pos) as *mut u64) = 0;

        *((layout.user_rsp) as *mut u64) = argv.len() as u64;

        zenus_arch::cpu::clac();
        zenus_mem::paging::set_cr3(old_cr3);
    }

    if irq_was_enabled {
        x86_64::instructions::interrupts::enable();
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;

    const TOP: u64 = 0x0000_7FFF_1234_5000u64;

    /// The regression that motivated the module: `sys_execve` grew the strings
    /// upwards from `stack_top - total`, the shell packed them downwards from
    /// `stack_top`, and the two disagreed about where `argv[0]` lives.
    #[test]
    fn layout_is_the_one_the_shell_built() {
        let argv: [&[u8]; 2] = [b"/bin/ls", b"-l"];
        let lens = [7usize, 2];
        let l = layout_user_stack(TOP, &lens).expect("fits");

        // Strings are packed downwards from the top of the stack, each with a
        // NUL, so `str_pos` is the lowest byte of `argv[0]` and the last
        // terminator lands exactly on `stack_top`.
        assert_eq!(l.str_pos, TOP - (7 + 1) - (2 + 1));
        assert_eq!(l.str_pos + 7 + 1 + 2 + 1, TOP);
        // The array holds two pointers and a NULL: 24 bytes, masked to 8,
        // below the strings.
        assert_eq!(l.ptr_array_start, (l.str_pos - 24) & !7);
        assert_eq!(l.ptr_array_start % 8, 0);
        // `argc` sits 8 bytes below the array.
        assert_eq!(l.user_rsp, l.ptr_array_start - 8);
        assert_eq!(l.bytes_used, TOP - l.user_rsp);
        assert_eq!(l.user_rsp % 8, 0);

        // And the layout is self-consistent for the writer.
        assert_eq!(argv.len(), 2);
    }

    #[test]
    fn empty_argv_still_terminates() {
        let l = layout_user_stack(TOP, &[]).expect("fits");
        // Nothing to pack, so `str_pos` is the top of the stack and the whole
        // layout is just the NULL terminator plus `argc`.
        assert_eq!(l.str_pos, TOP);
        assert_eq!(l.ptr_array_start, TOP - 8);
        assert_eq!(l.user_rsp, TOP - 16);
        assert!(l.user_rsp >= TOP - USER_STACK_BYTES);
    }

    #[test]
    fn a_null_argument_still_costs_a_byte() {
        // `argv_lens` excludes the NUL. An empty argument is length 0, which
        // must still reserve one byte for its terminator — otherwise two
        // arguments can share a byte and the second string is invisible.
        let l = layout_user_stack(TOP, &[0, 0]).expect("fits");
        assert_eq!(l.str_pos, TOP - 2);
        assert_eq!(l.ptr_array_start, (TOP - 2 - 24) & !7);
        assert!(l.bytes_used >= 2 + 24 + 8);
    }

    /// The unwrap that used to be there. `stack_top - total` with an
    /// underflowing `total` produced an address in the kernel half, and the
    /// writes went through it.
    #[test]
    fn an_argv_that_does_not_fit_is_refused_not_wrapped() {
        // 16 pages minus headroom and the pointer array is the real budget.
        assert!(layout_user_stack(TOP, &[USER_STACK_BYTES as usize]).is_none());
        // 4 * 16384 + 4 terminators + a 3-entry array + argc clears the
        // 512-byte headroom by 596 bytes.
        assert!(layout_user_stack(TOP, &[0x4000; 4]).is_none());
        // A stack top so low that even an empty argv underflows.
        assert!(layout_user_stack(0, &[]).is_none());
        assert!(layout_user_stack(8, &[]).is_none());
        // A single argument longer than the whole stack.
        assert!(layout_user_stack(TOP, &[USER_STACK_BYTES as usize * 4]).is_none());
    }

    /// `bytes_used` is a distance from `user_rsp` up to `stack_top`, so a
    /// layout that "fits" but reports more than the stack holds would let a
    /// caller with its own, looser budget write past the last mapped page.
    #[test]
    fn bytes_used_never_exceeds_the_stack() {
        for argc in 0..8usize {
            for len in [0usize, 1, 7, 8, 100, 1000] {
                let lens = alloc::vec![len; argc];
                if let Some(l) = layout_user_stack(TOP, &lens) {
                    assert!(
                        l.bytes_used + STACK_HEADROOM <= USER_STACK_BYTES,
                        "argc={argc} len={len} used {} bytes",
                        l.bytes_used
                    );
                    // An empty argv leaves `str_pos` on the top of the stack,
                    // which is where the first argument would start.
                    assert!(l.str_pos <= TOP);
                    assert!(l.ptr_array_start < l.str_pos);
                    assert!(l.user_rsp < l.ptr_array_start);
                    assert!(l.ptr_array_start % 8 == 0);
                }
            }
        }
    }

    /// A stack top that is not page aligned is still usable: `load_elf` rounds
    /// the top down, but the layout must not assume it.
    #[test]
    fn unaligned_stack_top_is_handled() {
        let l = layout_user_stack(TOP + 13, &[3, 5]).expect("fits");
        assert_eq!(l.ptr_array_start % 8, 0);
        assert_eq!(l.user_rsp % 8, 0);
        assert!(l.user_rsp >= TOP + 13 - USER_STACK_BYTES);
    }
}
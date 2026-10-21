//! The user view of an address space (KPTI groundwork).
//!
//! A ring-3 task runs with a PML4 whose entries 0..256 are its own and whose
//! entries 256..511 are a copy of the kernel's — so every kernel page is
//! mapped, supervisor-only but *present*, while user code runs. SMEP and SMAP
//! stop the kernel from executing or reading user pages; they do nothing about
//! a ring-3 load of a kernel address that the CPU speculates on before the
//! permission check retires.
//!
//! The fix is a second PML4 per task: the *user view*, which contains the user
//! half and nothing else. This module builds and destroys that table. It does
//! **not** activate it: switching CR3 at the user/kernel boundary needs entry
//! stubs for `syscall` and for all 256 IDT vectors (the CPU fetches the
//! handler through the *current* CR3, so the first instruction of every
//! handler must live in a page mapped in both views). Until those exist, a task
//! still runs with the kernel view, and nothing here is on the hot path.

use core::sync::atomic::Ordering;

use crate::paging;

/// PML4 entries that belong to the user half of the address space.
///
/// Everything at or above this index is kernel-only and must not appear in a
/// user view. The split is a property of the 4-level layout, not a choice.
pub const USER_HALF_ENTRIES: usize = 256;

/// Build the user view for `kernel_view_cr3`.
///
/// The user half is shared with the address space it came from: the two tables
/// point at the same PDPTs, so a mapping added to the task after this call is
/// visible in both. The kernel half is zeroed, which is the whole point — a
/// ring-3 load of any kernel address finds a non-present entry instead of a
/// supervisor page.
///
/// Returns the new CR3 value (physical PML4 address plus the caller's
/// PCID/flags bits), or `None` when the frame allocator is empty.
pub fn build_user_view(kernel_view_cr3: u64) -> Option<u64> {
    let hhdm = paging::hhdm_offset();
    if hhdm == 0 {
        return None;
    }

    let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
    let frame = allocator.alloc_frame()?;
    drop(allocator);

    let src = ((kernel_view_cr3 & !0xFFF) + hhdm) as *const u64;
    let dst = (frame.as_u64() + hhdm) as *mut u64;

    // SAFETY: both pointers address one 4 KiB PML4 through the HHDM; the source
    // is a live page table and the destination is a frame we just took from the
    // allocator and have not published anywhere.
    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, USER_HALF_ENTRIES);
        // Zeroed in bytes, not in u64 elements: `write_bytes` counts elements,
        // and the 512*8-on-`*mut u64` form used to zero 32 KiB past the end of
        // a page table (see `paging::page_table_bytes`).
        core::ptr::write_bytes(
            (dst as *mut u8).add(USER_HALF_ENTRIES * core::mem::size_of::<u64>()),
            0,
            USER_HALF_ENTRIES * core::mem::size_of::<u64>(),
        );
    }

    Some(frame.as_u64() | (kernel_view_cr3 & 0xFFF))
}

/// Free a user view's PML4.
///
/// Only the PML4 frame is freed: the user half is shared with the address space
/// the view was built from, and that space owns (and frees) those PDPTs. The
/// view must therefore be destroyed *before* the address space it came from —
/// after `destroy_address_space` this table would point at freed frames.
pub fn destroy_user_view(user_view_cr3: u64) {
    let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
    allocator.free_frame(x86_64::PhysAddr::new(user_view_cr3 & !0xFFF));
}

/// Whether a PML4 index is part of the user half.
pub const fn is_user_slot(index: usize) -> bool {
    index < USER_HALF_ENTRIES
}

/// The PML4 index a virtual address falls in.
pub const fn slot_of(virt: u64) -> usize {
    ((virt >> 39) & 0x1FF) as usize
}

#[cfg(test)]
mod host_tests {
    /// The split is at bit 47 of the canonical address, so every address below
    /// `0x0000_8000_0000_0000` is in the user half and every address at or
    /// above it is not.
    #[test]
    fn the_user_half_is_the_low_256_slots() {
        assert!(super::is_user_slot(0));
        assert!(super::is_user_slot(255));
        assert!(!super::is_user_slot(256));
        assert!(!super::is_user_slot(511));

        assert_eq!(super::slot_of(0), 0);
        assert_eq!(super::slot_of(0x0000_7FFF_FFFF_FFFF), 255);
        assert_eq!(super::slot_of(0xFFFF_8000_0000_0000), 256);
        assert_eq!(super::slot_of(0xFFFF_FFFF_8000_0000), 511);
    }

    /// A user address and a kernel address never share a slot, which is what
    /// lets the user view drop the kernel half without touching a user mapping.
    #[test]
    fn user_and_kernel_addresses_never_share_a_pml4_slot() {
        for addr in [0x400000u64, 0x7FFF_FFFF_F000, 0x0000_7FFF_0000_0000] {
            assert!(super::is_user_slot(super::slot_of(addr)), "{:#x}", addr);
        }
        for addr in [0xFFFF_8000_0000_0000u64, 0xFFFF_FFFF_8000_0000, 0xFFFF_C000_0000_0000] {
            assert!(!super::is_user_slot(super::slot_of(addr)), "{:#x}", addr);
        }
    }
}

/// In-kernel tests (`make test`). These need the frame allocator and real page
/// tables, so they cannot run on the host.
#[cfg(feature = "testing")]
pub mod tests {
    use super::*;

    /// A user view keeps the user mappings and drops every kernel mapping.
    ///
    /// This is the property the whole isolation depends on, and it is checked by
    /// walking the new table rather than by switching to it: switching would
    /// fault on the very next instruction, because kernel text is exactly what
    /// the view no longer maps — which is the point, and also why activation
    /// needs entry stubs before it can be tested end to end.
    pub fn test_user_view_hides_the_kernel() -> Result<(), &'static str> {
        const USER_VA: u64 = 0x0000_0000_0040_0000;

        let addr_space =
            crate::paging::create_address_space().ok_or("no frame for an address space")?;

        let frame = {
            let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
            match allocator.alloc_frame() {
                Some(f) => f,
                None => return Err("no frame for a user page"),
            }
        };
        if !crate::paging::map_user_page_raw(addr_space, USER_VA, frame.as_u64(), true, false) {
            return Err("could not map the user page");
        }

        let view = match build_user_view(addr_space) {
            Some(v) => v,
            None => return Err("no frame for the user view"),
        };

        if crate::paging::virt_to_phys_raw(view, USER_VA).is_none() {
            return Err("the user view must still map the user page");
        }

        // Kernel text, and the direct map: both must be gone.
        let kernel_text = test_user_view_hides_the_kernel as usize as u64;
        if crate::paging::virt_to_phys_raw(view, kernel_text).is_some() {
            return Err("the user view must not map kernel text");
        }
        if crate::paging::virt_to_phys_raw(view, crate::paging::hhdm_offset()).is_some() {
            return Err("the user view must not map the HHDM");
        }

        // The kernel view, by contrast, still has both.
        if crate::paging::virt_to_phys_raw(addr_space, kernel_text).is_none() {
            return Err("the kernel view must keep the kernel mapped");
        }

        destroy_user_view(view);
        crate::paging::destroy_address_space(addr_space);
        Ok(())
    }
}

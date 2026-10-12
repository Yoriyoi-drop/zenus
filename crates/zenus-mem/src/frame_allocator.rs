use x86_64::structures::paging::{FrameAllocator as FrameAllocatorTrait, PhysFrame, Size4KiB};
use x86_64::PhysAddr;
use zenus_sync::spinlock::SpinLock;

use crate::paging::PAGE_SIZE;

const FREE_STACK_SIZE: usize = 16384;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: u64,
}

pub static FRAME_ALLOCATOR: SpinLock<FrameAllocator> = SpinLock::new(FrameAllocator {
    regions: [MemRegion { base: 0, length: 0 }; MAX_REGIONS],
    region_count: 0,
    next_free: 0,
    floor: 0,
    low_cursor: [0; MAX_REGIONS],
    free_stack: [0; FREE_STACK_SIZE],
    free_count: 0,
    total_memory: 0,
    used_memory: 0,
});

impl MemoryRegion {
    pub fn is_usable(&self) -> bool {
        self.kind == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemRegion {
    pub base: u64,
    pub length: u64,
}

const MAX_REGIONS: usize = 64;

pub struct FrameAllocator {
    regions: [MemRegion; MAX_REGIONS],
    region_count: usize,
    next_free: u64,
    /// The floor `plan_regions` seeded [`Self::next_free`] with.
    ///
    /// Pages below it are not lost, they are deferred: the low pass serves them,
    /// and this is the ceiling that pass may not cross, because at and above it
    /// the high pass is the one that works.
    ///
    /// Without this the low pass used `next_free` as its ceiling, which is the
    /// same value only until the first allocation. `next_free` then climbs past
    /// every page the high pass serves, so a low pass bounded by it would turn
    /// around and serve those pages a second time. That is precisely what the
    /// first version of this cursor did, and
    /// `serving_a_frame_never_changes_the_region_list` caught it as "12800 pages
    /// served out of a 32 MiB region".
    floor: u64,
    /// Per-region watermark for the low pass: the next page to try in region `i`.
    ///
    /// Per region, not one address for the whole pass. A single monotonic cursor
    /// only works if the regions are sorted by base, and `reserve_region` can
    /// leave them unsorted (a split inserts a tail after the entry it came from,
    /// and the usable map's order is the bootloader's, not ours). Unsorted, a
    /// single cursor walks past a low region and never comes back, so its pages
    /// are simply gone.
    ///
    /// `0` means "not started"; the first candidate is `max(cursor[i], base)`.
    low_cursor: [u64; MAX_REGIONS],
    free_stack: [u64; FREE_STACK_SIZE],
    free_count: usize,
    total_memory: u64,
    used_memory: u64,
}

impl FrameAllocator {
    /// Empty this allocator in place.
    ///
    /// Separate from `new` because `*self = Self::new(...)` materialises a whole
    /// `FrameAllocator` as a temporary first, and that is 128 KiB — the free
    /// stack — on whatever stack the assignment happens on. On bare metal that
    /// is the boot stack, and blowing it is what BUG-031 was.
    fn reset(&mut self) {
        self.regions = [MemRegion { base: 0, length: 0 }; MAX_REGIONS];
        self.region_count = 0;
        self.next_free = 0;
        self.floor = 0;
        self.low_cursor = [0; MAX_REGIONS];
        // A loop, not `self.free_stack = [0; FREE_STACK_SIZE]`: the array
        // literal is a 128 KiB temporary, and assigning it is a 128 KiB stack
        // frame. This runs on the boot stack. That is BUG-031 a third time, and
        // the first two were found by looking at the assembly — so the rule here
        // is simply that nothing in this file may build a `FrameAllocator`
        // temporary.
        //
        // Only the live prefix is cleared: `free_count` is zero, so the rest is
        // unreachable and clearing it would be 128 KiB of pointless writes on
        // the stack-limited path.
        //
        // `regions` and `low_cursor` cannot be left half-cleared the way
        // `free_stack` can. `region_count` is zeroed below, and everything in
        // this file is bounded by it, so a stale entry *past* `region_count` is
        // unreachable — a dead store. `low_cursor` is read for every index below
        // `region_count`, so it is not: a cursor left over from a previous plan
        // bounds the low pass at a watermark from a region list that no longer
        // exists, and the pages under it are skipped for the rest of the boot.
        // That asymmetry is why one is cleared in full and the other is not.
        for i in 0..self.free_count {
            self.free_stack[i] = 0;
        }
        self.free_count = 0;
        self.total_memory = 0;
        self.used_memory = 0;
    }

    /// Build an allocator from a memory map, for a host test.
    ///
    /// Host-only in practice: this materialises a `FrameAllocator` — a 128 KiB
    /// free stack — as a stack local, which is exactly the temporary `reset`
    /// warns about, and it has no `rsp` to protect or linker symbols to read.
    ///
    /// It does *not* reimplement the seeding. It builds the same
    /// [`AllocPlanInput`] `global_init` would, with "no kernel image" and "no
    /// boot stack" standing in for the two things a test process cannot supply,
    /// and calls the same [`plan_regions`]. Before this, `new` and `global_init`
    /// were two copies of the same logic that differed only by inspection — so a
    /// host test could pass while `global_init` was wrong.
    pub fn new(memory_map: &[MemoryRegion]) -> Self {
        let mut allocator = FrameAllocator {
            regions: [MemRegion { base: 0, length: 0 }; MAX_REGIONS],
            region_count: 0,
            next_free: 0,
            floor: 0,
            low_cursor: [0; MAX_REGIONS],
            free_stack: [0; FREE_STACK_SIZE],
            free_count: 0,
            total_memory: 0,
            used_memory: 0,
        };

        let input = AllocPlanInput {
            memory_map,
            kernel_image_base: 0,
            kernel_image_end: 0,
            boot_stack_phys: 0,
        };
        plan_regions(&mut allocator, &input);

        zenus_console::kinfo!(
            "Memory: {} MB total",
            allocator.total_memory / (1024 * 1024)
        );

        allocator
    }

    pub fn alloc_frame(&mut self) -> Option<PhysAddr> {
        if self.free_count > 0 {
            self.free_count -= 1;
            let addr = PhysAddr::new(self.free_stack[self.free_count]);
            self.used_memory += PAGE_SIZE as u64;
            return Some(addr);
        }

        let frame_size = PAGE_SIZE as u64;

        for reg_idx in 0..self.region_count {
            let reg = self.regions[reg_idx];
            let start = core::cmp::max(reg.base, self.next_free);
            let start_aligned = (start + 0xFFF) & !0xFFF;
            let end = reg.base + reg.length;

            if start_aligned + frame_size <= end {
                self.next_free = start_aligned + frame_size;
                self.used_memory += frame_size;
                return Some(PhysAddr::new(start_aligned));
            }
        }

        // Fallback: the pages of a region that sit below `floor` and so are
        // invisible to the pass above. `plan_regions` raises the floor to 16 MiB
        // when the first region straddles it, to leave the kernel image and boot
        // stack alone, and every usable page under that is still real memory —
        // this is where it is served, once the high pass has taken everything it
        // can reach. High memory first is deliberate: it is the part that is
        // always safe.
        //
        // The region list is **read-only** here. What moves is `low_cursor`, the
        // allocator's own per-region watermark for this pass.
        //
        // It used to advance `regions[i].base` instead, which looked like the
        // same thing and was not: it grew that region's claimed end
        // (`base + length`) by a page on every allocation, so after enough of
        // them the region described memory that does not exist and the *first*
        // pass — which trusts `base + length` — would hand it out. It served
        // 3584 pages of nothing on a 32 MiB region before the fix, and
        // `total_memory` drifted up with it. Shrinking `length` by the same
        // amount fixes the accounting but collides with `reserve_region`, which
        // runs during initialisation and is entitled to assume a region
        // describes what exists rather than what is left.
        //
        // So the cursor lives outside `regions`, and `regions` means the same
        // thing to every reader at every point in the allocator's life. The
        // `max` is what makes it robust to reshaping: a reservation that moves a
        // region's base up leaves a stale cursor below it, and the max discards
        // it instead of serving a reserved page.
        //
        // The ceiling is `floor`, **not** `next_free`. `next_free` climbs past
        // every page the high pass serves, so a low pass bounded by it would
        // come back round and serve those pages again.
        for reg_idx in 0..self.region_count {
            let reg = self.regions[reg_idx];
            let base = (reg.base + PAGE_SIZE_U64 - 1) & !(PAGE_SIZE_U64 - 1);
            let cursor = core::cmp::max(self.low_cursor[reg_idx], base);
            let end = reg.base + reg.length;
            if cursor + frame_size <= end && cursor < self.floor {
                self.low_cursor[reg_idx] = cursor + frame_size;
                self.used_memory += frame_size;
                return Some(PhysAddr::new(cursor));
            }
        }

        None
    }

    /// Return a frame to the allocator.
    ///
    /// The frame goes on `free_stack`, and `alloc_frame` pops from there before
    /// it bumps `next_free`. Without that, every frame ever handed out is gone
    /// for good: the bump pointer only ever moves forward.
    ///
    /// ## What used to be here, and why it was wrong
    ///
    /// This used to refuse any frame that fell inside a known region, on the
    /// theory that this was a double-free guard. It is not one. Regions describe
    /// memory *available to be handed out*, which is every frame this allocator
    /// ever returned — so the guard rejected every legitimate free, `free_stack`
    /// stayed permanently empty, and `used_memory` (decremented *after* the
    /// guard, so also never reached) kept counting. `meminfo` reported
    /// "Used: 0 frames" for a whole boot no matter how much had been mapped, and
    /// "Free stack: 0 frames" on a machine with 2 GiB spare.
    ///
    /// Membership in `free_stack` is the guard that can actually tell a double
    /// free from a legitimate re-free: a frame that was freed, re-allocated and
    /// freed again is legitimately *absent* from the stack the second time.
    pub fn free_frame(&mut self, addr: PhysAddr) {
        let a = addr.as_u64();
        // Not a frame address; handing one out would give a caller a page whose
        // neighbours are whatever happens to be there.
        if a == 0 || a & (PAGE_SIZE_U64 - 1) != 0 {
            return;
        }
        // Already on the stack: a double free. Returning here also keeps
        // `used_memory` from being decremented twice for one frame.
        if self.free_stack[..self.free_count].contains(&a) {
            return;
        }
        if self.free_count < FREE_STACK_SIZE {
            self.free_stack[self.free_count] = a;
            self.free_count += 1;
            self.used_memory = self.used_memory.saturating_sub(PAGE_SIZE_U64);
            return;
        }
        // The stack is full. Appending a one-page *region* and lowering
        // `next_free` to reach it looked like a way to keep the memory, but
        // `next_free` is the high-water mark of what has been handed out, so
        // lowering it re-offers frames that are still live. Dropping the frame is
        // the honest option; the stack is 16384 entries.
        zenus_console::kwarn!(
            "Frame free stack full, dropping frame {:#x} ({} entries)",
            a,
            FREE_STACK_SIZE
        );
    }

    pub fn used_memory(&self) -> u64 {
        self.used_memory
    }
    pub fn total_memory(&self) -> u64 {
        self.total_memory
    }
    /// How many frames are still on the recycled stack.
    ///
    /// Not the same number as [`Self::free_frames_count`], which is what
    /// `alloc_frame` can serve from *everywhere*. This one is the stack depth
    /// alone, and exists so a test can watch a free land on the stack even when
    /// thousands of untouched pages sit behind it.
    pub fn recycled_frames_count(&self) -> usize {
        self.free_count
    }

    /// How many frames `alloc_frame` can still hand out, exactly.
    ///
    /// This used to be `free_count` — the depth of the recycled stack — which
    /// was wrong in both directions and wrong in a way that mattered: `sys_mmap`
    /// budgets against it, so with the stack below 16 384 entries it refused
    /// mappings the machine could satisfy, and a fresh allocator reported zero
    /// free memory. It is now the recycled stack plus the pages *each pass* can
    /// still reach, computed the same way `alloc_frame` reaches them.
    ///
    /// Exact rather than an estimate, because the two passes are disjoint: the
    /// high pass serves pages at or above `next_free` (which starts at `floor`
    /// and only climbs), the low pass serves pages strictly below `floor`, and
    /// the recycled stack holds pages both passes have already moved past. So
    /// nothing is counted twice and no servable page is left out.
    ///
    /// ## When it can still over-count
    ///
    /// [`Self::clear_free_stack`] drops recycled frames on the floor. Those pages
    /// were already served once, so no pass will reach them again, and this
    /// counts them as free until they would have been popped — the one case
    /// where the number is optimistic. `mmap` refusing on that number is the
    /// safe failure, and the alternative (a stale set of frames) is not.
    pub fn free_frames_count(&self) -> usize {
        self.free_count + self.high_pass_frames() + self.low_pass_frames()
    }

    /// The pages the high (bump) pass can still serve.
    ///
    /// `alloc_frame` picks the *first* region, in list order, whose next page
    /// fits — which drains region 0 completely before region 1 is ever touched,
    /// and so on down the list. A single ordered walk here therefore describes
    /// every future allocation: it advances the same cursor the allocator would,
    /// and per region counts the pages from that cursor up to the last page
    /// whose end stays inside the region.
    fn high_pass_frames(&self) -> usize {
        let frame = PAGE_SIZE_U64;
        let mut cursor = self.next_free;
        let mut total = 0usize;
        for i in 0..self.region_count {
            let reg = self.regions[i];
            let start = (core::cmp::max(reg.base, cursor) + frame - 1) & !(frame - 1);
            let end = reg.base + reg.length;
            if start + frame <= end {
                let pages = ((end - start) / frame) as usize;
                total += pages;
                cursor = start + (pages as u64) * frame;
            }
        }
        total
    }

    /// The pages the low pass can still serve.
    ///
    /// Per region, from `low_cursor` (or the page-aligned base if it has not
    /// started) up to the last page that fits before both the region's end and
    /// `floor`. The bound is `floor`, not `next_free`, for the same reason
    /// `alloc_frame` uses it: `next_free` has climbed past everything the high
    /// pass served, and a low pass bounded by it would count those pages twice.
    fn low_pass_frames(&self) -> usize {
        let frame = PAGE_SIZE_U64;
        let mut total = 0usize;
        for i in 0..self.region_count {
            let reg = self.regions[i];
            let base = (reg.base + frame - 1) & !(frame - 1);
            let cursor = core::cmp::max(self.low_cursor[i], base);
            let end = reg.base + reg.length;
            // The pass serves page `p` when `p + frame <= end` and `p < floor`,
            // so the last candidate is the page-aligned top that satisfies both.
            // Taking `floor - frame` directly would be wrong for a `floor` that
            // is not page aligned, which `plan_regions` does not promise.
            let limit = core::cmp::min(end, self.floor);
            if limit < frame {
                continue;
            }
            let last = (limit - frame) & !(frame - 1);
            if last >= cursor {
                total += ((last - cursor) / frame) as usize + 1;
            }
        }
        total
    }

    /// Clear all entries from the free stack.
    /// Used before loading user programs to prevent stale frame reuse
    /// from previous address space destruction.
    pub fn clear_free_stack(&mut self) {
        self.free_count = 0;
    }

    /// The allocatable regions, as `(base, length)` pairs, in list order.
    ///
    /// Read-only on purpose. `regions` is what `reserve_region` reshapes and what
    /// `alloc_frame` walks, so the tests for the first need to see it — and they
    /// need to see it without being able to write it, because a test that can
    /// mutate what it checks proves nothing.
    pub fn regions(&self) -> &[MemRegion] {
        &self.regions[..self.region_count]
    }

    /// How many entries of `regions` are in use.
    pub fn region_count(&self) -> usize {
        self.region_count
    }

    pub fn reserve_region(&mut self, base: u64, length: u64) {
        if length == 0 {
            return;
        }
        // Round the reservation out to whole pages.
        //
        // Without this, a sub-page range splits the region list on exact byte
        // boundaries and leaves a region whose base is not page-aligned —
        // `[0x1000, 0x5000)` minus `[0x1800, 0x2000)` becomes
        // `[0x1000, 0x1800)` and `[0x2000, 0x5000)`. `alloc_frame` rounds a
        // region's base *up*, so it happily serves page `0x1000` even though the
        // reservation overlaps its top half, and the reservation protects
        // nothing.
        //
        // Rounding out is the safe direction: it reserves at least what was
        // asked for. Every caller today is already page aligned — the memory map
        // entries are trimmed to pages by the bootloader, `reserve_boot_stack`
        // rounds its base down and adds a page, and the kernel image bounds come
        // from the linker script — so this changes nothing for them and closes
        // the hole for anything that is not.
        let base = base & !(PAGE_SIZE_U64 - 1);
        let end = (base.saturating_add(length) + PAGE_SIZE_U64 - 1) & !(PAGE_SIZE_U64 - 1);
        let mut i = 0;
        while i < self.region_count {
            let r = self.regions[i];
            let r_end = r.base + r.length;
            if end <= r.base || base >= r_end {
                i += 1;
                continue;
            }
            // Overlaps covers region completely
            if base <= r.base && end >= r_end {
                for j in i..self.region_count - 1 {
                    self.regions[j] = self.regions[j + 1];
                    // The low pass's watermark is part of the region, so it
                    // travels with it. Leaving it behind would hand the cursor
                    // of one region to another, and the low pass would then
                    // start from the wrong address — serving a page twice, or
                    // skipping one forever.
                    self.low_cursor[j] = self.low_cursor[j + 1];
                }
                self.region_count -= 1;
                continue;
            }
            // Overlap at start
            if base <= r.base {
                self.regions[i] = MemRegion {
                    base: end,
                    length: r_end - end,
                };
                i += 1;
                continue;
            }
            // Overlap at end
            if end >= r_end {
                self.regions[i] = MemRegion {
                    base: r.base,
                    length: base - r.base,
                };
                i += 1;
                continue;
            }
            // Split: kernel region in the middle
            self.regions[i] = MemRegion {
                base: r.base,
                length: base - r.base,
            };
            if self.region_count < MAX_REGIONS {
                // The served part of a below-floor region is [r.base, cursor).
                // The low half keeps up to `base`; whatever the cursor had
                // reached past that belongs to the tail, so the tail inherits
                // `max(cursor, end)` rather than starting over at its base —
                // starting over would serve the tail's already-served pages a
                // second time.
                let carried = core::cmp::max(self.low_cursor[i], end);
                let mut j = self.region_count;
                while j > i + 1 {
                    self.regions[j] = self.regions[j - 1];
                    self.low_cursor[j] = self.low_cursor[j - 1];
                    j -= 1;
                }
                self.regions[i + 1] = MemRegion {
                    base: end,
                    length: r_end - end,
                };
                self.low_cursor[i + 1] = carried;
                self.region_count += 1;
            }
            i += 1;
        }
        // Fix next_free if it landed in the reserved range
        if self.next_free >= base && self.next_free < end {
            self.next_free = end;
        }
    }
}

unsafe impl FrameAllocatorTrait<Size4KiB> for FrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.alloc_frame()
            .map(PhysFrame::containing_address)
    }
}

/// Reserve the physical pages that contain the kernel's boot stack (provided by
/// the bootloader, not tracked in the memory map). This must be called after
/// `global_init()` and before any frame allocator user that might get a stack page.
///
/// The kernel's initialization path (PCI, ATA, network, VFS, namespaces, etc.)
/// uses a substantial amount of stack space. After init, the shell runs directly
/// on the boot stack with interrupts enabled. When the PIT timer ISR fires
/// during `sti; hlt; cli` in the shell's readline loop, ~1-2 KiB of additional
/// stack is consumed by the ISR and its callees (schedule_tick, flush_output,
/// pit::tick, etc.). A 512 KiB reservation ensures the ISR never overflows the
/// stack and corrupts the shell's local variables.
pub fn reserve_boot_stack(hhdm_offset: u64) {
    let rsp: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nostack, preserves_flags));
    }
    let rsp_phys = rsp.wrapping_sub(hhdm_offset);
    // Round down to page boundary, then reserve 512 KiB (128 pages) below current RSP
    let stack_page_base = (rsp_phys - 524288) & !0xFFF;
    let mut fa = FRAME_ALLOCATOR.lock();
    fa.reserve_region(stack_page_base, 524288 + PAGE_SIZE as u64);
}

/// Excluded from the frame allocator so it can never hand out a page the kernel
/// is still using. `__kernel_start`/`__kernel_end` come from `linker.ld` and
/// cover `.text`, `.rodata`, `.data` **and** `.bss`.
extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

/// Reserve the kernel's own image.
///
/// Nothing in the tree reserved it. Limine knows where it loaded the kernel and
/// leaves those pages out of the *usable* map, but that is the bootloader's
/// bookkeeping: `.bss` in particular is not in the file Limine read (it is
/// zeroed at load time), so any static living there depends entirely on
/// Limine having reserved the right range.
///
/// The failure mode is quiet and severe: a frame handed out that overlaps a
/// static makes every later write through it land somewhere else. `make test`
/// died exactly that way — a write to `0xffffffff8054a9e8`, about a kilobyte past
/// `console::error::ERR_BUF`, with the boot stack unreadable at the same time.
pub fn reserve_kernel_image() {
    let start = unsafe { core::ptr::addr_of!(__kernel_start) as u64 } & !PAGE_SIZE_U64;
    let end = unsafe { core::ptr::addr_of!(__kernel_end) as u64 };
    if end <= start {
        return;
    }
    FRAME_ALLOCATOR.lock().reserve_region(start, end - start);
}

const PAGE_SIZE_U64: u64 = 4096;

/// How much memory below the boot stack to keep out of the allocator.
///
/// 512 KiB. The kernel's initialisation path uses a substantial amount of stack,
/// and after init the shell runs directly on the boot stack with interrupts on:
/// a timer tick during its readline loop costs another kilobyte or two. The
/// margin is not free memory, it is headroom.
pub const BOOT_STACK_RESERVE: u64 = 512 * 1024;

/// What `global_init` needs to know about the machine, as plain numbers.
///
/// Everything `global_init` uses to decide the region list arrives here as
/// values, so the decision itself is a pure function and can be tested on the
/// host. Previously it read `rsp` and the linker symbols inline, which meant no
/// test could call it — and the allocator's cursors are seeded there.
#[derive(Clone, Copy)]
pub struct AllocPlanInput<'a> {
    /// The bootloader's memory map, passed through unchanged.
    ///
    /// Borrowed, not owned: `global_init`'s callers build the map on the stack,
    /// and a `'static` slice would push that onto the one stack the allocator is
    /// trying to protect.
    pub memory_map: &'a [MemoryRegion],
    /// Physical address of the kernel image, already page aligned.
    pub kernel_image_base: u64,
    /// End of the kernel image.
    pub kernel_image_end: u64,
    /// Physical address of the base of the boot stack.
    pub boot_stack_phys: u64,
}

/// Decide the allocatable regions, and the high-water mark to bump from.
///
/// Pure with respect to the machine: no `rsp`, no linker symbols, no reads of
/// global allocator state. Everything it consults arrives in `input`, so a host
/// test can call it.
///
/// It *writes* `fa` rather than returning one. `FrameAllocator` embeds a 128 KiB
/// free stack, so returning it by value would put a 128 KiB temporary on
/// whatever stack the caller is using — on bare metal, the boot stack, which is
/// the single stack everything else is trying to keep clear. That is the same
/// trap as the 270 KiB block cache in BUG-031, reached from the other
/// direction.
///
/// The order is load-bearing and is why this is a function rather than a comment:
///
/// 1. usable regions in, so there is something to subtract from;
/// 2. every non-usable map entry, since the allocator must not hand out MMIO,
///    ACPI, reserved or bootloader pages;
/// 3. the kernel image, which is not in the map as a non-usable entry — the
///    bootloader leaves it out entirely, so nothing else would exclude it;
/// 4. the boot stack, for the same reason.
///
/// Steps 3 and 4 have to happen *here* rather than as `reserve_region` calls
/// afterwards: a reservation only shrinks the region list, and the free stack is
/// filled from those regions further down. A frame already queued when the
/// reservation lands is still handed out later, so reserving after the fact
/// protects nothing. `reserve_boot_stack` exists for the *public* entry point
/// and documents the same ordering hazard.
pub fn plan_regions(fa: &mut FrameAllocator, input: &AllocPlanInput<'_>) {
    fa.reset();

    for entry in input.memory_map {
        if entry.is_usable() && entry.length > 0 {
            let idx = fa.region_count;
            if idx < MAX_REGIONS {
                fa.regions[idx] = MemRegion {
                    base: entry.base,
                    length: entry.length,
                };
                fa.region_count = idx + 1;
            }
        }
    }

    for entry in input.memory_map {
        if entry.kind != 0 && entry.length > 0 {
            fa.reserve_region(entry.base, entry.length);
        }
    }

    if input.kernel_image_end > input.kernel_image_base {
        fa.reserve_region(
            input.kernel_image_base,
            input.kernel_image_end - input.kernel_image_base,
        );
    }

    // `0` means the caller has no boot stack to protect — a host test building a
    // synthetic allocator. Reserving `[0, 512 KiB)` there would silently eat the
    // low test regions, so it is skipped rather than guessed at.
    if input.boot_stack_phys != 0 {
        let stack_base =
            (input.boot_stack_phys.saturating_sub(BOOT_STACK_RESERVE)) & !PAGE_SIZE_U64;
        fa.reserve_region(stack_base, BOOT_STACK_RESERVE + PAGE_SIZE_U64);
    }
    // Only what is left is usable, so that is what `total_memory` counts.
    let mut total = 0u64;
    for r in fa.regions.iter() {
        total += r.length;
    }
    fa.total_memory = total;

    // Where the bump pointer starts. The 16 MiB line is deliberate: the first
    // 16 MiB holds the kernel image, the boot stack and the low BIOS data, and
    // none of it may be handed out. `kernel_end` then pushes the line further
    // up if the image reaches past it.
    fa.next_free = if fa.region_count() > 0 {
        let base = fa.regions[0].base;
        let end = base + fa.regions[0].length;
        let mut start = if base < 0x100_0000 && end > 0x100_0000 {
            0x100_0000
        } else {
            base
        };
        for entry in input.memory_map {
            if entry.kind == 6 {
                let candidate = entry.base + entry.length;
                if candidate > start {
                    start = candidate;
                }
            }
        }
        start
    } else {
        0
    };

    // The floor is the seeded bump pointer: it is the boundary between the two
    // passes. The high pass starts here and only moves up; the low pass serves
    // what is underneath, and must never cross it, because everything at and
    // above it belongs to the high pass.
    fa.floor = fa.next_free;
}

/// Initialise the global frame allocator from the bootloader's memory map.
///
/// Reads `rsp` and the linker symbols, so it cannot be called from a host test —
/// everything it decides is in [`plan_regions`], which can.
pub fn global_init(memory_map: &[MemoryRegion], hhdm_offset: u64) {
    let rsp_phys: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp_phys, options(nostack, preserves_flags));
    }
    let input = AllocPlanInput {
        memory_map,
        kernel_image_base: unsafe { core::ptr::addr_of!(__kernel_start) as u64 } & !PAGE_SIZE_U64,
        kernel_image_end: unsafe { core::ptr::addr_of!(__kernel_end) as u64 },
        boot_stack_phys: rsp_phys.wrapping_sub(hhdm_offset),
    };

    let mut fa = FRAME_ALLOCATOR.lock();
    plan_regions(&mut fa, &input);
}

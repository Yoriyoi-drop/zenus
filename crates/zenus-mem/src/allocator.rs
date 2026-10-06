use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use zenus_sync::spinlock::SpinLock;

static HEAP_LOCK: SpinLock<()> = SpinLock::new(());

/// Only the first free-list fault is reported in full. After that the allocator
/// is walking on broken data and every later report is the same fact; a loop of
/// them buries the dump that actually identifies the corruption.
static FAULT_REPORTED: AtomicBool = AtomicBool::new(false);

/// The chain `alloc_mut` was walking when it last split a block: (addr, size,
/// region_end) for each candidate it examined. `alloc_mut` cannot print on the
/// happy path (the formatter allocates), so it records here and
/// `report_list_fault` prints it. Static, so recording allocates nothing.
static SPLIT_TRACE: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

fn trace_slot(i: usize) -> &'static AtomicU64 {
    &SPLIT_TRACE[i]
}

const HEAP_SIZE: usize = 1024 * 1024 * 8; // 8 MB
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

const HEADER_SIZE: usize = core::mem::size_of::<BlockHeader>();
const MIN_BLOCK: usize = 32;
const MAGIC_FREE: u64 = 0x46524545_424C4F43;
const MAGIC_USED: u64 = 0x55534544_424C4F43;

/// Every block header sits **immediately before** its payload, i.e. a
/// payload pointer always satisfies `block = ptr - HEADER_SIZE`.
///
/// The previous layout kept the header at the *free region's* start and
/// stored the alignment padding in the 8 bytes just below the payload. That
/// padding word lives inside the header itself whenever `pad < 8`
/// (`pad = aligned_data - data_addr` can be anything in 0..align-1), so the
/// write clobbered the header canary. `dealloc` then failed to recognise
/// the pad-less case, mis-derived `block = ptr - HEADER_SIZE - pad` from the
/// corrupted value and freed an unrelated block — freeing live task stacks
/// so their frames got recycled into heap metadata (observed: `MAGIC_FREE`
/// headers and `0xdeadbeefcafebabe` canaries inside the shell's stack, then
/// jumps to addresses like 0x100000000).
#[repr(C)]
struct BlockHeader {
    magic: u64,
    size: usize,
    next: *mut BlockHeader,
    canary: u64,
}

const CANARY_VALUE: u64 = 0xDEADBEEF_CAFEBABE;

pub struct FreeListAllocator {
    free_head: AtomicUsize,
    initialized: AtomicBool,
}

impl FreeListAllocator {
    pub const fn new() -> Self {
        FreeListAllocator {
            free_head: AtomicUsize::new(0),
            initialized: AtomicBool::new(false),
        }
    }

    fn ensure_initialized(&self) {
        if self.initialized.load(Ordering::Acquire) {
            return;
        }
        #[allow(static_mut_refs)]
        let heap_start = core::ptr::addr_of_mut!(HEAP) as usize;

        let first = heap_start as *mut BlockHeader;
        unsafe {
            ptr::write(
                first,
                BlockHeader {
                    magic: MAGIC_FREE,
                    size: HEAP_SIZE - HEADER_SIZE,
                    next: ptr::null_mut(),
                    canary: CANARY_VALUE,
                },
            );
        }
        self.free_head.store(heap_start as usize, Ordering::Release);
        self.initialized.store(true, Ordering::Release);

        zenus_console::kinfo!("Heap: 8MB free-list allocator ready");
    }

    /// Record what `alloc_mut` decided, for `report_list_fault` to print.
    #[allow(clippy::too_many_arguments)]
    fn record_split_trace(
        &self,
        prev: usize,
        curr: usize,
        aligned_data: usize,
        size: usize,
        block_size: usize,
        region_end: usize,
    ) {
        for (i, v) in [curr as u64, block_size as u64, region_end as u64, prev as u64,
                       aligned_data as u64, size as u64]
            .into_iter()
            .enumerate()
        {
            trace_slot(i).store(v, Ordering::Relaxed);
        }
    }

    /// Verify the free list is a chain of `MAGIC_FREE` blocks, ascending and
    /// inside the arena. Reports the first thing that is wrong.
    ///
    /// This exists because the failure it guards is otherwise silent.
    /// `dealloc` refuses only a pointer whose header has the wrong `magic`, so a
    /// header that is *partly* overwritten — `magic` and `canary` intact, the
    /// fields between them clobbered — walks straight through it and the list
    /// quietly stops describing the heap.
    ///
    /// Three rules for anyone changing this, all learned the hard way while
    /// investigating BUG-034:
    ///
    /// * **Report, do not always print.** Every message goes through the
    ///   formatter, which allocates. A version that printed on the happy path
    ///   replaced a clean free list with a corrupt one and sent the bisect at
    ///   the wrong function entirely.
    /// * **Do not assert invariants that are not invariants.** This check
    ///   originally required `size >= MIN_BLOCK` for every free block. That is
    ///   false: `alloc_mut` only requires the *remainder* of a split to reach
    ///   `MIN_BLOCK`, and the block it hands out keeps the caller's size, so a
    ///   freed 3-byte `String` from `devfs::readdir` is a legal 3-byte free
    ///   block. The check reported a healthy list as broken hundreds of times
    ///   per `run` and buried the real fault under false positives.
    /// * It is called with `HEAP_LOCK` held; do not take the lock again.
    fn debug_check_free_list(&self, where_: &str) {
        // `addr_of!` and not `HEAP.as_ptr()`: this needs only the address, and
        // the `&`-forming borrow of a `static mut` would need an `unsafe` block
        // here for nothing.
        let arena_lo = core::ptr::addr_of!(HEAP) as usize;
        let arena_hi = arena_lo + HEAP_SIZE;
        let mut cur = self.free_head.load(Ordering::Acquire);
        let mut prev = 0usize;
        let mut n = 0usize;
        while cur != 0 {
            if n >= 4096 {
                self.report_list_fault(where_, n, cur, "chain longer than 4096 blocks");
                return;
            }
            if cur < arena_lo || cur + HEADER_SIZE > arena_hi {
                self.report_list_fault(where_, n, cur, "block outside the arena");
                return;
            }
            // The block's *extent* has to fit too, not just its header. This is
            // the check that matters: a header whose `size` has been inflated
            // still has a valid magic and canary and a header that starts inside
            // the arena, so the check above passes, but `alloc_mut` then computes
            // a `region_end` past the end of the heap and hands out payload
            // addresses beyond it. The BUG-034 run showed exactly this:
            // `region_end=0xffffffff81031CF0` against an arena ending at
            // `0xffffffff81030f70`.
            let size = unsafe { (*(cur as *mut BlockHeader)).size };
            if cur + HEADER_SIZE + size > arena_hi {
                self.report_list_fault(where_, n, cur, "block extends past the arena");
                return;
            }
            if cur <= prev {
                self.report_list_fault(where_, n, cur, "chain is not ascending");
                return;
            }
            let b = cur as *mut BlockHeader;
            let (magic, canary, next) =
                unsafe { ((*b).magic, (*b).canary, (*b).next as usize) };
            if magic != MAGIC_FREE || canary != CANARY_VALUE {
                self.report_list_fault(where_, n, cur, "bad magic or canary");
                return;
            }
            prev = cur;
            cur = next;
            n += 1;
        }
    }

    /// One line per fault, plus a raw dump of the neighbourhood on the *first*
    /// one only.
    ///
    /// The dump is the point. A list that reads as nonsense is either a corrupted
    /// header or the allocator walking into some other structure entirely, and
    /// the bytes settle that immediately — the BUG-034 dump showed `COLBERF`
    /// (`MAGIC_FREE` backwards) interleaved with `dev` and `tmp`, which is what
    /// finally showed the arena was full of legitimately tiny free blocks and
    /// that the fault was elsewhere. Repeating the dump would bury it.
    fn report_list_fault(&self, where_: &str, index: usize, addr: usize, why: &str) {
        if FAULT_REPORTED.swap(true, Ordering::AcqRel) {
            return;
        }
        zenus_console::kerror_code!(
            zenus_console::error::codes::MEM_PROTECTION,
            "free list broken at block {index} ({addr:#x}) during {where_}: {why}"
        );
        // 24 qwords from 64 bytes before the header onwards: enough to see the
        // neighbouring headers and whatever pattern sits in the payloads.
        let start = addr.saturating_sub(64);
        let s = zenus_console::serial::SerialPort::new(0x3F8);
        s.write_str("\n[HEAP] raw qwords around the bad block:\n");
        for i in 0..24usize {
            let a = start + i * 8;
            let v: u64 = unsafe { core::ptr::read_volatile(a as *const u64) };
            s.write_str(if i % 4 == 0 { "\n  " } else { " " });
            s.write_hex(a as u64);
            s.write_str(": ");
            s.write_hex(v);
        }
        s.write_str("\n[HEAP] last alloc_mut split: block={");
        s.write_hex(trace_slot(0).load(Ordering::Relaxed));
        s.write_str(" size=");
        s.write_hex(trace_slot(1).load(Ordering::Relaxed));
        s.write_str(" region_end=");
        s.write_hex(trace_slot(2).load(Ordering::Relaxed));
        s.write_str(" prev=");
        s.write_hex(trace_slot(3).load(Ordering::Relaxed));
        s.write_str(" aligned_data=");
        s.write_hex(trace_slot(4).load(Ordering::Relaxed));
        s.write_str(" req=");
        s.write_hex(trace_slot(5).load(Ordering::Relaxed));
        s.write_str("\n[HEAP] chain from head:");
        let mut c = self.free_head.load(Ordering::Acquire);
        let mut k = 0usize;
        while c != 0 && k < 12 {
            s.write_str(" [");
            s.write_hex(c as u64);
            s.write_str(" sz=");
            s.write_hex(unsafe { (*(c as *mut BlockHeader)).size as u64 });
            s.write_str("]");
            c = unsafe { (*(c as *mut BlockHeader)).next as usize };
            k += 1;
        }
        s.write_str("\n[HEAP] end dump\n");
        zenus_console::serial::flush_output_blocking();
    }

    fn alloc_mut(&self, layout: Layout) -> *mut u8 {
        let _lock = HEAP_LOCK.lock();
        self.ensure_initialized();
        self.debug_check_free_list("alloc");

        let size = layout.size().max(1);
        // 16 is the hard minimum: task stacks are context-switched with
        // iretq-compatible frames, and the SysV ABI keeps RSP 16-aligned.
        let align = layout.align().max(16);

        let mut prev: usize = 0;
        let mut curr = self.free_head.load(Ordering::Acquire);

        while curr != 0 {
            let block = curr as *mut BlockHeader;


            unsafe {
                let block_size = (*block).size;
                let region_start = curr + HEADER_SIZE;
                let region_end = curr + HEADER_SIZE + block_size;
                let aligned_data = (region_start + align - 1) & !(align - 1);

                if aligned_data + size <= region_end {
                    // Payload goes at `aligned_data`, header immediately
                    // before it — no padding bookkeeping needed.
                    let used_hdr = (aligned_data - HEADER_SIZE) as *mut BlockHeader;
                    // Bytes consumed from the *region start* (used payload +
                    // its header + alignment gap), and what is left over.
                    let consumed = (aligned_data + size - curr) as isize;
                    let remaining = block_size as isize - consumed;

                    if remaining >= (HEADER_SIZE + MIN_BLOCK) as isize {
                        let new_block = (aligned_data + size) as *mut BlockHeader;
                        ptr::write(
                            new_block,
                            BlockHeader {
                                magic: MAGIC_FREE,
                                size: (remaining as usize) - HEADER_SIZE,
                                next: (*block).next,
                                canary: CANARY_VALUE,
                            },
                        );

                        if prev == 0 {
                            self.free_head.store(new_block as usize, Ordering::Release);
                        } else {
                            (*(prev as *mut BlockHeader)).next = new_block;
                        }
                    } else {
                        if prev == 0 {
                            self.free_head
                                .store((*block).next as usize, Ordering::Release);
                        } else {
                            (*(prev as *mut BlockHeader)).next = (*block).next;
                        }
                    }

                    // `size` is the payload size: dealloc derives
                    // block_end = used_hdr + HEADER + size == aligned_data + size.
                    ptr::write(
                        used_hdr,
                        BlockHeader {
                            magic: MAGIC_USED,
                            size,
                            next: ptr::null_mut(),
                            canary: CANARY_VALUE,
                        },
                    );

                    self.record_split_trace(prev, curr, aligned_data, size, block_size, region_end);
                    return aligned_data as *mut u8;
                }

                prev = curr;
                curr = (*block).next as usize;
            }
        }
        zenus_console::kerror_code!(
            zenus_console::error::codes::MEM_ALLOC_FAILED,
            "Heap exhausted! free_head={:#x}, size={}",
            self.free_head.load(Ordering::Relaxed),
            size
        );
        ptr::null_mut()
    }

    fn dealloc_mut(&self, ptr: *mut u8, _layout: Layout) {
        let _lock = HEAP_LOCK.lock();
        self.ensure_initialized();
        self.debug_check_free_list("dealloc");

        if ptr.is_null() {
            return;
        }

        // Header is immediately below the payload — no padding arithmetic.
        let block = (ptr as usize - HEADER_SIZE) as *mut BlockHeader;

        let block_size;
        let block_start;
        let block_end;
        unsafe {
            if (*block).magic == MAGIC_FREE {
                return; // double free
            }
            if (*block).magic != MAGIC_USED || (*block).canary != CANARY_VALUE {
                // Name the shape of the failure, because "corrupted" does not
                // say which of the three it is and they have different fixes:
                //
                //   * the pointer is inside the arena but no block starts there
                //     -> the pointer is stale or wild, or the block was merged
                //     into a neighbour (a double free the MAGIC_FREE guard
                //     cannot catch, since the old header is now interior);
                //   * the pointer is outside the arena entirely
                //     -> something that never came from this allocator.
                //
                // Finding the enclosing free block also gives its size, which
                // is how a merged double free becomes obvious: the reported
                // offset into the block is the size of the original block.
                let arena_lo = HEAP.as_ptr() as usize;
                let arena_hi = arena_lo + HEAP_SIZE;
                let p = ptr as usize;
                let mut enclosing = 0usize;
                let mut inside_arena = p >= arena_lo && p < arena_hi;
                if inside_arena {
                    let mut cur = self.free_head.load(Ordering::Acquire);
                    while cur != 0 {
                        let b = cur as *mut BlockHeader;
                        let sz = unsafe { (*b).size };
                        if p >= cur && p < cur + HEADER_SIZE + sz {
                            enclosing = cur;
                            break;
                        }
                        cur = unsafe { (*b).next } as usize;
                    }
                }
                zenus_console::kerror_code!(
                    zenus_console::error::codes::MEM_PROTECTION,
                    "Heap header corrupted at {:#x} (magic={:#x} canary={:#x}) ptr={:#x} \
                     arena=[{:#x},{:#x}) in_arena={} enclosing_free_block={:#x} offset={:#x}",
                    block as usize,
                    (*block).magic,
                    (*block).canary,
                    p,
                    arena_lo,
                    arena_hi,
                    inside_arena,
                    enclosing,
                    if enclosing != 0 { p - enclosing } else { 0 }
                );
                zenus_console::kerror_code!(
                    zenus_console::error::codes::MEM_PROTECTION,
                    "free list (head={:#x}):",
                    self.free_head.load(Ordering::Relaxed)
                );
                {
                    let mut cur2 = self.free_head.load(Ordering::Acquire);
                    let mut n = 0;
                    while cur2 != 0 && n < 12 {
                        let b2 = cur2 as *mut BlockHeader;
                        zenus_console::kerror_code!(
                            zenus_console::error::codes::MEM_PROTECTION,
                            "  [{n}] at {:#x} size={:#x} magic={:#x} next={:#x}",
                            cur2,
                            unsafe { (*b2).size },
                            unsafe { (*b2).magic },
                            unsafe { (*b2).next } as usize
                        );
                        let nx = unsafe { (*b2).next } as usize;
                        if nx <= cur2 {
                            zenus_console::kerror_code!(
                                zenus_console::error::codes::MEM_PROTECTION,
                                "  free list is NOT ascending at {:#x} -> {:#x}",
                                cur2,
                                nx
                            );
                            break;
                        }
                        cur2 = nx;
                        n += 1;
                    }
                }
                return;
            }
            block_size = (*block).size;
            (*block).magic = MAGIC_FREE;
            block_start = block as usize;
            block_end = block_start + HEADER_SIZE + block_size;
        }


        let mut prev: usize = 0;
        let mut curr = self.free_head.load(Ordering::Acquire);

        unsafe {
            while curr != 0 {
                if curr > block_start {
                    break;
                }
                prev = curr;
                curr = (*(curr as *mut BlockHeader)).next as usize;
            }
        }

        let prev_end = if prev != 0 {
            unsafe { prev + HEADER_SIZE + (*(prev as *mut BlockHeader)).size }
        } else {
            0
        };

        let coalesce_prev = prev != 0 && prev_end == block_start;
        let coalesce_next = curr != 0 && block_end == curr;

        unsafe {
            if coalesce_prev && coalesce_next {
                let p = prev as *mut BlockHeader;
                let n = curr as *mut BlockHeader;
                (*p).size += HEADER_SIZE + block_size + HEADER_SIZE + (*n).size;
                (*p).next = (*n).next;
            } else if coalesce_prev {
                let p = prev as *mut BlockHeader;
                (*p).size += HEADER_SIZE + block_size;
            } else if coalesce_next {
                let n = curr as *mut BlockHeader;
                (*block).size += HEADER_SIZE + (*n).size;
                (*block).next = (*n).next;
                if prev == 0 {
                    self.free_head.store(block as usize, Ordering::Release);
                } else {
                    (*(prev as *mut BlockHeader)).next = block;
                }
            } else {
                (*block).next = curr as *mut BlockHeader;
                if prev == 0 {
                    self.free_head.store(block as usize, Ordering::Release);
                } else {
                    (*(prev as *mut BlockHeader)).next = block;
                }
            }
        }
    }
}

unsafe impl Sync for FreeListAllocator {}

impl FreeListAllocator {
    pub fn free_head_addr(&self) -> usize {
        self.free_head.load(Ordering::Relaxed)
    }

    pub fn total_size(&self) -> usize {
        HEAP_SIZE
    }

    pub fn free_size(&self) -> usize {
        let _lock = HEAP_LOCK.lock();
        self.ensure_initialized();
        let mut total = 0usize;
        let mut curr = self.free_head.load(Ordering::Acquire);
        while curr != 0 {
            unsafe {
                let block = curr as *mut BlockHeader;
                if (*block).magic == MAGIC_FREE {
                    total += (*block).size;
                }
                curr = (*block).next as usize;
            }
        }
        total
    }
}

unsafe impl GlobalAlloc for FreeListAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.alloc_mut(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.dealloc_mut(ptr, layout)
    }
}

/// The kernel's free-list allocator.
///
/// Installed as `#[global_allocator]` on bare metal only. The host test build
/// keeps the static (explicit kernel allocations still go through it, e.g.
/// task stacks in `zenus-sched`) but must not let the *test harness* allocate
/// from it: the harness allocates before the arena is initialised, and the
/// free list is built lazily from the 8 MiB static heap. That combination
/// wedged the host test binary before this was gated — `cargo test -p zenus-mem`
/// hung with no output at all, even for `--list`.
#[cfg_attr(target_os = "none", global_allocator)]
pub static ALLOCATOR: FreeListAllocator = FreeListAllocator::new();

/// Check the free list from outside the allocator. Reports only on failure, so
/// it does not perturb what it measures (see `debug_check_free_list`).
pub fn check_free_list(tag: &str) {
    let _lock = HEAP_LOCK.lock();
    ALLOCATOR.ensure_initialized();
    ALLOCATOR.debug_check_free_list(tag);
}

pub fn init_heap() {
    ALLOCATOR.ensure_initialized();
}
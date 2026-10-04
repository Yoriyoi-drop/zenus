use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use zenus_sync::spinlock::SpinLock;

static HEAP_LOCK: SpinLock<()> = SpinLock::new(());

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

    fn alloc_mut(&self, layout: Layout) -> *mut u8 {
        let _lock = HEAP_LOCK.lock();
        self.ensure_initialized();

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
                zenus_console::kerror_code!(
                    zenus_console::error::codes::MEM_PROTECTION,
                    "Heap header corrupted at {:#x} (magic={:#x} canary={:#x}) ptr={:#x}",
                    block as usize,
                    (*block).magic,
                    (*block).canary,
                    ptr as usize
                );
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

pub fn init_heap() {
    ALLOCATOR.ensure_initialized();
}
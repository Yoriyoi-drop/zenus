use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering, AtomicBool};
use zenus_console::serial::SerialPort;
use zenus_sync::spinlock::SpinLock;

static HEAP_LOCK: SpinLock<()> = SpinLock::new(());

const HEAP_SIZE: usize = 1024 * 1024 * 64;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

const HEADER_SIZE: usize = core::mem::size_of::<BlockHeader>();
const MIN_BLOCK: usize = 32;
const MAGIC_FREE: u64 = 0x46524545_424C4F43;
const MAGIC_USED: u64 = 0x55534544_424C4F43;

#[repr(C)]
struct BlockHeader {
    magic: u64,
    size: usize,
    next: *mut BlockHeader,
}

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
            ptr::write(first, BlockHeader {
                magic: MAGIC_FREE,
                size: HEAP_SIZE - HEADER_SIZE,
                next: ptr::null_mut(),
            });
        }
        self.free_head.store(heap_start as usize, Ordering::Release);
        self.initialized.store(true, Ordering::Release);

        let s = SerialPort::new(0x3F8);
        s.write_str("[OK] Heap: 64MB free-list allocator ready\n");
    }

    fn alloc_mut(&self, layout: Layout) -> *mut u8 {
        let _lock = HEAP_LOCK.lock();
        self.ensure_initialized();

        let size = layout.size().max(1);
        let align = layout.align().max(HEADER_SIZE);

        let mut prev: usize = 0;
        let mut curr = self.free_head.load(Ordering::Acquire);

        while curr != 0 {
            let block = curr as *mut BlockHeader;
            unsafe {
                let block_size = (*block).size;
                let data_addr = curr + HEADER_SIZE;
                let aligned_data = (data_addr + align - 1) & !(align - 1);
                let pad = aligned_data - data_addr;
                let needed = pad + size;

                if needed <= block_size {
                    let pad_ptr = (aligned_data as *mut usize).sub(1);
                    *pad_ptr = pad;

                    let remaining = block_size - needed;
                    if remaining >= HEADER_SIZE + MIN_BLOCK {
                        let new_block = (data_addr + needed) as *mut BlockHeader;
                        ptr::write(new_block, BlockHeader {
                            magic: MAGIC_FREE,
                            size: remaining - HEADER_SIZE,
                            next: (*block).next,
                        });

                        if prev == 0 {
                            self.free_head.store(new_block as usize, Ordering::Release);
                        } else {
                            (*(prev as *mut BlockHeader)).next = new_block;
                        }

                        (*block).magic = MAGIC_USED;
                        (*block).size = needed;

                        return aligned_data as *mut u8;
                    } else {
                        if prev == 0 {
                            self.free_head.store((*block).next as usize, Ordering::Release);
                        } else {
                            (*(prev as *mut BlockHeader)).next = (*block).next;
                        }

                        (*block).magic = MAGIC_USED;
                        (*block).size = block_size;

                        return aligned_data as *mut u8;
                    }
                }

                prev = curr;
                curr = (*block).next as usize;
            }
        }

        {
            use core::fmt::Write;
            let mut s = SerialPort::new(0x3F8);
            let _ = write!(s, "[OOM] Heap exhausted! free_head=0x{:x}, size_requested={}\n",
                self.free_head.load(Ordering::Relaxed), size);
        }
        ptr::null_mut()
    }

    fn dealloc_mut(&self, ptr: *mut u8, _layout: Layout) {
        let _lock = HEAP_LOCK.lock();
        self.ensure_initialized();

        if ptr.is_null() { return; }

        let pad = unsafe { *((ptr as *const usize).sub(1)) as usize };
        let block = (ptr as usize - HEADER_SIZE - pad) as *mut BlockHeader;

        unsafe {
            if (*block).magic == MAGIC_FREE {
                return;
            }
        }

        let block_size;
        let block_start;
        let block_end;
        unsafe {
            block_size = (*block).size;
            (*block).magic = MAGIC_FREE;
            block_start = block as usize;
            block_end = block_start + HEADER_SIZE + block_size;
        }

        let mut prev: usize = 0;
        let mut curr = self.free_head.load(Ordering::Acquire);

        unsafe {
            while curr != 0 {
                if curr > block_start { break; }
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

#[global_allocator]
pub static ALLOCATOR: FreeListAllocator = FreeListAllocator::new();

pub fn init_heap() {
    ALLOCATOR.ensure_initialized();
}

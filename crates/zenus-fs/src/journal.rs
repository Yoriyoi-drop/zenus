use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use crate::block_cache::{bc_flush, bc_invalidate, bc_read, bc_write};

/// Device-level write barrier, installed by the platform layer.
///
/// `bc_flush()` only drains *our* block cache into the device driver; on a
/// volatile-backed device (virtio-blk, and any host that reorders writes) that
/// is not durability. Before the journal marks a transaction COMMITTED it must
/// know the data blocks reached stable storage, otherwise a crash can leave a
/// committed journal pointing at data that was never written.
///
/// Stored as a `usize` function pointer in an atomic so the hook can be
/// registered after this module is initialised, without a lock on the commit
/// path.
static DEVICE_FLUSH: AtomicUsize = AtomicUsize::new(0);

/// Register the platform's write-barrier implementation.
pub fn set_device_flush(f: fn() -> bool) {
    DEVICE_FLUSH.store(f as usize, Ordering::Release);
}

/// Issue the device write barrier, if one was registered.
pub fn device_flush() -> bool {
    let f = DEVICE_FLUSH.load(Ordering::Acquire);
    if f == 0 {
        return true;
    }
    // SAFETY: `f` was published by `set_device_flush` as a plain `fn() -> bool`
    // and the module lives for the rest of the kernel's life.
    let func: fn() -> bool = unsafe { core::mem::transmute(f) };
    func()
}

const JOURNAL_MAGIC: u32 = 0x4A524E4C; // "JRNL"
const MAX_ENTRIES: usize = 123;
const JNL_STATE_EMPTY: u32 = 0;
const JNL_STATE_ACTIVE: u32 = 1;
const JNL_STATE_COMMITTED: u32 = 2;

const MAX_TARGET_BLOCKS: usize = 123;

#[repr(C, packed)]
struct JournalHeader {
    magic: u32,
    sequence: u32,
    num_entries: u32,
    state: u32,
    reserved: u32,
    targets: [u32; MAX_TARGET_BLOCKS],
}

/// Journal state.
///
/// These were five `static mut` words touched from task context and from the
/// block layer with no synchronisation at all. Each is a single machine word,
/// so making them atomics removes the data race (and any chance of a torn
/// value) without adding a lock that would have to be ordered against
/// `BLOCK_CACHE` — which every function here already takes.
static JNL_DEV_ID: AtomicU8 = AtomicU8::new(0xFF);
static JNL_START_BLOCK: AtomicU64 = AtomicU64::new(0);
static JNL_NUM_BLOCKS: AtomicU64 = AtomicU64::new(0);
static JNL_SEQUENCE: AtomicU32 = AtomicU32::new(0);
static JNL_ACTIVE: AtomicBool = AtomicBool::new(false);

/// How many redo entries a journal of `num_blocks` blocks can hold.
///
/// The first block is the header, so the remaining `num_blocks - 1` are data
/// blocks. `MAX_ENTRIES` is only the size of the in-memory `targets[]` array —
/// it is *not* how much room the device gave us. Bounding entries by the
/// constant alone meant a journal initialised with 16 blocks still accepted 123
/// entries and entry #16 wrote its redo image to `start_block + 17`, which is a
/// live ext2 block rather than journal space.
pub fn capacity_for_blocks(num_blocks: u64) -> usize {
    core::cmp::min(MAX_ENTRIES, num_blocks.saturating_sub(1) as usize)
}

/// Redo block for entry `idx`, or `None` if the journal is only `capacity`
/// blocks long. Pure so the bound is testable without a device.
pub fn data_block_for(start_block: u64, num_blocks: u64, idx: usize) -> Option<u64> {
    if idx >= capacity_for_blocks(num_blocks) {
        return None;
    }
    Some(start_block + 1 + idx as u64)
}

/// Number of entries a header claiming `num_entries` may actually replay.
///
/// The on-disk field is attacker-reachable (it is read back from a device that
/// may have been tampered with) and it indexes `targets[]`, so it has to be
/// clamped to both the array bound and the device's real journal size before
/// it is used as a loop bound.
pub fn replay_entry_limit(num_entries: u32, num_blocks: u64) -> usize {
    core::cmp::min(
        num_entries as usize,
        core::cmp::min(MAX_ENTRIES, capacity_for_blocks(num_blocks)),
    )
}

pub fn journal_init(dev_id: u8, start_block: u64, num_blocks: u64) -> bool {
    JNL_DEV_ID.store(dev_id, Ordering::Release);
    JNL_START_BLOCK.store(start_block, Ordering::Release);
    JNL_NUM_BLOCKS.store(num_blocks, Ordering::Release);
    JNL_SEQUENCE.store(0, Ordering::Release);
    JNL_ACTIVE.store(false, Ordering::Release);
    let hdr = JournalHeader {
        magic: JOURNAL_MAGIC,
        sequence: 0,
        num_entries: 0,
        state: JNL_STATE_EMPTY,
        reserved: 0,
        targets: [0; MAX_ENTRIES],
    };
    let raw = unsafe {
        core::slice::from_raw_parts(
            &hdr as *const JournalHeader as *const u8,
            core::mem::size_of::<JournalHeader>(),
        )
    };
    // Through the cache, not straight to the device: a direct
    // `block_device_write` left the *previous* image of this sector cached, and
    // the next `read_header()` (i.e. every `journal_begin`) then wrote that
    // stale header — including the old `num_entries`/`targets[]` — back over
    // the fresh one.
    let ok = bc_write(dev_id, start_block, raw) && bc_flush();
    // Either way, make sure no stale line survives.
    bc_invalidate(dev_id, start_block);
    ok
}

pub fn journal_begin() -> bool {
    // `swap` so only one caller can win the transaction, and so a nested
    // `journal_begin` fails instead of nesting.
    if JNL_DEV_ID.load(Ordering::Acquire) == 0xFF || JNL_ACTIVE.swap(true, Ordering::AcqRel) {
        return false;
    }
    JNL_SEQUENCE.fetch_add(1, Ordering::Relaxed);

    // Mark the header ACTIVE and make it durable *before* the caller starts
    // writing data blocks. If the header kept saying EMPTY/committed, a crash
    // mid-transaction would leave orphan data blocks that `journal_replay`
    // cannot attribute to anything, and `JNL_STATE_ACTIVE` was never actually
    // written anywhere.
    let mut hdr = match read_header() {
        Some(h) => h,
        None => {
            JNL_ACTIVE.store(false, Ordering::Release);
            return false;
        }
    };
    hdr.state = JNL_STATE_ACTIVE;
    if !write_header(&hdr) || !bc_flush() {
        JNL_ACTIVE.store(false, Ordering::Release);
        return false;
    }
    true
}

pub fn is_journal_active() -> bool {
    JNL_ACTIVE.load(Ordering::Acquire)
}

pub fn journal_write(target_block: u64, data: &[u8]) -> bool {
    if !is_journal_active() || JNL_DEV_ID.load(Ordering::Acquire) == 0xFF {
        return false;
    }
    let start_block = JNL_START_BLOCK.load(Ordering::Acquire);
    let num_blocks = JNL_NUM_BLOCKS.load(Ordering::Acquire);

    let hdr = read_header();
    let mut hdr = match hdr {
        Some(h) => h,
        None => return false,
    };

    let idx = hdr.num_entries as usize;
    // Bound by the device's journal size, not just by the size of `targets[]`.
    // See `capacity_for_blocks`: with a 16-block journal the old check let
    // entry #16 write its redo image outside the journal.
    let data_block = match data_block_for(start_block, num_blocks, idx) {
        Some(b) => b,
        None => return false,
    };

    let mut sector_buf = [0u8; 512];
    let copy_len = core::cmp::min(data.len(), 512);
    sector_buf[..copy_len].copy_from_slice(&data[..copy_len]);

    let dev_id = JNL_DEV_ID.load(Ordering::Acquire);
    if !bc_write(dev_id, data_block, &sector_buf) {
        return false;
    }

    if target_block > u32::MAX as u64 {
        return false;
    }
    hdr.targets[idx] = target_block as u32;
    hdr.num_entries += 1;
    if !write_header(&hdr) {
        return false;
    }
    bc_flush();
    true
}

pub fn journal_commit() -> bool {
    if !is_journal_active() || JNL_DEV_ID.load(Ordering::Acquire) == 0xFF {
        return false;
    }

    let hdr = read_header();
    let hdr = match hdr {
        Some(h) => h,
        None => return false,
    };

    // Phase 1: Write journal data blocks to their targets
    let start_block = JNL_START_BLOCK.load(Ordering::Acquire);
    let num_blocks = JNL_NUM_BLOCKS.load(Ordering::Acquire);
    let max_commit_entries = replay_entry_limit(hdr.num_entries, num_blocks);
    for i in 0..max_commit_entries {
        let target = hdr.targets[i] as u64;
        if target == 0 {
            continue;
        }
        let mut data = [0u8; 512];
        let data_block = match data_block_for(start_block, num_blocks, i) {
            Some(b) => b,
            None => return false,
        };
        if !bc_read(JNL_DEV_ID.load(Ordering::Acquire), data_block, &mut data) {
            return false;
        }
        if !bc_write(JNL_DEV_ID.load(Ordering::Acquire), target, &data) {
            return false;
        }
    }
    bc_flush();

    // Phase 2: Mark committed in header — but only after the device says the
    // data blocks are durable. Skipping the barrier here is what lets a crash
    // produce a COMMITTED journal whose data was never written.
    if !device_flush() {
        return false;
    }

    match read_header() {
        Some(mut h) => {
            h.state = JNL_STATE_COMMITTED;
            if !write_header(&h) {
                return false;
            }
        }
        None => return false,
    };
    bc_flush();

    // Phase 3: Clear journal (after commit is durable)
    if !device_flush() {
        return false;
    }
    let hdr = read_header();
    let hdr = match hdr {
        Some(mut h) => {
            h.num_entries = 0;
            h.state = JNL_STATE_EMPTY;
            h
        }
        None => return false,
    };
    write_header(&hdr);
    bc_flush();

    JNL_ACTIVE.store(false, Ordering::Release);
    true
}

/// Replay a committed transaction left on `dev_id` at `start_block`.
///
/// `num_blocks` is the journal's size on the device and is *not* recorded in the
/// header, so the caller has to supply it — the same geometry it passed to
/// [`journal_init`]. Passing `MAX_ENTRIES` here instead would let a tampered
/// `num_entries` field walk off the end of the journal.
pub fn journal_replay(dev_id: u8, start_block: u64, num_blocks: u64) -> bool {
    let mut buf = [0u8; 512];
    if !bc_read(dev_id, start_block, &mut buf) {
        return false;
    }

    let magic = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if magic != JOURNAL_MAGIC {
        return false;
    }

    let state = u32::from_ne_bytes([buf[12], buf[13], buf[14], buf[15]]);
    if state != JNL_STATE_COMMITTED {
        return true;
    }

    let num_entries = u32::from_ne_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let _sequence = u32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]);

    // Clamp to both `targets[]` and the device's real journal size: the field
    // comes off the disk and is used as a loop bound, so an oversized value
    // would index past the array and read data blocks that are not ours.
    let limit = replay_entry_limit(num_entries, num_blocks);
    if limit == 0 {
        return true;
    }

    for i in 0..limit {
        let off = 20 + i * 4;
        let target = u32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        if target == 0 {
            continue;
        }

        let mut data = [0u8; 512];
        let data_block = match data_block_for(start_block, num_blocks, i) {
            Some(b) => b,
            None => break,
        };
        if !bc_read(dev_id, data_block, &mut data) {
            continue;
        }

        let _ = bc_write(dev_id, target as u64, &data);
    }

    // Push the replayed blocks out before the header is retired below. Without
    // this the redo data can sit dirty in the block cache while the device-side
    // header is already marked EMPTY — a crash in that window loses the
    // transaction with no journal left to replay it.
    bc_flush();

    buf[12..16].copy_from_slice(&JNL_STATE_EMPTY.to_ne_bytes());
    // Same reasoning as `journal_init`: go through the cache and flush, so the
    // retired header is durable *and* no stale line is left behind.
    let _ = bc_write(dev_id, start_block, &buf);
    bc_flush();
    bc_invalidate(dev_id, start_block);

    JNL_DEV_ID.store(dev_id, Ordering::Release);
    JNL_START_BLOCK.store(start_block, Ordering::Release);
    JNL_NUM_BLOCKS.store(num_blocks, Ordering::Release);
    JNL_SEQUENCE.store(0, Ordering::Release);
    JNL_ACTIVE.store(false, Ordering::Release);

    true
}

fn read_header() -> Option<JournalHeader> {
    let mut buf = [0u8; 512];
    if !bc_read(
        JNL_DEV_ID.load(Ordering::Acquire),
        JNL_START_BLOCK.load(Ordering::Acquire),
        &mut buf,
    ) {
        return None;
    }
    let magic = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if magic != JOURNAL_MAGIC {
        return None;
    }
    let hdr = JournalHeader {
        magic,
        sequence: u32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]),
        num_entries: u32::from_ne_bytes([buf[8], buf[9], buf[10], buf[11]]),
        state: u32::from_ne_bytes([buf[12], buf[13], buf[14], buf[15]]),
        reserved: u32::from_ne_bytes([buf[16], buf[17], buf[18], buf[19]]),
        targets: core::array::from_fn(|i| {
            let off = 20 + i * 4;
            if off + 4 <= buf.len() {
                u32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
            } else {
                0
            }
        }),
    };
    Some(hdr)
}

fn write_header(hdr: &JournalHeader) -> bool {
    let mut buf = [0u8; 512];
    buf[0..4].copy_from_slice(&hdr.magic.to_ne_bytes());
    buf[4..8].copy_from_slice(&hdr.sequence.to_ne_bytes());
    buf[8..12].copy_from_slice(&hdr.num_entries.to_ne_bytes());
    buf[12..16].copy_from_slice(&hdr.state.to_ne_bytes());
    buf[16..20].copy_from_slice(&hdr.reserved.to_ne_bytes());
    for i in 0..MAX_ENTRIES {
        let off = 20 + i * 4;
        if off + 4 > buf.len() {
            break;
        }
        buf[off..off + 4].copy_from_slice(&hdr.targets[i].to_ne_bytes());
    }
    bc_write(
        JNL_DEV_ID.load(Ordering::Acquire),
        JNL_START_BLOCK.load(Ordering::Acquire),
        &buf,
    )
}

use crate::block_cache::{bc_read, bc_write, bc_flush};
use crate::devfs::block_device_write;

const JOURNAL_MAGIC: u32 = 0x4A524E4C; // "JRNL"
const MAX_ENTRIES: usize = 123;
const JNL_STATE_EMPTY: u32 = 0;
const JNL_STATE_ACTIVE: u32 = 1;
const JNL_STATE_COMMITTED: u32 = 2;

#[repr(C, packed)]
struct JournalHeader {
    magic: u32,
    sequence: u32,
    num_entries: u32,
    state: u32,
    reserved: u32,
    targets: [u32; MAX_ENTRIES],
}

static mut JNL_DEV_ID: u8 = 0xFF;
static mut JNL_START_BLOCK: u64 = 0;
static mut JNL_NUM_BLOCKS: u64 = 0;
static mut JNL_SEQUENCE: u32 = 0;
static mut JNL_ACTIVE: bool = false;

pub fn journal_init(dev_id: u8, start_block: u64, num_blocks: u64) -> bool {
    unsafe {
        JNL_DEV_ID = dev_id;
        JNL_START_BLOCK = start_block;
        JNL_NUM_BLOCKS = num_blocks;
        JNL_SEQUENCE = 0;
        JNL_ACTIVE = false;
    }
    let hdr = JournalHeader {
        magic: JOURNAL_MAGIC,
        sequence: 0,
        num_entries: 0,
        state: JNL_STATE_EMPTY,
        reserved: 0,
        targets: [0; MAX_ENTRIES],
    };
    let raw = unsafe {
        core::slice::from_raw_parts(&hdr as *const JournalHeader as *const u8, core::mem::size_of::<JournalHeader>())
    };
    block_device_write(dev_id as usize, start_block, raw)
}

pub fn journal_begin() -> bool {
    unsafe {
        if JNL_DEV_ID == 0xFF || JNL_ACTIVE {
            return false;
        }
        JNL_ACTIVE = true;
        JNL_SEQUENCE += 1;
        true
    }
}

pub fn is_journal_active() -> bool {
    unsafe { JNL_ACTIVE }
}

pub fn journal_write(target_block: u64, data: &[u8]) -> bool {
    unsafe {
        if !JNL_ACTIVE || JNL_DEV_ID == 0xFF {
            return false;
        }
    }
    let hdr = read_header();
    let mut hdr = match hdr {
        Some(h) => h,
        None => return false,
    };

    if hdr.num_entries as usize >= MAX_ENTRIES {
        return false;
    }
    let idx = hdr.num_entries as usize;

    let mut sector_buf = [0u8; 512];
    let copy_len = core::cmp::min(data.len(), 512);
    sector_buf[..copy_len].copy_from_slice(&data[..copy_len]);

    let max_data_block = unsafe { JNL_START_BLOCK + 1 + MAX_ENTRIES as u64 - 1 };
    let data_block = unsafe { JNL_START_BLOCK + 1 + idx as u64 };
    if data_block > max_data_block { return false; }
    if !bc_write(unsafe { JNL_DEV_ID }, data_block, &sector_buf) {
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
    unsafe {
        if !JNL_ACTIVE || JNL_DEV_ID == 0xFF {
            return false;
        }
    }

    let hdr = read_header();
    let hdr = match hdr {
        Some(mut h) => {
            h.state = JNL_STATE_COMMITTED;
            if !write_header(&h) {
                return false;
            }
            h
        }
        None => return false,
    };

    let max_commit_entries = core::cmp::min(hdr.num_entries as usize, MAX_ENTRIES);
    for i in 0..max_commit_entries {
        let target = hdr.targets[i] as u64;
        if target == 0 {
            continue;
        }
        let mut data = [0u8; 512];
        let max_data_block = unsafe { JNL_START_BLOCK + 1 + MAX_ENTRIES as u64 - 1 };
        let data_block = unsafe { JNL_START_BLOCK + 1 + i as u64 };
        if data_block > max_data_block { return false; }
        if !bc_read(unsafe { JNL_DEV_ID }, data_block, &mut data) {
            return false;
        }
        if !bc_write(unsafe { JNL_DEV_ID }, target, &data) {
            return false;
        }
    }
    bc_flush();

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

    unsafe { JNL_ACTIVE = false; }
    true
}

pub fn journal_replay(dev_id: u8, start_block: u64) -> bool {
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

    if num_entries == 0 || num_entries as usize > MAX_ENTRIES {
        return true;
    }

    for i in 0..num_entries as usize {
        let off = 20 + i * 4;
        let target = u32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        if target == 0 {
            continue;
        }

        let mut data = [0u8; 512];
        let data_block = start_block + 1 + i as u64;
        if !bc_read(dev_id, data_block, &mut data) {
            continue;
        }

        let _ = bc_write(dev_id, target as u64, &data);
    }

    buf[12..16].copy_from_slice(&JNL_STATE_EMPTY.to_ne_bytes());
    let _ = block_device_write(dev_id as usize, start_block, &buf);

    unsafe {
        JNL_DEV_ID = dev_id;
        JNL_START_BLOCK = start_block;
        JNL_NUM_BLOCKS = 0;
        JNL_SEQUENCE = 0;
        JNL_ACTIVE = false;
    }

    true
}

fn read_header() -> Option<JournalHeader> {
    let mut buf = [0u8; 512];
    if !bc_read(unsafe { JNL_DEV_ID }, unsafe { JNL_START_BLOCK }, &mut buf) {
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
    bc_write(unsafe { JNL_DEV_ID }, unsafe { JNL_START_BLOCK }, &buf)
}

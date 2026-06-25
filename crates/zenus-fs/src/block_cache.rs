use crate::devfs::{block_device_read, block_device_write};
use zenus_sync::spinlock::SpinLock;

const CACHE_SIZE: usize = 128;
const SECTOR_SIZE: usize = 512;

fn hash(dev_id: u8, block: u64) -> usize {
    let h = (dev_id as u64).wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(block);
    (h ^ (h >> 16) ^ (h >> 32) ^ (h >> 48)) as usize & (CACHE_SIZE - 1)
}

#[derive(Clone, Copy)]
struct CacheEntry {
    dev_id: u8,
    block: u64,
    dirty: bool,
    valid: bool,
    data: [u8; SECTOR_SIZE],
}

pub struct BlockCache {
    entries: [CacheEntry; CACHE_SIZE],
    hits: u64,
    misses: u64,
}

impl BlockCache {
    const fn new() -> Self {
        BlockCache {
            entries: [CacheEntry {
                dev_id: 0,
                block: 0,
                dirty: false,
                valid: false,
                data: [0; SECTOR_SIZE],
            }; CACHE_SIZE],
            hits: 0,
            misses: 0,
        }
    }

    fn find_entry(&self, dev_id: u8, block: u64) -> Option<usize> {
        let start = hash(dev_id, block);
        for i in 0..4 {
            let idx = (start + i) & (CACHE_SIZE - 1);
            if self.entries[idx].valid && self.entries[idx].dev_id == dev_id && self.entries[idx].block == block {
                return Some(idx);
            }
        }
        None
    }

    fn evict_one(&mut self, dev_id: u8, block: u64) -> Option<usize> {
        let start = hash(dev_id, block);
        for i in 0..4 {
            let idx = (start + i) & (CACHE_SIZE - 1);
            if !self.entries[idx].valid {
                return Some(idx);
            }
        }
        Some(start)
    }

    fn flush_entry(&mut self, idx: usize) {
        if self.entries[idx].dirty {
            block_device_write(
                self.entries[idx].dev_id as usize,
                self.entries[idx].block,
                &self.entries[idx].data,
            );
            self.entries[idx].dirty = false;
        }
    }

    pub fn read_block(&mut self, dev_id: u8, block: u64, buf: &mut [u8]) -> bool {
        if let Some(idx) = self.find_entry(dev_id, block) {
            self.hits += 1;
            let len = buf.len().min(SECTOR_SIZE);
            buf[..len].copy_from_slice(&self.entries[idx].data[..len]);
            return true;
        }

        self.misses += 1;
        let idx = match self.evict_one(dev_id, block) {
            Some(i) => i,
            None => return false,
        };

        self.flush_entry(idx);

        let mut sector_buf = [0u8; SECTOR_SIZE];
        if !block_device_read(dev_id as usize, block, &mut sector_buf) {
            return false;
        }

        self.entries[idx].dev_id = dev_id;
        self.entries[idx].block = block;
        self.entries[idx].dirty = false;
        self.entries[idx].valid = true;
        self.entries[idx].data = sector_buf;

        let len = buf.len().min(SECTOR_SIZE);
        buf[..len].copy_from_slice(&self.entries[idx].data[..len]);
        true
    }

    pub fn write_block(&mut self, dev_id: u8, block: u64, buf: &[u8]) -> bool {
        let idx = match self.find_entry(dev_id, block) {
            Some(i) => i,
            None => {
                let idx = match self.evict_one(dev_id, block) {
                    Some(i) => i,
                    None => return false,
                };
                self.flush_entry(idx);
                if buf.len() < SECTOR_SIZE {
                    let mut sector_buf = [0u8; SECTOR_SIZE];
                    block_device_read(dev_id as usize, block, &mut sector_buf);
                    self.entries[idx].data = sector_buf;
                } else {
                    self.entries[idx].data = [0; SECTOR_SIZE];
                }
                self.entries[idx].dev_id = dev_id;
                self.entries[idx].block = block;
                self.entries[idx].valid = true;
                self.entries[idx].dirty = false;
                idx
            }
        };

        let len = buf.len().min(SECTOR_SIZE);
        self.entries[idx].data[..len].copy_from_slice(&buf[..len]);
        self.entries[idx].dirty = true;
        true
    }

    pub fn flush_all(&mut self) {
        for i in 0..CACHE_SIZE {
            if self.entries[i].valid && self.entries[i].dirty {
                block_device_write(
                    self.entries[i].dev_id as usize,
                    self.entries[i].block,
                    &self.entries[i].data,
                );
                self.entries[i].dirty = false;
            }
        }
    }

    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }
}

pub static BLOCK_CACHE: SpinLock<BlockCache> = SpinLock::new(BlockCache::new());

pub fn bc_read(dev_id: u8, block: u64, buf: &mut [u8]) -> bool {
    BLOCK_CACHE.lock().read_block(dev_id, block, buf)
}

pub fn bc_write(dev_id: u8, block: u64, buf: &[u8]) -> bool {
    BLOCK_CACHE.lock().write_block(dev_id, block, buf)
}

pub fn bc_flush() {
    BLOCK_CACHE.lock().flush_all();
}

pub fn bc_stats() -> (u64, u64) {
    BLOCK_CACHE.lock().stats()
}

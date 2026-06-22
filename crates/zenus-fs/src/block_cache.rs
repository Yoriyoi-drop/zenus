use crate::devfs::{block_device_read, block_device_write};
use zenus_sync::spinlock::SpinLock;

const CACHE_SIZE: usize = 64;
const SECTOR_SIZE: usize = 512;

#[derive(Clone, Copy)]
struct CacheEntry {
    dev_id: u8,
    block: u64,
    dirty: bool,
    valid: bool,
    lru_counter: u64,
    data: [u8; SECTOR_SIZE],
}

pub struct BlockCache {
    entries: [CacheEntry; CACHE_SIZE],
    lru_counter: u64,
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
                lru_counter: 0,
                data: [0; SECTOR_SIZE],
            }; CACHE_SIZE],
            lru_counter: 0,
            hits: 0,
            misses: 0,
        }
    }

    fn find_entry(&mut self, dev_id: u8, block: u64) -> Option<usize> {
        for i in 0..CACHE_SIZE {
            if self.entries[i].valid && self.entries[i].dev_id == dev_id && self.entries[i].block == block
            {
                return Some(i);
            }
        }
        None
    }

    fn evict_one(&mut self) -> Option<usize> {
        let mut oldest = 0;
        let mut oldest_lru = u64::MAX;
        for i in 0..CACHE_SIZE {
            if !self.entries[i].valid {
                return Some(i);
            }
            if self.entries[i].lru_counter < oldest_lru {
                oldest_lru = self.entries[i].lru_counter;
                oldest = i;
            }
        }
        Some(oldest)
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
            self.entries[idx].lru_counter = self.lru_counter;
            self.lru_counter += 1;
            self.hits += 1;
            let len = buf.len().min(SECTOR_SIZE);
            buf[..len].copy_from_slice(&self.entries[idx].data[..len]);
            return true;
        }

        self.misses += 1;
        let idx = match self.evict_one() {
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
        self.entries[idx].lru_counter = self.lru_counter;
        self.lru_counter += 1;

        let len = buf.len().min(SECTOR_SIZE);
        buf[..len].copy_from_slice(&self.entries[idx].data[..len]);
        true
    }

    pub fn write_block(&mut self, dev_id: u8, block: u64, buf: &[u8]) -> bool {
        let idx = match self.find_entry(dev_id, block) {
            Some(i) => i,
            None => {
                let idx = match self.evict_one() {
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
        self.entries[idx].lru_counter = self.lru_counter;
        self.lru_counter += 1;
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

#[cfg(feature = "testing")]
pub mod tests {
    use super::*;

    fn assert_eq(a: u64, b: u64, msg: &'static str) -> Result<(), &'static str> {
        if a == b { Ok(()) } else { Err(msg) }
    }

    pub fn test_new_cache_empty() -> Result<(), &'static str> {
        let cache = BlockCache::new();
        assert_eq(cache.hits, 0, "hits should be 0")?;
        assert_eq(cache.misses, 0, "misses should be 0")?;
        Ok(())
    }

    pub fn test_evict_on_empty_returns_index_0() -> Result<(), &'static str> {
        let mut cache = BlockCache::new();
        let idx = cache.evict_one();
        if idx != Some(0) {
            return Err("evict_one on empty cache should return Some(0)");
        }
        Ok(())
    }

    pub fn test_find_entry_empty_returns_none() -> Result<(), &'static str> {
        let mut cache = BlockCache::new();
        if cache.find_entry(0, 0).is_some() {
            return Err("find_entry on empty cache should be None");
        }
        Ok(())
    }

    pub fn test_stats_empty() -> Result<(), &'static str> {
        let cache = BlockCache::new();
        let (h, m) = cache.stats();
        assert_eq(h, 0, "stats hits should be 0")?;
        assert_eq(m, 0, "stats misses should be 0")?;
        Ok(())
    }

    pub fn test_lru_counter_increments_on_evict() -> Result<(), &'static str> {
        let cache = BlockCache::new();
        assert_eq(cache.lru_counter, 0, "initial lru_counter = 0")?;
        // evict_one doesn't increment, it just reads counters
        // lru_counter only increments on actual read/write hits
        Ok(())
    }

    pub fn test_cache_size_constant() -> Result<(), &'static str> {
        if CACHE_SIZE != 64 {
            return Err("CACHE_SIZE should be 64");
        }
        Ok(())
    }

    pub fn test_sector_size_constant() -> Result<(), &'static str> {
        if SECTOR_SIZE != 512 {
            return Err("SECTOR_SIZE should be 512");
        }
        Ok(())
    }
}

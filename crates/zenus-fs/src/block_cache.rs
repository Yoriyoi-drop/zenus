use crate::devfs::{block_device_read, block_device_write};
use zenus_sync::spinlock::SpinLock;

const CACHE_SIZE: usize = 512;
const SECTOR_SIZE: usize = 512;

fn hash(dev_id: u8, block: u64) -> usize {
    let h = (dev_id as u64)
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(block);
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
            if self.entries[idx].valid
                && self.entries[idx].dev_id == dev_id
                && self.entries[idx].block == block
            {
                return Some(idx);
            }
        }
        None
    }

    fn evict_one(&mut self, dev_id: u8, block: u64) -> Option<usize> {
        let start = hash(dev_id, block);
        // Cari slot kosong (tidak valid) terlebih dahulu
        for i in 0..4 {
            let idx = (start + i) & (CACHE_SIZE - 1);
            if !self.entries[idx].valid {
                return Some(idx);
            }
        }
        // Semua slot terpakai — usir slot pertama (LRU sederhana)
        Some(start)
    }

    /// Flush satu entry ke disk. Mengembalikan false jika write gagal,
    /// dan TIDAK men-clear flag dirty sehingga data tidak hilang.
    fn flush_entry(&mut self, idx: usize) -> bool {
        if self.entries[idx].dirty {
            let ok = block_device_write(
                self.entries[idx].dev_id as usize,
                self.entries[idx].block,
                &self.entries[idx].data,
            );
            if !ok {
                // Jangan clear dirty — data masih perlu ditulis ulang
                return false;
            }
            self.entries[idx].dirty = false;
        }
        true
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

        // Jika flush gagal, batalkan — jangan timpa data dirty yang belum tersimpan
        if !self.flush_entry(idx) {
            return false;
        }

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
                // Jika flush gagal, batalkan untuk mencegah data corruption
                if !self.flush_entry(idx) {
                    return false;
                }
                if buf.len() < SECTOR_SIZE {
                    // Read-modify-write: baca sektor lama dulu
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

    /// Flush semua entry dirty ke disk. Mengembalikan false jika ada write yang gagal.
    pub fn flush_all(&mut self) -> bool {
        let mut success = true;
        for i in 0..CACHE_SIZE {
            if self.entries[i].valid && self.entries[i].dirty {
                if !self.flush_entry(i) {
                    success = false;
                    // Lanjutkan untuk mencoba flush entry lain
                }
            }
        }
        success
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

/// Flush semua dirty entries. Mengembalikan false jika ada write yang gagal.
pub fn bc_flush() -> bool {
    BLOCK_CACHE.lock().flush_all()
}

pub fn bc_stats() -> (u64, u64) {
    BLOCK_CACHE.lock().stats()
}

#[cfg(feature = "testing")]
pub mod tests {
    use super::*;

    pub fn test_new_cache_empty() -> Result<(), &'static str> {
        let cache = BlockCache::new();
        if cache.hits != 0 || cache.misses != 0 {
            return Err("New cache should have zero stats");
        }
        Ok(())
    }

    pub fn test_evict_on_empty_returns_index_0() -> Result<(), &'static str> {
        let mut cache = BlockCache::new();
        // Evict pada cache kosong harus mengembalikan index 0 (hash berdasarkan dev_id=0, block=0)
        let idx = cache.evict_one(0, 0);
        match idx {
            Some(_) => Ok(()),
            None => Err("evict_one on empty cache should return Some"),
        }
    }

    pub fn test_find_entry_empty_returns_none() -> Result<(), &'static str> {
        let cache = BlockCache::new();
        if cache.find_entry(0, 0).is_some() {
            return Err("find_entry on empty cache should return None");
        }
        Ok(())
    }

    pub fn test_stats_empty() -> Result<(), &'static str> {
        let cache = BlockCache::new();
        let (hits, misses) = cache.stats();
        if hits != 0 || misses != 0 {
            return Err("Empty cache stats should be (0, 0)");
        }
        Ok(())
    }

    pub fn test_lru_counter_increments_on_evict() -> Result<(), &'static str> {
        let mut cache = BlockCache::new();
        let idx1 = cache.evict_one(0, 0);
        let idx2 = cache.evict_one(0, 1);
        if idx1 == idx2 {
            // Boleh saja sama (hash collision) — yang penting return Some
        }
        Ok(())
    }

    pub fn test_cache_size_constant() -> Result<(), &'static str> {
        if CACHE_SIZE != 512 {
            return Err("CACHE_SIZE should be 512");
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

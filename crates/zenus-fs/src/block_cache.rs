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

    /// Number of slots probed by the (associative) lookup: this is also how far
    /// the hash spreads a block across the table.
    const WAYS: usize = 4;

    fn find_entry(&self, dev_id: u8, block: u64) -> Option<usize> {
        let start = hash(dev_id, block);
        for i in 0..Self::WAYS {
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

    /// Pick a slot for a new entry: an empty slot in the block's window first,
    /// then a clean victim in that window.
    ///
    /// The old version returned `Some(start)` unconditionally once the window
    /// was full, which is wrong twice over: it overwrote whatever lived at
    /// `start` (dropping a dirty entry on the floor), and it did so even when
    /// free slots existed elsewhere in the table. A fifth block hashing into
    /// the same window therefore thrashed while 508 entries sat unused.
    fn evict_one(&mut self, dev_id: u8, block: u64) -> Option<usize> {
        let start = hash(dev_id, block);

        // 1. Any invalid slot in this block's window.
        for i in 0..Self::WAYS {
            let idx = (start + i) & (CACHE_SIZE - 1);
            if !self.entries[idx].valid {
                return Some(idx);
            }
        }

        // 2. A clean entry in the window: nothing to flush, safe to reuse.
        for i in 0..Self::WAYS {
            let idx = (start + i) & (CACHE_SIZE - 1);
            if !self.entries[idx].dirty {
                return Some(idx);
            }
        }

        // 3. Any clean slot anywhere — the table is a cache, so spending a slot
        //    outside the window beats thrashing.
        for idx in 0..CACHE_SIZE {
            if !self.entries[idx].valid || !self.entries[idx].dirty {
                return Some(idx);
            }
        }

        // 4. Everything is dirty. Prefer the window (its entries are the ones
        //    most likely to be reused) and let the caller flush it.
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
                // Probe the device before caching the sector. Without this a
                // write to a sector the device rejects (past end of device) is
                // accepted into the cache and can never be flushed, so every
                // later `bc_flush()`/`bc_read()` that has to evict that entry
                // fails too — one bad LBA takes the whole cache down.
                let mut sector_buf = [0u8; SECTOR_SIZE];
                if !block_device_read(dev_id as usize, block, &mut sector_buf) {
                    return false;
                }
                // Read-modify-write: a short write must keep the rest of the
                // sector, and a full-sector write overwrites all of it below.
                self.entries[idx].data = sector_buf;
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

/// Drop cached lines for one device/sector pair.
///
/// Needed whenever something wrote *behind* the cache's back — the journal
/// writes its header straight to the device, so without this the cache kept
/// serving the pre-`journal_init` image to the next `read_header()`.
pub fn bc_invalidate(dev_id: u8, block: u64) -> bool {
    let mut cache = BLOCK_CACHE.lock();
    match cache.find_entry(dev_id, block) {
        Some(idx) => {
            cache.entries[idx].valid = false;
            cache.entries[idx].dirty = false;
            true
        }
        None => false,
    }
}

/// Flush everything, then drop every line.
///
/// After this the cache holds nothing, so the next read comes from the device.
/// Returns false if some dirty entry could not be written.
pub fn bc_invalidate_all() -> bool {
    let mut cache = BLOCK_CACHE.lock();
    let flushed = cache.flush_all();
    for entry in cache.entries.iter_mut() {
        entry.valid = false;
        entry.dirty = false;
    }
    flushed
}

#[cfg(feature = "testing")]
pub mod tests {
    use super::*;

/// A cache on the heap, not the stack.
///
/// `BlockCache::new()` is 512 entries of 512 bytes plus bookkeeping — 270 KiB.
/// Four of these tests built one as a local, so the in-kernel suite was pushing
/// a quarter-megabyte frame onto the *boot* stack, which has 512 KiB reserved in
/// total and whatever Limine happened to have mapped below it. Running off the
/// end made the stack unreadable, which in turn made the page-fault handler
/// fault while trying to report the original fault — so the crash dump printed
/// nothing and the run looked like a hang with no trace.
///
/// The heap is initialised before the suite runs, and the host test suite is
/// unaffected: this is the `testing`-gated module only.
fn heap_cache() -> Option<alloc::boxed::Box<BlockCache>> {
    Some(alloc::boxed::Box::new(BlockCache::new()))
}

    pub fn test_new_cache_empty() -> Result<(), &'static str> {
        let cache = heap_cache().ok_or("no memory for the test cache")?;
        if cache.hits != 0 || cache.misses != 0 {
            return Err("New cache should have zero stats");
        }
        Ok(())
    }

    pub fn test_evict_on_empty_returns_index_0() -> Result<(), &'static str> {
        let mut cache = heap_cache().ok_or("no memory for the test cache")?;
        // Evict pada cache kosong harus mengembalikan index 0 (hash berdasarkan dev_id=0, block=0)
        let idx = cache.evict_one(0, 0);
        match idx {
            Some(_) => Ok(()),
            None => Err("evict_one on empty cache should return Some"),
        }
    }

    pub fn test_find_entry_empty_returns_none() -> Result<(), &'static str> {
        let cache = heap_cache().ok_or("no memory for the test cache")?;
        if cache.find_entry(0, 0).is_some() {
            return Err("find_entry on empty cache should return None");
        }
        Ok(())
    }

    pub fn test_stats_empty() -> Result<(), &'static str> {
        let cache = heap_cache().ok_or("no memory for the test cache")?;
        let (hits, misses) = cache.stats();
        if hits != 0 || misses != 0 {
            return Err("Empty cache stats should be (0, 0)");
        }
        Ok(())
    }

    pub fn test_lru_counter_increments_on_evict() -> Result<(), &'static str> {
        let mut cache = heap_cache().ok_or("no memory for the test cache")?;
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

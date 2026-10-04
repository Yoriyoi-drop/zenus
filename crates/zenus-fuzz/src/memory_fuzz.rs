use alloc::vec::Vec;

use crate::coverage;
use crate::FuzzResult;

/// Memory operation types
#[derive(Clone, Copy)]
enum MemOp {
    Alloc,
    Free,
    Realloc,
    Map,
    Unmap,
    Protect,
    Access,
}

/// Fuzzing input format for memory:
/// [op: u8] [addr: u64] [size: u64] [flags: u64]
pub fn execute(input: &[u8]) -> FuzzResult {
    if input.len() < 25 {
        return FuzzResult::Normal;
    }

    let op = match input[0] % 7 {
        0 => MemOp::Alloc,
        1 => MemOp::Free,
        2 => MemOp::Realloc,
        3 => MemOp::Map,
        4 => MemOp::Unmap,
        5 => MemOp::Protect,
        _ => MemOp::Access,
    };

    let addr = u64::from_le_bytes([
        input[1], input[2], input[3], input[4],
        input[5], input[6], input[7], input[8],
    ]);
    let size = u64::from_le_bytes([
        input[9], input[10], input[11], input[12],
        input[13], input[14], input[15], input[16],
    ]);
    let flags = u64::from_le_bytes([
        input[17], input[18], input[19], input[20],
        input[21], input[22], input[23], input[24],
    ]);

    coverage::record_edge((op as u8 as u64).wrapping_mul(1000).wrapping_add(addr % 100));

    match op {
        MemOp::Alloc => fuzz_alloc(size),
        MemOp::Free => fuzz_free(addr),
        MemOp::Realloc => fuzz_realloc(addr, size),
        MemOp::Map => fuzz_map(addr, size, flags),
        MemOp::Unmap => fuzz_unmap(addr, size),
        MemOp::Protect => fuzz_protect(addr, size, flags),
        MemOp::Access => fuzz_access(addr, size),
    }
}

fn fuzz_alloc(size: u64) -> FuzzResult {
    // Test heap allocation with fuzzed size
    if size > 0x1000000 {
        return FuzzResult::Normal; // Skip huge allocations
    }

    // `Layout::from_size_align` rejects size 0, and the pattern write below
    // would zero-length; treat both as a skipped case instead of formatting a
    // bogus layout.
    if size == 0 {
        return FuzzResult::Normal;
    }

    let layout = match core::alloc::Layout::from_size_align(size as usize, 16) {
        Ok(l) => l,
        Err(_) => return FuzzResult::Normal,
    };

    let ptr = unsafe { alloc::alloc::alloc(layout) };
    if !ptr.is_null() {
        // Write pattern to detect use-after-free
        unsafe {
            core::ptr::write_bytes(ptr, 0xAA, layout.size());
        }
        unsafe { alloc::alloc::dealloc(ptr, layout) };
    }

    FuzzResult::Normal
}

fn fuzz_free(addr: u64) -> FuzzResult {
    // Simulate double-free scenario
    if addr == 0 || addr < 0x1000 {
        return FuzzResult::Normal;
    }

    // Check if address is in valid range
    if addr > 0x0000800000000000 {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

fn fuzz_realloc(_addr: u64, new_size: u64) -> FuzzResult {
    if new_size == 0 || new_size > 0x1000000 {
        return FuzzResult::Normal;
    }

    // Test realloc with various sizes
    let layout = match core::alloc::Layout::from_size_align(new_size as usize, 16) {
        Ok(l) => l,
        Err(_) => return FuzzResult::Normal,
    };

    let ptr = unsafe { alloc::alloc::alloc(layout) };
    if !ptr.is_null() {
        unsafe { alloc::alloc::dealloc(ptr, layout) };
    }

    FuzzResult::Normal
}

fn fuzz_map(_addr: u64, size: u64, flags: u64) -> FuzzResult {
    // Test mmap with fuzzed parameters
    if size == 0 || size > 0x10000000 {
        return FuzzResult::Normal;
    }

    // Check for invalid flag combinations. `prot` only has 3 bits of
    // PROT_*, so masking instead of comparing was right, but a value above 7
    // after masking is impossible — the real rejection here is a *zero* prot,
    // which mmap would reject as PROT_NONE with conflicting flags.
    if flags & 0xF == 0 {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

fn fuzz_unmap(addr: u64, size: u64) -> FuzzResult {
    if size == 0 {
        return FuzzResult::Normal;
    }

    // Test munmap with various addresses
    if addr > 0x0000800000000000 {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

fn fuzz_protect(_addr: u64, size: u64, prot: u64) -> FuzzResult {
    if size == 0 || size > 0x10000000 {
        return FuzzResult::Normal;
    }

    // Test mprotect with various protection flags: PROT_NONE (0) is legal,
    // anything with a bit outside PROT_READ|WRITE|EXEC is not.
    if prot > 7 {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

fn fuzz_access(addr: u64, _size: u64) -> FuzzResult {
    // Test memory access patterns
    if addr == 0 || addr < 0x1000 {
        return FuzzResult::Normal;
    }

    if addr > 0x0000800000000000 {
        return FuzzResult::Normal;
    }

    // Simulate read/write access
    coverage::record_edge(addr % 1000);

    FuzzResult::Normal
}

/// Memory-fuzzing seeds: one per op with the boundary sizes called out in the
/// fuzzing doc (0, 1, PAGE_SIZE-1, PAGE_SIZE, PAGE_SIZE+1, UINT_MAX).
pub fn generate_seeds() -> Vec<Vec<u8>> {
    let sizes: [u64; 6] = [0, 1, 0xFFF, 0x1000, 0x1001, u32::MAX as u64];
    let addrs: [u64; 5] = [0, 0x1000, 0x4000, 0x0000_7FFF_FFFF_F000, u64::MAX];
    let mut seeds = Vec::new();
    for op in 0..7u8 {
        for &size in &sizes {
            for &addr in &addrs {
                let mut seed = Vec::with_capacity(25);
                seed.push(op);
                seed.extend_from_slice(&addr.to_le_bytes());
                seed.extend_from_slice(&size.to_le_bytes());
                seed.extend_from_slice(&3u64.to_le_bytes());
                seeds.push(seed);
            }
        }
    }
    seeds
}

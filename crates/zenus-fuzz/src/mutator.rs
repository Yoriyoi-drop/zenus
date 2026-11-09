use alloc::vec::Vec;

/// Mutation strategies for fuzzing
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MutationStrategy {
    BitFlip,
    ByteFlip,
    Arithmetic,
    InterestingValues,
    Dictionary,
    Splice,
    Shrink,
    InsertByte,
    DeleteByte,
    MultipleBitFlip,
}

pub const STRATEGIES: [MutationStrategy; 10] = [
    MutationStrategy::BitFlip,
    MutationStrategy::ByteFlip,
    MutationStrategy::Arithmetic,
    MutationStrategy::InterestingValues,
    MutationStrategy::Dictionary,
    MutationStrategy::Splice,
    MutationStrategy::Shrink,
    MutationStrategy::InsertByte,
    MutationStrategy::DeleteByte,
    MutationStrategy::MultipleBitFlip,
];

const INTERESTING_VALUES: &[u64] = &[
    0,
    1,
    2,
    0x7F,
    0x80,
    0xFF,
    0x100,
    0x7FFF,
    0x8000,
    0xFFFF,
    0x10000,
    0x7FFFFFFF,
    0x80000000,
    0xFFFFFFFF,
    0x100000000,
    0x7FFFFFFFFFFFFFFF,
    0x8000000000000000,
    0xFFFFFFFFFFFFFFFF,
    0x1000,
    0x10000,
    0x100000,
    0x1000000,
    0x10000000,
    4095,
    4096,
    4097,
];

/// Hard cap so a mutator can never grow an input without bound (a corpus
/// entry above `MAX_INPUT_SIZE` is rejected by the corpus anyway, and a
/// runaway Vec in a kernel fuzz loop is a memory-exhaustion bug of its own).
const MAX_MUTATED_SIZE: usize = 8192;

/// Apply one mutation strategy to `input`.
///
/// The strategies work on a `Vec` (not a fixed slice) because `Dictionary` and
/// `Splice` legitimately *grow* the input; the old signature took `&mut [u8]`
/// and silently failed to compile the moment it tried to store the grown
/// buffer back.
pub fn mutate(input: &[u8], strategy: MutationStrategy) -> Vec<u8> {
    let mut output = input.to_vec();
    if output.is_empty() {
        return output;
    }

    match strategy {
        MutationStrategy::BitFlip => bit_flip(&mut output),
        MutationStrategy::ByteFlip => byte_flip(&mut output),
        MutationStrategy::Arithmetic => arithmetic(&mut output),
        MutationStrategy::InterestingValues => interesting_values(&mut output),
        MutationStrategy::Dictionary => dictionary(&mut output),
        MutationStrategy::Splice => splice(&mut output),
        MutationStrategy::Shrink => shrink(&mut output),
        MutationStrategy::InsertByte => insert_byte(&mut output),
        MutationStrategy::DeleteByte => delete_byte(&mut output),
        MutationStrategy::MultipleBitFlip => multiple_bit_flip(&mut output),
    }

    if output.len() > MAX_MUTATED_SIZE {
        output.truncate(MAX_MUTATED_SIZE);
    }
    output
}

fn bit_flip(data: &mut [u8]) {
    if data.is_empty() {
        return;
    }
    let idx = (data[0] as usize) % data.len();
    let bit = data.get(1).copied().unwrap_or(0) % 8;
    data[idx] ^= 1 << bit;
}

fn byte_flip(data: &mut [u8]) {
    if data.is_empty() {
        return;
    }
    let idx = (data[0] as usize) % data.len();
    data[idx] = !data[idx];
}

fn arithmetic(data: &mut [u8]) {
    if data.len() < 2 {
        return;
    }
    let idx = (data[0] as usize) % (data.len() - 1);
    let delta = (data[1] as i8) as i16;
    let val = data[idx] as i16;
    data[idx] = val.wrapping_add(delta) as u8;
}

fn interesting_values(data: &mut [u8]) {
    if data.is_empty() {
        return;
    }
    let idx = (data[0] as usize) % data.len();
    let val_idx = (data.get(1).copied().unwrap_or(0) as usize) % INTERESTING_VALUES.len();
    let val = INTERESTING_VALUES[val_idx];
    let bytes = val.to_le_bytes();
    let n = (data.len() - idx).min(8);
    data[idx..idx + n].copy_from_slice(&bytes[..n]);
}

fn dictionary(data: &mut Vec<u8>) {
    // Insert common syscall / length / boundary patterns.
    static DICT: &[&[u8]] = &[
        &[0, 0, 0, 0],             // read
        &[1, 0, 0, 0],             // write
        &[2, 0, 0, 0],             // open
        &[60, 0, 0, 0],            // exit
        &[9, 0, 0, 0],             // mmap
        &[11, 0, 0, 0],            // munmap
        &[0x00, 0x10, 0, 0],       // page size
        &[0xFF, 0xFF, 0xFF, 0xFF], // -1
        &[0x00, 0x00, 0x00, 0x80], // kernel-space address
        &[0x00, 0x10, 0x00, 0x00], // 0x1000 boundary
    ];

    if data.is_empty() {
        return;
    }
    let dict_idx = (data[0] as usize) % DICT.len();
    let insert_pos = (data.get(1).copied().unwrap_or(0) as usize) % (data.len() + 1);
    let dict_entry = DICT[dict_idx];
    let insert_pos = insert_pos.min(data.len());

    // Insert dictionary entry at position
    let mut new_data = Vec::with_capacity(data.len() + dict_entry.len());
    new_data.extend_from_slice(&data[..insert_pos]);
    new_data.extend_from_slice(dict_entry);
    new_data.extend_from_slice(&data[insert_pos..]);
    *data = new_data;
}

fn splice(data: &mut Vec<u8>) {
    // Splice with a second buffer derived from the input itself: prefix from
    // the input, then a synthetic tail, then the rest of the input.
    if data.len() < 4 {
        return;
    }
    let split = (data[0] as usize) % data.len();
    let other_len =
        ((data[1] as usize) % 64).min(MAX_MUTATED_SIZE - data.len().min(MAX_MUTATED_SIZE));
    let other: Vec<u8> = (0..other_len)
        .map(|i| data.get(i + 2).copied().unwrap_or(0).wrapping_add(i as u8))
        .collect();

    let mut new_data = Vec::with_capacity(data.len() + other.len());
    new_data.extend_from_slice(&data[..split]);
    new_data.extend_from_slice(&other);
    new_data.extend_from_slice(&data[split..]);
    *data = new_data;
}

/// Delete a byte range. The old version used `copy_from_slice` on two slices
/// that could overlap and whose lengths never matched, which is both a panic
/// (`copy_from_slice` asserts equal lengths) and an out-of-bounds copy.
fn shrink(data: &mut Vec<u8>) {
    if data.len() <= 1 {
        return;
    }
    let start = (data[0] as usize) % data.len();
    let remaining = data.len() - start;
    let remove = 1 + ((data[1] as usize) % remaining);
    let end = start + remove;
    data.copy_within(end.., start);
    data.truncate(data.len() - remove);
}

/// Insert a random interesting byte at a random position.
fn insert_byte(data: &mut Vec<u8>) {
    if data.is_empty() {
        return;
    }
    let pos = (data[0] as usize) % (data.len() + 1);
    let byte = data.get(1).copied().unwrap_or(0).wrapping_add(1);
    if data.len() >= MAX_MUTATED_SIZE {
        return;
    }
    data.insert(pos, byte);
}

/// Delete a single byte at a random position.
fn delete_byte(data: &mut Vec<u8>) {
    if data.len() <= 1 {
        return;
    }
    let pos = (data[0] as usize) % data.len();
    data.remove(pos);
}

/// Flip multiple bits at once: up to 4 distinct byte positions.
fn multiple_bit_flip(data: &mut [u8]) {
    if data.is_empty() {
        return;
    }
    let n = 1 + ((data[0] as usize) % 4);
    for i in 0..n {
        let idx = (data.get(i + 1).copied().unwrap_or(0) as usize) % data.len();
        let bit = (data.get(i + 2).copied().unwrap_or(0) % 8) as u8;
        data[idx] ^= 1 << bit;
    }
}

/// Generate a sequence of mutations
pub fn generate_mutations(input: &[u8], count: usize) -> Vec<Vec<u8>> {
    let mut results = Vec::with_capacity(count);
    for i in 0..count {
        let strategy = STRATEGIES[i % STRATEGIES.len()];
        results.push(mutate(input, strategy));
    }
    results
}

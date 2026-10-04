use alloc::vec::Vec;
use zenus_sync::spinlock::SpinLock;

const MAX_CORPUS_SIZE: usize = 4096;
const MAX_INPUT_SIZE: usize = 4096;

static CORPUS: SpinLock<Corpus> = SpinLock::new(Corpus::new());

struct Corpus {
    inputs: [Option<Vec<u8>>; MAX_CORPUS_SIZE],
    size: usize,
    next_idx: usize,
}

impl Corpus {
    const fn new() -> Self {
        Corpus {
            inputs: [const { None }; MAX_CORPUS_SIZE],
            size: 0,
            next_idx: 0,
        }
    }
}

pub fn init() {
    // Resetting `size` alone would orphan every `Vec` already stored in the
    // array (they would only be freed if the slot is overwritten), so drop
    // them explicitly before starting a new campaign.
    clear();
}

/// Fill the corpus with the built-in seeds plus the subsystem seed sets.
pub fn seed_defaults() {
    for seed in get_default_seeds() {
        add_input(seed.to_vec());
    }
    for seed in crate::syscall_fuzz::generate_seeds() {
        add_input(seed);
    }
    for seed in crate::shell_fuzz::generate_seeds() {
        add_input(seed);
    }
    for seed in crate::filesystem_fuzz::generate_seeds() {
        add_input(seed);
    }
    for seed in crate::memory_fuzz::generate_seeds() {
        add_input(seed);
    }
    for seed in crate::scheduler_fuzz::generate_seeds() {
        add_input(seed);
    }
    for seed in crate::device_fuzz::generate_seeds() {
        add_input(seed);
    }
}

pub fn add_input(input: Vec<u8>) {
    if input.is_empty() || input.len() > MAX_INPUT_SIZE {
        return;
    }
    let mut c = CORPUS.lock();
    if c.size < MAX_CORPUS_SIZE {
        // Bind the index first: `c.inputs[c.size] = …` borrows `c`
        // immutably to compute the index and mutably to assign, which the
        // borrow checker rejects.
        let idx = c.size;
        c.inputs[idx] = Some(input);
        c.size = idx + 1;
    } else {
        // Corpus is full: replace a deterministic slot instead of allocating
        // a new one. `next_idx` must never overflow into the unused tail of
        // the array, so the modulo is applied to the live region only.
        let idx = c.next_idx % MAX_CORPUS_SIZE;
        c.inputs[idx] = Some(input);
    }
    c.next_idx = c.next_idx.wrapping_add(1);
    drop(c);
}

pub fn get_next_input() -> Vec<u8> {
    let mut c = CORPUS.lock();
    if c.size == 0 {
        drop(c);
        // Return default seed inputs
        return get_default_seeds()[0].to_vec();
    }
    let idx = c.next_idx % c.size;
    // Clone out before touching `next_idx`: indexing the array borrows `c`
    // immutably, and the counter update needs it mutably.
    let input = c.inputs[idx].clone().unwrap_or_default();
    // Advance exactly once. This used to be incremented twice — once before
    // the clone and once after — so with a corpus of 2 (or any even size) the
    // cursor moved by 2 per call and `idx` stayed 0 forever: the campaign
    // replayed the first input and never tried the rest.
    c.next_idx = c.next_idx.wrapping_add(1);
    drop(c);
    input
}

pub fn get_input(idx: usize) -> Option<Vec<u8>> {
    let c = CORPUS.lock();
    c.inputs.get(idx).and_then(|i| i.clone())
}

pub fn size() -> u64 {
    let c = CORPUS.lock();
    c.size as u64
}

pub fn clear() {
    let mut c = CORPUS.lock();
    for i in 0..MAX_CORPUS_SIZE {
        c.inputs[i] = None;
    }
    c.size = 0;
    c.next_idx = 0;
    drop(c);
}

fn get_default_seeds() -> &'static [&'static [u8]] {
    &[
        &[0, 0, 0, 0, 0, 0, 0, 0],                    // syscall: read(0, 0, 0)
        &[0, 1, 0, 0, 0, 0, 0, 0],                    // syscall: write(1, 0, 0)
        &[0, 2, 0x10, 0, 0, 0, 0, 0],                 // syscall: open
        &[1, 0, 0, 0, 0, 0, 0, 0],                    // memory: alloc(0)
        &[1, 1, 0, 0, 0, 0, 0, 0],                    // memory: free(0)
        &[2, 0, 0, 0, 0, 0, 0, 0],                    // device: keyboard
        &[3, 0, 0, 0, 0, 0, 0, 0],                    // filesystem
        &[4, 0, 0, 0, 0, 0, 0, 0],                    // shell
        &[5, 0, 0, 0, 0, 0, 0, 0],                    // scheduler
    ]
}

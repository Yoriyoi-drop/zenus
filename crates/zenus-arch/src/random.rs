use core::sync::atomic::{AtomicU64, Ordering};

static PRNG_STATE: AtomicU64 = AtomicU64::new(0);

fn prng_next() -> u64 {
    let mut state = PRNG_STATE.load(Ordering::Relaxed);
    loop {
        let next = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        match PRNG_STATE.compare_exchange_weak(state, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next.wrapping_mul(2685821657736338717),
            Err(x) => state = x,
        }
    }
}

pub fn get_random_u64() -> u64 {
    prng_next()
}

pub fn get_random_range(min: u64, max: u64) -> u64 {
    if min >= max {
        return min;
    }
    let range = max - min;
    let val = get_random_u64();
    min + (val % range)
}

pub fn get_random_page_aligned(min: u64, max: u64) -> u64 {
    let val = get_random_range(min, max);
    val & !0xFFF
}

pub fn init_rng() {
    // Gunakan boot_time (dari static, TANPA I/O hardware) + TSC + PIT ticks
    // untuk menghindari pembacaan RTC hardware kedua yang bisa hang.
    let boot_entropy = match crate::rtc::boot_time() {
        Some(t) => {
            (t.year as u64) << 32
                | (t.month as u64) << 24
                | (t.day as u64) << 16
                | (t.hour as u64) << 8
                | (t.minute as u64)
                | (t.second as u64)
        }
        None => 0,
    };
    let ticks = crate::interrupts::pit::get_ticks();
    let mut mixed = boot_entropy
        .wrapping_mul(6364136223846793005)
        .wrapping_add(ticks);
    let cycles = unsafe { core::arch::x86_64::_rdtsc() };
    mixed = mixed.wrapping_mul(2685821657736338717).wrapping_add(cycles);
    PRNG_STATE.store(mixed, Ordering::Relaxed);
    for _ in 0..8 {
        prng_next();
    }
}

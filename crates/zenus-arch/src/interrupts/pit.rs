use core::sync::atomic::{AtomicU64, Ordering};

static TICKS: AtomicU64 = AtomicU64::new(0);
const PIT_FREQ: u32 = 1193182;

pub fn init() {
    let divisor: u16 = (PIT_FREQ / 100) as u16;
    unsafe {
        core::arch::asm!("out 0x43, al", in("al") 0x36u8);
        core::arch::asm!("out 0x40, al", in("al") (divisor & 0xFF) as u8);
        core::arch::asm!("out 0x40, al", in("al") (divisor >> 8) as u8);
    }
}

pub fn tick() {
    TICKS.fetch_add(1, Ordering::SeqCst);
}

pub fn get_ticks() -> u64 {
    TICKS.load(Ordering::SeqCst)
}

pub fn sleep_ms(ms: u64) {
    let start = get_ticks();
    while get_ticks() - start < ms {
        x86_64::instructions::hlt();
    }
}

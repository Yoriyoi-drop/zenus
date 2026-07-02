use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq)]
pub enum WatchdogType {
    Software,
    Hardware,
}

static WDT_ENABLED: AtomicBool = AtomicBool::new(false);
static WDT_TIMEOUT_SECS: AtomicU32 = AtomicU32::new(30);
static WDT_LAST_PET: AtomicU64 = AtomicU64::new(0);
static WDT_TICK_COUNT: AtomicU64 = AtomicU64::new(0);

const TICKS_PER_SEC: u64 = 1000;

pub fn watchdog_init(wdt_type: WatchdogType, timeout_secs: u32) -> bool {
    if WDT_ENABLED.load(Ordering::SeqCst) {
        return false;
    }
    WDT_TIMEOUT_SECS.store(timeout_secs, Ordering::Release);
    WDT_LAST_PET.store(0, Ordering::Release);
    WDT_TICK_COUNT.store(0, Ordering::Release);

    if wdt_type == WatchdogType::Hardware {
        unsafe {
            core::arch::asm!("out 0x64, al", in("al") 0xAEu8);
        }
    }
    WDT_ENABLED.store(true, Ordering::Release);
    zenus_console::kinfo!("Watchdog initialized");
    true
}

pub fn watchdog_pet() {
    if !WDT_ENABLED.load(Ordering::Acquire) {
        return;
    }
    WDT_LAST_PET.store(WDT_TICK_COUNT.load(Ordering::Acquire), Ordering::Release);
}

pub fn watchdog_stop() {
    WDT_ENABLED.store(false, Ordering::Release);
}

pub fn watchdog_is_active() -> bool {
    WDT_ENABLED.load(Ordering::Acquire)
}

pub fn watchdog_get_remaining() -> u32 {
    if !WDT_ENABLED.load(Ordering::Acquire) {
        return 0;
    }
    let last = WDT_LAST_PET.load(Ordering::Acquire);
    let now = WDT_TICK_COUNT.load(Ordering::Acquire);
    let elapsed_ticks = now.wrapping_sub(last);
    let elapsed_secs = (elapsed_ticks / TICKS_PER_SEC) as u32;
    WDT_TIMEOUT_SECS.load(Ordering::Acquire).saturating_sub(elapsed_secs)
}

pub fn watchdog_tick() {
    if !WDT_ENABLED.load(Ordering::Acquire) {
        return;
    }
    let count = WDT_TICK_COUNT.load(Ordering::Acquire);
    WDT_TICK_COUNT.store(count.wrapping_add(1), Ordering::Release);

    let tick_count = count.wrapping_add(1);
    if tick_count % TICKS_PER_SEC == 0 {
        let last = WDT_LAST_PET.load(Ordering::Acquire);
        let elapsed_ticks = tick_count.wrapping_sub(last);
        let elapsed_secs = (elapsed_ticks / TICKS_PER_SEC) as u32;
        if elapsed_secs >= WDT_TIMEOUT_SECS.load(Ordering::Acquire) {
            zenus_console::kpanic_code!(zenus_console::error::codes::DRV_COMM_TIMEOUT, "Watchdog timeout! Rebooting...");
        }
    }
}

pub fn watchdog_set_timeout(secs: u32) {
    WDT_TIMEOUT_SECS.store(secs, Ordering::Release);
}

pub fn watchdog_get_timeout() -> u32 {
    WDT_TIMEOUT_SECS.load(Ordering::Acquire)
}

pub fn watchdog_force_reboot() {
    zenus_console::kpanic_code!(zenus_console::error::codes::DRV_COMM_TIMEOUT, "Watchdog forced reboot");
}

use zenus_sync::spinlock::SpinLock;

static IO_SCHEDULER: SpinLock<IoScheduler> = SpinLock::new(IoScheduler::new());

struct IoScheduler {
    total_ios: u64,
}

impl IoScheduler {
    const fn new() -> Self {
        IoScheduler { total_ios: 0 }
    }
}

pub fn io_submit_read(dev_id: u8, block: u64, buf: &mut [u8]) -> bool {
    let result = crate::block_cache::bc_read(dev_id, block, buf);
    if result {
        IO_SCHEDULER.lock().total_ios += 1;
    }
    result
}

pub fn io_submit_write(dev_id: u8, block: u64, buf: &[u8]) -> bool {
    let result = crate::block_cache::bc_write(dev_id, block, buf);
    if result {
        IO_SCHEDULER.lock().total_ios += 1;
    }
    result
}

pub fn io_flush() {
    crate::block_cache::bc_flush();
}

pub fn io_stats() -> (u64, u64, u64) {
    let sched = IO_SCHEDULER.lock();
    (sched.total_ios, 0, 0)
}

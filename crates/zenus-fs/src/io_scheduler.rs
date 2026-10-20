use zenus_sync::spinlock::SpinLock;

static IO_SCHEDULER: SpinLock<IoScheduler> = SpinLock::new(IoScheduler::new());

struct IoScheduler {
    total_ios: u64,
    reads: u64,
    writes: u64,
}

impl IoScheduler {
    const fn new() -> Self {
        IoScheduler {
            total_ios: 0,
            reads: 0,
            writes: 0,
        }
    }
}

pub fn io_submit_read(dev_id: u8, block: u64, buf: &mut [u8]) -> bool {
    let result = crate::block_cache::bc_read(dev_id, block, buf);
    if result {
        let mut sched = IO_SCHEDULER.lock();
        sched.total_ios += 1;
        sched.reads += 1;
    }
    result
}

pub fn io_submit_write(dev_id: u8, block: u64, buf: &[u8]) -> bool {
    let result = crate::block_cache::bc_write(dev_id, block, buf);
    if result {
        let mut sched = IO_SCHEDULER.lock();
        sched.total_ios += 1;
        sched.writes += 1;
    }
    result
}

pub fn io_flush() {
    crate::block_cache::bc_flush();
}

/// `(total, reads, writes)`.
///
/// All three are tracked, and `total` is always `reads + writes`. The split
/// used to be hardcoded to zero, which made the numbers useless for diagnosing
/// I/O: a caller could not tell a read-heavy workload from a write-heavy one.
pub fn io_stats() -> (u64, u64, u64) {
    let sched = IO_SCHEDULER.lock();
    (sched.total_ios, sched.reads, sched.writes)
}

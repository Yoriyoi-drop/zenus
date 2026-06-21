use zenus_sync::spinlock::SpinLock;

const MAX_PENDING: usize = 64;

#[derive(Clone, Copy)]
struct IoRequest {
    dev_id: u8,
    block: u64,
    is_write: bool,
    completed: bool,
}

pub struct IoScheduler {
    pending: [Option<IoRequest>; MAX_PENDING],
    head: usize,
    tail: usize,
    count: usize,
    total_ios: u64,
}

impl IoScheduler {
    const fn new() -> Self {
        IoScheduler {
            pending: [None; MAX_PENDING],
            head: 0,
            tail: 0,
            count: 0,
            total_ios: 0,
        }
    }

    fn push(&mut self, req: IoRequest) -> bool {
        if self.count >= MAX_PENDING {
            return false;
        }
        self.pending[self.tail] = Some(req);
        self.tail = (self.tail + 1) % MAX_PENDING;
        self.count += 1;
        self.total_ios += 1;
        true
    }
}

static IO_SCHEDULER: SpinLock<IoScheduler> = SpinLock::new(IoScheduler::new());

pub fn io_submit_read(dev_id: u8, block: u64, buf: &mut [u8]) -> bool {
    let mut sched = IO_SCHEDULER.lock();
    let req = IoRequest {
        dev_id,
        block,
        is_write: false,
        completed: false,
    };
    sched.push(req);
    drop(sched);
    crate::block_cache::bc_read(dev_id, block, buf)
}

pub fn io_submit_write(dev_id: u8, block: u64, buf: &[u8]) -> bool {
    let mut sched = IO_SCHEDULER.lock();
    let req = IoRequest {
        dev_id,
        block,
        is_write: true,
        completed: false,
    };
    sched.push(req);
    drop(sched);
    crate::block_cache::bc_write(dev_id, block, buf)
}

pub fn io_flush() {
    crate::block_cache::bc_flush();
}

pub fn io_stats() -> (u64, u64, u64) {
    let sched = IO_SCHEDULER.lock();
    (sched.total_ios, sched.count as u64, 0)
}

use core::sync::atomic::{AtomicU64, Ordering};
use zenus_sync::spinlock::SpinLock;

const MAX_EDGES: usize = 65536;

static COVERAGE_MAP: SpinLock<CoverageMap> = SpinLock::new(CoverageMap::new());
static EDGE_COUNT: AtomicU64 = AtomicU64::new(0);
/// `EDGE_COUNT` as it was when the current test case started.
static DELTA_BASE: AtomicU64 = AtomicU64::new(0);
/// Set to 1 by `record_edge` whenever a genuinely new edge is seen, so the
/// caller does not have to snapshot/restore the counters around every case.
static DIRTY: AtomicU64 = AtomicU64::new(0);

struct CoverageMap {
    edges: [u64; MAX_EDGES],
    size: usize,
}

impl CoverageMap {
    const fn new() -> Self {
        CoverageMap {
            edges: [0; MAX_EDGES],
            size: 0,
        }
    }
}

pub fn init() {
    let mut map = COVERAGE_MAP.lock();
    map.edges = [0; MAX_EDGES];
    map.size = 0;
    drop(map);
    EDGE_COUNT.store(0, Ordering::Release);
    DIRTY.store(0, Ordering::Release);
    DELTA_BASE.store(0, Ordering::Release);
}

/// Record a new edge in the coverage map
pub fn record_edge(edge_id: u64) {
    let idx = (edge_id as usize) % MAX_EDGES;
    let mut map = COVERAGE_MAP.lock();
    if map.edges[idx] == 0 {
        map.edges[idx] = 1;
        map.size += 1;
        let size = map.size as u64;
        EDGE_COUNT.store(size, Ordering::Release);
        DIRTY.store(1, Ordering::Release);
    }
    drop(map);
}

/// Start a new test case: everything recorded from here on is "this case".
#[inline]
pub fn reset_delta() {
    DELTA_BASE.store(EDGE_COUNT.load(Ordering::Acquire), Ordering::Release);
    DIRTY.store(0, Ordering::Release);
}

/// True when the current test case reached at least one edge that had not
/// been covered before.
#[inline]
pub fn grew() -> bool {
    DIRTY.load(Ordering::Acquire) != 0
}

/// Number of edges discovered by the current test case.
#[inline]
pub fn delta() -> u64 {
    EDGE_COUNT
        .load(Ordering::Acquire)
        .saturating_sub(DELTA_BASE.load(Ordering::Acquire))
}

/// Check if an edge is new (not seen before)
pub fn is_new_edge(edge_id: u64) -> bool {
    let idx = (edge_id as usize) % MAX_EDGES;
    let map = COVERAGE_MAP.lock();
    let is_new = map.edges[idx] == 0;
    drop(map);
    is_new
}

/// Get total number of unique edges covered
pub fn edge_count() -> u64 {
    EDGE_COUNT.load(Ordering::Acquire)
}

/// Check if current coverage has new edges compared to baseline
pub fn has_new_coverage(baseline: u64) -> bool {
    edge_count() > baseline
}

/// Reset coverage tracking
pub fn reset() {
    init();
}

/// Get coverage percentage (approximate)
pub fn coverage_percent() -> u32 {
    let count = edge_count();
    ((count * 100) / MAX_EDGES as u64) as u32
}

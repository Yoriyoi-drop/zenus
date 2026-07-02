use zenus_sync::spinlock::SpinLock;
use zenus_mem::paging;
use crate::pci::VirtioPciTransport;
use crate::queue::{VirtioQueue, VirtioQueueMem, VirtioAvail, VirtioDesc, VRING_DESC_F_WRITE};
use crate::QUEUE_SIZE;

const VIRTIO_NET_F_MAC: u64 = 5;
const VIRTIO_NET_F_STATUS: u64 = 16;
const VIRTIO_NET_F_MQ: u64 = 17;

const MAX_QUEUE_PAIRS: usize = 2;
const RX_BUF_COUNT: usize = 64;
const TX_BUF_COUNT: usize = 64;
const BUF_SIZE: usize = 2048;

static mut RX_BUFS: [u8; RX_BUF_COUNT * BUF_SIZE] = [0u8; RX_BUF_COUNT * BUF_SIZE];
static mut RX_BUF_BUSY: [bool; RX_BUF_COUNT] = [false; RX_BUF_COUNT];
static mut TX_BUFS: [u8; TX_BUF_COUNT * BUF_SIZE] = [0u8; TX_BUF_COUNT * BUF_SIZE];
static mut TX_BUF_BUSY: [bool; TX_BUF_COUNT] = [false; TX_BUF_COUNT];

struct QueuePair {
    rx_queue: VirtioQueue,
    tx_queue: VirtioQueue,
}

static mut QUEUE_MEMS: [VirtioQueueMem; MAX_QUEUE_PAIRS * 2] = [
    VirtioQueueMem::new(), VirtioQueueMem::new(),
    VirtioQueueMem::new(), VirtioQueueMem::new(),
];

pub struct VirtioNet {
    pub mac: [u8; 6],
    pub link_up: bool,
    pub transport: VirtioPciTransport,
    queue_pairs: [Option<QueuePair>; MAX_QUEUE_PAIRS],
    num_pairs: usize,
    tx_pair_idx: core::sync::atomic::AtomicUsize,
}

fn rx_buf_alloc() -> Option<u16> {
    unsafe {
        for i in 0..RX_BUF_COUNT {
            if !RX_BUF_BUSY[i] {
                RX_BUF_BUSY[i] = true;
                return Some(i as u16);
            }
        }
    }
    None
}

fn tx_buf_alloc() -> Option<u16> {
    unsafe {
        for i in 0..TX_BUF_COUNT {
            if !TX_BUF_BUSY[i] {
                TX_BUF_BUSY[i] = true;
                return Some(i as u16);
            }
        }
    }
    None
}

static mut VIRTIO_NET: Option<VirtioNet> = None;
static NET_LOCK: SpinLock<()> = SpinLock::new(());

impl VirtioNet {
    pub unsafe fn new(transport: VirtioPciTransport) -> Option<Self> {
        zenus_console::kinfo!("VIRTIO-NET: Initializing...");

        transport.set_device_status(0);
        while transport.device_status() != 0 { core::hint::spin_loop(); }
        transport.set_device_status(transport.device_status() | 1);
        transport.set_device_status(transport.device_status() | 2);

        let device_features = transport.read_device_features();
        let mut feats = 0u64;
        if device_features & (1 << VIRTIO_NET_F_MAC) != 0 {
            feats |= 1 << VIRTIO_NET_F_MAC;
        }
        if device_features & (1 << VIRTIO_NET_F_STATUS) != 0 {
            feats |= 1 << VIRTIO_NET_F_STATUS;
        }
        if device_features & (1 << VIRTIO_NET_F_MQ) != 0 {
            feats |= 1 << VIRTIO_NET_F_MQ;
        }
        transport.negotiate_features(feats);

        transport.set_device_status(transport.device_status() | 8);
        if transport.device_status() & 8 == 0 {
            zenus_console::kerror_code!(zenus_console::error::codes::DRV_INIT_FAILED, "VIRTIO-NET: FEATURES_OK rejected");
            return None;
        }

        let num_pairs = if device_features & (1 << VIRTIO_NET_F_MQ) != 0 {
            let raw = transport.device_read16(8);
            core::cmp::max(core::cmp::min(raw as usize, MAX_QUEUE_PAIRS), 1)
        } else {
            1
        };

        zenus_console::kinfo!("VIRTIO-NET: {} queue pair(s)", num_pairs);

        let mac = {
            let mut m = [0u8; 6];
            for i in 0..6 {
                m[i] = transport.device_read8(i as u16);
            }
            m
        };

        let cr3 = paging::kernel_cr3();
        let mut queue_pairs: [Option<QueuePair>; MAX_QUEUE_PAIRS] = [None, None];

        for i in 0..num_pairs {
            let rx_idx = (i * 2) as u16;
            let tx_idx = (i * 2 + 1) as u16;

            let rx_mem: &'static mut VirtioQueueMem = &mut QUEUE_MEMS[i * 2];
            let rx_dp = paging::virt_to_phys_raw(cr3, rx_mem as *mut VirtioQueueMem as u64).unwrap_or(0);
            let rx_ap = rx_dp + core::mem::size_of::<[VirtioDesc; QUEUE_SIZE]>() as u64;
            let rx_up = rx_ap + core::mem::size_of::<VirtioAvail>() as u64;
            let rx_qsize = transport.setup_queue(rx_idx, rx_dp, rx_ap, rx_up);
            if rx_qsize == 0 {
                zenus_console::kerror_code!(zenus_console::error::codes::DRV_INIT_FAILED, "VIRTIO-NET: RX queue setup failed");
                continue;
            }

            let tx_mem: &'static mut VirtioQueueMem = &mut QUEUE_MEMS[i * 2 + 1];
            let tx_dp = paging::virt_to_phys_raw(cr3, tx_mem as *mut VirtioQueueMem as u64).unwrap_or(0);
            let tx_ap = tx_dp + core::mem::size_of::<[VirtioDesc; QUEUE_SIZE]>() as u64;
            let tx_up = tx_ap + core::mem::size_of::<VirtioAvail>() as u64;
            let tx_qsize = transport.setup_queue(tx_idx, tx_dp, tx_ap, tx_up);
            if tx_qsize == 0 {
                zenus_console::kerror_code!(zenus_console::error::codes::DRV_INIT_FAILED, "VIRTIO-NET: TX queue setup failed");
                continue;
            }

            let rx_notify = transport.queue_notify_addr(rx_idx);
            let tx_notify = transport.queue_notify_addr(tx_idx);

            let rx_queue = VirtioQueue::new(rx_mem, rx_qsize, rx_idx, rx_notify, cr3);
            let tx_queue = VirtioQueue::new(tx_mem, tx_qsize, tx_idx, tx_notify, cr3);

            queue_pairs[i] = Some(QueuePair { rx_queue, tx_queue });
        }

        transport.set_device_status(transport.device_status() | 4);

        let mut net = VirtioNet {
            mac,
            link_up: true,
            transport,
            queue_pairs,
            num_pairs,
            tx_pair_idx: core::sync::atomic::AtomicUsize::new(0),
        };

        net.setup_rx_bufs();

        zenus_console::kinfo!("VIRTIO-NET: Ready (MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})", mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

        Some(net)
    }

    pub unsafe fn setup_rx_bufs(&mut self) {
        let cr3 = paging::kernel_cr3();
        for pair in self.queue_pairs.iter_mut().flatten() {
            for _ in 0..32 {
                if let Some(buf_idx) = rx_buf_alloc() {
                    if let Some(desc_idx) = pair.rx_queue.alloc_desc() {
                        let buf_off = (buf_idx as usize) * BUF_SIZE;
                        let virt = &mut RX_BUFS[buf_off] as *mut u8 as u64;
                        let phys = paging::virt_to_phys_raw(cr3, virt).unwrap_or(0);
                        pair.rx_queue.mem.desc[desc_idx as usize] = VirtioDesc {
                            addr: phys,
                            len: BUF_SIZE as u32,
                            flags: VRING_DESC_F_WRITE,
                            next: 0,
                        };
                        pair.rx_queue.submit(desc_idx);
                    }
                }
            }
        }
    }

    fn select_tx_queue(&self) -> usize {
        self.tx_pair_idx.fetch_add(1, core::sync::atomic::Ordering::Relaxed) % self.num_pairs
    }

    pub fn send_raw(&mut self, data: &[u8]) -> bool {
        let _lock = NET_LOCK.lock();
        unsafe {
            if data.len() > 1500 {
                return false;
            }
            let pair_idx = self.select_tx_queue();
            let pair = match self.queue_pairs[pair_idx].as_mut() {
                Some(p) => p,
                None => return false,
            };

            let buf_idx = match tx_buf_alloc() {
                Some(idx) => idx,
                None => return false,
            };
            let buf_off = (buf_idx as usize) * BUF_SIZE;
            let cr3 = paging::kernel_cr3();

            TX_BUFS[buf_off..buf_off + data.len()].copy_from_slice(data);
            let virt = &mut TX_BUFS[buf_off] as *mut u8 as u64;
            let phys = paging::virt_to_phys_raw(cr3, virt).unwrap_or(0);

            let desc_idx = match pair.tx_queue.alloc_desc() {
                Some(d) => d,
                None => {
                    TX_BUF_BUSY[buf_idx as usize] = false;
                    return false;
                }
            };

            pair.tx_queue.mem.desc[desc_idx as usize] = VirtioDesc {
                addr: phys,
                len: data.len() as u32,
                flags: 0,
                next: 0,
            };

            pair.tx_queue.submit(desc_idx);
            pair.tx_queue.kick();

            for _ in 0..10000 {
                if let Some((id, _)) = pair.tx_queue.collect_used() {
                    if id == desc_idx as u32 {
                        TX_BUF_BUSY[buf_idx as usize] = false;
                        return true;
                    }
                }
                core::hint::spin_loop();
            }

            TX_BUF_BUSY[buf_idx as usize] = false;
            false
        }
    }

    pub fn receive(&mut self, buf: &mut [u8]) -> Option<usize> {
        let _lock = NET_LOCK.lock();
        unsafe {
            for pair in self.queue_pairs.iter_mut().flatten() {
                if let Some((id, len)) = pair.rx_queue.collect_used() {
                    let buf_idx = id as usize;
                    if buf_idx < RX_BUF_COUNT {
                        let buf_off = buf_idx * BUF_SIZE;
                        let copy_len = core::cmp::min(len as usize, buf.len());
                        buf[..copy_len].copy_from_slice(&RX_BUFS[buf_off..buf_off + copy_len]);

                        RX_BUF_BUSY[buf_idx] = false;
                        let cr3 = paging::kernel_cr3();
                        if let Some(new_idx) = rx_buf_alloc() {
                            let new_off = (new_idx as usize) * BUF_SIZE;
                            let new_virt = &mut RX_BUFS[new_off] as *mut u8 as u64;
                            let new_phys = paging::virt_to_phys_raw(cr3, new_virt).unwrap_or(0);
                            if let Some(d_desc) = pair.rx_queue.alloc_desc() {
                                pair.rx_queue.mem.desc[d_desc as usize] = VirtioDesc {
                                    addr: new_phys,
                                    len: BUF_SIZE as u32,
                                    flags: VRING_DESC_F_WRITE,
                                    next: 0,
                                };
                                pair.rx_queue.submit(d_desc);
                            }
                        }
                        return Some(copy_len);
                    }
                }
            }
        }
        None
    }

    pub fn poll(&mut self) {
        let _lock = NET_LOCK.lock();
        unsafe {
            let isr = self.transport.read_isr();
            if isr != 0 {
                for pair in self.queue_pairs.iter_mut().flatten() {
                    while let Some((_id, _len)) = pair.rx_queue.collect_used() {}
                }
            }
        }
    }

    pub fn with_nic<R>(f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let _guard = NET_LOCK.lock();
        unsafe { VIRTIO_NET.as_mut().map(f) }
    }

    pub fn is_present() -> bool {
        unsafe { VIRTIO_NET.is_some() }
    }

    pub fn nic_ref() -> Option<&'static mut Self> {
        unsafe { VIRTIO_NET.as_mut() }
    }

    pub fn queue_pair_count(&self) -> usize {
        self.num_pairs
    }
}

pub unsafe fn probe_and_init(trans: VirtioPciTransport) -> Option<&'static mut VirtioNet> {
    let net = VirtioNet::new(trans)?;
    VIRTIO_NET = Some(net);
    VIRTIO_NET.as_mut()
}

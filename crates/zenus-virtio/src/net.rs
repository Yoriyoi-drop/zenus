use zenus_sync::spinlock::SpinLock;
use zenus_mem::paging;
use crate::pci::VirtioPciTransport;
use crate::queue::{VirtioQueue, VirtioQueueMem, VirtioAvail, VirtioDesc, VRING_DESC_F_WRITE};
use crate::{serial, QUEUE_SIZE};

const VIRTIO_NET_F_MAC: u64 = 5;
const VIRTIO_NET_F_STATUS: u64 = 16;

pub enum VirtioSendRecv {
    Ok(usize),
    NoBuf,
    Error,
}

pub struct VirtioNet {
    pub mac: [u8; 6],
    pub link_up: bool,
    pub transport: VirtioPciTransport,
    rx_queue: VirtioQueue,
    tx_queue: VirtioQueue,
}

const RX_BUF_COUNT: usize = 64;
const TX_BUF_COUNT: usize = 64;
const BUF_SIZE: usize = 2048;

static mut RX_BUFS: [u8; RX_BUF_COUNT * BUF_SIZE] = [0u8; RX_BUF_COUNT * BUF_SIZE];
static mut RX_BUF_BUSY: [bool; RX_BUF_COUNT] = [false; RX_BUF_COUNT];
static mut TX_BUFS: [u8; TX_BUF_COUNT * BUF_SIZE] = [0u8; TX_BUF_COUNT * BUF_SIZE];
static mut TX_BUF_BUSY: [bool; TX_BUF_COUNT] = [false; TX_BUF_COUNT];

static mut RX_QUEUE_MEM: VirtioQueueMem = VirtioQueueMem::new();
static mut TX_QUEUE_MEM: VirtioQueueMem = VirtioQueueMem::new();

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
    pub fn base_offset() -> u16 {
        0
    }

    pub unsafe fn new(transport: VirtioPciTransport) -> Option<Self> {
        let s = serial();

        s.write_str("[VIRTIO-NET] Initializing...\n");

        let device_features = transport.read_device_features();
        let mut feats = 0u64;
        if device_features & (1 << VIRTIO_NET_F_MAC) != 0 {
            feats |= 1 << VIRTIO_NET_F_MAC;
        }
        if device_features & (1 << VIRTIO_NET_F_STATUS) != 0 {
            feats |= 1 << VIRTIO_NET_F_STATUS;
        }

        let _negotiated = transport.negotiate_features(feats);

        let cr3 = paging::kernel_cr3();
        let rx_mem: &'static mut VirtioQueueMem = &mut RX_QUEUE_MEM;
        let rx_dp = paging::virt_to_phys_raw(cr3, rx_mem as *mut VirtioQueueMem as u64).unwrap_or(0);
        let rx_ap = rx_dp + core::mem::size_of::<[VirtioDesc; QUEUE_SIZE]>() as u64;
        let rx_up = rx_ap + core::mem::size_of::<VirtioAvail>() as u64;
        transport.setup_queue(0, rx_dp, rx_ap, rx_up);

        let tx_mem: &'static mut VirtioQueueMem = &mut TX_QUEUE_MEM;
        let tx_dp = paging::virt_to_phys_raw(cr3, tx_mem as *mut VirtioQueueMem as u64).unwrap_or(0);
        let tx_ap = tx_dp + core::mem::size_of::<[VirtioDesc; QUEUE_SIZE]>() as u64;
        let tx_up = tx_ap + core::mem::size_of::<VirtioAvail>() as u64;
        transport.setup_queue(1, tx_dp, tx_ap, tx_up);

        let rx_queue = VirtioQueue::new(rx_mem, QUEUE_SIZE as u16, 0, transport.notify_base, cr3);
        let tx_queue = VirtioQueue::new(tx_mem, QUEUE_SIZE as u16, 1, transport.notify_base, cr3);

        let mut mac = [0u8; 6];
        for i in 0..6 {
            mac[i] = transport.device_read8(Self::base_offset() + i as u16);
        }

        let mut net = VirtioNet {
            mac,
            link_up: true,
            transport,
            rx_queue,
            tx_queue,
        };

        net.transport.set_device_status(
            net.transport.device_status() | 4
        );

        net.setup_rx_bufs();

        s.write_str("[VIRTIO-NET] Ready (MAC ");
        for (i, b) in mac.iter().enumerate() {
            if i > 0 { s.write_str(":"); }
            s.write_hex(*b as u64);
        }
        s.write_str(")\n");

        Some(net)
    }

    pub unsafe fn setup_rx_bufs(&mut self) {
        let cr3 = paging::kernel_cr3();
        for _ in 0..32 {
            if let Some(buf_idx) = rx_buf_alloc() {
                if let Some(desc_idx) = self.rx_queue.alloc_desc() {
                    let buf_off = (buf_idx as usize) * BUF_SIZE;
                    let virt = &mut RX_BUFS[buf_off] as *mut u8 as u64;
                    let phys = paging::virt_to_phys_raw(cr3, virt).unwrap_or(0);
                    self.rx_queue.mem.desc[desc_idx as usize] = VirtioDesc {
                        addr: phys,
                        len: BUF_SIZE as u32,
                        flags: VRING_DESC_F_WRITE,
                        next: 0,
                    };
                    self.rx_queue.submit(desc_idx);
                }
            }
        }
    }

    pub fn send_raw(&mut self, data: &[u8]) -> bool {
        let _lock = NET_LOCK.lock();
        unsafe {
            if data.len() > 1500 {
                return false;
            }
            let buf_idx = match tx_buf_alloc() {
                Some(idx) => idx,
                None => return false,
            };
            let buf_off = (buf_idx as usize) * BUF_SIZE;
            let cr3 = paging::kernel_cr3();

            TX_BUFS[buf_off..buf_off + data.len()].copy_from_slice(data);
            let virt = &mut TX_BUFS[buf_off] as *mut u8 as u64;
            let phys = paging::virt_to_phys_raw(cr3, virt).unwrap_or(0);

            let desc_idx = match self.tx_queue.alloc_desc() {
                Some(d) => d,
                None => {
                    TX_BUF_BUSY[buf_idx as usize] = false;
                    return false;
                }
            };

            self.tx_queue.mem.desc[desc_idx as usize] = VirtioDesc {
                addr: phys,
                len: data.len() as u32,
                flags: 0,
                next: 0,
            };

            self.tx_queue.submit(desc_idx);
            self.tx_queue.kick();

            for _ in 0..10000 {
                if let Some((id, _)) = self.tx_queue.collect_used() {
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
            if let Some((id, len)) = self.rx_queue.collect_used() {
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
                        let desc_idx = self.rx_queue.alloc_desc().unwrap_or(0);
                        self.rx_queue.mem.desc[desc_idx as usize] = VirtioDesc {
                            addr: new_phys,
                            len: BUF_SIZE as u32,
                            flags: VRING_DESC_F_WRITE,
                            next: 0,
                        };
                        self.rx_queue.submit(desc_idx);
                    }

                    return Some(copy_len);
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
                while let Some((_id, _len)) = self.rx_queue.collect_used() {
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
}

pub unsafe fn probe_and_init(trans: VirtioPciTransport) -> Option<&'static mut VirtioNet> {
    let net = VirtioNet::new(trans)?;
    VIRTIO_NET = Some(net);
    VIRTIO_NET.as_mut()
}

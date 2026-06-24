use core::ptr;
use alloc::format;
use alloc::boxed::Box;
use zenus_sync::spinlock::SpinLock;
use zenus_mem::paging;
use zenus_fs::devfs::{self, BlockDeviceOps};
use crate::pci::VirtioPciTransport;
use crate::queue::{VirtioQueue, VirtioQueueMem, VirtioAvail, VirtioDesc, VRING_DESC_F_NEXT, VRING_DESC_F_WRITE};
use crate::{serial, QUEUE_SIZE};

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_FLUSH: u32 = 4;

#[repr(C, align(16))]
struct VirtioBlkReqHdr {
    type_: u32,
    reserved: u32,
    sector: u64,
}

#[repr(C, align(16))]
struct VirtioBlkResp {
    status: u8,
    padding: [u8; 15],
}

const STATUS_OK: u8 = 0;
const BLK_BUF_SIZE: usize = 512;
const BLK_BUF_COUNT: usize = 32;

static mut BLK_BUFS: [u8; BLK_BUF_COUNT * BLK_BUF_SIZE] = [0u8; BLK_BUF_COUNT * BLK_BUF_SIZE];
static mut BLK_BUF_BUSY: [bool; BLK_BUF_COUNT] = [false; BLK_BUF_COUNT];
static mut BLK_QUEUE_MEM: VirtioQueueMem = VirtioQueueMem::new();

fn blk_buf_alloc() -> Option<u16> {
    unsafe {
        for i in 0..BLK_BUF_COUNT {
            if !BLK_BUF_BUSY[i] {
                BLK_BUF_BUSY[i] = true;
                return Some(i as u16);
            }
        }
    }
    None
}

pub struct VirtioBlk {
    pub capacity: u64,
    transport: VirtioPciTransport,
    queue: VirtioQueue,
    dev_idx: usize,
}

static mut VIRTIO_BLK: Option<VirtioBlk> = None;
static BLK_LOCK: SpinLock<()> = SpinLock::new(());

impl VirtioBlk {
    pub unsafe fn new(transport: VirtioPciTransport) -> Option<Self> {
        let s = serial();
        s.write_str("[VIRTIO-BLK] Initializing...\n");

        transport.set_device_status(0);
        while transport.device_status() != 0 { core::hint::spin_loop(); }
        transport.set_device_status(transport.device_status() | 1);
        transport.set_device_status(transport.device_status() | 2);

        let device_features = transport.read_device_features();
        let _negotiated = transport.negotiate_features(device_features);

        transport.set_device_status(transport.device_status() | 8);
        if transport.device_status() & 8 == 0 {
            s.write_str("[VIRTIO-BLK] FEATURES_OK rejected, status=");
            s.write_hex(transport.device_status() as u64);
            s.write_str("\n");
            return None;
        }



        let cr3 = paging::kernel_cr3();
        let queue_mem: &'static mut VirtioQueueMem = &mut BLK_QUEUE_MEM;

        let dp = paging::virt_to_phys_raw(cr3, queue_mem as *mut VirtioQueueMem as u64).unwrap_or(0);
        let ap = dp + core::mem::size_of::<[VirtioDesc; QUEUE_SIZE]>() as u64;
        let up = ap + core::mem::size_of::<VirtioAvail>() as u64;

        let size = transport.setup_queue(0, dp, ap, up);
        if size == 0 {
            s.write_str("[VIRTIO-BLK] Queue setup failed\n");
            return None;
        }

        let queue = VirtioQueue::new(queue_mem, size, 0, transport.notify_base, cr3);

        let capacity = {
            let cfg_base = transport.get_device_config_space();
            if cfg_base == 0 {
                s.write_str("[VIRTIO-BLK] No device config space\n");
                return None;
            }
            let cap = cfg_base as *const u64;
            cap.read_volatile()
        };

        transport.set_device_status(transport.device_status() | 4);

        let dev_idx = 0;

        let name = format!("vd{}", dev_idx);
        let name_str = Box::leak(name.into_boxed_str());
        devfs::register_block_device(name_str, BlockDeviceOps {
            read: blk_read0,
            write: blk_write0,
            size: capacity * 512,
        });

        s.write_str("[VIRTIO-BLK] Capacity: ");
        s.write_u64(capacity);
        s.write_str(" sectors (");
        s.write_u64(capacity / 2048);
        s.write_str(" MB)\n");
        s.write_str("[VIRTIO-BLK] Registered as /dev/");
        s.write_str(name_str);
        s.write_str("\n");

        Some(VirtioBlk { capacity, transport, queue, dev_idx })
    }

    pub unsafe fn read_sectors(&mut self, lba: u64, count: u16, buf: &mut [u8]) -> bool {
        if count == 0 || lba * 512 + (count as u64) * 512 > self.capacity * 512 {
            return false;
        }
        if buf.len() < (count as usize) * 512 {
            return false;
        }

        let buf_idx = match blk_buf_alloc() {
            Some(i) => i,
            None => return false,
        };
        let buf_off = (buf_idx as usize) * BLK_BUF_SIZE;
        let cr3 = paging::kernel_cr3();
        let buf_virt = &mut BLK_BUFS[buf_off] as *mut u8 as u64;
        let buf_phys = paging::virt_to_phys_raw(cr3, buf_virt).unwrap_or(0);

        let mut hdr = VirtioBlkReqHdr { type_: VIRTIO_BLK_T_IN, reserved: 0, sector: lba };
        let mut resp = VirtioBlkResp { status: 0xFF, padding: [0; 15] };
        let hdr_virt = &mut hdr as *mut VirtioBlkReqHdr as u64;
        let hdr_phys = paging::virt_to_phys_raw(cr3, hdr_virt).unwrap_or(0);
        let resp_virt = &mut resp as *mut VirtioBlkResp as u64;
        let resp_phys = paging::virt_to_phys_raw(cr3, resp_virt).unwrap_or(0);

        let d0 = self.queue.alloc_desc().unwrap_or(0);
        let d1 = self.queue.alloc_desc().unwrap_or(0);
        let d2 = self.queue.alloc_desc().unwrap_or(0);

        self.queue.mem.desc[d0 as usize] = VirtioDesc {
            addr: hdr_phys, len: 16, flags: VRING_DESC_F_NEXT, next: d1,
        };
        self.queue.mem.desc[d1 as usize] = VirtioDesc {
            addr: buf_phys, len: (count as u32) * 512,
            flags: VRING_DESC_F_WRITE | VRING_DESC_F_NEXT, next: d2,
        };
        self.queue.mem.desc[d2 as usize] = VirtioDesc {
            addr: resp_phys, len: 1, flags: VRING_DESC_F_WRITE, next: 0,
        };

        self.queue.submit(d0);
        self.queue.kick();

        for _ in 0..50000 {
            if let Some((_id, _len)) = self.queue.collect_used() {
                break;
            }
            core::hint::spin_loop();
        }

        BLK_BUF_BUSY[buf_idx as usize] = false;
        let status = ptr::read_volatile(&resp.status as *const u8);
        if status == STATUS_OK {
            let copy_len = core::cmp::min((count as usize) * 512, buf.len());
            buf[..copy_len].copy_from_slice(&BLK_BUFS[buf_off..buf_off + copy_len]);
            true
        } else {
            false
        }
    }

    pub unsafe fn write_sectors(&mut self, lba: u64, count: u16, buf: &[u8]) -> bool {
        if count == 0 || lba * 512 + (count as u64) * 512 > self.capacity * 512 {
            return false;
        }
        if buf.len() < (count as usize) * 512 {
            return false;
        }

        let buf_idx = match blk_buf_alloc() {
            Some(i) => i,
            None => return false,
        };
        let buf_off = (buf_idx as usize) * BLK_BUF_SIZE;
        let cr3 = paging::kernel_cr3();

        BLK_BUFS[buf_off..buf_off + (count as usize) * 512].copy_from_slice(&buf[..(count as usize) * 512]);
        let buf_virt = &mut BLK_BUFS[buf_off] as *mut u8 as u64;
        let buf_phys = paging::virt_to_phys_raw(cr3, buf_virt).unwrap_or(0);

        let mut hdr = VirtioBlkReqHdr {
            type_: VIRTIO_BLK_T_OUT, reserved: 0, sector: lba,
        };
        let mut resp = VirtioBlkResp { status: 0xFF, padding: [0; 15] };
        let hdr_virt = &mut hdr as *mut VirtioBlkReqHdr as u64;
        let hdr_phys = paging::virt_to_phys_raw(cr3, hdr_virt).unwrap_or(0);
        let resp_virt = &mut resp as *mut VirtioBlkResp as u64;
        let resp_phys = paging::virt_to_phys_raw(cr3, resp_virt).unwrap_or(0);

        let d0 = self.queue.alloc_desc().unwrap_or(0);
        let d1 = self.queue.alloc_desc().unwrap_or(0);
        let d2 = self.queue.alloc_desc().unwrap_or(0);

        self.queue.mem.desc[d0 as usize] = VirtioDesc {
            addr: hdr_phys, len: 16, flags: VRING_DESC_F_NEXT, next: d1,
        };
        self.queue.mem.desc[d1 as usize] = VirtioDesc {
            addr: buf_phys, len: (count as u32) * 512,
            flags: VRING_DESC_F_NEXT, next: d2,
        };
        self.queue.mem.desc[d2 as usize] = VirtioDesc {
            addr: resp_phys, len: 1, flags: VRING_DESC_F_WRITE, next: 0,
        };

        self.queue.submit(d0);
        self.queue.kick();

        for _ in 0..50000 {
            if let Some((_id, _len)) = self.queue.collect_used() {
                break;
            }
            core::hint::spin_loop();
        }

        BLK_BUF_BUSY[buf_idx as usize] = false;
        ptr::read_volatile(&resp.status as *const u8) == STATUS_OK
    }
}

fn blk_read0(lba: u64, buf: &mut [u8]) -> bool {
    let _lock = BLK_LOCK.lock();
    unsafe {
        let blk = match VIRTIO_BLK.as_mut() {
            Some(b) => b,
            None => return false,
        };
        blk.read_sectors(lba, (buf.len() / 512) as u16, buf)
    }
}

fn blk_write0(lba: u64, buf: &[u8]) -> bool {
    let _lock = BLK_LOCK.lock();
    unsafe {
        let blk = match VIRTIO_BLK.as_mut() {
            Some(b) => b,
            None => return false,
        };
        blk.write_sectors(lba, (buf.len() / 512) as u16, buf)
    }
}

pub unsafe fn probe_and_init(trans: VirtioPciTransport) -> Option<&'static mut VirtioBlk> {
    let blk = VirtioBlk::new(trans)?;
    VIRTIO_BLK = Some(blk);
    VIRTIO_BLK.as_mut()
}

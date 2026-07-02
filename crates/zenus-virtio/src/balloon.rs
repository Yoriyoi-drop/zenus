use zenus_mem::paging;
use crate::pci::VirtioPciTransport;
use crate::queue::{VirtioQueue, VirtioQueueMem, VirtioAvail, VirtioDesc};
use crate::QUEUE_SIZE;

const VIRTIO_BALLOON_F_MUST_TELL_HOST: u64 = 0;
const VIRTIO_BALLOON_F_STATS_VQ: u64 = 1;
const VIRTIO_BALLOON_F_DEFLATE_ON_OOM: u64 = 2;

static mut PAGE_BUF: [u8; 4096] = [0u8; 4096];
static mut INFLATE_QUEUE_MEM: VirtioQueueMem = VirtioQueueMem::new();
static mut DEFLATE_QUEUE_MEM: VirtioQueueMem = VirtioQueueMem::new();

pub struct VirtioBalloon {
    transport: VirtioPciTransport,
    inflate_queue: VirtioQueue,
    deflate_queue: VirtioQueue,
    current_pages: u32,
}

impl VirtioBalloon {
    pub unsafe fn new(transport: VirtioPciTransport) -> Option<Self> {
        zenus_console::kinfo!("VIRTIO-BALLOON: Initializing...");

        transport.set_device_status(0);
        while transport.device_status() != 0 { core::hint::spin_loop(); }
        transport.set_device_status(transport.device_status() | 1);
        transport.set_device_status(transport.device_status() | 2);

        let our_features = (1 << VIRTIO_BALLOON_F_MUST_TELL_HOST) | (1 << VIRTIO_BALLOON_F_DEFLATE_ON_OOM);
        transport.negotiate_features(our_features);
        transport.set_device_status(transport.device_status() | 8);
        if transport.device_status() & 8 == 0 {
            zenus_console::kerror_code!(zenus_console::error::codes::DRV_INIT_FAILED, "VIRTIO-BALLOON: FEATURES_OK rejected");
            return None;
        }

        let cr3 = paging::kernel_cr3();
        let inf_mem: &'static mut VirtioQueueMem = &mut INFLATE_QUEUE_MEM;
        let inf_dp = paging::virt_to_phys_raw(cr3, inf_mem as *mut VirtioQueueMem as u64).unwrap_or(0);
        let inf_ap = inf_dp + core::mem::size_of::<[VirtioDesc; QUEUE_SIZE]>() as u64;
        let inf_up = inf_ap + core::mem::size_of::<VirtioAvail>() as u64;

        let qsize0 = transport.setup_queue(0, inf_dp, inf_ap, inf_up);
        if qsize0 == 0 {
            zenus_console::kerror_code!(zenus_console::error::codes::DRV_INIT_FAILED, "VIRTIO-BALLOON: Inflate queue setup failed");
            return None;
        }

        let def_mem: &'static mut VirtioQueueMem = &mut DEFLATE_QUEUE_MEM;
        let def_dp = paging::virt_to_phys_raw(cr3, def_mem as *mut VirtioQueueMem as u64).unwrap_or(0);
        let def_ap = def_dp + core::mem::size_of::<[VirtioDesc; QUEUE_SIZE]>() as u64;
        let def_up = def_ap + core::mem::size_of::<VirtioAvail>() as u64;

        let qsize1 = transport.setup_queue(1, def_dp, def_ap, def_up);
        if qsize1 == 0 {
            zenus_console::kerror_code!(zenus_console::error::codes::DRV_INIT_FAILED, "VIRTIO-BALLOON: Deflate queue setup failed");
            return None;
        }

        let inflate_queue = VirtioQueue::new(inf_mem, qsize0, 0, transport.queue_notify_addr(0), cr3);
        let deflate_queue = VirtioQueue::new(def_mem, qsize1, 1, transport.queue_notify_addr(1), cr3);

        transport.set_device_status(transport.device_status() | 4);

        zenus_console::kinfo!("VIRTIO-BALLOON: Ready");

        Some(VirtioBalloon {
            transport,
            inflate_queue,
            deflate_queue,
            current_pages: 0,
        })
    }

    pub unsafe fn get_num_pages(&self) -> u32 {
        let cfg_base = self.transport.get_device_config_space();
        if cfg_base == 0 { return 0; }
        let ptr = cfg_base as *const u32;
        ptr.read_volatile()
    }

    pub unsafe fn set_actual(&mut self, pages: u32) {
        let cfg_base = self.transport.get_device_config_space();
        if cfg_base == 0 { return; }
        let ptr = (cfg_base + 4) as *mut u32;
        ptr.write_volatile(pages);
        self.current_pages = pages;
    }

    pub unsafe fn inflate(&mut self, pfn: u32) {
        let cr3 = paging::kernel_cr3();
        let desc_idx = match self.inflate_queue.alloc_desc() {
            Some(d) => d,
            None => return,
        };
        PAGE_BUF[..4].copy_from_slice(&pfn.to_le_bytes());
        let buf_virt = &PAGE_BUF as *const u8 as u64;
        let buf_phys = paging::virt_to_phys_raw(cr3, buf_virt).unwrap_or(0);
        self.inflate_queue.mem.desc[desc_idx as usize] = VirtioDesc {
            addr: buf_phys,
            len: 4,
            flags: 0,
            next: 0,
        };
        self.inflate_queue.submit(desc_idx);
        self.inflate_queue.kick();
    }

    pub unsafe fn deflate(&mut self, pfn: u32) {
        let cr3 = paging::kernel_cr3();
        let desc_idx = match self.deflate_queue.alloc_desc() {
            Some(d) => d,
            None => return,
        };
        PAGE_BUF[..4].copy_from_slice(&pfn.to_le_bytes());
        let buf_virt = &PAGE_BUF as *const u8 as u64;
        let buf_phys = paging::virt_to_phys_raw(cr3, buf_virt).unwrap_or(0);
        self.deflate_queue.mem.desc[desc_idx as usize] = VirtioDesc {
            addr: buf_phys,
            len: 4,
            flags: 0,
            next: 0,
        };
        self.deflate_queue.submit(desc_idx);
        self.deflate_queue.kick();
    }
}

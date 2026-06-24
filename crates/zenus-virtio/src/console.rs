use zenus_mem::paging;
use crate::pci::VirtioPciTransport;
use crate::queue::{VirtioQueue, VirtioQueueMem, VirtioAvail, VirtioDesc};
use crate::{serial, QUEUE_SIZE};

static mut CONSOLE_BUF: [u8; 4096] = [0u8; 4096];
static mut RX_QUEUE_MEM: VirtioQueueMem = VirtioQueueMem::new();
static mut TX_QUEUE_MEM: VirtioQueueMem = VirtioQueueMem::new();

pub struct VirtioConsole {
    transport: VirtioPciTransport,
    rx_queue: VirtioQueue,
    tx_queue: VirtioQueue,
}

impl VirtioConsole {
    pub unsafe fn new(transport: VirtioPciTransport) -> Option<Self> {
        let s = serial();
        s.write_str("[VIRTIO-CONSOLE] Initializing...\n");

        transport.set_device_status(0);
        while transport.device_status() != 0 { core::hint::spin_loop(); }
        transport.set_device_status(transport.device_status() | 1);
        transport.set_device_status(transport.device_status() | 2);

        transport.negotiate_features(0);
        transport.set_device_status(transport.device_status() | 8);
        if transport.device_status() & 8 == 0 {
            s.write_str("[VIRTIO-CONSOLE] FEATURES_OK rejected\n");
            return None;
        }

        let cr3 = paging::kernel_cr3();
        let rx_mem: &'static mut VirtioQueueMem = &mut RX_QUEUE_MEM;
        let rx_dp = paging::virt_to_phys_raw(cr3, rx_mem as *mut VirtioQueueMem as u64).unwrap_or(0);
        let rx_ap = rx_dp + core::mem::size_of::<[VirtioDesc; QUEUE_SIZE]>() as u64;
        let rx_up = rx_ap + core::mem::size_of::<VirtioAvail>() as u64;

        let qsize0 = transport.setup_queue(0, rx_dp, rx_ap, rx_up);
        if qsize0 == 0 {
            s.write_str("[VIRTIO-CONSOLE] RX queue setup failed\n");
            return None;
        }

        let tx_mem: &'static mut VirtioQueueMem = &mut TX_QUEUE_MEM;
        let tx_dp = paging::virt_to_phys_raw(cr3, tx_mem as *mut VirtioQueueMem as u64).unwrap_or(0);
        let tx_ap = tx_dp + core::mem::size_of::<[VirtioDesc; QUEUE_SIZE]>() as u64;
        let tx_up = tx_ap + core::mem::size_of::<VirtioAvail>() as u64;

        let qsize1 = transport.setup_queue(1, tx_dp, tx_ap, tx_up);
        if qsize1 == 0 {
            s.write_str("[VIRTIO-CONSOLE] TX queue setup failed\n");
            return None;
        }

        let rx_queue = VirtioQueue::new(rx_mem, qsize0, 0, transport.queue_notify_addr(0), cr3);
        let tx_queue = VirtioQueue::new(tx_mem, qsize1, 1, transport.queue_notify_addr(1), cr3);

        transport.set_device_status(transport.device_status() | 4);

        s.write_str("[VIRTIO-CONSOLE] Ready\n");
        Some(VirtioConsole { transport, rx_queue, tx_queue })
    }

    pub fn write(&mut self, data: &[u8]) -> bool {
        unsafe {
            let cr3 = paging::kernel_cr3();
            let desc_idx = match self.tx_queue.alloc_desc() {
                Some(d) => d,
                None => return false,
            };

            let copy_len = core::cmp::min(data.len(), 4096);
            CONSOLE_BUF[..copy_len].copy_from_slice(&data[..copy_len]);
            let buf_virt = &CONSOLE_BUF as *const u8 as u64;
            let buf_phys = paging::virt_to_phys_raw(cr3, buf_virt).unwrap_or(0);

            self.tx_queue.mem.desc[desc_idx as usize] = VirtioDesc {
                addr: buf_phys,
                len: copy_len as u32,
                flags: 0,
                next: 0,
            };

            self.tx_queue.submit(desc_idx);
            self.tx_queue.kick();
            true
        }
    }
}

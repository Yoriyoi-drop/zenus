use x86_64::instructions::port::Port;
use zenus_console::serial::SerialPort;
use zenus_sync::spinlock::SpinLock;
use crate::ethernet;
use crate::arp;
use crate::icmp;

const RTL_VENDOR: u16 = 0x10EC;
const RTL_DEVICE: u16 = 0x8139;

const RTL_IDR0: u16 = 0x00;
const RTL_CR: u16 = 0x37;
const RTL_CAPR: u16 = 0x38;
const RTL_CBR: u16 = 0x38;
const RTL_RBSTART: u16 = 0x30;
const RTL_IMR: u16 = 0x3C;
const RTL_ISR: u16 = 0x3E;
const RTL_RCR: u16 = 0x44;
const RTL_TSD0: u16 = 0x10;
const RTL_TSAD0: u16 = 0x20;
const RTL_CONFIG1: u16 = 0x52;

const CR_RST: u8 = 0x10;
const CR_RE: u8 = 0x08;
const CR_TE: u8 = 0x04;

const RCR_AB: u32 = 0x00000004;
const RCR_AM: u32 = 0x00000008;
const RCR_APM: u32 = 0x00000010;
const RCR_AAP: u32 = 0x00000020;
const RCR_WRAP: u32 = 0x00000080;
const RCR_MXDMA_1024: u32 = 0x00000700;
const RCR_RBLEN_8K: u32 = 0x00000300;

const TSD_TOK: u32 = 0x00008000;
const TSD_TUN: u32 = 0x00004000;
const TSD_OWN: u32 = 0x00002000;
const TSD_SIZE_MASK: u32 = 0x00001FFF;

const ISR_ROK: u16 = 0x0001;
const ISR_RER: u16 = 0x0004;
const ISR_TOK: u16 = 0x0002;
const ISR_TER: u16 = 0x0008;

const RX_BUF_SIZE: usize = 32768;
const RX_BUF_ALIGN: usize = 16;
const TX_DESC_COUNT: usize = 4;

const RX_BUF_PAGES: usize = 8;
const RX_BUF_BYTES: usize = RX_BUF_PAGES * 0x1000;

static RTL_LOCK: SpinLock<()> = SpinLock::new(());
static mut RTL_IFACE: Option<Rtl8139> = None;
static NIC_IO_BASE: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);

#[repr(C, align(4096))]
struct AlignedBuf([u8; RX_BUF_BYTES]);

static mut RX_BUF: AlignedBuf = AlignedBuf([0; RX_BUF_BYTES]);

pub struct Rtl8139 {
    io_base: u16,
    mac: [u8; 6],
    ip: [u8; 4],
    subnet: [u8; 4],
    gateway: [u8; 4],
    rx_buf_phys: u32,
    rx_cur: u16,
    tx_cur: usize,
    link_up: bool,
    irq_line: u8,
    tx_bounce: [u8; 1792],
}

impl Rtl8139 {
    fn virt_to_phys(virt: u64) -> u64 {
        let hhdm = zenus_mem::paging::hhdm_offset();
        // Fast path: already in HHDM direct-map range
        // Kernel virtual addresses (>= 0xFFFFFFFF80000000) are NOT HHDM-mapped
        if virt >= hhdm && virt < 0xFFFFFFFF80000000 {
            return virt - hhdm;
        }

        let levels = [4usize, 3, 2, 1];
        let cr3_val = zenus_mem::paging::kernel_cr3();
        let cr3_phys = cr3_val & !0xFFF;

        unsafe {
            let mut table_virt = (cr3_phys + hhdm) as *const u64;

            for &level in &levels {
                let shift = 12 + (level - 1) * 9;
                let idx = (virt >> shift) & 0x1FF;
                let entry = *table_virt.add(idx as usize);
                if (entry & 1) == 0 {
                    return 0;
                }
                if (entry & 0x80) != 0 && level > 1 {
                    let page_bits = entry & 0x000FFFFFFFFFFFFF;
                    let huge_shift = 12 + (level - 1) * 9;
                    let huge_mask = !((1u64 << huge_shift) - 1);
                    return (page_bits & huge_mask) | (virt & !huge_mask);
                }
                let next = entry & 0x000FFFFFFFFFF000;
                table_virt = (next + hhdm) as *const u64;
                if level == 1 {
                    return next | (virt & 0xFFF);
                }
            }
        }

        0
    }

    fn new(io_base: u16, mac: [u8; 6], irq_line: u8) -> Option<Self> {
        let virt = unsafe { RX_BUF.0.as_ptr() as u64 };
        let phys = Self::virt_to_phys(virt);
        // RTL8139 is a 32-bit PCI device — can only DMA to <4GB
        if phys == 0 || phys > 0xFFFF_FFFF {
            let s = SerialPort::new(0x3F8);
            s.write_str("[RTL8139] ERROR: RX buffer phys addr >4GB or invalid\n");
            return None;
        }

        Some(Rtl8139 {
            io_base,
            mac,
            ip: [10, 0, 2, 15],
            subnet: [255, 255, 255, 0],
            gateway: [10, 0, 2, 2],
            rx_buf_phys: phys as u32,
            rx_cur: 0,
            tx_cur: 0,
            link_up: false,
            irq_line,
            tx_bounce: [0u8; 1792],
        })
    }

    fn read_mac_from_nic(io_base: u16) -> [u8; 6] {
        let mut mac = [0u8; 6];
        for i in 0..6 {
            unsafe {
                let mut port = Port::<u8>::new(io_base + i);
                mac[i as usize] = port.read();
            }
        }
        mac
    }

    pub fn probe_and_init() -> Option<&'static mut Self> {
        let mut found: Option<(u16, [u8; 6], u8)> = None;

        for i in 0..zenus_arch::pci::MAX_PCI_DEVICES {
            unsafe {
                let dev = &zenus_arch::pci::PCI_DEVICES[i];
                if dev.vendor_id == 0 && dev.device_id == 0 {
                    break;
                }
                if dev.vendor_id == RTL_VENDOR && dev.device_id == RTL_DEVICE {
                    let io_base = (dev.bar0 & 0xFFFFFFF0) as u16;
                    let irq_line = dev.interrupt_line;

                    zenus_arch::pci::enable_bus_master(dev.bus, dev.device, dev.function);

                    let mac = Self::read_mac_from_nic(io_base);
                    found = Some((io_base, mac, irq_line));
                }
            }
        }

        match found {
            Some((io_base, mac, irq_line)) => {
                let s = SerialPort::new(0x3F8);
                s.write_str("[RTL8139] Found at IO 0x");
                s.write_hex(io_base as u64);
                s.write_str(" MAC ");
                for b in &mac {
                    s.write_hex(*b as u64);
                    s.write_str(":");
                }
                s.write_str(" IRQ ");
                s.write_u64(irq_line as u64);
                s.write_str("\n");

                // Route NIC IRQ via I/O APIC if available
                if zenus_arch::interrupts::ioapic::is_initialized() && irq_line > 0 {
                    let vector = 32u8 + irq_line;
                    let apic_id = zenus_arch::interrupts::apic::current_apic_id() as u8;
                    if zenus_arch::interrupts::ioapic::route_irq(irq_line, vector, apic_id) {
                        s.write_str("[IOAPIC] IRQ ");
                        s.write_u64(irq_line as u64);
                        s.write_str(" -> vector ");
                        s.write_u64(vector as u64);
                        s.write_str("\n");
                    } else {
                        s.write_str("[IOAPIC] Failed to route IRQ\n");
                    }
                }

                let nic = match Self::new(io_base, mac, irq_line) {
                    Some(n) => n,
                    None => { return None; }
                };
                NIC_IO_BASE.store(io_base, core::sync::atomic::Ordering::Relaxed);
                zenus_arch::interrupts::handler::set_nic_irq_handler(Self::handle_irq);
                unsafe {
                    RTL_IFACE = Some(nic);
                    RTL_IFACE.as_mut().map(|n| {
                        n.init_hw();
                        n.link_up = true;
                        n
                    })
                }
            }
            None => {
                let s = SerialPort::new(0x3F8);
                s.write_str("[RTL8139] No RTL8139 NIC found\n");
                None
            }
        }
    }

    fn write8(&self, reg: u16, val: u8) {
        unsafe {
            let mut port = Port::<u8>::new(self.io_base + reg);
            port.write(val);
        }
    }

    fn read8(&self, reg: u16) -> u8 {
        unsafe {
            let mut port = Port::<u8>::new(self.io_base + reg);
            port.read()
        }
    }

    fn write16(&self, reg: u16, val: u16) {
        unsafe {
            let mut port = Port::<u16>::new(self.io_base + reg);
            port.write(val);
        }
    }

    fn read16(&self, reg: u16) -> u16 {
        unsafe {
            let mut port = Port::<u16>::new(self.io_base + reg);
            port.read()
        }
    }

    fn read_cbr(&self) -> u16 {
        self.read8(RTL_CBR) as u16 | ((self.read8(RTL_CBR + 1) as u16) << 8)
    }

    fn write32(&self, reg: u16, val: u32) {
        unsafe {
            let mut port = Port::<u32>::new(self.io_base + reg);
            port.write(val);
        }
    }

    fn read32(&self, reg: u16) -> u32 {
        unsafe {
            let mut port = Port::<u32>::new(self.io_base + reg);
            port.read()
        }
    }

    pub fn reset(&self) {
        self.write8(RTL_CR, CR_RST);

        for _ in 0..50000 {
            unsafe { Port::<u8>::new(0x80).read(); }
        }

        let s = SerialPort::new(0x3F8);
        s.write_str("[RTL8139] Reset done\n");
    }

    pub fn init_hw(&self) {
        let s = SerialPort::new(0x3F8);
        s.write_str("[RTL8139] rx_buf_phys=0x");
        s.write_hex(self.rx_buf_phys as u64);
        s.write_str(" virt=0x");
        s.write_hex(unsafe { RX_BUF.0.as_ptr() as u64 });
        s.write_str("\n");
        self.write32(RTL_RBSTART, self.rx_buf_phys);
        self.write16(RTL_IMR, ISR_ROK | ISR_TOK | ISR_RER | ISR_TER);
        let rcr = 0x8BD;
        self.write32(RTL_RCR, rcr);
        self.write8(RTL_CR, CR_TE | CR_RE);

        let s = SerialPort::new(0x3F8);
        s.write_str("[RTL8139] rx_buf_phys=0x");
        s.write_hex(self.rx_buf_phys as u64);
        s.write_str(" RB=0x");
        s.write_hex(self.read32(RTL_RBSTART) as u64);
        s.write_str(" RCR=0x");
        s.write_hex(self.read32(RTL_RCR) as u64);
        s.write_str(" CR=0x");
        s.write_hex(self.read8(RTL_CR) as u64);
        s.write_str(" ISR=0x");
        s.write_hex(self.read16(RTL_ISR) as u64);
        s.write_str(" CBR=0x");
        s.write_hex(self.read_cbr() as u64);
        s.write_str("\n");
    }

    pub fn mac(&self) -> &[u8; 6] { &self.mac }
    pub fn ip(&self) -> &[u8; 4] { &self.ip }
    pub fn is_link_up(&self) -> bool { self.link_up }

    pub fn set_ip(&mut self, ip: [u8; 4]) { self.ip = ip; }
    pub fn set_subnet(&mut self, subnet: [u8; 4]) { self.subnet = subnet; }
    pub fn set_gateway(&mut self, gateway: [u8; 4]) { self.gateway = gateway; }

    pub fn send_raw(&mut self, data: &[u8]) -> bool {
        if data.len() > 1792 {
            return false;
        }

        let tsd_addr = RTL_TSD0 + (self.tx_cur as u16 * 4);
        let tsad_addr = RTL_TSAD0 + (self.tx_cur as u16 * 4);

        self.tx_bounce[..data.len()].copy_from_slice(data);
        let virt_addr = self.tx_bounce.as_ptr() as u64;
        let phys_addr = Self::virt_to_phys(virt_addr);
        if phys_addr == 0 || phys_addr > 0xFFFF_FFFFu64 {
            return false;
        }

        self.write32(tsad_addr, phys_addr as u32);
        self.write32(tsd_addr, data.len() as u32 & TSD_SIZE_MASK);

        // Wait for DMA to complete so caller's buffer (possibly stack) stays valid
        for _ in 0..50000 {
            let tsd = self.read32(tsd_addr);
            if (tsd & TSD_TOK) != 0 {
                break;
            }
            unsafe { Port::<u8>::new(0x80).read(); }
        }

        self.tx_cur = (self.tx_cur + 1) % TX_DESC_COUNT;
        true
    }

    pub fn send_eth(&mut self, dst_mac: &[u8; 6], ether_type: u16, payload: &[u8]) -> bool {
        let total_len = 14 + payload.len();
        if total_len > 1792 {
            return false;
        }

        let mut buf = [0u8; 1792];
        buf[0..6].copy_from_slice(dst_mac);
        buf[6..12].copy_from_slice(&self.mac);
        buf[12..14].copy_from_slice(&ether_type.to_be_bytes());
        buf[14..total_len].copy_from_slice(payload);

        if total_len < 60 {
            let pad_len = 60 - total_len;
            for i in 0..pad_len {
                buf[total_len + i] = 0;
            }
            self.send_raw(&buf[..60])
        } else {
            self.send_raw(&buf[..total_len])
        }
    }

    pub fn receive_copy(&mut self, buf: &mut [u8]) -> Option<usize> {
        let isr = self.read16(RTL_ISR);
        let cbr = self.read_cbr();
        {
            let s = SerialPort::new(0x3F8);
            s.write_str("[RX] ISR=");
            s.write_hex(isr as u64);
            s.write_str(" CAPR=");
            s.write_hex(cbr as u64);
            s.write_str(" cur=");
            s.write_hex(self.rx_cur as u64);
            s.write_str("\n");
        }
        if isr == 0 || (isr & ISR_ROK) == 0 {
            return None;
        }
        self.write16(RTL_ISR, isr & !ISR_ROK);

        let rx_buf = unsafe { &RX_BUF.0[..RX_BUF_SIZE] };

        if self.rx_cur == cbr {
            return None;
        }

        let offset = (self.rx_cur as usize) % RX_BUF_SIZE;
        if offset + 4 > RX_BUF_SIZE {
            self.rx_cur = 0;
            return None;
        }

        let rx_status = u32::from_le_bytes([
            rx_buf[offset], rx_buf[offset + 1],
            rx_buf[offset + 2], rx_buf[offset + 3],
        ]);
        let pkt_len = ((rx_status >> 16) as usize) & 0x3FFF;

        if pkt_len == 0 || pkt_len > RX_BUF_SIZE {
            self.rx_cur = (self.rx_cur + 4) & (RX_BUF_SIZE as u16 - 1);
            self.write16(RTL_CAPR, self.rx_cur);
            return None;
        }

        let copy_len = core::cmp::min(pkt_len.saturating_sub(4), 1514);
        let pkt_start = (offset + 4) % RX_BUF_SIZE;

        if pkt_start + copy_len <= RX_BUF_SIZE {
            let copy_end = core::cmp::min(copy_len, buf.len());
            buf[..copy_end].copy_from_slice(&rx_buf[pkt_start..pkt_start + copy_end]);
        } else {
            let first = RX_BUF_SIZE - pkt_start;
            let copy_first = core::cmp::min(first, buf.len());
            buf[..copy_first].copy_from_slice(&rx_buf[pkt_start..pkt_start + copy_first]);
            if copy_first < buf.len() {
                let second = core::cmp::min(copy_len - first, buf.len() - copy_first);
                buf[copy_first..copy_first + second].copy_from_slice(&rx_buf[..second]);
            }
        }

        self.rx_cur = ((self.rx_cur as u16) + 4 + ((pkt_len as u16 + 3) & !3)) % RX_BUF_SIZE as u16;
        self.write16(RTL_CAPR, self.rx_cur);

        Some(core::cmp::min(copy_len, buf.len()))
    }

    pub fn with_nic<R>(f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let _guard = RTL_LOCK.lock();
        unsafe { RTL_IFACE.as_mut().map(f) }
    }

    pub fn handle_irq() {
    let _rtl_guard = RTL_LOCK.lock_no_irq();
    let io_base = NIC_IO_BASE.load(core::sync::atomic::Ordering::Relaxed);
    if io_base == 0 { return; }
    unsafe {
        let mut isr_port = x86_64::instructions::port::Port::<u16>::new(io_base + RTL_ISR);
        let isr = isr_port.read();
        if isr != 0 {
            isr_port.write(isr);
            if (isr & ISR_ROK) != 0 {
                if let Some(ref mut nic) = RTL_IFACE {
                    nic.process_rx();
                }
            }
        }
    }
}

    pub fn poll(&mut self) {
        let _rtl_guard = RTL_LOCK.lock_no_irq();
        let isr = self.read16(RTL_ISR);
        if isr != 0 {
            self.write16(RTL_ISR, isr);
            if (isr & ISR_ROK) != 0 {
                self.process_rx();
            }
        }
    }

    fn process_rx(&mut self) {
        let rx_buf = unsafe { &RX_BUF.0[..RX_BUF_SIZE] };
        for _ in 0..256 {
            let offset = (self.rx_cur as usize) % RX_BUF_SIZE;
            if offset + 4 > RX_BUF_SIZE {
                self.rx_cur = 0;
                continue;
            }

            let hdr = u32::from_le_bytes([
                rx_buf[offset], rx_buf[offset + 1],
                rx_buf[offset + 2], rx_buf[offset + 3],
            ]);
            let pkt_len = ((hdr >> 16) as usize) & 0x3FFF;

            if pkt_len == 0 || pkt_len > RX_BUF_SIZE {
                break;
            }

            let copy_len = core::cmp::min(pkt_len.saturating_sub(4), 1514);
            let pkt_start = (offset + 4) % RX_BUF_SIZE;

            let mut packet = [0u8; 1514];
            if pkt_start + copy_len <= RX_BUF_SIZE {
                packet[..copy_len].copy_from_slice(&rx_buf[pkt_start..pkt_start + copy_len]);
            } else {
                let first = RX_BUF_SIZE - pkt_start;
                packet[..first].copy_from_slice(&rx_buf[pkt_start..]);
                packet[first..copy_len].copy_from_slice(&rx_buf[..copy_len - first]);
            }

            self.rx_cur = ((self.rx_cur as u16) + 4 + ((pkt_len as u16 + 3) & !3)) % RX_BUF_SIZE as u16;
            let capr_val = self.rx_cur.saturating_sub(0x10);
            self.write16(RTL_CAPR, capr_val);

            self.handle_packet(&packet[..copy_len]);
        }
    }

    fn handle_packet(&mut self, data: &[u8]) {
        if data.len() < 14 {
            return;
        }
        let (eth_hdr, eth_payload) = match ethernet::parse(data) {
            Some(h) => h,
            None => return,
        };

        match eth_hdr.ether_type {
            ethernet::ETH_ARP => {
                if let Some(arp_resp) = arp::handle(&eth_hdr, eth_payload, &self.ip, &self.mac) {
                    self.send_raw(&arp_resp);
                }
            }
            ethernet::ETH_IPV4 => {
                if let Some((ip_hdr, ip_payload)) = crate::ipv4::parse(eth_payload) {
                    if ip_hdr.dst_ip != self.ip && ip_hdr.dst_ip != [255; 4] {
                        return;
                    }
                    match ip_hdr.protocol {
                        crate::ipv4::PROTO_ICMP => {
                            if let Some(reply) = icmp::handle_echo(
                                &ip_hdr, ip_payload, &self.mac, &eth_hdr.src_mac, &self.ip
                            ) {
                                self.send_raw(&reply);
                            }
                        }
                        crate::ipv4::PROTO_TCP => {
                            crate::tcp::handle_receive(1, ip_hdr.src_ip, ip_hdr.dst_ip, ip_payload);
                        }
                        crate::ipv4::PROTO_UDP => {
                            crate::udp::handle_receive(1, ip_hdr.src_ip, ip_hdr.dst_ip, ip_payload);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

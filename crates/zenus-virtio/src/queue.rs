use core::sync::atomic::Ordering;

use zenus_mem::paging;
use crate::QUEUE_SIZE;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct VirtioDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

pub const VRING_DESC_F_NEXT: u16 = 1;
pub const VRING_DESC_F_WRITE: u16 = 2;

#[repr(C, align(2))]
pub struct VirtioAvail {
    pub flags: u16,
    pub idx: u16,
    pub ring: [u16; QUEUE_SIZE],
}

#[repr(C, align(4))]
#[derive(Clone, Copy)]
pub struct VirtioUsedElem {
    pub id: u32,
    pub len: u32,
}

#[repr(C, align(4))]
pub struct VirtioUsed {
    pub flags: u16,
    pub idx: u16,
    pub ring: [VirtioUsedElem; QUEUE_SIZE],
}

#[repr(C, align(4096))]
pub struct VirtioQueueMem {
    pub desc: [VirtioDesc; QUEUE_SIZE],
    pub avail: VirtioAvail,
    pub used: VirtioUsed,
}

impl VirtioQueueMem {
    pub const fn new() -> Self {
        VirtioQueueMem {
            desc: [VirtioDesc { addr: 0, len: 0, flags: 0, next: 0 }; QUEUE_SIZE],
            avail: VirtioAvail { flags: 0, idx: 0, ring: [0; QUEUE_SIZE] },
            used: VirtioUsed { flags: 0, idx: 0, ring: [VirtioUsedElem { id: 0, len: 0 }; QUEUE_SIZE] },
        }
    }
}

pub struct VirtioQueue {
    pub mem: &'static mut VirtioQueueMem,
    pub size: u16,
    pub queue_idx: u16,
    pub free_head: u16,
    pub last_seen_used: u16,
    pub notify_base: u64,
    pub desc_phys: u64,
}

impl VirtioQueue {
    pub unsafe fn new(
        mem: &'static mut VirtioQueueMem,
        size: u16,
        queue_idx: u16,
        notify_base: u64,
        cr3: u64,
    ) -> Self {
        let virt = mem as *mut VirtioQueueMem as u64;
        let desc_phys = paging::virt_to_phys_raw(cr3, virt).unwrap_or(0);
        VirtioQueue {
            mem,
            size,
            queue_idx,
            free_head: 0,
            last_seen_used: 0,
            notify_base,
            desc_phys,
        }
    }

    pub unsafe fn alloc_desc(&mut self) -> Option<u16> {
        if self.free_head >= self.size {
            return None;
        }
        let idx = self.free_head;
        self.free_head = (self.free_head + 1) % self.size;
        Some(idx)
    }

    pub unsafe fn submit(&mut self, head_idx: u16) {
        let avail_idx = (*self.mem).avail.idx;
        (*self.mem).avail.ring[(avail_idx as usize) % self.size as usize] = head_idx;
        core::sync::atomic::compiler_fence(Ordering::Release);
        (*self.mem).avail.idx = avail_idx.wrapping_add(1);
    }

    pub unsafe fn kick(&self) {
        core::sync::atomic::compiler_fence(Ordering::Release);
        let notify_virt = self.notify_base as *mut u16;
        notify_virt.write_volatile(self.queue_idx);
    }

    pub unsafe fn collect_used(&mut self) -> Option<(u32, u32)> {
        let used_idx = (*self.mem).used.idx;
        if self.last_seen_used == used_idx {
            return None;
        }
        let slot = self.last_seen_used as usize % self.size as usize;
        let id = (*self.mem).used.ring[slot].id;
        let len = (*self.mem).used.ring[slot].len;
        self.last_seen_used = self.last_seen_used.wrapping_add(1);
        Some((id, len))
    }
}

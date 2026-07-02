use core::sync::atomic::{AtomicU32, Ordering};

use crate::limine::{self, LimineMpInfo};

static CPU_COUNT: AtomicU32 = AtomicU32::new(1);
static AP_READY_COUNT: AtomicU32 = AtomicU32::new(0);

static mut AP_IDLE_FN: Option<fn() -> !> = None;

pub fn set_ap_idle_fn(f: fn() -> !) {
    unsafe { AP_IDLE_FN = Some(f); }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CpuInfo {
    pub apic_id: u32,
    pub cpu_number: u32,
}

const MAX_CPUS: usize = 64;
static mut CPU_TABLE: [CpuInfo; MAX_CPUS] = [CpuInfo { apic_id: 0, cpu_number: 0 }; MAX_CPUS];

pub fn init() {
    if limine::MP_REQUEST.response.is_null() {
        zenus_console::kwarn!("MP request not supported, assuming 1 CPU");
        return;
    }

    let resp: &limine::LimineMpResponse =
        unsafe { &*limine::MP_REQUEST.response.as_ptr() };
    let count = resp.cpu_count as u32;
    CPU_COUNT.store(count, Ordering::Relaxed);

    let info_ptrs: *mut *mut LimineMpInfo = resp.cpus.0 as *mut *mut LimineMpInfo;
    unsafe {
        for i in 0..(count as usize) {
            let info = &**info_ptrs.add(i);
            CPU_TABLE[i] = CpuInfo {
                apic_id: info.lapic_id,
                cpu_number: i as u32,
            };
        }
    }

    zenus_console::kinfo!("Detected {} CPU(s)", count);
}

pub fn wake_aps() {
    if limine::MP_REQUEST.response.is_null() {
        return;
    }

    let resp: &limine::LimineMpResponse =
        unsafe { &*limine::MP_REQUEST.response.as_ptr() };
    if resp.cpu_count <= 1 {
        return;
    }

    let bsp_lapic_id = resp.bsp_lapic_id;
    let info_ptrs: *mut *mut LimineMpInfo = resp.cpus.0 as *mut *mut LimineMpInfo;
    let total = resp.cpu_count as usize;

    for i in 0..total {
        let info_ptr: *mut LimineMpInfo = unsafe { *info_ptrs.add(i) };
        let info = unsafe { &mut *info_ptr };
        if info.lapic_id == bsp_lapic_id {
            continue;
        }
        info.goto_address = ap_entry as extern "C" fn(&LimineMpInfo) -> ! as u64;
    }
    // Ensure all APs see the updated goto_address
    core::sync::atomic::fence(Ordering::Release);

    let ap_count = (total - 1) as u32;
    zenus_console::kinfo!("Waiting for APs to start...");
    while AP_READY_COUNT.load(Ordering::Acquire) < ap_count {
        core::hint::spin_loop();
    }
    zenus_console::kinfo!("All APs started");
}

pub extern "C" fn ap_entry(info: &LimineMpInfo) -> ! {
    crate::cpu::enable_sse();
    crate::gdt::init_ap();
    let cpu_id = cpu_number_for_apic(info.lapic_id);
    crate::cpu::init_syscall_ap(cpu_id);

    let apic_base = unsafe { crate::cpu::read_msr(0x1B) & 0xFFFFF000 };
    let hhdm_offset = crate::limine::hhdm_offset();
    crate::interrupts::apic::init_ap(apic_base + hhdm_offset);

    x86_64::instructions::interrupts::enable();
    crate::interrupts::apic::init_timer_ap(48);

    AP_READY_COUNT.fetch_add(1, Ordering::Release);
    zenus_console::kinfo!("AP CPU started");

    let idle_fn = unsafe { AP_IDLE_FN };
    if let Some(f) = idle_fn {
        f()
    } else {
        loop { x86_64::instructions::hlt(); }
    }
}

pub fn cpu_number_for_apic(lapic_id: u32) -> u32 {
    let count = CPU_COUNT.load(Ordering::Relaxed) as usize;
    if count == 0 || count > MAX_CPUS { return 0; }
    unsafe {
        for i in 0..count {
            if CPU_TABLE[i].apic_id == lapic_id {
                return i as u32;
            }
        }
    }
    0
}

pub fn cpu_count() -> u32 {
    CPU_COUNT.load(Ordering::Relaxed)
}

pub fn current_cpu() -> u32 {
    let count = CPU_COUNT.load(Ordering::Relaxed) as usize;
    if count == 0 { return 0; }
    if count > MAX_CPUS { return 0; }
    let apic_id = crate::interrupts::apic::current_apic_id();
    unsafe {
        for i in 0..count {
            if CPU_TABLE[i].apic_id == apic_id {
                return i as u32;
            }
        }
    }
    0
}

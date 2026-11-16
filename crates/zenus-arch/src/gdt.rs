use core::mem::MaybeUninit;
use x86_64::instructions::segmentation::{Segment, CS, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

// GDT capacity per CPU: we need 7 entries (NULL + 4 UserSegments + 2 TSS).
// Use 16 for headroom (IST hooks, future expansion, NMIs).
const GDT_CAPACITY: usize = 16;
type GdtTable = GlobalDescriptorTable<GDT_CAPACITY>;

pub const KERNEL_CODE: SegmentSelector = SegmentSelector::new(1, x86_64::PrivilegeLevel::Ring0);
pub const KERNEL_DATA: SegmentSelector = SegmentSelector::new(2, x86_64::PrivilegeLevel::Ring0);
pub const USER_CODE: SegmentSelector = SegmentSelector::new(4, x86_64::PrivilegeLevel::Ring3);
pub const USER_DATA: SegmentSelector = SegmentSelector::new(3, x86_64::PrivilegeLevel::Ring3);
pub const TSS_SEL: SegmentSelector = SegmentSelector::new(5, x86_64::PrivilegeLevel::Ring0);

// IST index into TaskStateSegment.interrupt_stack_table (0-indexed).
// The IDT set_stack_index() takes a 1-based IST index (1-7), while
// x86_64 crate maps interrupt_stack_table as 0-indexed array where
//   set_stack_index(1) → CPU reads interrupt_stack_table[0]
//   set_stack_index(2) → CPU reads interrupt_stack_table[1]
//   ...
// See Intel SDM Vol 3, Figure 7-11 (TSS format: IST[1] at offset 36).
pub const DF_IST_IDX: usize = 0; // set_stack_index(1) → CPU reads interrupt_stack_table[0]
pub const TIMER_IST_IDX: usize = 1; // set_stack_index(2) → CPU reads interrupt_stack_table[1]
                                    // See Intel SDM Vol 3, Figure 7-11 (TSS format): IST[1] at TSS offset 36.
                                    // x86_64 crate maps interrupt_stack_table as 0-indexed array where
                                    //   set_stack_index(N) → CPU reads interrupt_stack_table[N-1]

const MAX_CPUS: usize = 8;
const AP_STACK_SIZE: usize = 4096 * 32;
const AP_DF_STACK_SIZE: usize = 4096 * 8;
const AP_TIMER_STACK_SIZE: usize = 4096 * 4;

#[allow(static_mut_refs)]
static mut TSS: MaybeUninit<TaskStateSegment> = MaybeUninit::uninit();
#[allow(static_mut_refs)]
static mut GDT: MaybeUninit<GdtTable> = MaybeUninit::uninit();

#[allow(static_mut_refs)]
static mut AP_STACKS: [[u8; AP_STACK_SIZE]; MAX_CPUS] = [[0; AP_STACK_SIZE]; MAX_CPUS];
#[allow(static_mut_refs)]
static mut AP_DF_STACKS: [[u8; AP_DF_STACK_SIZE]; MAX_CPUS] = [[0; AP_DF_STACK_SIZE]; MAX_CPUS];
#[allow(static_mut_refs)]
static mut AP_TIMER_STACKS: [[u8; AP_TIMER_STACK_SIZE]; MAX_CPUS] =
    [[0; AP_TIMER_STACK_SIZE]; MAX_CPUS];
#[allow(static_mut_refs)]
static mut AP_TSS: [MaybeUninit<TaskStateSegment>; MAX_CPUS] =
    [const { MaybeUninit::uninit() }; MAX_CPUS];
#[allow(static_mut_refs)]
static mut AP_GDT: [MaybeUninit<GdtTable>; MAX_CPUS] = [const { MaybeUninit::uninit() }; MAX_CPUS];

pub fn init() {
    let tss = unsafe { &mut *TSS.as_mut_ptr() };
    tss.privilege_stack_table[0] = {
        const STACK_SIZE: usize = 4096 * 32;
        static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
        let top_addr = unsafe { STACK.as_ptr().add(STACK_SIZE) };
        VirtAddr::from_ptr(top_addr)
    };
    // DF IST: set_stack_index(1) → CPU reads interrupt_stack_table[0]
    // PREVIOUSLY (BUG): wrote to [DF_IST_IDX + 1], CPU read uninitialized [0]
    tss.interrupt_stack_table[DF_IST_IDX] = {
        const DF_STACK_SIZE: usize = 4096 * 8;
        static mut DF_STACK: [u8; DF_STACK_SIZE] = [0; DF_STACK_SIZE];
        let top_addr = unsafe { DF_STACK.as_ptr().add(DF_STACK_SIZE) };
        VirtAddr::from_ptr(top_addr)
    };
    // Timer IST: set_stack_index(2) → CPU reads interrupt_stack_table[1]
    tss.interrupt_stack_table[TIMER_IST_IDX] = {
        const TIMER_STACK_SIZE: usize = 4096 * 4;
        static mut TIMER_STACK: [u8; TIMER_STACK_SIZE] = [0; TIMER_STACK_SIZE];
        let top_addr = unsafe { TIMER_STACK.as_ptr().add(TIMER_STACK_SIZE) };
        VirtAddr::from_ptr(top_addr)
    };

    let mut gdt = GdtTable::empty();
    gdt.append(Descriptor::kernel_code_segment());
    gdt.append(Descriptor::kernel_data_segment());
    gdt.append(Descriptor::user_data_segment());
    gdt.append(Descriptor::user_code_segment());
    gdt.append(Descriptor::tss_segment(unsafe { &*TSS.as_ptr() }));

    unsafe {
        GDT.as_mut_ptr().write(gdt);
        (*GDT.as_ptr()).load();
        CS::set_reg(KERNEL_CODE);
        SS::set_reg(KERNEL_DATA);
        load_tss(TSS_SEL);
    }
}

pub fn init_ap(cpu: u32) {
    let idx = (cpu as usize).min(MAX_CPUS - 1);

    let tss = unsafe { &mut *AP_TSS[idx].as_mut_ptr() };
    *tss = TaskStateSegment::new();
    tss.privilege_stack_table[0] = {
        let top_addr = unsafe { AP_STACKS[idx].as_ptr().add(AP_STACK_SIZE) };
        VirtAddr::from_ptr(top_addr)
    };
    // DF IST: set_stack_index(1) → CPU reads interrupt_stack_table[0]
    tss.interrupt_stack_table[DF_IST_IDX] = {
        let top_addr = unsafe { AP_DF_STACKS[idx].as_ptr().add(AP_DF_STACK_SIZE) };
        VirtAddr::from_ptr(top_addr)
    };
    // Timer IST: set_stack_index(2) → CPU reads interrupt_stack_table[1]
    tss.interrupt_stack_table[TIMER_IST_IDX] = {
        let top_addr = unsafe { AP_TIMER_STACKS[idx].as_ptr().add(AP_TIMER_STACK_SIZE) };
        VirtAddr::from_ptr(top_addr)
    };

    let gdt = unsafe { &mut *AP_GDT[idx].as_mut_ptr() };
    *gdt = GdtTable::empty();
    gdt.append(Descriptor::kernel_code_segment());
    gdt.append(Descriptor::kernel_data_segment());
    gdt.append(Descriptor::user_data_segment());
    gdt.append(Descriptor::user_code_segment());
    gdt.append(Descriptor::tss_segment(unsafe { &*AP_TSS[idx].as_ptr() }));

    unsafe {
        gdt.load();
        CS::set_reg(KERNEL_CODE);
        SS::set_reg(KERNEL_DATA);
        load_tss(TSS_SEL);
    }
}

pub fn set_tss_stack(stack_ptr: VirtAddr) {
    let cpu = crate::smp::current_cpu();
    let idx = (cpu as usize).min(MAX_CPUS - 1);
    if idx == 0 {
        let tss = unsafe { &mut *TSS.as_mut_ptr() };
        tss.privilege_stack_table[0] = stack_ptr;
    } else {
        let tss = unsafe { &mut *AP_TSS[idx].as_mut_ptr() };
        tss.privilege_stack_table[0] = stack_ptr;
    }
}

/// BSP GDT virtual address range, for KPTI user-view mapping.
///
/// The CPU reads segment descriptors (including the CS/SS reload performed by a
/// plain `iretq` back to ring 3) from the GDT at the GDTR linear address,
/// through whatever CR3 is loaded. A user view zeroes the kernel half, so the
/// GDT (a kernel BSS page) must be remapped into it as a supervisor page.
pub fn gdt_range() -> (u64, u64) {
    let start = core::ptr::addr_of!(GDT) as u64;
    (start, core::mem::size_of::<GdtTable>() as u64)
}

/// BSP TSS virtual address range, for KPTI user-view mapping.
///
/// A ring-3 → ring-0 transition (any interrupt/exception) reads `TSS.RSP0` to
/// find the kernel stack, via the TR → GDT descriptor → TSS linear address,
/// through whatever CR3 is loaded. A user view zeroes the kernel half, so the
/// TSS (a kernel BSS page) must be remapped into it as a supervisor page.
pub fn tss_range() -> (u64, u64) {
    let start = core::ptr::addr_of!(TSS) as u64;
    (start, core::mem::size_of::<TaskStateSegment>() as u64)
}

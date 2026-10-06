use core::mem::MaybeUninit;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use zenus_console::serial::SerialPort;

use crate::gdt;
use crate::fuzz_guard;

/// Fuzzing containment hook. Every CPU exception handler must call this
/// *before* doing anything else: while a fuzz checkpoint is armed the fault is
/// a test-case result, not a system failure, and the handler must resume the
/// campaign instead of panicking. Returns true when the handler should stop —
/// the call then never returns.
#[inline]
fn fuzz_contain(vector: u64, frame: &InterruptStackFrame, addr: u64, err: u64) -> bool {
    if !fuzz_guard::should_recover() {
        return false;
    }
    // SAFETY: `should_recover` returned true, so a checkpoint is armed on this
    // CPU. `recover_to_checkpoint` restores that checkpoint and never returns.
    unsafe {
        fuzz_guard::recover_to_checkpoint(vector, frame.instruction_pointer.as_u64(), addr, err);
    }
}

fn is_kernel_addr(addr: u64) -> bool {
    addr >= 0xFFFF800000000000
}

/// Read a faulting task's stack word without ever faulting ourselves.
///
/// The dump below walks addresses taken from the interrupted frame's `RSP`,
/// which for a ring-3 task is a *user* address. Two things used to go wrong:
///
/// * The address was read with a bare `read_volatile`. With SMAP off, ring 0 can
///   read a user page, so this worked by accident. With SMAP on it is a nested
///   `#PF` inside the page-fault handler, the handler re-enters itself, and the
///   machine wedges in a silent loop — which is why SMAP could not simply be
///   switched on and why the original dump printed one header and then stopped.
/// * Even without SMAP, nothing checked that the address was mapped. `RSP` on a
///   half-set-up task points below its lowest mapped page, and the bare read
///   faulted.
///
/// So: walk the page tables first, and read the *physical* address through the
/// HHDM. The direct map is supervisor-only, so the access needs no `stac`, and
/// every frame the allocator hands out came out of a Limine usable region, which
/// is exactly what the HHDM covers.
fn read_via_hhdm(addr: u64, width: u64) -> Option<u64> {
    let hhdm = zenus_mem::paging::hhdm_offset();
    if hhdm == 0 {
        return None;
    }
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }
    let phys = zenus_mem::paging::virt_to_phys_raw(cr3, addr)?;
    let p = (hhdm + phys) as *const u8;
    Some(match width {
        1 => (unsafe { core::ptr::read_volatile(p) }) as u64,
        8 => unsafe { core::ptr::read_volatile(p as *const u64) },
        _ => return None,
    })
}

fn try_read_u64(addr: u64) -> Option<u64> {
    if addr < 0x1000 {
        return None;
    }
    // A u64 read that straddles a page boundary could cross into an unmapped
    // page even when `addr` itself translates.
    if (addr & 0xFFF) > 0xFF8 {
        return None;
    }
    read_via_hhdm(addr, 8)
}

/// Tulis angka u64 sebagai hex ke display (framebuffer/VGA). Format: 0x1234.
fn write_hex_display(v: u64) {
    let hex_chars = b"0123456789ABCDEF";
    let mut nibbles = [0u8; 16];
    for i in 0..16 {
        nibbles[i] = hex_chars[((v >> (60 - i * 4)) & 0xF) as usize];
    }
    let mut start = 0;
    while start < 15 && nibbles[start] == b'0' {
        start += 1;
    }
    // Always show at least one digit
    let display = &nibbles[start..];
    let mut buf = [0u8; 19];
    buf[0] = b'0';
    buf[1] = b'x';
    let len = display.len().min(16);
    buf[2..2 + len].copy_from_slice(&display[..len]);
    let total = 2 + len;
    if let Ok(s) = core::str::from_utf8(&buf[..total]) {
        zenus_console::display::write_str(s);
    }
}

fn try_read_u8(addr: u64) -> Option<u8> {
    if addr < 0x1000 {
        return None;
    }
    read_via_hhdm(addr, 1).map(|v| v as u8)
}

#[allow(static_mut_refs)]
static mut IDT: MaybeUninit<InterruptDescriptorTable> = MaybeUninit::uninit();

pub fn init() {
    let idt = unsafe { &mut *IDT.as_mut_ptr() };

    idt.divide_error.set_handler_fn(divide_error_handler);
    idt.debug.set_handler_fn(debug_handler);
    idt.non_maskable_interrupt.set_handler_fn(nmi_handler);
    // BUG FIX: breakpoint DPL must be 3 for int3 to work at Ring 3.
    // Default DPL is 0, which causes #GP when user-mode code executes int3.
    // Zenus linker adds int3 padding after function bodies, so any code
    // that falls through past a syscall (or other instruction) will hit
    // int3 padding. Without DPL=3, this becomes a silent #GP instead of
    // a debuggable breakpoint trap.
    idt.breakpoint
        .set_handler_fn(breakpoint_handler)
        .set_privilege_level(x86_64::PrivilegeLevel::Ring3);
    idt.overflow.set_handler_fn(overflow_handler);
    idt.bound_range_exceeded.set_handler_fn(bound_range_handler);
    idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
    idt.device_not_available
        .set_handler_fn(device_not_available_handler);

    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index((gdt::DF_IST_IDX + 1) as u16);
    }

    idt.invalid_tss.set_handler_fn(invalid_tss_handler);
    idt.segment_not_present
        .set_handler_fn(segment_not_present_handler);
    idt.stack_segment_fault
        .set_handler_fn(stack_segment_handler);
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.x87_floating_point.set_handler_fn(x87_fp_handler);
    idt.alignment_check.set_handler_fn(alignment_check_handler);
    idt.machine_check.set_handler_fn(machine_check_handler);
    idt.simd_floating_point.set_handler_fn(simd_fp_handler);
    idt.virtualization.set_handler_fn(virtualization_handler);

    // IRQ 0-15 mapped to vectors 32-47
    // Vector 32: preemption timer (LAPIC timer). Runs on the dedicated IST
    // stack — see the comment on the entry below.
    unsafe {
        extern "C" {
            static apic_timer_isr_stub: u8;
        }
        let addr = &apic_timer_isr_stub as *const u8 as u64;
        idt[32]
            .set_handler_addr(x86_64::VirtAddr::new(addr))
            .disable_interrupts(true)
            .set_privilege_level(x86_64::PrivilegeLevel::Ring0);
        // NOTE: deliberately NO IST here. With an IST stack the CPU pushes
        // the frame on the shared per-CPU IST stack, so every task's saved
        // context would live at the same address and the next timer tick
        // overwrote it (frame with a garbage CS, kernel dies). The TSS.RSP0
        // design is what keeps each task's saved context on its own stack.
    }
    idt[33].set_handler_fn(super::handler::interrupt_keyboard);
    idt[39].set_handler_fn(super::handler::interrupt_spurious);

    // Every remaining vector gets an ack-and-return handler. A zeroed IDT
    // slot makes the CPU jump to address 0 on the first stray interrupt
    // (QEMU raises IRQ7 spuriously and a PIC EOI from the wrong CPU can
    // re-aim a line), which showed up as APs executing garbage.
    // SAFETY: the IDT is a `static mut` and this is the only place that writes
    // the stray-vector slots, during single-threaded `init()`.
    unsafe {
        let stray = super::handler::interrupt_stray as *const () as u64;
        for vec in 34u8..=255u8 {
            if super::RESERVED_VECTORS.contains(&vec) {
                continue;
            }
            idt[vec].set_handler_addr(x86_64::VirtAddr::new(stray));
        }
    }

    // NIC interrupt — vector is fixed (see `interrupts::NIC_VECTOR`), the
    // driver routes whatever IRQ line the device reports onto it.
    idt[super::NIC_VECTOR].set_handler_fn(super::handler::interrupt_nic);

    // Serial (UART) interrupt (IRQ 4 for COM1)
    idt[super::SERIAL_VECTOR].set_handler_fn(super::handler::interrupt_serial);

    idt.load();
}

/// Load the kernel IDT on a secondary CPU (AP).
///
/// IDTR is a per-CPU register. `init()` only loads it on the BSP; an AP
/// that takes any interrupt/exception with a null IDTR double-faults and
/// then triple-faults (observed in qemu `-d cpu_reset`: IDT=00000000
/// on every AP during wake_aps). Call this on each AP after `gdt::init_ap()`
/// (the double-fault/timer entries use IST, which needs the AP's TSS
/// loaded) and before enabling interrupts.
pub fn load_ap() {
    unsafe {
        let idt = &*IDT.as_ptr();
        idt.load();
    }
}

extern "x86-interrupt" fn divide_error_handler(frame: InterruptStackFrame) {
    if fuzz_contain(0, &frame, 0, 0) {}
    kpanic("Divide Error", frame);
}

extern "x86-interrupt" fn debug_handler(frame: InterruptStackFrame) {
    if fuzz_contain(1, &frame, 0, 0) {}
    kpanic("Debug", frame);
}

extern "x86-interrupt" fn nmi_handler(_frame: InterruptStackFrame) {
    let s = SerialPort::new(0x3F8);
    s.write_str("!!! NMI !!!\n");
    zenus_console::display::write_str("\n!!! NMI !!!\n");
    zenus_console::serial::flush_output_blocking();
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn breakpoint_handler(_frame: InterruptStackFrame) {
    SerialPort::new(0x3F8).write_str("Breakpoint\n");
}

extern "x86-interrupt" fn overflow_handler(frame: InterruptStackFrame) {
    if fuzz_contain(4, &frame, 0, 0) {}
    kpanic("Overflow", frame);
}

extern "x86-interrupt" fn bound_range_handler(frame: InterruptStackFrame) {
    if fuzz_contain(5, &frame, 0, 0) {}
    kpanic("Bound Range", frame);
}

extern "x86-interrupt" fn invalid_opcode_handler(frame: InterruptStackFrame) {
    if fuzz_contain(6, &frame, 0, 0) {}
    kpanic("Invalid Opcode", frame);
}

extern "x86-interrupt" fn device_not_available_handler(_frame: InterruptStackFrame) {
    unsafe {
        let mut cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nostack, preserves_flags));
        cr0 &= !(1 << 3);
        core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nostack, preserves_flags));
    }
}

extern "x86-interrupt" fn double_fault_handler(frame: InterruptStackFrame, _code: u64) -> ! {
    let s = SerialPort::new(0x3F8);
    s.write_str("!!! DOUBLE FAULT !!!\n");
    s.write_str("RIP: ");
    s.write_hex(frame.instruction_pointer.as_u64());
    s.write_str("\n");
    zenus_console::display::write_str("\n!!! DOUBLE FAULT !!!\nRIP=");
    write_hex_display(frame.instruction_pointer.as_u64());
    zenus_console::display::write_str("\n");
    zenus_console::serial::flush_output_blocking();
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn invalid_tss_handler(frame: InterruptStackFrame, _code: u64) {
    if fuzz_contain(10, &frame, 0, 0) {}
    kpanic("Invalid TSS", frame);
}

extern "x86-interrupt" fn segment_not_present_handler(frame: InterruptStackFrame, _code: u64) {
    if fuzz_contain(11, &frame, 0, 0) {}
    kpanic("Segment Not Present", frame);
}

extern "x86-interrupt" fn stack_segment_handler(frame: InterruptStackFrame, _code: u64) {
    if fuzz_contain(12, &frame, 0, 0) {}
    kpanic("Stack Segment Fault", frame);
}

extern "x86-interrupt" fn gpf_handler(frame: InterruptStackFrame, _code: u64) {
    if fuzz_contain(13, &frame, 0, _code) {}
    let s = SerialPort::new(0x3F8);
    let stk = frame.stack_pointer.as_u64();
    s.write_str("\n[GPF] RIP: ");
    s.write_hex(frame.instruction_pointer.as_u64());
    s.write_str(" Code: ");
    s.write_hex(_code);
    s.write_str(" RSP: ");
    s.write_hex(stk);
    if is_kernel_addr(stk) {
        for i in 0..6 {
            let addr = stk + i * 8;
            match try_read_u64(addr) {
                Some(val) => {
                    s.write_str(" [");
                    s.write_hex(i * 8);
                    s.write_str("]=");
                    s.write_hex(val);
                }
                None => {
                    s.write_str(" [");
                    s.write_hex(i * 8);
                    s.write_str("]=INVALID");
                    break;
                }
            }
        }
    }
    // Actual CPU registers at fault time
    let rax: u64;
    let rbx: u64;
    let rcx: u64;
    let rdx: u64;
    let rsi: u64;
    let rdi: u64;
    let rbp: u64;
    let r8: u64;
    let r9: u64;
    let r10: u64;
    let r11: u64;
    let r12: u64;
    let r13: u64;
    let r14: u64;
    let r15: u64;
    unsafe {
        core::arch::asm!(
            "mov {}, rax", "mov {}, rbx", "mov {}, rcx", "mov {}, rdx",
            "mov {}, rsi", "mov {}, rdi", "mov {}, rbp",
            "mov {}, r8",  "mov {}, r9",  "mov {}, r10", "mov {}, r11",
            "mov {}, r12", "mov {}, r13", "mov {}, r14", "mov {}, r15",
            out(reg) rax,  out(reg) rbx,  out(reg) rcx,  out(reg) rdx,
            out(reg) rsi,  out(reg) rdi,  out(reg) rbp,
            out(reg) r8,   out(reg) r9,   out(reg) r10,  out(reg) r11,
            out(reg) r12,  out(reg) r13,  out(reg) r14,  out(reg) r15,
            options(nostack, preserves_flags),
        );
    }
    s.write_str(" RAX=");
    s.write_hex(rax);
    s.write_str(" RCX=");
    s.write_hex(rcx);
    s.write_str(" RSI=");
    s.write_hex(rsi);
    s.write_str(" RDI=");
    s.write_hex(rdi);
    s.write_str(" RBX=");
    s.write_hex(rbx);
    s.write_str(" RDX=");
    s.write_hex(rdx);
    s.write_str(" RBP=");
    s.write_hex(rbp);
    s.write_str(" R8=");
    s.write_hex(r8);
    s.write_str(" R9=");
    s.write_hex(r9);
    s.write_str(" R10=");
    s.write_hex(r10);
    s.write_str(" R11=");
    s.write_hex(r11);
    s.write_str(" R12=");
    s.write_hex(r12);
    s.write_str(" R13=");
    s.write_hex(r13);
    s.write_str(" R14=");
    s.write_hex(r14);
    s.write_str(" R15=");
    s.write_hex(r15);
    s.write_str("\n");
    // Show on display
    zenus_console::display::write_str("\n!!! GPF !!! RIP=");
    write_hex_display(frame.instruction_pointer.as_u64());
    zenus_console::display::write_str(" Code=");
    write_hex_display(_code);
    zenus_console::display::write_str("\n");
    zenus_console::serial::flush_output_blocking();
    loop {
        x86_64::instructions::hlt();
    }
}

fn try_handle_user_page_fault(addr: u64, code: PageFaultErrorCode) -> bool {
    if (code.bits() & 0x4) == 0 {
        return false;
    }
    if addr < 0x1000 {
        return false;
    }
    if addr >= 0x6000_0000_0000 && addr < 0x8000_0000_0000 {
        let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
        let frame = match allocator.alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        drop(allocator);

        let hhdm = zenus_mem::paging::hhdm_offset();
        if hhdm == 0 {
            return false;
        }
        unsafe {
            core::ptr::write_bytes((hhdm + frame.as_u64()) as *mut u8, 0, 4096);
        }

        let writable = (code.bits() & 0x2) != 0;
        let executable = (code.bits() & 0x10) != 0;
        let page_virt = addr & !0xFFF;
        let cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
        }
        return zenus_mem::paging::map_user_page_raw(
            cr3,
            page_virt,
            frame.as_u64(),
            writable,
            executable,
        );
    }
    false
}

extern "x86-interrupt" fn page_fault_handler(frame: InterruptStackFrame, code: PageFaultErrorCode) {
    // Capture CPU registers BEFORE any Rust code can modify them.
    // The x86-interrupt ABI saves them on entry, but as soon as
    // this function body starts, the compiler may reuse GP regs.
    let (r_r15, r_r14, r_r13, r_r12, r_r11, r_r10, r_r9, r_r8);
    let (r_rdi, r_rsi, r_rbp, r_rbx, r_rdx, r_rcx, r_rax);
    unsafe {
        core::arch::asm!(
            "mov {}, r15",
            "mov {}, r14",
            "mov {}, r13",
            "mov {}, r12",
            "mov {}, r11",
            "mov {}, r10",
            "mov {}, r9",
            "mov {}, r8",
            "mov {}, rdi",
            "mov {}, rsi",
            "mov {}, rbp",
            "mov {}, rbx",
            "mov {}, rdx",
            "mov {}, rcx",
            "mov {}, rax",
            out(reg) r_r15, out(reg) r_r14, out(reg) r_r13, out(reg) r_r12,
            out(reg) r_r11, out(reg) r_r10, out(reg) r_r9, out(reg) r_r8,
            out(reg) r_rdi, out(reg) r_rsi, out(reg) r_rbp, out(reg) r_rbx,
            out(reg) r_rdx, out(reg) r_rcx, out(reg) r_rax,
            options(nostack, preserves_flags),
        );
    }

    let addr = x86_64::registers::control::Cr2::read_raw();

    // Fuzz containment comes first: while a checkpoint is armed, a page fault
    // is the fuzzer's test-case outcome. It must NOT be routed into the
    // user-fault recovery path below — that path may map a page and retry the
    // faulting instruction forever.
    if fuzz_contain(14, &frame, addr, code.bits() as u64) {}

    if try_handle_user_page_fault(addr, code) {
        return;
    }

    let s = SerialPort::new(0x3F8);

    // Bit 0 is P, bit 1 is W/R and bit 2 is U/S, so the three low bits say
    // "present? write? user?" — in that order. The table this replaces listed
    // them as if the order were present? user? write?, which mislabelled five
    // of the eight cases: 0x1 is a *supervisor read of a present page* (a
    // protection violation, and what SMAP raises), not a write to a missing
    // one. That mislabelling is what made a SMAP violation look like an
    // unrelated non-present write for as long as it did.
    let pf_type = match code.bits() & 0x7 {
        0x0 => "supervisor-read-nonpresent",
        0x1 => "supervisor-read-protection",
        0x2 => "supervisor-write-nonpresent",
        0x3 => "supervisor-write-protection",
        0x4 => "user-read-nonpresent",
        0x5 => "user-read-protection",
        0x6 => "user-write-nonpresent",
        0x7 => "user-write-protection",
        _ => "unknown",
    };
    // SMAP raises a supervisor protection violation on a page whose U/S bit is
    // 1, so say so: "supervisor-read-protection" alone sends you looking for a
    // supervisor/write permission bug that is not there. A caller that forgot
    // `stac` around a user access lands here, and nothing else does.
    let smap_suspect = matches!(code.bits() & 0x7, 0x1 | 0x3)
        && addr < 0x0000_8000_0000_0000
        && addr >= 0x1000
        && crate::cpu::smap_enabled();
    let cause = if (code.bits() & 0x10) != 0 {
        "instruction-fetch"
    } else if (code.bits() & 0x02) != 0 {
        "write"
    } else {
        "read"
    };

    s.write_str("\n!!! PAGE FAULT !!!\n");
    s.write_str("TYPE: ");
    s.write_str(pf_type);
    if (code.bits() & 0x10) != 0 {
        s.write_str(" [IF]");
    }
    if smap_suspect {
        s.write_str(" [SMAP: supervisor touched a user page without stac]");
    }

    s.write_str("\nADDR=");
    s.write_hex(addr);
    s.write_str(" RIP=");
    s.write_hex(frame.instruction_pointer.as_u64());
    s.write_str(" CS=");
    s.write_hex(frame.code_segment.index() as u64);
    s.write_str(" RFLAGS=");
    s.write_hex(frame.cpu_flags.bits());
    s.write_str(" RSP=");
    s.write_hex(frame.stack_pointer.as_u64());
    s.write_str(" CAUSE=");
    s.write_str(cause);
    s.write_str(" CODE=");
    s.write_hex(code.bits() as u64);
    // Dump code bytes near faulting instruction to help find the culprit.
    if is_kernel_addr(frame.instruction_pointer.as_u64()) {
        let rip = frame.instruction_pointer.as_u64();
        let page_off = rip & 0xFFF;
        let count = 256usize.min((4096 - page_off) as usize);
        s.write_str("\n[CODE @ RIP+8]\n");
        for i in 8..count {
            match try_read_u8(rip + i as u64) {
                Some(byte) => {
                    s.write_hex(byte as u64);
                    s.write_str(" ");
                    if i % 16 == 15 {
                        s.write_str("\n");
                    }
                }
                None => {
                    s.write_str("?? ");
                    break;
                }
            }
        }
        s.write_str("\n[CODE @ RIP]\n");
        for i in 0..8 {
            match try_read_u8(rip + i as u64) {
                Some(byte) => {
                    s.write_hex(byte as u64);
                    s.write_str(" ");
                }
                None => {
                    s.write_str("?? ");
                    break;
                }
            }
        }
        s.write_str("\n");
    }

    if addr < 0x1000 {
        s.write_str("\n*** NEAR-NULL ADDRESS ***");
    }

    // Show on display (framebuffer/VGA) too
    zenus_console::display::write_str("\n!!! PAGE FAULT !!!\n");
    zenus_console::display::write_str("ADDR=");
    write_hex_display(addr);
    zenus_console::display::write_str(" RIP=");
    write_hex_display(frame.instruction_pointer.as_u64());
    zenus_console::display::write_str(" ");
    zenus_console::display::write_str(cause);
    if addr < 0x1000 {
        zenus_console::display::write_str(" NULL");
    }
    zenus_console::display::write_str("\n");

    s.write_str(" RAX=");
    s.write_hex(r_rax);
    s.write_str(" RBX=");
    s.write_hex(r_rbx);
    s.write_str(" RCX=");
    s.write_hex(r_rcx);
    s.write_str(" RDX=");
    s.write_hex(r_rdx);
    s.write_str("\n RSI=");
    s.write_hex(r_rsi);
    s.write_str(" RDI=");
    s.write_hex(r_rdi);
    s.write_str(" RBP=");
    s.write_hex(r_rbp);
    s.write_str(" R8=");
    s.write_hex(r_r8);
    s.write_str(" R9=");
    s.write_hex(r_r9);
    s.write_str("\n R10=");
    s.write_hex(r_r10);
    s.write_str(" R11=");
    s.write_hex(r_r11);
    s.write_str(" R12=");
    s.write_hex(r_r12);
    s.write_str(" R13=");
    s.write_hex(r_r13);
    s.write_str(" R14=");
    s.write_hex(r_r14);
    s.write_str(" R15=");
    s.write_hex(r_r15);

    let stack = frame.stack_pointer.as_u64();
    s.write_str("\n[STACK ABOVE]\n");
    let stack_valid = is_kernel_addr(stack) || (stack >= 0x1000 && stack < 0x800000000000);
    if stack_valid {
        for i in 0..16u64 {
            let p = stack.wrapping_add(i * 8);
            match try_read_u64(p) {
                Some(val) => {
                    s.write_hex(p);
                    s.write_str(": ");
                    s.write_hex(val);
                    if val < 0x1000 {
                        s.write_str(" <--- LOW ADDR");
                    } else if val >= 0xFFFF800000000000 {
                        s.write_str(" (kern)");
                    }
                    s.write_str("\n");
                }
                None => {
                    s.write_hex(p);
                    s.write_str(": INVALID\n");
                    break;
                }
            }
        }
    }
    s.write_str("[STACK BELOW]\n");
    let stack_valid = is_kernel_addr(stack) || (stack >= 0x1000 && stack < 0x800000000000);
    if stack_valid {
        for i in 0..16u64 {
            let p = stack.wrapping_sub(i * 8);
            if p < 0x1000 {
                continue;
            }
            match try_read_u64(p) {
                Some(val) => {
                    s.write_hex(p);
                    s.write_str(": ");
                    s.write_hex(val);
                    if val == 0x3333333333333333 {
                        s.write_str(" <--- FREED/UNINIT");
                    } else if val < 0x1000 {
                        s.write_str(" <--- LOW ADDR");
                    } else if val >= 0xFFFF800000000000 {
                        s.write_str(" (kern)");
                    }
                    s.write_str("\n");
                }
                None => {
                    s.write_hex(p);
                    s.write_str(": INVALID\n");
                    break;
                }
            }
        }
    } else {
        s.write_str("(invalid stack pointer)\n");
    }
    zenus_console::serial::flush_output_blocking();
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn x87_fp_handler(frame: InterruptStackFrame) {
    kpanic("x87 FP", frame);
}

extern "x86-interrupt" fn alignment_check_handler(frame: InterruptStackFrame, _code: u64) {
    kpanic("Alignment Check", frame);
}

extern "x86-interrupt" fn machine_check_handler(_frame: InterruptStackFrame) -> ! {
    let s = SerialPort::new(0x3F8);
    s.write_str("!!! MACHINE CHECK !!!\n");
    zenus_console::display::write_str("\n!!! MACHINE CHECK !!!\n");
    zenus_console::serial::flush_output_blocking();
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn simd_fp_handler(frame: InterruptStackFrame) {
    kpanic("SIMD FP", frame);
}

extern "x86-interrupt" fn virtualization_handler(frame: InterruptStackFrame) {
    kpanic("Virtualization", frame);
}

fn kpanic(name: &str, frame: InterruptStackFrame) -> ! {
    let rip = frame.instruction_pointer.as_u64();
    let rsp = frame.stack_pointer.as_u64();
    let cs_idx = frame.code_segment.index() as u64;
    let rflags = frame.cpu_flags.bits();

    let rax: u64;
    let rbx: u64;
    let rcx: u64;
    let rdx: u64;
    let rsi: u64;
    let rdi: u64;
    let rbp: u64;
    let r8: u64;
    let r9: u64;
    let r10: u64;
    let r11: u64;
    let r12: u64;
    let r13: u64;
    let r14: u64;
    let r15: u64;
    unsafe {
        core::arch::asm!(
            "mov {}, rax", "mov {}, rbx", "mov {}, rcx", "mov {}, rdx",
            "mov {}, rsi", "mov {}, rdi", "mov {}, rbp",
            "mov {}, r8",  "mov {}, r9",  "mov {}, r10", "mov {}, r11",
            "mov {}, r12", "mov {}, r13", "mov {}, r14", "mov {}, r15",
            out(reg) rax,  out(reg) rbx,  out(reg) rcx,  out(reg) rdx,
            out(reg) rsi,  out(reg) rdi,  out(reg) rbp,
            out(reg) r8,   out(reg) r9,   out(reg) r10,  out(reg) r11,
            out(reg) r12,  out(reg) r13,  out(reg) r14,  out(reg) r15,
            options(nostack, preserves_flags),
        );
    }

    let s = SerialPort::new(0x3F8);
    s.write_str("!!! ");
    s.write_str(name);
    s.write_str(" !!!\n");
    if crate::limine::hhdm_offset() != 0 {
        zenus_console::display::write_str("!!! ");
        zenus_console::display::write_str(name);
        zenus_console::display::write_str(" !!!\n");
    }
    s.write_str("RIP: ");
    s.write_hex(rip);
    s.write_str(" CS: ");
    s.write_hex(cs_idx);
    s.write_str(" RFLAGS: ");
    s.write_hex(rflags);
    s.write_str(" RSP: ");
    s.write_hex(rsp);
    s.write_str("\n");

    s.write_str("[CODE]\n");
    if is_kernel_addr(rip) {
        let page_off = rip & 0xFFF;
        let count = 16.min((4096 - page_off) as usize);
        for i in 0..count {
            match try_read_u8(rip + i as u64) {
                Some(byte) => {
                    s.write_hex(byte as u64);
                    s.write_str(" ");
                }
                None => {
                    s.write_str("?? ");
                    break;
                }
            }
        }
    } else {
        s.write_str("(invalid rip)\n");
    }
    s.write_str("\n");

    s.write_str(" RAX=");
    s.write_hex(rax);
    s.write_str(" RBX=");
    s.write_hex(rbx);
    s.write_str(" RCX=");
    s.write_hex(rcx);
    s.write_str(" RDX=");
    s.write_hex(rdx);
    s.write_str("\n RSI=");
    s.write_hex(rsi);
    s.write_str(" RDI=");
    s.write_hex(rdi);
    s.write_str(" RBP=");
    s.write_hex(rbp);
    s.write_str(" R8=");
    s.write_hex(r8);
    s.write_str(" R9=");
    s.write_hex(r9);
    s.write_str("\n R10=");
    s.write_hex(r10);
    s.write_str(" R11=");
    s.write_hex(r11);
    s.write_str(" R12=");
    s.write_hex(r12);
    s.write_str(" R13=");
    s.write_hex(r13);
    s.write_str(" R14=");
    s.write_hex(r14);
    s.write_str(" R15=");
    s.write_hex(r15);
    s.write_str("\n");

    s.write_str("[STACK]\n");
    let stack_valid = is_kernel_addr(rsp) || (rsp >= 0x1000 && rsp < 0x800000000000);
    if stack_valid {
        for i in 0..20u64 {
            let p = rsp.wrapping_add(i * 8);
            if p < 0x1000 {
                continue;
            }
            match try_read_u64(p) {
                Some(val) => {
                    s.write_hex(p);
                    s.write_str(": ");
                    s.write_hex(val);
                    if val < 0x1000 && val != 0 {
                        s.write_str(" <--- small");
                    }
                    s.write_str("\n");
                }
                None => {
                    s.write_hex(p);
                    s.write_str(": INVALID\n");
                    break;
                }
            }
        }
    } else {
        s.write_str("(invalid stack pointer)\n");
    }

    zenus_console::serial::flush_output_blocking();
    loop {
        x86_64::instructions::hlt();
    }
}

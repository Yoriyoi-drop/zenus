use x86_64::structures::idt::{InterruptDescriptorTable, PageFaultErrorCode, InterruptStackFrame};
use core::mem::MaybeUninit;
use zenus_console::serial::SerialPort;

use crate::gdt;

#[allow(static_mut_refs)]
static mut IDT: MaybeUninit<InterruptDescriptorTable> = MaybeUninit::uninit();

pub fn init() {
    let idt = unsafe { &mut *IDT.as_mut_ptr() };

    idt.divide_error.set_handler_fn(divide_error_handler);
    idt.debug.set_handler_fn(debug_handler);
    idt.non_maskable_interrupt.set_handler_fn(nmi_handler);
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt.overflow.set_handler_fn(overflow_handler);
    idt.bound_range_exceeded.set_handler_fn(bound_range_handler);
    idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
    idt.device_not_available.set_handler_fn(device_not_available_handler);

    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index((gdt::DF_IST_IDX + 1) as u16);
    }

    idt.invalid_tss.set_handler_fn(invalid_tss_handler);
    idt.segment_not_present.set_handler_fn(segment_not_present_handler);
    idt.stack_segment_fault.set_handler_fn(stack_segment_handler);
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.x87_floating_point.set_handler_fn(x87_fp_handler);
    idt.alignment_check.set_handler_fn(alignment_check_handler);
    idt.machine_check.set_handler_fn(machine_check_handler);
    idt.simd_floating_point.set_handler_fn(simd_fp_handler);
    idt.virtualization.set_handler_fn(virtualization_handler);

    // IRQ 0-15 mapped to vectors 32-47
    idt[32].set_handler_fn(super::handler::interrupt_timer);
    idt[39].set_handler_fn(super::handler::interrupt_spurious);

    // NIC interrupt (vector 43 = IRQ 11)
    idt[43].set_handler_fn(super::handler::interrupt_nic);

    unsafe {
        extern "C" { static apic_timer_isr_stub: u8; }
        let addr = &apic_timer_isr_stub as *const u8 as u64;
        idt[48].set_handler_addr(x86_64::VirtAddr::new(addr))
            .disable_interrupts(true)
            .set_privilege_level(x86_64::PrivilegeLevel::Ring0);
    }

    idt.load();
}

extern "x86-interrupt" fn divide_error_handler(frame: InterruptStackFrame) {
    kpanic("Divide Error", frame);
}

extern "x86-interrupt" fn debug_handler(frame: InterruptStackFrame) {
    kpanic("Debug", frame);
}

extern "x86-interrupt" fn nmi_handler(_frame: InterruptStackFrame) {
    SerialPort::new(0x3F8).write_str("NMI\n");
}

extern "x86-interrupt" fn breakpoint_handler(_frame: InterruptStackFrame) {
    SerialPort::new(0x3F8).write_str("Breakpoint\n");
}

extern "x86-interrupt" fn overflow_handler(frame: InterruptStackFrame) {
    kpanic("Overflow", frame);
}

extern "x86-interrupt" fn bound_range_handler(frame: InterruptStackFrame) {
    kpanic("Bound Range", frame);
}

extern "x86-interrupt" fn invalid_opcode_handler(frame: InterruptStackFrame) {
    kpanic("Invalid Opcode", frame);
}

extern "x86-interrupt" fn device_not_available_handler(_frame: InterruptStackFrame) {
    // Handle FPU/SSE context switch
}

extern "x86-interrupt" fn double_fault_handler(frame: InterruptStackFrame, _code: u64) -> ! {
    let mut s = SerialPort::new(0x3F8);
    s.write_str("!!! DOUBLE FAULT !!!\n");
    s.write_str("RIP: ");
    s.write_hex(frame.instruction_pointer.as_u64());
    s.write_str("\n");
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn invalid_tss_handler(frame: InterruptStackFrame, _code: u64) {
    kpanic("Invalid TSS", frame);
}

extern "x86-interrupt" fn segment_not_present_handler(frame: InterruptStackFrame, _code: u64) {
    kpanic("Segment Not Present", frame);
}

extern "x86-interrupt" fn stack_segment_handler(frame: InterruptStackFrame, _code: u64) {
    kpanic("Stack Segment Fault", frame);
}

extern "x86-interrupt" fn gpf_handler(frame: InterruptStackFrame, _code: u64) {
    let mut s = SerialPort::new(0x3F8);
    let stk = frame.stack_pointer.as_u64();
    s.write_str("\n[GPF] RIP: ");
    s.write_hex(frame.instruction_pointer.as_u64());
    s.write_str(" Code: ");
    s.write_hex(_code);
    s.write_str(" RSP: ");
    s.write_hex(stk);
    if stk >= 0xFFFF800000000000 {
        for i in 0..6 {
            let val: u64 = unsafe { core::ptr::read_volatile((stk + i*8) as *const u64) };
            s.write_str(" [");
            s.write_hex(i*8);
            s.write_str("]=");
            s.write_hex(val);
        }
    }
    s.write_str("\n");
    loop { x86_64::instructions::hlt(); }
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
        unsafe {
            core::ptr::write_bytes((hhdm + frame.as_u64()) as *mut u8, 0, 4096);
        }

        let writable = (code.bits() & 0x2) != 0;
        let page_virt = addr & !0xFFF;
        let cr3: u64;
        unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3); }
        return zenus_mem::paging::map_user_page_raw(
            cr3, page_virt, frame.as_u64(), writable,
        );
    }
    false
}

extern "x86-interrupt" fn page_fault_handler(frame: InterruptStackFrame, code: PageFaultErrorCode) {
    let addr = x86_64::registers::control::Cr2::read_raw();

    if try_handle_user_page_fault(addr, code) {
        return;
    }

    let mut s = SerialPort::new(0x3F8);

    let pf_type = match code.bits() & 0x7 {
        0x0 => "supervisor-read-nonpresent",
        0x1 => "supervisor-write-nonpresent",
        0x2 => "supervisor-read-protection",
        0x3 => "supervisor-write-protection",
        0x4 => "user-read-nonpresent",
        0x5 => "user-write-nonpresent",
        0x6 => "user-read-protection",
        0x7 => "user-write-protection",
        _ => "unknown",
    };
    let cause = if (code.bits() & 0x10) != 0 {
        "instruction-fetch"
    } else if (code.bits() & 0x02) != 0 {
        "write"
    } else {
        "read"
    };

    s.write_str("\n!!! PAGE FAULT !!!\n");
    s.write_str("TYPE: "); s.write_str(pf_type);
    if (code.bits() & 0x10) != 0 { s.write_str(" [IF]"); }

    s.write_str("\nADDR="); s.write_hex(addr);
    s.write_str(" RIP="); s.write_hex(frame.instruction_pointer.as_u64());
    s.write_str(" CS="); s.write_hex(frame.code_segment.index() as u64);
    s.write_str(" RFLAGS="); s.write_hex(frame.cpu_flags.bits());
    s.write_str(" RSP="); s.write_hex(frame.stack_pointer.as_u64());
    s.write_str(" CAUSE="); s.write_str(cause);
    s.write_str(" CODE="); s.write_hex(code.bits() as u64);

    if addr < 0x1000 {
        s.write_str("\n*** NEAR-NULL ADDRESS ***");
    }

    let rax_val: u64;
    unsafe {
        core::arch::asm!("mov {}, rax", out(reg) rax_val);
    }
    s.write_str(" RAX="); s.write_hex(rax_val);

    let stack = frame.stack_pointer.as_u64();
    s.write_str("\n[STACK]\n");
    let stack_valid = stack >= 0xFFFF800000000000 || (stack >= 0x1000 && stack < 0x800000000000);
    if stack_valid {
        for i in 0..16u64 {
            let p = stack.wrapping_sub(i * 8);
            if p < 0x1000 { continue; }
            let val: u64 = unsafe { core::ptr::read_volatile(p as *const u64) };
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
    } else {
        s.write_str("(invalid stack pointer)\n");
    }
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn x87_fp_handler(frame: InterruptStackFrame) {
    kpanic("x87 FP", frame);
}

extern "x86-interrupt" fn alignment_check_handler(frame: InterruptStackFrame, _code: u64) {
    kpanic("Alignment Check", frame);
}

extern "x86-interrupt" fn machine_check_handler(_frame: InterruptStackFrame) -> ! {
    let mut s = SerialPort::new(0x3F8);
    s.write_str("!!! MACHINE CHECK !!!\n");
    loop { x86_64::instructions::hlt(); }
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

    let rax: u64; let rbx: u64; let rcx: u64; let rdx: u64;
    let rsi: u64; let rdi: u64; let rbp: u64; let r8: u64;
    let r9: u64; let r10: u64; let r11: u64; let r12: u64;
    let r13: u64; let r14: u64; let r15: u64;
    unsafe {
        core::arch::asm!("mov {}, rax", out(reg) rax);
        core::arch::asm!("mov {}, rbx", out(reg) rbx);
        core::arch::asm!("mov {}, rcx", out(reg) rcx);
        core::arch::asm!("mov {}, rdx", out(reg) rdx);
        core::arch::asm!("mov {}, rsi", out(reg) rsi);
        core::arch::asm!("mov {}, rdi", out(reg) rdi);
        core::arch::asm!("mov {}, rbp", out(reg) rbp);
        core::arch::asm!("mov {}, r8", out(reg) r8);
        core::arch::asm!("mov {}, r9", out(reg) r9);
        core::arch::asm!("mov {}, r10", out(reg) r10);
        core::arch::asm!("mov {}, r11", out(reg) r11);
        core::arch::asm!("mov {}, r12", out(reg) r12);
        core::arch::asm!("mov {}, r13", out(reg) r13);
        core::arch::asm!("mov {}, r14", out(reg) r14);
        core::arch::asm!("mov {}, r15", out(reg) r15);
    }

    let mut s = SerialPort::new(0x3F8);
    s.write_str("!!! ");
    s.write_str(name);
    s.write_str(" !!!\n");
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
    for i in 0..16u64 {
        let addr = rip.wrapping_add(i);
        let byte: u8 = unsafe { core::ptr::read_volatile(addr as *const u8) };
        s.write_hex(byte as u64);
        s.write_str(" ");
    }
    s.write_str("\n");

    s.write_str(" RAX="); s.write_hex(rax);
    s.write_str(" RBX="); s.write_hex(rbx);
    s.write_str(" RCX="); s.write_hex(rcx);
    s.write_str(" RDX="); s.write_hex(rdx);
    s.write_str("\n RSI="); s.write_hex(rsi);
    s.write_str(" RDI="); s.write_hex(rdi);
    s.write_str(" RBP="); s.write_hex(rbp);
    s.write_str(" R8=");  s.write_hex(r8);
    s.write_str(" R9=");  s.write_hex(r9);
    s.write_str("\n R10="); s.write_hex(r10);
    s.write_str(" R11="); s.write_hex(r11);
    s.write_str(" R12="); s.write_hex(r12);
    s.write_str(" R13="); s.write_hex(r13);
    s.write_str(" R14="); s.write_hex(r14);
    s.write_str(" R15="); s.write_hex(r15);
    s.write_str("\n");

    s.write_str("[STACK]\n");
    let stack_valid = rsp >= 0xFFFF800000000000 || (rsp >= 0x1000 && rsp < 0x800000000000);
    if stack_valid {
        for i in 0..20u64 {
            let p = rsp.wrapping_add(i * 8);
            if p < 0x1000 { continue; }
            let val: u64 = unsafe { core::ptr::read_volatile(p as *const u64) };
            s.write_hex(p);
            s.write_str(": ");
            s.write_hex(val);
            if val < 0x1000 && val != 0 {
                s.write_str(" <--- small");
            }
            s.write_str("\n");
        }
    } else {
        s.write_str("(invalid stack pointer)\n");
    }

    loop { x86_64::instructions::hlt(); }
}

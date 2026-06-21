use x86_64::instructions::port::Port;
use crate::limine;

const KBD_CMD: u16 = 0x64;
const KBD_DATA: u16 = 0x60;

const KBD_RESET_CPU: u8 = 0xFE;

pub fn reboot_via_keyboard() -> ! {
    let mut s = zenus_console::serial::SerialPort::new(0x3F8);

    // Try ACPI reset register from FADT
    let fadt_addr = find_fadt();
    if fadt_addr != 0 {
        unsafe {
            let header_len = *(fadt_addr as *const u8).add(8) as usize;
            let flags = *(fadt_addr as *const u32).add(112/4);
            if header_len >= 129 && (flags & (1 << 10)) != 0 {
                let addr_space_id = *(fadt_addr as *const u8).add(116);
                let reg_width = *(fadt_addr as *const u8).add(117);
                let reg_addr = *(fadt_addr as *const u64).add(120/8);
                let reset_val = *(fadt_addr as *const u8).add(128);
                if addr_space_id == 1 && reg_width == 8 && reg_addr != 0 {
                    s.write_str("[ACPI] Rebooting via ACPI reset register...\n");
                    let mut p = Port::<u8>::new(reg_addr as u16);
                    p.write(reset_val);
                    for _ in 0..10000000 { core::hint::spin_loop(); }
                }
            }
        }
    }

    // Try Intel reset port 0xCF9
    unsafe {
        s.write_str("[ACPI] Rebooting via 0xCF9 reset port...\n");
        let mut p = Port::<u8>::new(0xCF9);
        p.write(0x06);
        for _ in 0..10000000 { core::hint::spin_loop(); }
        p.write(0x0E);
        for _ in 0..10000000 { core::hint::spin_loop(); }
    }

    // Try keyboard controller with proper status check
    unsafe {
        s.write_str("[ACPI] Rebooting via keyboard controller...\n");
        let mut status = Port::<u8>::new(KBD_CMD);
        for _ in 0..100 {
            let st: u8;
            core::arch::asm!("in al, dx", out("al") st, in("dx") 0x64u16);
            if st & 0x02 == 0 {
                status.write(KBD_RESET_CPU);
                for _ in 0..10000000 { core::hint::spin_loop(); }
            }
        }
    }

    // Triple fault as last resort — load zero IDT to crash
    s.write_str("[ACPI] Rebooting via triple fault...\n");
    unsafe {
        core::arch::asm!("push 0; push 0; lidt [rsp]; add rsp, 16; ud2");
    }
    loop { x86_64::instructions::hlt(); }
}

pub fn shutdown_via_acpi() -> ! {
    let mut s = zenus_console::serial::SerialPort::new(0x3F8);
    s.write_str("[ACPI] Attempting ACPI shutdown...\n");

    let fadt_addr = find_fadt();
    if fadt_addr == 0 {
        s.write_str("[ACPI] FADT not found, shutdown not possible\n");
        loop { x86_64::instructions::hlt(); }
    }
    unsafe {
        let pm1a_cnt_blk = *(fadt_addr as *const u32).add(64/4) as u16;

        if pm1a_cnt_blk == 0 {
            s.write_str("[ACPI] PM1a_CNT_BLK is 0\n");
            loop { x86_64::instructions::hlt(); }
        }

        let mut port = Port::<u16>::new(pm1a_cnt_blk);
        let pm1a_cnt_val = port.read();
        let slp_typa = 0u16;
        let slp_en = 1u16 << 13;
        let val = (pm1a_cnt_val & !0x3FFF) | (slp_typa << 10) | slp_en;
        port.write(val);
    }
    loop { x86_64::instructions::hlt(); }
}

fn find_fadt() -> u64 {
    let rsdp = match get_rsdp() {
        Some(r) => r,
        None => return 0,
    };
    let mut s = zenus_console::serial::SerialPort::new(0x3F8);
    s.write_str("[ACPI] RSDP at 0x");
    s.write_hex(rsdp);
    s.write_str("\n");

    let hhdm = limine::hhdm_offset();

    let revision = unsafe { *(rsdp as *const u8).add(15) };
    let (table_addr, is_xsdt) = if revision == 0 {
        let ptr = rsdp as *const u32;
        (unsafe { *(ptr.add(16/4)) as u64 }, false)
    } else {
        let ptr = rsdp as *const u64;
        (unsafe { *(ptr.add(24/8)) }, true)
    };

    if table_addr == 0 {
        s.write_str("[ACPI] Root SDT address is 0\n");
        return 0;
    }

    s.write_str("[ACPI] Root SDT at 0x");
    s.write_hex(table_addr);
    s.write_str("\n");

    let table_virt = table_addr + hhdm;
    let table_len = unsafe { *(table_virt as *const u32).add(1) } as usize;
    let entry_size = if is_xsdt { 8 } else { 4 };
    let entry_count = (table_len - 36) / entry_size;
    s.write_str("[ACPI] Root SDT entries: ");
    s.write_u64(entry_count as u64);
    s.write_str("\n");

    for i in 0..entry_count {
        let entry = if is_xsdt {
            unsafe { *(table_virt as *const u64).add(36/8 + i) }
        } else {
            unsafe { *(table_virt as *const u32).add(36/4 + i) as u64 }
        };
        let entry_virt = entry + hhdm;
        let sig = unsafe {
            let p = entry_virt as *const u8;
            [*p, *p.add(1), *p.add(2), *p.add(3)]
        };
        if &sig == b"FACP" {
            s.write_str("[ACPI] Found FADT\n");
            return entry_virt;
        }
    }
    s.write_str("[ACPI] FADT not found\n");
    0
}

fn get_rsdp() -> Option<u64> {
    if limine::RSDP_REQUEST.response.is_null() {
        return None;
    }
    let resp: &limine::LimineRsdpResponse =
        unsafe { &*limine::RSDP_REQUEST.response.as_ptr() };
    if resp.address.is_null() {
        None
    } else {
        Some(resp.address.0)
    }
}

pub fn init() {
    let mut s = zenus_console::serial::SerialPort::new(0x3F8);
    s.write_str("[OK] ACPI subsystem initialized\n");
}

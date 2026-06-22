use zenus_console::serial::SerialPort;
use zenus_mem::paging;
use zenus_sched::scheduler;

const USER_BINARY: &[u8] = include_bytes!("../user.bin");

fn log(msg: &str) {
    let mut s = SerialPort::new(0x3F8);
    s.write_str(msg);
}

pub fn spawn_user() -> u64 {
    log("[USER] Loading user task with proper address space...\n");

    let user_cr3 = match paging::create_address_space() {
        Some(cr3) => cr3,
        None => {
            log("[USER] Failed to create address space\n");
            return 0;
        }
    };
    log("[USER] New address space CR3=0x");
    let mut s = SerialPort::new(0x3F8);
    s.write_hex(user_cr3);
    s.write_str("\n");

    let loaded = match zenus_syscall::elf::load_flat_binary(USER_BINARY, 0x400000, user_cr3) {
        Some(elf) => elf,
        None => {
            log("[USER] Failed to load ELF binary\n");
            paging::destroy_address_space(user_cr3);
            return 0;
        }
    };

    log("[USER] ELF loaded: entry=0x");
    s.write_hex(loaded.entry);
    s.write_str(" stack_top=0x");
    s.write_hex(loaded.stack_top);
    s.write_str("\n");

    if paging::virt_to_phys_raw(user_cr3, loaded.entry).is_none() {
        log("[USER] FATAL: Entry not mapped, aborting\n");
        return 0;
    }

    let task_id = scheduler::create_user_task(
        loaded.entry,
        65536,
        loaded.stack_top,
        user_cr3,
        loaded.heap_base,
    );
    if task_id == 0 {
        log("[USER] Failed to create user task\n");
        return 0;
    }

    let mut s = SerialPort::new(0x3F8);
    s.write_str("[OK] Real user mode task spawned (tid=");
    s.write_u64(task_id);
    s.write_str(")\n");

    task_id
}

use zenus_console::serial::SerialPort;
use zenus_mem::paging;
use zenus_mem::frame_allocator::FRAME_ALLOCATOR;
use zenus_sched::scheduler;

const USER_BINARY: &[u8] = include_bytes!("../user_demo.bin");
const USER_ENTRY: u64 = 0x400000;
const USER_STACK_TOP: u64 = 0x7FFF_FFFF_F000;

fn log(msg: &str) {
    let mut s = SerialPort::new(0x3F8);
    s.write_str(msg);
}

pub fn spawn_user_demo() -> u64 {
    log("[USER] Loading user demo binary...\n");

    {
        let fa = FRAME_ALLOCATOR.lock();
        let mut s = SerialPort::new(0x3F8);
        s.write_str("[USER] Frame allocator: ");
        s.write_u64(fa.used_memory());
        s.write_str(" used / ");
        s.write_u64(fa.total_memory());
        s.write_str(" total / ");
        s.write_u64(fa.free_frames_count() as u64);
        s.write_str(" free stack\n");
    }

    let code_pages = (USER_BINARY.len() + 4095) / 4096;
    if code_pages > 4 {
        log("[USER] Binary too large\n");
        return 0;
    }

    let mut code_frames = [0u64; 4];
    let stack_frame;

    // Step 1: allocate all frames (hold lock once)
    {
        let mut allocator = FRAME_ALLOCATOR.lock();
        for i in 0..code_pages {
            code_frames[i] = match allocator.alloc_frame() {
                Some(f) => f.as_u64(),
                None => { log("[USER] No memory\n"); return 0; }
            };
        }
        stack_frame = match allocator.alloc_frame() {
            Some(f) => f.as_u64(),
            None => { log("[USER] No memory for stack\n"); return 0; }
        };
    }

    // Step 2: use kernel's CR3 (no separate address space)
    let cr3 = paging::kernel_cr3();

    // Step 3: create fresh page tables in the user address space (bypass existing ones)
    let hhdm = paging::hhdm_offset();
    {
        let kernel_l4 = ((cr3 & !0xFFF) + hhdm) as *mut u64;
        let mut s;

        // Force-allocate a new PDP table
        let mut allocator = FRAME_ALLOCATOR.lock();
        let pdp_frame = allocator.alloc_frame().unwrap().as_u64();
        drop(allocator);

        // Also copy code data into code frames
        for i in 0..code_pages {
            let phys = code_frames[i];
            let dst = unsafe { core::slice::from_raw_parts_mut((phys + hhdm) as *mut u8, 4096) };
            let src_start = i * 4096;
            let src_end = core::cmp::min(src_start + 4096, USER_BINARY.len());
            dst[..src_end - src_start].copy_from_slice(&USER_BINARY[src_start..src_end]);
        }
        s = SerialPort::new(0x3F8);
        s.write_str("[USER] Code copied to "); s.write_hex(code_frames[0]); s.write_str("\n");

        // PML4[0] = pdp_frame (new PDP)
        let pdp_virt = (pdp_frame + hhdm) as *mut u64;
        unsafe {
            for j in 0..512 { pdp_virt.add(j).write(0); }
        }
        unsafe { *kernel_l4.add(0) = pdp_frame | 0x7; }
        s.write_str("[USER] PML4[0]="); unsafe { s.write_hex(*kernel_l4.add(0)); } s.write_str("\n");

        // Allocate PD table
        let mut allocator = FRAME_ALLOCATOR.lock();
        let pd_frame = allocator.alloc_frame().unwrap().as_u64();
        drop(allocator);
        let pd_virt = (pd_frame + hhdm) as *mut u64;
        unsafe { for j in 0..512 { pd_virt.add(j).write(0); } }
        // PDP[0] = pd_frame
        unsafe { *pdp_virt.add(0) = pd_frame | 0x7; }

        // Allocate PT table
        let mut allocator = FRAME_ALLOCATOR.lock();
        let pt_frame = allocator.alloc_frame().unwrap().as_u64();
        drop(allocator);
        let pt_virt = (pt_frame + hhdm) as *mut u64;
        unsafe { for j in 0..512 { pt_virt.add(j).write(0); } }
        // PD[2] = pt_frame
        unsafe { *pd_virt.add(2) = pt_frame | 0x7; }

        // PT[0] = code frame
        unsafe { *pt_virt.add(0) = code_frames[0] | 0x7; }

        s = SerialPort::new(0x3F8);
        s.write_str("[USER] Walk: PML4[0]="); unsafe { s.write_hex(*kernel_l4.add(0)); }
        s.write_str(" PDP[0]="); unsafe { s.write_hex(*pdp_virt.add(0)); }
        s.write_str(" PD[2]="); unsafe { s.write_hex(*pd_virt.add(2)); }
        s.write_str(" PT[0]="); unsafe { s.write_hex(*pt_virt.add(0)); }
        s.write_str("\n");
    }

    // Step 4: map stack page using kernel page tables
    let stack_virt = USER_STACK_TOP - 4096;
    unsafe {
        let kernel_l4 = ((cr3 & !0xFFF) + hhdm) as *mut u64;

        // Stack at 0x7FFF_FFFF_F000, needs PML4[255] present
        let pml4_idx = (stack_virt >> 39) & 0x1FF; // 255

        // Allocate stack PDP, PD, PT
        let mut allocator = FRAME_ALLOCATOR.lock();
        let st_pdp_frame = allocator.alloc_frame().unwrap().as_u64();
        let st_pd_frame = allocator.alloc_frame().unwrap().as_u64();
        let st_pt_frame = allocator.alloc_frame().unwrap().as_u64();
        drop(allocator);

        let st_pdp_virt = (st_pdp_frame + hhdm) as *mut u64;
        let st_pd_virt = (st_pd_frame + hhdm) as *mut u64;
        let st_pt_virt = (st_pt_frame + hhdm) as *mut u64;

        for j in 0..512 { st_pdp_virt.add(j).write(0); }
        for j in 0..512 { st_pd_virt.add(j).write(0); }
        for j in 0..512 { st_pt_virt.add(j).write(0); }

        let st_pdp_idx = (stack_virt >> 30) & 0x1FF;
        let st_pd_idx = (stack_virt >> 21) & 0x1FF;
        let st_pt_idx = (stack_virt >> 12) & 0x1FF;

        *kernel_l4.add(pml4_idx as usize) = st_pdp_frame | 0x7;
        *st_pdp_virt.add(st_pdp_idx as usize) = st_pd_frame | 0x7;
        *st_pd_virt.add(st_pd_idx as usize) = st_pt_frame | 0x7;
        *st_pt_virt.add(st_pt_idx as usize) = stack_frame | 0x7;
    }

    // Step 4b: flush TLB for the mappings we just created
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack, preserves_flags));
    }

    // Step 4c: verify mapping by reading page table
    {
        let l4 = ((cr3 & !0xFFF) + hhdm) as *const u64;
        let pml4_idx = (USER_ENTRY >> 39) & 0x1FF;
        let pdp_idx = (USER_ENTRY >> 30) & 0x1FF;
        let pd_idx = (USER_ENTRY >> 21) & 0x1FF;
        let pt_idx = (USER_ENTRY >> 12) & 0x1FF;
        let mut s = SerialPort::new(0x3F8);
        s.write_str("[USER] idx: ");
        s.write_u64(pml4_idx); s.write_str(",");
        s.write_u64(pdp_idx); s.write_str(",");
        s.write_u64(pd_idx); s.write_str(",");
        s.write_u64(pt_idx); s.write_str("\n");

        let pml4e = unsafe { *l4.add(pml4_idx as usize) };
        s.write_str("[USER] PML4="); s.write_hex(pml4e); s.write_str("\n");
        if pml4e & 1 == 0 { log("[USER] PML4 NP\n"); return 0; }

        let pdpt = ((pml4e & 0xFFFFFFFFFF000) + hhdm) as *const u64;
        let pdpe = unsafe { *pdpt.add(pdp_idx as usize) };
        s.write_str("[USER] PDP="); s.write_hex(pdpe); s.write_str("\n");
        if pdpe & 1 == 0 { log("[USER] PDP NP\n"); return 0; }

        let pd = ((pdpe & 0xFFFFFFFFFF000) + hhdm) as *const u64;
        let pde = unsafe { *pd.add(pd_idx as usize) };
        s.write_str("[USER] PD="); s.write_hex(pde); s.write_str("\n");
        if pde & 1 == 0 { log("[USER] PD NP\n"); return 0; }

        let pt = ((pde & 0xFFFFFFFFFF000) + hhdm) as *const u64;
        let pte = unsafe { *pt.add(pt_idx as usize) };
        s.write_str("[USER] PT="); s.write_hex(pte); s.write_str("\n");
        if pte & 1 == 0 { log("[USER] PT NP\n"); return 0; }

        log("[USER] Page walk OK\n");
    }

    // Step 5: verify binary
    let first_byte = unsafe { *((code_frames[0] + hhdm) as *const u8) };
    if first_byte != 0xb8 {
        log("[USER] Binary verification failed\n");
        return 0;
    }
    log("[USER] Binary verified OK\n");

    // Step 6: spawn user task
    let task_id = scheduler::create_user_task(USER_ENTRY, 65536, USER_STACK_TOP, cr3);
    if task_id == 0 {
        log("[USER] Failed to create user task\n");
        return 0;
    }

    let mut s = SerialPort::new(0x3F8);
    s.write_str("[OK] User demo task spawned (tid=");
    s.write_u64(task_id);
    s.write_str(")\n");

    task_id
}

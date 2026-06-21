use core::sync::atomic::{AtomicU64, Ordering};

static HHDM_OFFSET: AtomicU64 = AtomicU64::new(0);

pub fn store_hhdm_offset(offset: u64) {
    HHDM_OFFSET.store(offset, Ordering::Relaxed);
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LiminePtr(pub u64);

impl LiminePtr {
    pub fn as_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }

    pub fn as_ref<'a, T>(self) -> &'a T {
        unsafe { &*(self.0 as *const T) }
    }

    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

// Common magic for all Limine v11+ requests
const LIMINE_COMMON_MAGIC: [u64; 2] = [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b];

const LIMINE_BASE_REVISION_ID: [u64; 2] = [0xf9562b2d5c95a6c8, 0x6a7b384944536bdc];

#[repr(C)]
pub struct LimineBaseRevision {
    pub id: [u64; 2],
    pub revision: u64,
}

#[link_section = ".limine_reqs"]
#[used]
pub static BASE_REVISION: LimineBaseRevision = LimineBaseRevision {
    id: LIMINE_BASE_REVISION_ID,
    revision: 2,
};

// === Memory Map ===

#[repr(C)]
pub struct LimineMemmapEntry {
    pub base: u64,
    pub length: u64,
    pub kind: u64,
}

impl LimineMemmapEntry {
    pub fn is_usable(&self) -> bool {
        self.kind == 0
    }
}

#[repr(C)]
pub struct LimineMemmapResponse {
    pub revision: u64,
    pub entry_count: u64,
    pub entries: LiminePtr,
}

#[repr(C)]
pub struct LimineMemmapRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: LiminePtr,
}

const LIMINE_MEMMAP_REQUEST_ID: [u64; 4] = [
    LIMINE_COMMON_MAGIC[0], LIMINE_COMMON_MAGIC[1],
    0x67cf3d9d378a806f, 0xe304acdfc50c3c62,
];

#[link_section = ".limine_reqs"]
#[used]
pub static MEMMAP_REQUEST: LimineMemmapRequest = LimineMemmapRequest {
    id: LIMINE_MEMMAP_REQUEST_ID,
    revision: 0,
    response: LiminePtr(0),
};

// === RSDP ===

#[repr(C)]
pub struct LimineRsdpResponse {
    pub revision: u64,
    pub address: LiminePtr,
}

#[repr(C)]
pub struct LimineRsdpRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: LiminePtr,
}

const LIMINE_RSDP_REQUEST_ID: [u64; 4] = [
    LIMINE_COMMON_MAGIC[0], LIMINE_COMMON_MAGIC[1],
    0xc5e77b6b397e7b43, 0x27637845accdcf3c,
];

#[link_section = ".limine_reqs"]
#[used]
pub static RSDP_REQUEST: LimineRsdpRequest = LimineRsdpRequest {
    id: LIMINE_RSDP_REQUEST_ID,
    revision: 0,
    response: LiminePtr(0),
};

// === Module (initrd) ===

#[repr(C)]
pub struct LimineFile {
    pub revision: u64,
    pub address: LiminePtr,
    pub size: u64,
    pub path: LiminePtr,
    pub string: LiminePtr,
    pub media_type: u32,
    pub _pad: u32,
    pub tftp_ip: u32,
    pub tftp_port: u32,
    pub partition_index: u32,
    pub mbr_disk_id: u32,
    pub gpt_disk_uuid: [u8; 16],
    pub gpt_part_uuid: [u8; 16],
    pub part_uuid: [u8; 16],
}

#[repr(C)]
pub struct LimineModuleResponse {
    pub revision: u64,
    pub module_count: u64,
    pub modules: LiminePtr,
}

#[repr(C)]
pub struct LimineModuleRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: LiminePtr,
}

const LIMINE_MODULE_REQUEST_ID: [u64; 4] = [
    LIMINE_COMMON_MAGIC[0], LIMINE_COMMON_MAGIC[1],
    0x3e7e279702be32af, 0xca1c4f3bd1280cee,
];

#[link_section = ".limine_reqs"]
#[used]
pub static MODULE_REQUEST: LimineModuleRequest = LimineModuleRequest {
    id: LIMINE_MODULE_REQUEST_ID,
    revision: 0,
    response: LiminePtr(0),
};

// === HHDM ===

#[repr(C)]
pub struct LimineHhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[repr(C)]
pub struct LimineHhdmRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: LiminePtr,
}

const LIMINE_HHDM_REQUEST_ID: [u64; 4] = [
    LIMINE_COMMON_MAGIC[0], LIMINE_COMMON_MAGIC[1],
    0x48dcf1cb8ad2b852, 0x63984e959a98244b,
];

#[link_section = ".limine_reqs"]
#[used]
pub static HHDM_REQUEST: LimineHhdmRequest = LimineHhdmRequest {
    id: LIMINE_HHDM_REQUEST_ID,
    revision: 0,
    response: LiminePtr(0),
};

// === MP (Multiprocessor) ===

#[repr(C)]
pub struct LimineMpInfo {
    pub processor_id: u32,
    pub lapic_id: u32,
    pub reserved: u64,
    pub goto_address: u64,
    pub extra_argument: u64,
}

#[repr(C)]
pub struct LimineMpResponse {
    pub revision: u64,
    pub flags: u32,
    pub bsp_lapic_id: u32,
    pub cpu_count: u64,
    pub cpus: LiminePtr,
}

#[repr(C)]
pub struct LimineMpRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: LiminePtr,
    pub flags: u64,
}

const LIMINE_MP_REQUEST_ID: [u64; 4] = [
    LIMINE_COMMON_MAGIC[0], LIMINE_COMMON_MAGIC[1],
    0x95a67b819a1b857e, 0xa0b61b723b6a73e0,
];

#[link_section = ".limine_reqs"]
#[used]
pub static MP_REQUEST: LimineMpRequest = LimineMpRequest {
    id: LIMINE_MP_REQUEST_ID,
    revision: 0,
    response: LiminePtr(0),
    flags: 0,
};

pub fn phys_to_virt(phys: u64) -> u64 {
    if HHDM_REQUEST.response.is_null() {
        return phys;
    }
    let resp: &LimineHhdmResponse = unsafe { &*HHDM_REQUEST.response.as_ptr() };
    phys + resp.offset
}

pub fn virt_to_phys(virt: u64) -> u64 {
    if HHDM_REQUEST.response.is_null() {
        return virt;
    }
    let resp: &LimineHhdmResponse = unsafe { &*HHDM_REQUEST.response.as_ptr() };
    virt - resp.offset
}

pub fn hhdm_offset() -> u64 {
    HHDM_OFFSET.load(Ordering::Relaxed)
}

// === Boot Info ===

pub struct BootInfo;

impl BootInfo {
    pub fn get() -> &'static Self {
        &BootInfo
    }

    pub fn memmap_count(&self) -> usize {
        let response: &LimineMemmapResponse =
            unsafe { &*MEMMAP_REQUEST.response.as_ptr() };
        response.entry_count as usize
    }

    pub fn memmap_entry(&self, index: usize) -> LimineMemmapEntry {
        let response: &LimineMemmapResponse =
            unsafe { &*MEMMAP_REQUEST.response.as_ptr() };
        let entry_ptrs = response.entries.as_ptr::<*mut LimineMemmapEntry>();
        let entry_ptr = unsafe { *entry_ptrs.add(index) };
        unsafe { core::ptr::read(entry_ptr) }
    }

    pub fn memory_map(&self) -> &'static [LimineMemmapEntry] {
        let response: &LimineMemmapResponse =
            unsafe { &*MEMMAP_REQUEST.response.as_ptr() };
        let count = response.entry_count as usize;
        if count == 0 {
            return &[];
        }
        let entry_ptrs = response.entries.as_ptr::<*mut LimineMemmapEntry>();
        // Assume entries are contiguously allocated (Limine does this)
        let first = unsafe { *entry_ptrs };
        unsafe { core::slice::from_raw_parts(first, count) }
    }

    pub fn rsdp(&self) -> Option<u64> {
        if RSDP_REQUEST.response.is_null() {
            return None;
        }
        let resp: &LimineRsdpResponse = unsafe { &*RSDP_REQUEST.response.as_ptr() };
        if resp.address.is_null() {
            None
        } else {
            Some(resp.address.0)
        }
    }

    pub fn modules(&self) -> &[LimineFile] {
        if MODULE_REQUEST.response.is_null() {
            return &[];
        }
        let resp: &LimineModuleResponse = unsafe { &*MODULE_REQUEST.response.as_ptr() };
        let count = resp.module_count as usize;
        if count == 0 {
            return &[];
        }
        let module_ptrs = resp.modules.as_ptr::<*mut LimineFile>();
        let first = unsafe { *module_ptrs };
        unsafe { core::slice::from_raw_parts(first, count) }
    }

    pub fn module_count(&self) -> usize {
        if MODULE_REQUEST.response.is_null() {
            return 0;
        }
        let resp: &LimineModuleResponse = unsafe { &*MODULE_REQUEST.response.as_ptr() };
        resp.module_count as usize
    }
}

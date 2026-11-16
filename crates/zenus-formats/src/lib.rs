#![no_std]

// The host test harness needs std; the bare-metal build uses alloc for Vec.
extern crate alloc;

#[cfg(test)]
extern crate std;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// Magic number for a Zenus Native Executable: bytes `5A 4E 53 58` = "ZNSX".
pub const ZNS_MAGIC: u32 = 0x58_53_4E_5A; // little-endian "ZNSX"

/// Current `.zns` format version.
pub const ZNS_VERSION: u16 = 1;

/// Fictional AetherX ISA advertised in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ZnsArch {
    Aex32 = 32,
    Aex64 = 64,
}

impl ZnsArch {
    pub fn from_u16(v: u16) -> Option<Self> {
        match v {
            32 => Some(ZnsArch::Aex32),
            64 => Some(ZnsArch::Aex64),
            _ => None,
        }
    }
}

/// Segment flags for the program header table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZnsSegFlags {
    pub read: bool,
    pub write: bool,
    pub exec: bool,
}

impl ZnsSegFlags {
    pub const NONE: Self = Self { read: false, write: false, exec: false };
    pub const RX: Self = Self { read: true, write: false, exec: true };
    pub const RW: Self = Self { read: true, write: true, exec: false };
    pub const R: Self = Self { read: true, write: false, exec: false };

    pub fn bits(self) -> u8 {
        let mut b = 0u8;
        if self.read { b |= 1; }
        if self.write { b |= 2; }
        if self.exec { b |= 4; }
        b
    }
}

/// One program-header entry: a segment to map into RAM.
#[derive(Debug, Clone)]
pub struct ZnsProgramHeader {
    pub offset: u64,      // file offset of the segment
    pub vaddr: u64,       // virtual address to load it at
    pub paddr: u64,       // physical address (informational)
    pub filesz: u64,      // size in the file
    pub memsz: u64,       // size in memory (may be larger, zero-filled)
    pub flags: ZnsSegFlags,
}

/// Executable header fixed-size prefix after the magic/version.
#[derive(Debug, Clone)]
pub struct ZnsExecHeader {
    pub entry_point: u64,
    pub arch: ZnsArch,
    pub image_size: u64,
    pub segment_count: u16,
    pub section_count: u16,
    pub flags: u32,
}

/// A parsed `.zns` executable.
#[derive(Debug, Clone)]
pub struct ZnsFile {
    pub header: ZnsExecHeader,
    pub segments: Vec<ZnsProgramHeader>,
    pub raw: Vec<u8>,
}

/// Errors while parsing or validating a `.zns` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZnsError {
    TooSmall,
    BadMagic { found: u32 },
    BadVersion { found: u16 },
    Truncated,
    InvalidSegment { index: usize, reason: &'static str },
    UnsupportedArch(u16),
}

impl fmt::Display for ZnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZnsError::TooSmall => write!(f, "zns: file too small to contain a header"),
            ZnsError::BadMagic { found } => {
                write!(f, "zns: invalid magic number 0x{:08X}, expected 0x{:08X} (\"ZNSX\")", found, ZNS_MAGIC)
            }
            ZnsError::BadVersion { found } => {
                write!(f, "zns: unsupported format version {}, expected {}", found, ZNS_VERSION)
            }
            ZnsError::Truncated => write!(f, "zns: file truncated while reading tables"),
            ZnsError::InvalidSegment { index, reason } => {
                write!(f, "zns: segment {} invalid: {}", index, reason)
            }
            ZnsError::UnsupportedArch(a) => write!(f, "zns: unsupported architecture {}", a),
        }
    }
}

impl ZnsFile {
    /// Parse a `.zns` image from a byte buffer. This is a pure, host-testable
    /// parser: it validates the magic, version and segment table but does not
    /// map memory or relocate anything.
    pub fn parse(data: &[u8]) -> Result<Self, ZnsError> {
        if data.len() < 8 {
            return Err(ZnsError::TooSmall);
        }

        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != ZNS_MAGIC {
            return Err(ZnsError::BadMagic { found: magic });
        }

        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != ZNS_VERSION {
            return Err(ZnsError::BadVersion { found: version });
        }

        // After magic(4) + version(2) = 6 bytes. The remaining fixed header
        // fields are read below. We require enough bytes for the fixed header.
        if data.len() < 30 {
            return Err(ZnsError::TooSmall);
        }

        let entry_point = u64::from_le_bytes(data[6..14].try_into().unwrap());
        let arch_raw = u16::from_le_bytes(data[14..16].try_into().unwrap());
        let arch = ZnsArch::from_u16(arch_raw).ok_or(ZnsError::UnsupportedArch(arch_raw))?;
        let image_size = u64::from_le_bytes(data[16..24].try_into().unwrap());
        let segment_count = u16::from_le_bytes(data[24..26].try_into().unwrap());
        let section_count = u16::from_le_bytes(data[26..28].try_into().unwrap());
        let flags = u32::from_le_bytes(data[28..32].try_into().unwrap());

        let header = ZnsExecHeader {
            entry_point,
            arch,
            image_size,
            segment_count,
            section_count,
            flags,
        };

        // Each program header is 48 bytes (6 x u64 + 1 u8 flags padded).
        let seg_size = 48usize;
        let seg_table_start = 32usize;
        let needed = seg_table_start + (segment_count as usize) * seg_size;
        if data.len() < needed {
            return Err(ZnsError::Truncated);
        }

        let mut segments = Vec::with_capacity(segment_count as usize);
        for i in 0..segment_count as usize {
            let base = seg_table_start + i * seg_size;
            let offset = u64::from_le_bytes(data[base..base + 8].try_into().unwrap());
            let vaddr = u64::from_le_bytes(data[base + 8..base + 16].try_into().unwrap());
            let paddr = u64::from_le_bytes(data[base + 16..base + 24].try_into().unwrap());
            let filesz = u64::from_le_bytes(data[base + 24..base + 32].try_into().unwrap());
            let memsz = u64::from_le_bytes(data[base + 32..base + 40].try_into().unwrap());
            let flags_raw = data[base + 40];
            let flags = ZnsSegFlags {
                read: flags_raw & 1 != 0,
                write: flags_raw & 2 != 0,
                exec: flags_raw & 4 != 0,
            };

            if memsz < filesz {
                return Err(ZnsError::InvalidSegment {
                    index: i,
                    reason: "memsz smaller than filesz",
                });
            }
            if offset as u128 + filesz as u128 > data.len() as u128 {
                return Err(ZnsError::InvalidSegment {
                    index: i,
                    reason: "segment data extends past end of file",
                });
            }

            segments.push(ZnsProgramHeader {
                offset,
                vaddr,
                paddr,
                filesz,
                memsz,
                flags,
            });
        }

        Ok(ZnsFile {
            header,
            segments,
            raw: Vec::from(data),
        })
    }

    /// Return the file bytes covering a given segment's initialized data.
    pub fn segment_data(&self, index: usize) -> Option<&[u8]> {
        let seg = self.segments.get(index)?;
        let start = seg.offset as usize;
        let end = start + seg.filesz as usize;
        self.raw.get(start..end)
    }
}

/// Magic number for a Zenus Native Library: bytes `5A 4E 4C 42` = "ZNLB".
pub const ZNL_MAGIC: u32 = 0x42_4C_4E_5A; // little-endian "ZNLB"

/// Current `.znl` format version.
pub const ZNL_VERSION: u16 = 1;

/// Build type advertised in a library header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ZnlBuild {
    Debug = 0,
    Release = 1,
}

impl ZnlBuild {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(ZnlBuild::Debug),
            1 => Some(ZnlBuild::Release),
            _ => None,
        }
    }
}

/// Library flags (shared, plugin, system).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZnlFlags {
    pub shared: bool,
    pub plugin: bool,
    pub system: bool,
}

impl ZnlFlags {
    pub const NONE: Self = Self { shared: false, plugin: false, system: false };
    pub const SHARED: Self = Self { shared: true, plugin: false, system: false };

    pub fn bits(self) -> u8 {
        let mut b = 0u8;
        if self.shared { b |= 1; }
        if self.plugin { b |= 2; }
        if self.system { b |= 4; }
        b
    }
}

/// Fixed-size library header after magic/version.
#[derive(Debug, Clone)]
pub struct ZnlLibraryHeader {
    pub abi_version: u16,
    pub arch: ZnsArch,
    pub build: ZnlBuild,
    pub flags: ZnlFlags,
    pub export_count: u16,
    pub import_count: u16,
}

/// One exported symbol: name + address.
#[derive(Debug, Clone)]
pub struct ZnlExport {
    pub name: String,
    pub address: u64,
}

/// One imported symbol: name, address, and (optional) library it comes from.
#[derive(Debug, Clone)]
pub struct ZnlImport {
    pub name: String,
    pub address: u64,
    pub library: Option<String>,
}

/// A parsed `.znl` shared library.
#[derive(Debug, Clone)]
pub struct ZnlFile {
    pub header: ZnlLibraryHeader,
    pub exports: Vec<ZnlExport>,
    pub imports: Vec<ZnlImport>,
    pub raw: Vec<u8>,
}

/// Errors while parsing or validating a `.znl` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZnlError {
    TooSmall,
    BadMagic { found: u32 },
    BadVersion { found: u16 },
    Truncated,
    InvalidUtf8,
    UnsupportedArch(u16),
    InvalidBuild(u8),
}

impl fmt::Display for ZnlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZnlError::TooSmall => write!(f, "znl: file too small to contain a header"),
            ZnlError::BadMagic { found } => {
                write!(f, "znl: invalid magic number 0x{:08X}, expected 0x{:08X} (\"ZNLB\")", found, ZNL_MAGIC)
            }
            ZnlError::BadVersion { found } => {
                write!(f, "znl: unsupported format version {}, expected {}", found, ZNL_VERSION)
            }
            ZnlError::Truncated => write!(f, "znl: file truncated while reading tables"),
            ZnlError::InvalidUtf8 => write!(f, "znl: symbol name is not valid UTF-8"),
            ZnlError::UnsupportedArch(a) => write!(f, "znl: unsupported architecture {}", a),
            ZnlError::InvalidBuild(b) => write!(f, "znl: invalid build type {}", b),
        }
    }
}

impl ZnlFile {
    /// Parse a `.znl` image from a byte buffer. Pure, host-testable parser:
    /// validates magic/version, reads the fixed header, then the export and
    /// import symbol tables (each entry: u64 address + u16 name length + name
    /// bytes + u8 library-length for imports).
    pub fn parse(data: &[u8]) -> Result<Self, ZnlError> {
        if data.len() < 8 {
            return Err(ZnlError::TooSmall);
        }

        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != ZNL_MAGIC {
            return Err(ZnlError::BadMagic { found: magic });
        }

        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != ZNL_VERSION {
            return Err(ZnlError::BadVersion { found: version });
        }

        if data.len() < 22 {
            return Err(ZnlError::TooSmall);
        }

        let abi_version = u16::from_le_bytes(data[6..8].try_into().unwrap());
        let arch_raw = u16::from_le_bytes(data[8..10].try_into().unwrap());
        let arch = ZnsArch::from_u16(arch_raw).ok_or(ZnlError::UnsupportedArch(arch_raw))?;
        let build_raw = data[10];
        let build = ZnlBuild::from_u8(build_raw).ok_or(ZnlError::InvalidBuild(build_raw))?;
        let flags_raw = data[11];
        let flags = ZnlFlags {
            shared: flags_raw & 1 != 0,
            plugin: flags_raw & 2 != 0,
            system: flags_raw & 4 != 0,
        };
        let export_count = u16::from_le_bytes(data[12..14].try_into().unwrap());
        let import_count = u16::from_le_bytes(data[14..16].try_into().unwrap());

        let header = ZnlLibraryHeader {
            abi_version,
            arch,
            build,
            flags,
            export_count,
            import_count,
        };

        let mut off = 16usize;
        let mut exports = Vec::with_capacity(export_count as usize);
        for i in 0..export_count as usize {
            let addr = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
            off += 8;
            let nlen = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
            off += 2;
            if off + nlen > data.len() {
                return Err(ZnlError::Truncated);
            }
            let name = core::str::from_utf8(&data[off..off + nlen])
                .map_err(|_| ZnlError::InvalidUtf8)?
                .to_string();
            off += nlen;
            exports.push(ZnlExport { name, address: addr });
            let _ = i;
        }

        let mut imports = Vec::with_capacity(import_count as usize);
        for i in 0..import_count as usize {
            let addr = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
            off += 8;
            let nlen = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
            off += 2;
            if off + nlen > data.len() {
                return Err(ZnlError::Truncated);
            }
            let name = core::str::from_utf8(&data[off..off + nlen])
                .map_err(|_| ZnlError::InvalidUtf8)?
                .to_string();
            off += nlen;
            let lib_len = data[off] as usize;
            off += 1;
            let library = if off + lib_len > data.len() {
                return Err(ZnlError::Truncated);
            } else if lib_len == 0 {
                None
            } else {
                Some(
                    core::str::from_utf8(&data[off..off + lib_len])
                        .map_err(|_| ZnlError::InvalidUtf8)?
                        .to_string(),
                )
            };
            off += lib_len;
            imports.push(ZnlImport {
                name,
                address: addr,
                library,
            });
            let _ = i;
        }

        Ok(ZnlFile {
            header,
            exports,
            imports,
            raw: Vec::from(data),
        })
    }

    /// Look up an exported symbol by name, returning its address if present.
    pub fn export_address(&self, name: &str) -> Option<u64> {
        self.exports.iter().find(|e| e.name == name).map(|e| e.address)
    }
}

/// Magic number for a Zenus Native Driver: bytes `5A 4E 44 52` = "ZNDR".
pub const ZND_MAGIC: u32 = 0x52_44_4E_5A; // little-endian "ZNDR"

/// Current `.znd` format version.
pub const ZND_VERSION: u16 = 1;

/// Class of hardware the driver targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ZndDriverType {
    Storage = 0,
    Gpu = 1,
    Usb = 2,
    Audio = 3,
    Ethernet = 4,
    Wifi = 5,
    Bluetooth = 6,
    Camera = 7,
    Touchscreen = 8,
}

impl ZndDriverType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(ZndDriverType::Storage),
            1 => Some(ZndDriverType::Gpu),
            2 => Some(ZndDriverType::Usb),
            3 => Some(ZndDriverType::Audio),
            4 => Some(ZndDriverType::Ethernet),
            5 => Some(ZndDriverType::Wifi),
            6 => Some(ZndDriverType::Bluetooth),
            7 => Some(ZndDriverType::Camera),
            8 => Some(ZndDriverType::Touchscreen),
            _ => None,
        }
    }
}

/// Driver flags (boot, system, hotplug, secure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZndFlags {
    pub boot: bool,
    pub system: bool,
    pub hotplug: bool,
    pub secure: bool,
}

impl ZndFlags {
    pub const NONE: Self = Self { boot: false, system: false, hotplug: false, secure: false };

    pub fn bits(self) -> u8 {
        let mut b = 0u8;
        if self.boot { b |= 1; }
        if self.system { b |= 2; }
        if self.hotplug { b |= 4; }
        if self.secure { b |= 8; }
        b
    }
}

/// One hardware ID the driver matches against (vendor + device).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZndHardwareId {
    pub vendor_id: u16,
    pub device_id: u16,
}

/// Fixed-size driver header after magic/version.
#[derive(Debug, Clone)]
pub struct ZndDriverHeader {
    pub version: u16,
    pub arch: ZnsArch,
    pub driver_type: ZndDriverType,
    pub build: ZnlBuild,
    pub flags: ZndFlags,
    pub hardware_id_count: u16,
    pub capability_count: u16,
}

/// A parsed `.znd` driver.
#[derive(Debug, Clone)]
pub struct ZndFile {
    pub header: ZndDriverHeader,
    pub hardware_ids: Vec<ZndHardwareId>,
    pub capabilities: Vec<String>,
    pub raw: Vec<u8>,
}

/// Errors while parsing or validating a `.znd` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZndError {
    TooSmall,
    BadMagic { found: u32 },
    BadVersion { found: u16 },
    Truncated,
    InvalidUtf8,
    UnsupportedArch(u16),
    InvalidDriverType(u8),
    InvalidBuild(u8),
}

impl fmt::Display for ZndError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZndError::TooSmall => write!(f, "znd: file too small to contain a header"),
            ZndError::BadMagic { found } => {
                write!(f, "znd: invalid magic number 0x{:08X}, expected 0x{:08X} (\"ZNDR\")", found, ZND_MAGIC)
            }
            ZndError::BadVersion { found } => {
                write!(f, "znd: unsupported format version {}, expected {}", found, ZND_VERSION)
            }
            ZndError::Truncated => write!(f, "znd: file truncated while reading tables"),
            ZndError::InvalidUtf8 => write!(f, "znd: capability string is not valid UTF-8"),
            ZndError::UnsupportedArch(a) => write!(f, "znd: unsupported architecture {}", a),
            ZndError::InvalidDriverType(t) => write!(f, "znd: invalid driver type {}", t),
            ZndError::InvalidBuild(b) => write!(f, "znd: invalid build type {}", b),
        }
    }
}

impl ZndFile {
    /// Parse a `.znd` image from a byte buffer. Pure, host-testable parser:
    /// validates magic/version, reads the fixed header, then the hardware ID
    /// table (each: u16 vendor + u16 device) and the capability table (each:
    /// u16 length + UTF-8 bytes).
    pub fn parse(data: &[u8]) -> Result<Self, ZndError> {
        if data.len() < 8 {
            return Err(ZndError::TooSmall);
        }

        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != ZND_MAGIC {
            return Err(ZndError::BadMagic { found: magic });
        }

        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != ZND_VERSION {
            return Err(ZndError::BadVersion { found: version });
        }

        if data.len() < 18 {
            return Err(ZndError::TooSmall);
        }

        let hdr_version = u16::from_le_bytes(data[6..8].try_into().unwrap());
        let arch_raw = u16::from_le_bytes(data[8..10].try_into().unwrap());
        let arch = ZnsArch::from_u16(arch_raw).ok_or(ZndError::UnsupportedArch(arch_raw))?;
        let driver_type_raw = data[10];
        let driver_type = ZndDriverType::from_u8(driver_type_raw)
            .ok_or(ZndError::InvalidDriverType(driver_type_raw))?;
        let build_raw = data[11];
        let build = ZnlBuild::from_u8(build_raw).ok_or(ZndError::InvalidBuild(build_raw))?;
        let flags_raw = data[12];
        let flags = ZndFlags {
            boot: flags_raw & 1 != 0,
            system: flags_raw & 2 != 0,
            hotplug: flags_raw & 4 != 0,
            secure: flags_raw & 8 != 0,
        };
        let hardware_id_count = u16::from_le_bytes(data[13..15].try_into().unwrap());
        let capability_count = u16::from_le_bytes(data[15..17].try_into().unwrap());

        let header = ZndDriverHeader {
            version: hdr_version,
            arch,
            driver_type,
            build,
            flags,
            hardware_id_count,
            capability_count,
        };

        let mut off = 17usize;
        let mut hardware_ids = Vec::with_capacity(hardware_id_count as usize);
        for _ in 0..hardware_id_count as usize {
            if off + 4 > data.len() {
                return Err(ZndError::Truncated);
            }
            let vendor_id = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
            let device_id = u16::from_le_bytes(data[off + 2..off + 4].try_into().unwrap());
            off += 4;
            hardware_ids.push(ZndHardwareId { vendor_id, device_id });
        }

        let mut capabilities = Vec::with_capacity(capability_count as usize);
        for _ in 0..capability_count as usize {
            if off + 2 > data.len() {
                return Err(ZndError::Truncated);
            }
            let clen = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
            off += 2;
            if off + clen > data.len() {
                return Err(ZndError::Truncated);
            }
            let cap = core::str::from_utf8(&data[off..off + clen])
                .map_err(|_| ZndError::InvalidUtf8)?
                .to_string();
            off += clen;
            capabilities.push(cap);
        }

        Ok(ZndFile {
            header,
            hardware_ids,
            capabilities,
            raw: Vec::from(data),
        })
    }

    /// Return true if the driver claims to handle the given vendor/device ID.
    pub fn matches_hardware(&self, vendor_id: u16, device_id: u16) -> bool {
        self.hardware_ids
            .iter()
            .any(|id| id.vendor_id == vendor_id && id.device_id == device_id)
    }
}

/// Magic number for a Zenus Native Module: bytes `5A 4E 4B 4D` = "ZNKM".
pub const ZNM_MAGIC: u32 = 0x4D_4B_4E_5A; // little-endian "ZNKM"

/// Current `.znm` format version.
pub const ZNM_VERSION: u16 = 1;

/// Module flags (core, optional, experimental).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZnmFlags {
    pub core: bool,
    pub optional: bool,
    pub experimental: bool,
}

impl ZnmFlags {
    pub const NONE: Self = Self { core: false, optional: false, experimental: false };
    pub const CORE: Self = Self { core: true, optional: false, experimental: false };

    pub fn bits(self) -> u8 {
        let mut b = 0u8;
        if self.core { b |= 1; }
        if self.optional { b |= 2; }
        if self.experimental { b |= 4; }
        b
    }
}

/// Fixed-size module header after magic/version.
#[derive(Debug, Clone)]
pub struct ZnmModuleHeader {
    pub abi_version: u16,
    pub arch: ZnsArch,
    pub minimum_kernel: u16,
    pub build: ZnlBuild,
    pub flags: ZnmFlags,
    pub dependency_count: u16,
    pub export_count: u16,
    pub import_count: u16,
    pub init_count: u16,
    pub cleanup_count: u16,
    pub permission_count: u16,
}

/// A parsed `.znm` kernel module.
#[derive(Debug, Clone)]
pub struct ZnmFile {
    pub header: ZnmModuleHeader,
    pub dependencies: Vec<String>,
    pub exports: Vec<String>,
    pub imports: Vec<String>,
    pub init_functions: Vec<String>,
    pub cleanup_functions: Vec<String>,
    pub permissions: Vec<String>,
    pub raw: Vec<u8>,
}

/// Errors while parsing or validating a `.znm` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZnmError {
    TooSmall,
    BadMagic { found: u32 },
    BadVersion { found: u16 },
    Truncated,
    InvalidUtf8,
    UnsupportedArch(u16),
    InvalidBuild(u8),
}

impl fmt::Display for ZnmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZnmError::TooSmall => write!(f, "znm: file too small to contain a header"),
            ZnmError::BadMagic { found } => {
                write!(f, "znm: invalid magic number 0x{:08X}, expected 0x{:08X} (\"ZNKM\")", found, ZNM_MAGIC)
            }
            ZnmError::BadVersion { found } => {
                write!(f, "znm: unsupported format version {}, expected {}", found, ZNM_VERSION)
            }
            ZnmError::Truncated => write!(f, "znm: file truncated while reading tables"),
            ZnmError::InvalidUtf8 => write!(f, "znm: symbol/dependency string is not valid UTF-8"),
            ZnmError::UnsupportedArch(a) => write!(f, "znm: unsupported architecture {}", a),
            ZnmError::InvalidBuild(b) => write!(f, "znm: invalid build type {}", b),
        }
    }
}

impl ZnmFile {
    /// Parse a `.znm` image from a byte buffer. Pure, host-testable parser:
    /// validates magic/version, reads the fixed header, then reads a series of
    /// length-prefixed UTF-8 string tables (dependencies, exports, imports,
    /// init, cleanup, permissions).
    pub fn parse(data: &[u8]) -> Result<Self, ZnmError> {
        if data.len() < 8 {
            return Err(ZnmError::TooSmall);
        }

        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != ZNM_MAGIC {
            return Err(ZnmError::BadMagic { found: magic });
        }

        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != ZNM_VERSION {
            return Err(ZnmError::BadVersion { found: version });
        }

        if data.len() < 30 {
            return Err(ZnmError::TooSmall);
        }

        let abi_version = u16::from_le_bytes(data[6..8].try_into().unwrap());
        let arch_raw = u16::from_le_bytes(data[8..10].try_into().unwrap());
        let arch = ZnsArch::from_u16(arch_raw).ok_or(ZnmError::UnsupportedArch(arch_raw))?;
        let minimum_kernel = u16::from_le_bytes(data[10..12].try_into().unwrap());
        let build_raw = data[12];
        let build = ZnlBuild::from_u8(build_raw).ok_or(ZnmError::InvalidBuild(build_raw))?;
        let flags_raw = data[13];
        let flags = ZnmFlags {
            core: flags_raw & 1 != 0,
            optional: flags_raw & 2 != 0,
            experimental: flags_raw & 4 != 0,
        };
        let dependency_count = u16::from_le_bytes(data[14..16].try_into().unwrap());
        let export_count = u16::from_le_bytes(data[16..18].try_into().unwrap());
        let import_count = u16::from_le_bytes(data[18..20].try_into().unwrap());
        let init_count = u16::from_le_bytes(data[20..22].try_into().unwrap());
        let cleanup_count = u16::from_le_bytes(data[22..24].try_into().unwrap());
        let permission_count = u16::from_le_bytes(data[24..26].try_into().unwrap());

        let header = ZnmModuleHeader {
            abi_version,
            arch,
            minimum_kernel,
            build,
            flags,
            dependency_count,
            export_count,
            import_count,
            init_count,
            cleanup_count,
            permission_count,
        };

        let mut off = 26usize;
        let mut read_strings = |count: u16, target: &mut Vec<String>| -> Result<(), ZnmError> {
            for _ in 0..count as usize {
                if off + 2 > data.len() {
                    return Err(ZnmError::Truncated);
                }
                let len = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
                off += 2;
                if off + len > data.len() {
                    return Err(ZnmError::Truncated);
                }
                let s = core::str::from_utf8(&data[off..off + len])
                    .map_err(|_| ZnmError::InvalidUtf8)?
                    .to_string();
                off += len;
                target.push(s);
            }
            Ok(())
        };

        let mut dependencies = Vec::with_capacity(dependency_count as usize);
        read_strings(dependency_count, &mut dependencies)?;
        let mut exports = Vec::with_capacity(export_count as usize);
        read_strings(export_count, &mut exports)?;
        let mut imports = Vec::with_capacity(import_count as usize);
        read_strings(import_count, &mut imports)?;
        let mut init_functions = Vec::with_capacity(init_count as usize);
        read_strings(init_count, &mut init_functions)?;
        let mut cleanup_functions = Vec::with_capacity(cleanup_count as usize);
        read_strings(cleanup_count, &mut cleanup_functions)?;
        let mut permissions = Vec::with_capacity(permission_count as usize);
        read_strings(permission_count, &mut permissions)?;

        Ok(ZnmFile {
            header,
            dependencies,
            exports,
            imports,
            init_functions,
            cleanup_functions,
            permissions,
            raw: Vec::from(data),
        })
    }

    /// Return true if the module exports the given symbol name.
    pub fn exports_symbol(&self, name: &str) -> bool {
        self.exports.iter().any(|e| e == name)
    }

    /// Return true if the module requires the given dependency.
    pub fn requires(&self, dep: &str) -> bool {
        self.dependencies.iter().any(|d| d == dep)
    }
}

/// Magic number for a Zenus Native Package: bytes `5A 4E 50 4B` = "ZNPK".
pub const ZNPKG_MAGIC: u32 = 0x4B_50_4E_5A; // little-endian "ZNPK"

/// Current `.znpkg` format version.
pub const ZNPKG_VERSION: u16 = 1;

/// Type of a file contained inside a `.znpkg` package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ZnpkgFileType {
    Executable = 0,
    Library = 1,
    Driver = 2,
    Module = 3,
    Archive = 4,
    Firmware = 5,
    Configuration = 6,
    Log = 7,
    Other = 255,
}

impl ZnpkgFileType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(ZnpkgFileType::Executable),
            1 => Some(ZnpkgFileType::Library),
            2 => Some(ZnpkgFileType::Driver),
            3 => Some(ZnpkgFileType::Module),
            4 => Some(ZnpkgFileType::Archive),
            5 => Some(ZnpkgFileType::Firmware),
            6 => Some(ZnpkgFileType::Configuration),
            7 => Some(ZnpkgFileType::Log),
            255 => Some(ZnpkgFileType::Other),
            _ => None,
        }
    }
}

/// One file entry inside a package: name, offset, size, and type.
#[derive(Debug, Clone)]
pub struct ZnpkgEntry {
    pub name: String,
    pub offset: u64,
    pub size: u64,
    pub file_type: ZnpkgFileType,
}

/// Fixed-size package header after magic/version.
#[derive(Debug, Clone)]
pub struct ZnpkgHeader {
    pub version: u16,
    pub file_count: u16,
    pub manifest_offset: u64,
    pub manifest_size: u64,
    pub script_offset: u64,
    pub script_size: u64,
}

/// A parsed `.znpkg` package (a container of `.zn*` files plus a manifest and
/// an install script).
#[derive(Debug, Clone)]
pub struct ZnpkgFile {
    pub header: ZnpkgHeader,
    pub entries: Vec<ZnpkgEntry>,
    pub manifest: Vec<u8>,
    pub install_script: Vec<u8>,
    pub raw: Vec<u8>,
}

/// Errors while parsing or validating a `.znpkg` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZnpkgError {
    TooSmall,
    BadMagic { found: u32 },
    BadVersion { found: u16 },
    Truncated,
    InvalidUtf8,
    InvalidFileType(u8),
    EntryOutOfBounds,
}

impl fmt::Display for ZnpkgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZnpkgError::TooSmall => write!(f, "znpkg: file too small to contain a header"),
            ZnpkgError::BadMagic { found } => {
                write!(f, "znpkg: invalid magic number 0x{:08X}, expected 0x{:08X} (\"ZNPK\")", found, ZNPKG_MAGIC)
            }
            ZnpkgError::BadVersion { found } => {
                write!(f, "znpkg: unsupported format version {}, expected {}", found, ZNPKG_VERSION)
            }
            ZnpkgError::Truncated => write!(f, "znpkg: file truncated while reading tables"),
            ZnpkgError::InvalidUtf8 => write!(f, "znpkg: entry name or manifest is not valid UTF-8"),
            ZnpkgError::InvalidFileType(t) => write!(f, "znpkg: invalid file type {}", t),
            ZnpkgError::EntryOutOfBounds => write!(f, "znpkg: entry data extends past end of package"),
        }
    }
}

impl ZnpkgFile {
    /// Parse a `.znpkg` image from a byte buffer. Pure, host-testable parser:
    /// validates magic/version, reads the fixed header, then the entry table
    /// (each: u16 name length + name + u64 offset + u64 size + u8 type), the
    /// manifest blob, and the install script blob.
    pub fn parse(data: &[u8]) -> Result<Self, ZnpkgError> {
        if data.len() < 8 {
            return Err(ZnpkgError::TooSmall);
        }

        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != ZNPKG_MAGIC {
            return Err(ZnpkgError::BadMagic { found: magic });
        }

        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != ZNPKG_VERSION {
            return Err(ZnpkgError::BadVersion { found: version });
        }

        if data.len() < 30 {
            return Err(ZnpkgError::TooSmall);
        }

        let hdr_version = u16::from_le_bytes(data[6..8].try_into().unwrap());
        let file_count = u16::from_le_bytes(data[8..10].try_into().unwrap());
        let manifest_offset = u64::from_le_bytes(data[10..18].try_into().unwrap());
        let manifest_size = u64::from_le_bytes(data[18..26].try_into().unwrap());
        let script_offset = u64::from_le_bytes(data[26..34].try_into().unwrap());
        let script_size = u64::from_le_bytes(data[34..42].try_into().unwrap());

        let header = ZnpkgHeader {
            version: hdr_version,
            file_count,
            manifest_offset,
            manifest_size,
            script_offset,
            script_size,
        };

        let mut off = 42usize;
        let mut entries = Vec::with_capacity(file_count as usize);
        for _ in 0..file_count as usize {
            if off + 2 > data.len() {
                return Err(ZnpkgError::Truncated);
            }
            let nlen = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
            off += 2;
            if off + nlen > data.len() {
                return Err(ZnpkgError::Truncated);
            }
            let name = core::str::from_utf8(&data[off..off + nlen])
                .map_err(|_| ZnpkgError::InvalidUtf8)?
                .to_string();
            off += nlen;
            if off + 16 > data.len() {
                return Err(ZnpkgError::Truncated);
            }
            let offset = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
            let size = u64::from_le_bytes(data[off + 8..off + 16].try_into().unwrap());
            off += 16;
            let type_raw = data[off];
            off += 1;
            let file_type = ZnpkgFileType::from_u8(type_raw).ok_or(ZnpkgError::InvalidFileType(type_raw))?;
            if offset as u128 + size as u128 > data.len() as u128 {
                return Err(ZnpkgError::EntryOutOfBounds);
            }
            entries.push(ZnpkgEntry { name, offset, size, file_type });
        }

        // Manifest blob.
        let manifest = if header.manifest_offset as usize + header.manifest_size as usize > data.len() {
            return Err(ZnpkgError::Truncated);
        } else {
            Vec::from(&data[header.manifest_offset as usize
                ..(header.manifest_offset + header.manifest_size) as usize])
        };

        // Install script blob.
        let install_script = if header.script_offset as usize + header.script_size as usize > data.len() {
            return Err(ZnpkgError::Truncated);
        } else {
            Vec::from(&data[header.script_offset as usize
                ..(header.script_offset + header.script_size) as usize])
        };

        Ok(ZnpkgFile {
            header,
            entries,
            manifest,
            install_script,
            raw: Vec::from(data),
        })
    }

    /// Return the data bytes for the entry with the given name, if present.
    pub fn entry_data(&self, name: &str) -> Option<&[u8]> {
        let entry = self.entries.iter().find(|e| e.name == name)?;
        let start = entry.offset as usize;
        let end = start + entry.size as usize;
        self.raw.get(start..end)
    }
}

/// Magic number for a Zenus Native Image: bytes `5A 4E 49 4D` = "ZNIM".
pub const ZNI_MAGIC: u32 = 0x4D_49_4E_5A; // little-endian "ZNIM"

/// Current `.zni` format version.
pub const ZNI_VERSION: u16 = 1;

/// Type of system image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ZniImageType {
    Boot = 0,
    Recovery = 1,
    Installer = 2,
    Vm = 3,
    Live = 4,
    Server = 5,
    Embedded = 6,
    ContainerBase = 7,
}

impl ZniImageType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(ZniImageType::Boot),
            1 => Some(ZniImageType::Recovery),
            2 => Some(ZniImageType::Installer),
            3 => Some(ZniImageType::Vm),
            4 => Some(ZniImageType::Live),
            5 => Some(ZniImageType::Server),
            6 => Some(ZniImageType::Embedded),
            7 => Some(ZniImageType::ContainerBase),
            _ => None,
        }
    }
}

/// Compression algorithm advertised in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ZniCompression {
    None = 0,
    Lz4 = 1,
    Zstd = 2,
    Xz = 3,
}

impl ZniCompression {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(ZniCompression::None),
            1 => Some(ZniCompression::Lz4),
            2 => Some(ZniCompression::Zstd),
            3 => Some(ZniCompression::Xz),
            _ => None,
        }
    }
}

/// Encryption method advertised in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ZniEncryption {
    None = 0,
    Aes256 = 1,
}

impl ZniEncryption {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(ZniEncryption::None),
            1 => Some(ZniEncryption::Aes256),
            _ => None,
        }
    }
}

/// One partition described in the image's partition table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZniPartition {
    pub name: String,
    pub image_type: ZniImageType,
    pub offset: u64,
    pub size: u64,
}

/// Fixed-size image header after magic/version.
#[derive(Debug, Clone)]
pub struct ZniImageHeader {
    pub version: u16,
    pub arch: ZnsArch,
    pub image_type: ZniImageType,
    pub compression: ZniCompression,
    pub encryption: ZniEncryption,
    pub image_size: u64,
    pub partition_count: u16,
}

/// A parsed `.zni` system image.
#[derive(Debug, Clone)]
pub struct ZniFile {
    pub header: ZniImageHeader,
    pub partitions: Vec<ZniPartition>,
    pub metadata: Vec<u8>,
    pub checksum: Vec<u8>,
    pub signature: Vec<u8>,
    pub raw: Vec<u8>,
}

/// Errors while parsing or validating a `.zni` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZniError {
    TooSmall,
    BadMagic { found: u32 },
    BadVersion { found: u16 },
    Truncated,
    InvalidUtf8,
    UnsupportedArch(u16),
    InvalidImageType(u8),
    InvalidCompression(u8),
    InvalidEncryption(u8),
    PartitionOutOfBounds,
}

impl fmt::Display for ZniError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZniError::TooSmall => write!(f, "zni: file too small to contain a header"),
            ZniError::BadMagic { found } => {
                write!(f, "zni: invalid magic number 0x{:08X}, expected 0x{:08X} (\"ZNIM\")", found, ZNI_MAGIC)
            }
            ZniError::BadVersion { found } => {
                write!(f, "zni: unsupported format version {}, expected {}", found, ZNI_VERSION)
            }
            ZniError::Truncated => write!(f, "zni: file truncated while reading tables"),
            ZniError::InvalidUtf8 => write!(f, "zni: partition name is not valid UTF-8"),
            ZniError::UnsupportedArch(a) => write!(f, "zni: unsupported architecture {}", a),
            ZniError::InvalidImageType(t) => write!(f, "zni: invalid image type {}", t),
            ZniError::InvalidCompression(c) => write!(f, "zni: invalid compression {}", c),
            ZniError::InvalidEncryption(e) => write!(f, "zni: invalid encryption {}", e),
            ZniError::PartitionOutOfBounds => write!(f, "zni: partition data extends past end of image"),
        }
    }
}

impl ZniFile {
    /// Parse a `.zni` image from a byte buffer. Pure, host-testable parser:
    /// validates magic/version, reads the fixed header, then the partition
    /// table (each: u16 name length + name + u8 type + u64 offset + u64 size),
    /// followed by the metadata, checksum, and signature blobs.
    pub fn parse(data: &[u8]) -> Result<Self, ZniError> {
        if data.len() < 8 {
            return Err(ZniError::TooSmall);
        }

        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != ZNI_MAGIC {
            return Err(ZniError::BadMagic { found: magic });
        }

        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != ZNI_VERSION {
            return Err(ZniError::BadVersion { found: version });
        }

        if data.len() < 38 {
            return Err(ZniError::TooSmall);
        }

        let hdr_version = u16::from_le_bytes(data[6..8].try_into().unwrap());
        let arch_raw = u16::from_le_bytes(data[8..10].try_into().unwrap());
        let arch = ZnsArch::from_u16(arch_raw).ok_or(ZniError::UnsupportedArch(arch_raw))?;
        let image_type_raw = data[10];
        let image_type = ZniImageType::from_u8(image_type_raw)
            .ok_or(ZniError::InvalidImageType(image_type_raw))?;
        let compression_raw = data[11];
        let compression = ZniCompression::from_u8(compression_raw)
            .ok_or(ZniError::InvalidCompression(compression_raw))?;
        let encryption_raw = data[12];
        let encryption = ZniEncryption::from_u8(encryption_raw)
            .ok_or(ZniError::InvalidEncryption(encryption_raw))?;
        let image_size = u64::from_le_bytes(data[13..21].try_into().unwrap());
        let partition_count = u16::from_le_bytes(data[21..23].try_into().unwrap());
        let metadata_offset = u64::from_le_bytes(data[23..31].try_into().unwrap());
        let metadata_size = u64::from_le_bytes(data[31..39].try_into().unwrap());
        let checksum_offset = u64::from_le_bytes(data[39..47].try_into().unwrap());
        let checksum_size = u64::from_le_bytes(data[47..55].try_into().unwrap());
        let signature_offset = u64::from_le_bytes(data[55..63].try_into().unwrap());
        let signature_size = u64::from_le_bytes(data[63..71].try_into().unwrap());

        let header = ZniImageHeader {
            version: hdr_version,
            arch,
            image_type,
            compression,
            encryption,
            image_size,
            partition_count,
        };

        let mut off = 71usize;
        let mut partitions = Vec::with_capacity(partition_count as usize);
        for _ in 0..partition_count as usize {
            if off + 2 > data.len() {
                return Err(ZniError::Truncated);
            }
            let nlen = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
            off += 2;
            if off + nlen > data.len() {
                return Err(ZniError::Truncated);
            }
            let name = core::str::from_utf8(&data[off..off + nlen])
                .map_err(|_| ZniError::InvalidUtf8)?
                .to_string();
            off += nlen;
            if off + 17 > data.len() {
                return Err(ZniError::Truncated);
            }
            let part_type_raw = data[off];
            off += 1;
            let part_type = ZniImageType::from_u8(part_type_raw)
                .ok_or(ZniError::InvalidImageType(part_type_raw))?;
            let offset = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
            let size = u64::from_le_bytes(data[off + 8..off + 16].try_into().unwrap());
            off += 16;
            if offset as u128 + size as u128 > data.len() as u128 {
                return Err(ZniError::PartitionOutOfBounds);
            }
            partitions.push(ZniPartition { name, image_type: part_type, offset, size });
        }

        let blob = |off: u64, size: u64| -> Result<Vec<u8>, ZniError> {
            let o = off as usize;
            let s = size as usize;
            if o + s > data.len() {
                return Err(ZniError::Truncated);
            }
            Ok(Vec::from(&data[o..o + s]))
        };

        let metadata = blob(metadata_offset, metadata_size)?;
        let checksum = blob(checksum_offset, checksum_size)?;
        let signature = blob(signature_offset, signature_size)?;

        Ok(ZniFile {
            header,
            partitions,
            metadata,
            checksum,
            signature,
            raw: Vec::from(data),
        })
    }

    /// Return the data bytes for the partition with the given name, if present.
    pub fn partition_data(&self, name: &str) -> Option<&[u8]> {
        let part = self.partitions.iter().find(|p| p.name == name)?;
        let start = part.offset as usize;
        let end = start + part.size as usize;
        self.raw.get(start..end)
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;

    fn make_valid_image() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&ZNS_MAGIC.to_le_bytes());
        buf.extend_from_slice(&ZNS_VERSION.to_le_bytes());
        // entry_point, arch(Aex64), image_size, segment_count, section_count, flags
        buf.extend_from_slice(&0x0040_0000u64.to_le_bytes());
        buf.extend_from_slice(&64u16.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        // one RX segment: offset 0x1000, vaddr 0x400000, size 4096
        buf.extend_from_slice(&0x1000u64.to_le_bytes());
        buf.extend_from_slice(&0x0040_0000u64.to_le_bytes());
        buf.extend_from_slice(&0x0040_0000u64.to_le_bytes());
        buf.extend_from_slice(&4096u64.to_le_bytes());
        buf.extend_from_slice(&4096u64.to_le_bytes());
        buf.extend_from_slice(&[ZnsSegFlags::RX.bits(), 0, 0, 0, 0, 0, 0, 0]);
        // pad out to cover the segment data
        buf.resize(0x1000 + 4096, 0);
        buf
    }

    #[test]
    fn zns_parses_valid_image() {
        let img = make_valid_image();
        let f = ZnsFile::parse(&img).expect("valid image should parse");
        assert_eq!(f.header.entry_point, 0x0040_0000);
        assert_eq!(f.header.arch, ZnsArch::Aex64);
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].flags, ZnsSegFlags::RX);
        assert_eq!(f.segment_data(0).map(|d| d.len()), Some(4096));
    }

    #[test]
    fn zns_rejects_bad_magic() {
        let mut img = make_valid_image();
        img[0] = 0x00;
        match ZnsFile::parse(&img) {
            Err(ZnsError::BadMagic { found }) => assert_ne!(found, ZNS_MAGIC),
            other => panic!("expected BadMagic, got {:?}", other),
        }
    }

    #[test]
    fn zns_rejects_bad_version() {
        let mut img = make_valid_image();
        img[4] = 9;
        match ZnsFile::parse(&img) {
            Err(ZnsError::BadVersion { found }) => assert_eq!(found, 9),
            other => panic!("expected BadVersion, got {:?}", other),
        }
    }

    #[test]
    fn zns_rejects_segment_past_eof() {
        // Build an image with two segment headers. Segment 0 is valid (offset
        // 0x1000, filesz 4096). Segment 1 claims data that runs past EOF.
        let seg_data_len = 4096usize;
        let file_len = 0x1000 + seg_data_len;
        let mut img = alloc::vec![0u8; file_len];
        // magic + version
        img[0..4].copy_from_slice(&ZNS_MAGIC.to_le_bytes());
        img[4..6].copy_from_slice(&ZNS_VERSION.to_le_bytes());
        // fixed exec header: entry, arch, image_size, seg_count=2, sec_count, flags
        img[6..14].copy_from_slice(&0x0040_0000u64.to_le_bytes());
        img[14..16].copy_from_slice(&64u16.to_le_bytes());
        img[16..24].copy_from_slice(&(file_len as u64).to_le_bytes());
        img[24..26].copy_from_slice(&2u16.to_le_bytes());
        // segment table starts at 32; each entry 48 bytes
        // segment 0: valid RX segment
        img[32..40].copy_from_slice(&0x1000u64.to_le_bytes());      // offset
        img[40..48].copy_from_slice(&0x0040_0000u64.to_le_bytes()); // vaddr
        img[48..56].copy_from_slice(&0x0040_0000u64.to_le_bytes()); // paddr
        img[56..64].copy_from_slice(&4096u64.to_le_bytes());        // filesz
        img[64..72].copy_from_slice(&4096u64.to_le_bytes());        // memsz
        img[72] = ZnsSegFlags::RX.bits();
        // segment 1: offset = file_len (past data), filesz large -> past EOF
        img[80..88].copy_from_slice(&(file_len as u64).to_le_bytes()); // offset past EOF
        img[88..96].copy_from_slice(&0u64.to_le_bytes());
        img[96..104].copy_from_slice(&0u64.to_le_bytes());
        img[104..112].copy_from_slice(&100u64.to_le_bytes());       // filesz
        img[112..120].copy_from_slice(&100u64.to_le_bytes());       // memsz
        img[120] = 1;

        match ZnsFile::parse(&img) {
            Err(ZnsError::InvalidSegment { index, .. }) => assert_eq!(index, 1),
            other => panic!("expected InvalidSegment, got {:?}", other),
        }
    }

    #[test]
    fn zns_rejects_unsupported_arch() {
        let mut img = make_valid_image();
        img[14..16].copy_from_slice(&99u16.to_le_bytes());
        match ZnsFile::parse(&img) {
            Err(ZnsError::UnsupportedArch(a)) => assert_eq!(a, 99),
            other => panic!("expected UnsupportedArch, got {:?}", other),
        }
    }

    #[test]
    fn zns_segment_flags_bits() {
        assert_eq!(ZnsSegFlags::RX.bits(), 0b101);
        assert_eq!(ZnsSegFlags::RW.bits(), 0b011);
        assert_eq!(ZnsSegFlags::R.bits(), 0b001);
        assert_eq!(ZnsSegFlags::NONE.bits(), 0);
    }

    fn make_valid_znl() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&ZNL_MAGIC.to_le_bytes());
        buf.extend_from_slice(&ZNL_VERSION.to_le_bytes());
        // abi_version, arch(Aex64), build(Release), flags(shared),
        // export_count=1, import_count=1
        buf.extend_from_slice(&5u16.to_le_bytes());
        buf.extend_from_slice(&64u16.to_le_bytes());
        buf.extend_from_slice(&[ZnlBuild::Release as u8]);
        buf.extend_from_slice(&[ZnlFlags::SHARED.bits()]);
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        // export: address 0x1000, name "libgraphics_draw" (17 bytes)
        buf.extend_from_slice(&0x1000u64.to_le_bytes());
        let name = b"libgraphics_draw";
        buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
        buf.extend_from_slice(name);
        // import: address 0, name "libc_malloc" (11 bytes), lib len 0 (none)
        buf.extend_from_slice(&0u64.to_le_bytes());
        let iname = b"libc_malloc";
        buf.extend_from_slice(&(iname.len() as u16).to_le_bytes());
        buf.extend_from_slice(iname);
        buf.push(0);
        buf
    }

    #[test]
    fn znl_parses_valid_library() {
        let img = make_valid_znl();
        let f = ZnlFile::parse(&img).expect("valid znl should parse");
        assert_eq!(f.header.abi_version, 5);
        assert_eq!(f.header.arch, ZnsArch::Aex64);
        assert_eq!(f.header.build, ZnlBuild::Release);
        assert_eq!(f.header.flags, ZnlFlags::SHARED);
        assert_eq!(f.exports.len(), 1);
        assert_eq!(f.imports.len(), 1);
        assert_eq!(f.exports[0].name, "libgraphics_draw");
        assert_eq!(f.exports[0].address, 0x1000);
        assert_eq!(f.imports[0].name, "libc_malloc");
        assert_eq!(f.imports[0].address, 0);
        assert_eq!(f.imports[0].library, None);
        assert_eq!(f.export_address("libgraphics_draw"), Some(0x1000));
        assert_eq!(f.export_address("missing"), None);
    }

    #[test]
    fn znl_rejects_bad_magic() {
        let mut img = make_valid_znl();
        img[0] = 0x00;
        match ZnlFile::parse(&img) {
            Err(ZnlError::BadMagic { found }) => assert_ne!(found, ZNL_MAGIC),
            other => panic!("expected BadMagic, got {:?}", other),
        }
    }

    #[test]
    fn znl_rejects_bad_version() {
        let mut img = make_valid_znl();
        img[4] = 9;
        match ZnlFile::parse(&img) {
            Err(ZnlError::BadVersion { found }) => assert_eq!(found, 9),
            other => panic!("expected BadVersion, got {:?}", other),
        }
    }

    #[test]
    fn znl_rejects_truncated_export_name() {
        let mut img = make_valid_znl();
        // cut the buffer right after export 0's address+name-length, before
        // the name bytes → parser must report Truncated.
        img.truncate(26);
        match ZnlFile::parse(&img) {
            Err(ZnlError::Truncated) => {}
            other => panic!("expected Truncated, got {:?}", other),
        }
    }

    #[test]
    fn znl_export_address_lookup() {
        let img = make_valid_znl();
        let f = ZnlFile::parse(&img).unwrap();
        assert_eq!(f.export_address("libgraphics_draw"), Some(0x1000));
        assert_eq!(f.export_address("missing"), None);
    }

    fn make_valid_znd() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&ZND_MAGIC.to_le_bytes());
        buf.extend_from_slice(&ZND_VERSION.to_le_bytes());
        // version, arch(Aex64), driver_type(Storage), build(Release),
        // flags(boot|system), hardware_id_count=2, capability_count=2
        buf.extend_from_slice(&3u16.to_le_bytes());
        buf.extend_from_slice(&64u16.to_le_bytes());
        buf.push(ZndDriverType::Storage as u8);
        buf.push(ZnlBuild::Release as u8);
        buf.push(ZndFlags { boot: true, system: true, hotplug: false, secure: false }.bits());
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());
        // hardware id 0: vendor 0x8086, device 0x1234
        buf.extend_from_slice(&0x8086u16.to_le_bytes());
        buf.extend_from_slice(&0x1234u16.to_le_bytes());
        // hardware id 1: vendor 0x10DE, device 0x2684
        buf.extend_from_slice(&0x10DEu16.to_le_bytes());
        buf.extend_from_slice(&0x2684u16.to_le_bytes());
        // capability 0: "DMA"
        let cap0 = b"DMA";
        buf.extend_from_slice(&(cap0.len() as u16).to_le_bytes());
        buf.extend_from_slice(cap0);
        // capability 1: "MSI-X"
        let cap1 = b"MSI-X";
        buf.extend_from_slice(&(cap1.len() as u16).to_le_bytes());
        buf.extend_from_slice(cap1);
        buf
    }

    #[test]
    fn znd_parses_valid_driver() {
        let img = make_valid_znd();
        let f = ZndFile::parse(&img).expect("valid znd should parse");
        assert_eq!(f.header.version, 3);
        assert_eq!(f.header.arch, ZnsArch::Aex64);
        assert_eq!(f.header.driver_type, ZndDriverType::Storage);
        assert_eq!(f.header.build, ZnlBuild::Release);
        assert!(f.header.flags.boot);
        assert!(f.header.flags.system);
        assert_eq!(f.hardware_ids.len(), 2);
        assert_eq!(f.hardware_ids[0], ZndHardwareId { vendor_id: 0x8086, device_id: 0x1234 });
        assert_eq!(f.hardware_ids[1], ZndHardwareId { vendor_id: 0x10DE, device_id: 0x2684 });
        assert_eq!(f.capabilities.len(), 2);
        assert_eq!(f.capabilities[0], "DMA");
        assert_eq!(f.capabilities[1], "MSI-X");
        assert!(f.matches_hardware(0x8086, 0x1234));
        assert!(!f.matches_hardware(0xFFFF, 0xFFFF));
    }

    #[test]
    fn znd_rejects_bad_magic() {
        let mut img = make_valid_znd();
        img[0] = 0x00;
        match ZndFile::parse(&img) {
            Err(ZndError::BadMagic { found }) => assert_ne!(found, ZND_MAGIC),
            other => panic!("expected BadMagic, got {:?}", other),
        }
    }

    #[test]
    fn znd_rejects_bad_version() {
        let mut img = make_valid_znd();
        img[4] = 9;
        match ZndFile::parse(&img) {
            Err(ZndError::BadVersion { found }) => assert_eq!(found, 9),
            other => panic!("expected BadVersion, got {:?}", other),
        }
    }

    #[test]
    fn znd_rejects_invalid_driver_type() {
        let mut img = make_valid_znd();
        img[10] = 99;
        match ZndFile::parse(&img) {
            Err(ZndError::InvalidDriverType(t)) => assert_eq!(t, 99),
            other => panic!("expected InvalidDriverType, got {:?}", other),
        }
    }

    #[test]
    fn znd_rejects_truncated_hardware_id() {
        let mut img = make_valid_znd();
        // cut after reading one hardware id (17 + 4 = 21 bytes)
        img.truncate(21);
        match ZndFile::parse(&img) {
            Err(ZndError::Truncated) => {}
            other => panic!("expected Truncated, got {:?}", other),
        }
    }

    fn make_valid_znm() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&ZNM_MAGIC.to_le_bytes());
        buf.extend_from_slice(&ZNM_VERSION.to_le_bytes());
        // abi_version, arch(Aex64), minimum_kernel, build(Release),
        // flags(core), dep=1, export=1, import=1, init=1, cleanup=1, perm=1
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&64u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.push(ZnlBuild::Release as u8);
        buf.push(ZnmFlags::CORE.bits());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        // dependency: "memory.znm"
        let dep = b"memory.znm";
        buf.extend_from_slice(&(dep.len() as u16).to_le_bytes());
        buf.extend_from_slice(dep);
        // export: "AllocatePage"
        let exp = b"AllocatePage";
        buf.extend_from_slice(&(exp.len() as u16).to_le_bytes());
        buf.extend_from_slice(exp);
        // import: "RegisterInterrupt"
        let imp = b"RegisterInterrupt";
        buf.extend_from_slice(&(imp.len() as u16).to_le_bytes());
        buf.extend_from_slice(imp);
        // init: "ModuleInit"
        let init = b"ModuleInit";
        buf.extend_from_slice(&(init.len() as u16).to_le_bytes());
        buf.extend_from_slice(init);
        // cleanup: "ModuleExit"
        let cleanup = b"ModuleExit";
        buf.extend_from_slice(&(cleanup.len() as u16).to_le_bytes());
        buf.extend_from_slice(cleanup);
        // permission: "Memory Manager"
        let perm = b"Memory Manager";
        buf.extend_from_slice(&(perm.len() as u16).to_le_bytes());
        buf.extend_from_slice(perm);
        buf
    }

    #[test]
    fn znm_parses_valid_module() {
        let img = make_valid_znm();
        let f = ZnmFile::parse(&img).expect("valid znm should parse");
        assert_eq!(f.header.abi_version, 2);
        assert_eq!(f.header.arch, ZnsArch::Aex64);
        assert_eq!(f.header.minimum_kernel, 1);
        assert_eq!(f.header.build, ZnlBuild::Release);
        assert!(f.header.flags.core);
        assert_eq!(f.dependencies.len(), 1);
        assert_eq!(f.dependencies[0], "memory.znm");
        assert_eq!(f.exports.len(), 1);
        assert_eq!(f.exports[0], "AllocatePage");
        assert_eq!(f.imports.len(), 1);
        assert_eq!(f.imports[0], "RegisterInterrupt");
        assert_eq!(f.init_functions.len(), 1);
        assert_eq!(f.init_functions[0], "ModuleInit");
        assert_eq!(f.cleanup_functions.len(), 1);
        assert_eq!(f.cleanup_functions[0], "ModuleExit");
        assert_eq!(f.permissions.len(), 1);
        assert_eq!(f.permissions[0], "Memory Manager");
        assert!(f.exports_symbol("AllocatePage"));
        assert!(!f.exports_symbol("FreePage"));
        assert!(f.requires("memory.znm"));
        assert!(!f.requires("scheduler.znm"));
    }

    #[test]
    fn znm_rejects_bad_magic() {
        let mut img = make_valid_znm();
        img[0] = 0x00;
        match ZnmFile::parse(&img) {
            Err(ZnmError::BadMagic { found }) => assert_ne!(found, ZNM_MAGIC),
            other => panic!("expected BadMagic, got {:?}", other),
        }
    }

    #[test]
    fn znm_rejects_bad_version() {
        let mut img = make_valid_znm();
        img[4] = 9;
        match ZnmFile::parse(&img) {
            Err(ZnmError::BadVersion { found }) => assert_eq!(found, 9),
            other => panic!("expected BadVersion, got {:?}", other),
        }
    }

    #[test]
    fn znm_rejects_truncated_dependency() {
        let mut img = make_valid_znm();
        // header (26 bytes) + dep length prefix (2 bytes) kept, but the dep
        // name bytes are cut → parser must report Truncated, not TooSmall.
        img.truncate(30);
        match ZnmFile::parse(&img) {
            Err(ZnmError::Truncated) => {}
            other => panic!("expected Truncated, got {:?}", other),
        }
    }

    fn make_valid_znpkg() -> Vec<u8> {
        // Layout: header(42) + entry table + file data + manifest + script.
        let file_data = b"hello from browser.zns"; // 22 bytes
        let manifest = b"name=browser\nversion=1.0"; // 20 bytes
        let script = b"echo installing browser"; // 23 bytes

        let entry_count = 1u16;
        let entry_name = b"browser.zns";
        let entry_name_len = entry_name.len() as u16;
        // entry: name_len(2) + name(9) + offset(8) + size(8) + type(1) = 28
        let entry_size = 2 + entry_name_len as usize + 8 + 8 + 1;
        let data_offset = 42 + entry_size as u64; // 70
        let manifest_offset = data_offset + file_data.len() as u64; // 92
        let script_offset = manifest_offset + manifest.len() as u64; // 112
        let total = script_offset + script.len() as u64; // 135

        let mut buf = Vec::with_capacity(total as usize);
        buf.extend_from_slice(&ZNPKG_MAGIC.to_le_bytes());
        buf.extend_from_slice(&ZNPKG_VERSION.to_le_bytes());
        // header: hdr_version, file_count, manifest_offset, manifest_size,
        // script_offset, script_size
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&entry_count.to_le_bytes());
        buf.extend_from_slice(&manifest_offset.to_le_bytes());
        buf.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
        buf.extend_from_slice(&script_offset.to_le_bytes());
        buf.extend_from_slice(&(script.len() as u64).to_le_bytes());
        // entry table
        buf.extend_from_slice(&entry_name_len.to_le_bytes());
        buf.extend_from_slice(entry_name);
        buf.extend_from_slice(&data_offset.to_le_bytes());
        buf.extend_from_slice(&(file_data.len() as u64).to_le_bytes());
        buf.push(ZnpkgFileType::Executable as u8);
        // file data + manifest + script
        buf.extend_from_slice(file_data);
        buf.extend_from_slice(manifest);
        buf.extend_from_slice(script);
        assert_eq!(buf.len(), total as usize);
        buf
    }

    #[test]
    fn znpkg_parses_valid_package() {
        let img = make_valid_znpkg();
        let f = ZnpkgFile::parse(&img).expect("valid znpkg should parse");
        assert_eq!(f.header.version, 1);
        assert_eq!(f.header.file_count, 1);
        assert_eq!(f.entries.len(), 1);
        assert_eq!(f.entries[0].name, "browser.zns");
        assert_eq!(f.entries[0].file_type, ZnpkgFileType::Executable);
        assert_eq!(f.manifest, b"name=browser\nversion=1.0");
        assert_eq!(f.install_script, b"echo installing browser");
        assert_eq!(
            f.entry_data("browser.zns"),
            Some(&b"hello from browser.zns"[..]),
        );
        assert_eq!(f.entry_data("missing.zns"), None);
    }

    #[test]
    fn znpkg_rejects_bad_magic() {
        let mut img = make_valid_znpkg();
        img[0] = 0x00;
        match ZnpkgFile::parse(&img) {
            Err(ZnpkgError::BadMagic { found }) => assert_ne!(found, ZNPKG_MAGIC),
            other => panic!("expected BadMagic, got {:?}", other),
        }
    }

    #[test]
    fn znpkg_rejects_bad_version() {
        let mut img = make_valid_znpkg();
        img[4] = 9;
        match ZnpkgFile::parse(&img) {
            Err(ZnpkgError::BadVersion { found }) => assert_eq!(found, 9),
            other => panic!("expected BadVersion, got {:?}", other),
        }
    }

    #[test]
    fn znpkg_rejects_invalid_file_type() {
        let mut img = make_valid_znpkg();
        // file type byte: header(42) + name_len(2) + name(11) + offset(8) + size(8)
        let type_offset = 42 + 2 + 11 + 8 + 8;
        img[type_offset] = 99;
        match ZnpkgFile::parse(&img) {
            Err(ZnpkgError::InvalidFileType(t)) => assert_eq!(t, 99),
            other => panic!("expected InvalidFileType, got {:?}", other),
        }
    }

    #[test]
    fn znpkg_rejects_entry_out_of_bounds() {
        let mut img = make_valid_znpkg();
        // size field: header(42) + name_len(2) + name(11) + offset(8)
        let size_offset = 42 + 2 + 11 + 8;
        let size_bytes = u64::to_le_bytes(img.len() as u64 + 1000);
        img[size_offset..size_offset + 8].copy_from_slice(&size_bytes);
        match ZnpkgFile::parse(&img) {
            Err(ZnpkgError::EntryOutOfBounds) => {}
            other => panic!("expected EntryOutOfBounds, got {:?}", other),
        }
    }

    fn make_valid_zni() -> Vec<u8> {
        // Layout: header(71) + partition table + kernel data + metadata +
        // checksum + signature.
        let kernel_data = b"ZENUS KERNEL IMAGE";
        let metadata = b"name=zenus-boot\nversion=1.0";
        let checksum = b"abcdef0123456789";
        let signature = b"SIGNATUREBYTES";

        let partition_count = 1u16;
        let part_name = b"kernel";
        let part_name_len = part_name.len() as u16;
        // partition: name_len(2) + name + type(1) + offset(8) + size(8) = 20
        let part_size = 2 + part_name_len as usize + 1 + 8 + 8;
        let kernel_offset = 71 + part_size as u64;
        let metadata_offset = kernel_offset + kernel_data.len() as u64;
        let checksum_offset = metadata_offset + metadata.len() as u64;
        let signature_offset = checksum_offset + checksum.len() as u64;
        let total = signature_offset + signature.len() as u64;

        let mut buf = Vec::with_capacity(total as usize);
        buf.extend_from_slice(&ZNI_MAGIC.to_le_bytes());
        buf.extend_from_slice(&ZNI_VERSION.to_le_bytes());
        // header: hdr_version, arch(Aex64), image_type(Boot),
        // compression(None), encryption(None), image_size, partition_count,
        // metadata_offset, metadata_size, checksum_offset, checksum_size,
        // signature_offset, signature_size
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&64u16.to_le_bytes());
        buf.push(ZniImageType::Boot as u8);
        buf.push(ZniCompression::None as u8);
        buf.push(ZniEncryption::None as u8);
        buf.extend_from_slice(&(kernel_data.len() as u64).to_le_bytes());
        buf.extend_from_slice(&partition_count.to_le_bytes());
        buf.extend_from_slice(&metadata_offset.to_le_bytes());
        buf.extend_from_slice(&(metadata.len() as u64).to_le_bytes());
        buf.extend_from_slice(&checksum_offset.to_le_bytes());
        buf.extend_from_slice(&(checksum.len() as u64).to_le_bytes());
        buf.extend_from_slice(&signature_offset.to_le_bytes());
        buf.extend_from_slice(&(signature.len() as u64).to_le_bytes());
        // partition table
        buf.extend_from_slice(&part_name_len.to_le_bytes());
        buf.extend_from_slice(part_name);
        buf.push(ZniImageType::Boot as u8);
        buf.extend_from_slice(&kernel_offset.to_le_bytes());
        buf.extend_from_slice(&(kernel_data.len() as u64).to_le_bytes());
        // kernel data + metadata + checksum + signature
        buf.extend_from_slice(kernel_data);
        buf.extend_from_slice(metadata);
        buf.extend_from_slice(checksum);
        buf.extend_from_slice(signature);
        assert_eq!(buf.len(), total as usize);
        buf
    }

    #[test]
    fn zni_parses_valid_image() {
        let img = make_valid_zni();
        let f = ZniFile::parse(&img).expect("valid zni should parse");
        assert_eq!(f.header.version, 1);
        assert_eq!(f.header.arch, ZnsArch::Aex64);
        assert_eq!(f.header.image_type, ZniImageType::Boot);
        assert_eq!(f.header.compression, ZniCompression::None);
        assert_eq!(f.header.encryption, ZniEncryption::None);
        assert_eq!(f.partitions.len(), 1);
        assert_eq!(f.partitions[0].name, "kernel");
        assert_eq!(f.partitions[0].image_type, ZniImageType::Boot);
        assert_eq!(f.metadata, b"name=zenus-boot\nversion=1.0");
        assert_eq!(f.checksum, b"abcdef0123456789");
        assert_eq!(f.signature, b"SIGNATUREBYTES");
        assert_eq!(
            f.partition_data("kernel"),
            Some(&b"ZENUS KERNEL IMAGE"[..])
        );
        assert_eq!(f.partition_data("missing"), None);
    }

    #[test]
    fn zni_rejects_bad_magic() {
        let mut img = make_valid_zni();
        img[0] = 0x00;
        match ZniFile::parse(&img) {
            Err(ZniError::BadMagic { found }) => assert_ne!(found, ZNI_MAGIC),
            other => panic!("expected BadMagic, got {:?}", other),
        }
    }

    #[test]
    fn zni_rejects_bad_version() {
        let mut img = make_valid_zni();
        img[4] = 9;
        match ZniFile::parse(&img) {
            Err(ZniError::BadVersion { found }) => assert_eq!(found, 9),
            other => panic!("expected BadVersion, got {:?}", other),
        }
    }

    #[test]
    fn zni_rejects_invalid_image_type() {
        let mut img = make_valid_zni();
        // image type byte at offset 10
        img[10] = 99;
        match ZniFile::parse(&img) {
            Err(ZniError::InvalidImageType(t)) => assert_eq!(t, 99),
            other => panic!("expected InvalidImageType, got {:?}", other),
        }
    }

    #[test]
    fn zni_rejects_partition_out_of_bounds() {
        let mut img = make_valid_zni();
        // partition size field: header(71) + name_len(2) + name(6) + type(1) + offset(8)
        let size_offset = 71 + 2 + 6 + 1 + 8;
        let size_bytes = u64::to_le_bytes(img.len() as u64 + 1000);
        img[size_offset..size_offset + 8].copy_from_slice(&size_bytes);
        match ZniFile::parse(&img) {
            Err(ZniError::PartitionOutOfBounds) => {}
            other => panic!("expected PartitionOutOfBounds, got {:?}", other),
        }
    }
}

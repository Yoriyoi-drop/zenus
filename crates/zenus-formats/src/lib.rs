#![no_std]

// The host test harness needs std; the bare-metal build uses alloc for Vec.
extern crate alloc;

#[cfg(test)]
extern crate std;

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
}

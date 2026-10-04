use crate::vfs::{self, FileType};
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

pub const PKG_INSTALL_DIR: &str = "/usr/local";
pub const PKG_DB_DIR: &str = "/var/db/zpk";

#[repr(C)]
pub struct ZpkHeader {
    pub magic: [u8; 4],
    pub name: [u8; 64],
    pub version: [u8; 16],
    pub file_count: u32,
    pub total_size: u32,
    _reserved: [u8; 408],
}

#[repr(C)]
pub struct ZpkFileEntry {
    pub path: [u8; 128],
    pub size: u32,
    pub mode: u16,
    pub file_type: u8,
    _reserved: [u8; 365],
}

#[derive(Debug, Clone)]
pub struct PkgInfo {
    pub name: String,
    pub version: String,
    pub file_count: u32,
    pub total_size: u32,
    pub files: Vec<String>,
}

fn ensure_dir(path: &str) -> bool {
    if vfs::open(path).is_some() {
        return true;
    }
    let trimmed = path.trim_end_matches('/');
    let mut prev_end = 0usize;
    loop {
        let next_slash = trimmed[prev_end..].find('/');
        let segment_end = match next_slash {
            Some(pos) => prev_end + pos,
            None => break,
        };
        let sub = &trimmed[..=segment_end];
        if !sub.is_empty() && sub != "/" && vfs::open(sub).is_none() {
            if !vfs::create_dir(sub) {
                return false;
            }
        }
        prev_end = segment_end + 1;
    }
    if vfs::open(trimmed).is_none() {
        vfs::create_dir(trimmed)
    } else {
        true
    }
}

fn write_file(path: &str, data: &[u8]) -> bool {
    if vfs::open(path).is_some() {
        vfs::remove(path);
    }
    if !vfs::create_file(path) {
        return false;
    }
    let node = match vfs::open(path) {
        Some(n) => n,
        None => return false,
    };
    node.fs.write(node.inode, 0, data).is_some()
}

fn read_file(path: &str) -> Option<Vec<u8>> {
    let node = vfs::open(path)?;
    let stat = node.fs.stat(node.inode);
    let size = stat.size as usize;
    if size == 0 {
        return Some(Vec::new());
    }
    let mut buf = alloc::vec![0u8; size];
    let n = node.fs.read(node.inode, 0, &mut buf)?;
    buf.truncate(n as usize);
    Some(buf)
}

fn str_from_bytes(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..end]).unwrap_or("")
}

/// Resolve a `.zpk` entry path to its absolute install path.
///
/// Returns `None` for anything that could escape [`PKG_INSTALL_DIR`] or that is
/// not a plain relative/absolute path. The package format is attacker-supplied
/// data: `path = "../../../etc/x"` used to be prefixed with `/usr/local` and
/// then resolved *through* `..` by `vfs::open_in_ns`, so `pkg_install` could
/// write anywhere the filesystem could reach, and `pkg_remove` deleted it
/// again on uninstall.
fn install_path_for(path_str: &str) -> Option<String> {
    if path_str.is_empty() || path_str.contains('\0') {
        return None;
    }
    // Reject any `..` component, wherever it appears.
    for segment in path_str.split('/') {
        if segment == ".." {
            return None;
        }
    }
    if path_str.starts_with('/') {
        Some(alloc::format!("{}{}", PKG_INSTALL_DIR, path_str))
    } else {
        Some(alloc::format!("{}/{}", PKG_INSTALL_DIR, path_str))
    }
}

fn manifest_path(name: &str) -> String {
    alloc::format!("{}/{}/manifest", PKG_DB_DIR, name)
}

fn pkg_dir_path(name: &str) -> String {
    alloc::format!("{}/{}", PKG_DB_DIR, name)
}

fn read_manifest(name: &str) -> Option<PkgInfo> {
    let data = read_file(&manifest_path(name))?;
    let text = core::str::from_utf8(&data).ok()?;
    let mut lines = text.lines();

    let name_s = String::from(lines.next()?.trim());
    let version = String::from(lines.next()?.trim());
    let file_count: u32 = lines.next()?.trim().parse().ok()?;
    let total_size: u32 = lines.next()?.trim().parse().ok()?;
    let mut files = Vec::new();
    for line in lines {
        let f = line.trim();
        if !f.is_empty() {
            files.push(String::from(f));
        }
    }

    Some(PkgInfo {
        name: name_s,
        version,
        file_count,
        total_size,
        files,
    })
}

pub fn pkg_init() -> bool {
    let ok = ensure_dir(PKG_DB_DIR);
    if ok {
        zenus_console::kinfo!("Package manager initialized");
    } else {
        zenus_console::kerror_code!(
            zenus_console::error::codes::FS_MOUNT_FAILED,
            "Package manager: failed to create DB dir"
        );
    }
    ok
}

/// One validated file entry of a `.zpk` image.
struct PlannedEntry {
    /// Absolute path under [`PKG_INSTALL_DIR`].
    path: String,
    /// Byte offset of the payload inside the image.
    offset: usize,
    len: usize,
    is_dir: bool,
}

/// Copy a fixed-size array field out of a byte slice without assuming the
/// slice is aligned for it.
///
/// The `.zpk` container is byte-packed: `data.as_ptr()` is a `*const u8`
/// (alignment 1) and every record offset inside it inherits that. Casting those
/// addresses to `*const ZpkHeader` / `*const ZpkFileEntry` — which have a 4-byte
/// alignment because of their `u32` fields — was undefined behaviour. A package
/// whose first payload had a length that was not a multiple of 4 put the next
/// record at a misaligned address; a two-file image with a 5-byte payload
/// reproduced it immediately.
fn read_fixed_array<const N: usize>(data: &[u8], at: usize) -> Option<[u8; N]> {
    let bytes = data.get(at..at.checked_add(N)?)?;
    let mut out = [0u8; N];
    out.copy_from_slice(bytes);
    Some(out)
}

fn read_u16_at(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32_at(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Field offsets inside the on-disk records.
///
/// Duplicated from the `#[repr(C)]` structs on purpose: the reader must not
/// depend on the struct's alignment, and `zpk_field_offsets_are_pinned` asserts
/// that these numbers still match `offset_of!`.
pub mod layout {
    pub const HEADER_SIZE: usize = 500;
    pub const ENTRY_SIZE: usize = 500;
    pub const MAGIC: usize = 0;
    pub const NAME: usize = 4;
    pub const VERSION: usize = 68;
    pub const FILE_COUNT: usize = 84;
    pub const TOTAL_SIZE: usize = 88;

    pub const ENTRY_PATH: usize = 0;
    pub const ENTRY_LEN: usize = 128;
    pub const ENTRY_MODE: usize = 132;
    pub const ENTRY_FILE_TYPE: usize = 134;
}

/// Validate the whole image before touching the filesystem.
///
/// The old code created `/var/db/zpk/<name>` and then validated entries one at
/// a time, so a rejected image (bad magic aside) still left an empty package
/// directory behind: `pkg_installed_count` counted it, `pkg_list` skipped it
/// because it had no manifest, and the two disagreed. Validating up front also
/// means a traversal attempt cannot create anything at all.
fn plan_install(data: &[u8]) -> Option<(String, String, u32, u32, Vec<PlannedEntry>)> {
    use layout as L;

    if data.len() < L::HEADER_SIZE {
        return None;
    }
    if read_fixed_array::<4>(data, L::MAGIC)? != *b"ZPK1" {
        return None;
    }

    let pkg_name = str_from_bytes(&read_fixed_array::<64>(data, L::NAME)?).to_string();
    let pkg_version = str_from_bytes(&read_fixed_array::<16>(data, L::VERSION)?).to_string();
    if pkg_name.is_empty() || pkg_name.contains('/') || pkg_name.contains("..") {
        return None;
    }

    let file_count = read_u32_at(data, L::FILE_COUNT)?;
    let total_size = read_u32_at(data, L::TOTAL_SIZE)?;

    let mut offset = L::HEADER_SIZE;
    let mut planned = Vec::new();

    for _ in 0..file_count {
        if offset.checked_add(L::ENTRY_SIZE)? > data.len() {
            return None;
        }

        let raw_path = read_fixed_array::<128>(data, offset + L::ENTRY_PATH)?;
        let path = install_path_for(str_from_bytes(&raw_path))?;
        let len = read_u32_at(data, offset + L::ENTRY_LEN)? as usize;
        let _mode = read_u16_at(data, offset + L::ENTRY_MODE)?;
        let file_type = *data.get(offset + L::ENTRY_FILE_TYPE)?;
        let body_at = offset + L::ENTRY_SIZE;
        let data_end = body_at.checked_add(len)?;
        if data_end > data.len() {
            return None;
        }

        planned.push(PlannedEntry {
            path,
            offset: body_at,
            len,
            is_dir: file_type == 1,
        });
        offset = data_end;
    }

    Some((pkg_name, pkg_version, file_count, total_size, planned))
}

pub fn pkg_install(data: &[u8], _dev_id: usize) -> bool {
    let (pkg_name, pkg_version, file_count, total_size, planned) = match plan_install(data) {
        Some(p) => p,
        None => return false,
    };

    let pkg_dir = pkg_dir_path(&pkg_name);
    if !ensure_dir(&pkg_dir) {
        return false;
    }

    let mut installed_files: Vec<String> = Vec::new();

    for entry in &planned {
        let parent = crate::vfs::parent_dir(&entry.path).unwrap_or(PKG_INSTALL_DIR);
        if !ensure_dir(parent) {
            return false;
        }
        if entry.is_dir {
            if !vfs::create_dir(&entry.path) {
                return false;
            }
        } else {
            let file_data = &data[entry.offset..entry.offset + entry.len];
            if !write_file(&entry.path, file_data) {
                return false;
            }
        }
        installed_files.push(entry.path.clone());
    }

    let mut manifest = alloc::format!(
        "{}\n{}\n{}\n{}\n",
        pkg_name, pkg_version, file_count, total_size
    );
    for f in &installed_files {
        manifest.push_str(f);
        manifest.push('\n');
    }

    if !write_file(&manifest_path(&pkg_name), manifest.as_bytes()) {
        return false;
    }

    true
}

pub fn pkg_remove(name: &str) -> bool {
    let info = match read_manifest(name) {
        Some(i) => i,
        None => return false,
    };

    for f in &info.files {
        vfs::remove(f);
    }

    let pkg_dir = pkg_dir_path(name);
    let manifest = manifest_path(name);
    vfs::remove(&manifest);
    vfs::remove(&pkg_dir);

    true
}

pub fn pkg_list() -> Vec<PkgInfo> {
    let mut result = Vec::new();
    let entries = vfs::read_dir(PKG_DB_DIR);
    for e in entries {
        if e.file_type == FileType::Directory {
            if let Some(info) = read_manifest(&e.name) {
                result.push(info);
            }
        }
    }
    result
}

pub fn pkg_info(name: &str) -> Option<PkgInfo> {
    read_manifest(name)
}

pub fn pkg_installed_count() -> usize {
    let entries = vfs::read_dir(PKG_DB_DIR);
    entries
        .iter()
        .filter(|e| e.file_type == FileType::Directory)
        .count()
}

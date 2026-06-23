use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use crate::vfs::{self, FileType};

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
    ensure_dir(PKG_DB_DIR)
}

pub fn pkg_install(data: &[u8], _dev_id: usize) -> bool {
    if data.len() < core::mem::size_of::<ZpkHeader>() {
        return false;
    }

    let header = unsafe { &*(data.as_ptr() as *const ZpkHeader) };
    if &header.magic != b"ZPK1" {
        return false;
    }

    let pkg_name = str_from_bytes(&header.name).to_string();
    let pkg_version = str_from_bytes(&header.version).to_string();
    let file_count = header.file_count;
    let _total_size = header.total_size;

    let pkg_dir = pkg_dir_path(&pkg_name);
    if !ensure_dir(&pkg_dir) {
        return false;
    }

    let mut offset = core::mem::size_of::<ZpkHeader>();
    let mut installed_files: Vec<String> = Vec::new();

    for _i in 0..file_count {
        if offset + core::mem::size_of::<ZpkFileEntry>() > data.len() {
            return false;
        }

        let entry = unsafe { &*(data.as_ptr().add(offset) as *const ZpkFileEntry) };
        offset += core::mem::size_of::<ZpkFileEntry>();

        let path_str = str_from_bytes(&entry.path);
        let install_path = if path_str.starts_with('/') {
            alloc::format!("{}{}", PKG_INSTALL_DIR, path_str)
        } else {
            alloc::format!("{}/{}", PKG_INSTALL_DIR, path_str)
        };

        let data_size = entry.size as usize;
        if offset + data_size > data.len() {
            return false;
        }

        let file_data = &data[offset..offset + data_size];
        offset += data_size;

        if entry.file_type == 1 {
            let parent = crate::vfs::parent_dir(&install_path).unwrap_or(PKG_INSTALL_DIR);
            if !ensure_dir(parent) {
                return false;
            }
            if !vfs::create_dir(&install_path) {
                return false;
            }
        } else {
            let parent = crate::vfs::parent_dir(&install_path).unwrap_or(PKG_INSTALL_DIR);
            if !ensure_dir(parent) {
                return false;
            }
            if !write_file(&install_path, file_data) {
                return false;
            }
        }

        installed_files.push(install_path);
    }

    let mut manifest = alloc::format!(
        "{}\n{}\n{}\n{}\n",
        pkg_name, pkg_version, file_count, _total_size
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
    entries.iter().filter(|e| e.file_type == FileType::Directory).count()
}

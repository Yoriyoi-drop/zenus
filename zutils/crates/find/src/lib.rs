#![no_std]

extern crate alloc;

use zutils_common::{Args, Writer};
use zenus_fs::vfs::{self, FileSystem as _, FileType};

fn find_recursive<W: Writer + ?Sized>(path: &str, name: &str, w: &mut W) {
    if let Some(name_part) = path.rsplit('/').next() {
        if path == name || name_part.contains(name) {
            w.write_str(path);
            w.write_str("\r\n");
        }
    }
    if let Some(node) = vfs::open(path) {
        let stat = node.fs.stat(node.inode);
        if stat.file_type == FileType::Directory {
            let entries = node.fs.read_dir(node.inode);
            for entry in entries {
                let mut full = alloc::string::String::new();
                if path == "/" {
                    full.push_str("/");
                    full.push_str(&entry.name);
                } else {
                    full.push_str(path);
                    full.push_str("/");
                    full.push_str(&entry.name);
                }
                find_recursive(&full, name, w);
            }
        }
    }
}

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let name = match args.get(1) {
        Some(p) => p,
        None => {
            w.write_str("find: missing pattern\r\n");
            return;
        }
    };
    find_recursive("/", name, w);
}

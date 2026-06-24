#![no_std]

extern crate alloc;

use zutils_common::{Args, Writer};
use zenus_fs::vfs::{self, FileSystem as _, FileType};

fn du_recursive<W: Writer + ?Sized>(path: &str, w: &mut W) -> u64 {
    let mut total = 0u64;
    if let Some(node) = vfs::open(path) {
        let stat = node.fs.stat(node.inode);
        let size = stat.size;
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
                total += du_recursive(&full, w);
            }
            w.write_u64(total);
            w.write_byte(b'\t');
            w.write_str(path);
            w.write_str("\r\n");
        } else {
            total += size;
        }
    }
    total
}

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let path = args.args().iter().find(|a| !a.is_empty()).unwrap_or(&"/");
    du_recursive(path, w);
}

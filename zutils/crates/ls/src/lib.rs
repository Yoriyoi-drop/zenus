#![no_std]

extern crate alloc;

use zutils_common::{Args, Writer};
use zenus_fs::vfs::{self, FileSystem as _, FileType};

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let long = args.has_flag("-l");
    let path = args.args().iter().find(|a| !a.is_empty() && **a != "-l").unwrap_or(&"/");
    let path = if path.is_empty() { "/" } else { path };

    match vfs::open(path) {
        Some(node) => {
            let entries = node.fs.read_dir(node.inode);
            let mut count = 0u64;
            for entry in entries {
                count += 1;
                if long {
                    let stat = node.fs.stat(entry.inode);
                    let perm_buf = vfs::perm_str(stat.mode);
                    let perm = core::str::from_utf8(&perm_buf).unwrap_or("?????????");
                    w.write_str(perm);
                    w.write_byte(b' ');
                    w.write_u64(stat.uid as u64);
                    w.write_byte(b':');
                    w.write_u64(stat.gid as u64);
                    w.write_byte(b' ');
                    w.write_u64(stat.size);
                    w.write_byte(b' ');
                }
                w.write_str(entry.name.as_str());
                if entry.file_type == FileType::Directory {
                    w.write_byte(b'/');
                }
                w.write_str("  ");
            }
            if count == 0 {
                w.write_str("(empty)\r\n");
            } else {
                w.write_byte(b'\n');
            }
        }
        None => {
            w.write_str("ls: ");
            w.write_str(path);
            w.write_str(": not found\r\n");
        }
    }
}

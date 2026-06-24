#![no_std]

use zutils_common::{Args, Writer};
use zenus_fs::vfs::{self, FileSystem as _, FileType};

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let path = match args.args().iter().find(|a| !a.is_empty()) {
        Some(p) => p,
        None => {
            w.write_str("cat: missing operand\r\n");
            return;
        }
    };

    match vfs::open(path) {
        Some(node) => {
            let stat = node.fs.stat(node.inode);
            if stat.file_type == FileType::Directory {
                w.write_str("cat: ");
                w.write_str(path);
                w.write_str(": Is a directory\r\n");
                return;
            }
            let mut buf = [0u8; 512];
            let mut offset: u64 = 0;
            let mut last_byte: u8 = 0;
            loop {
                match node.fs.read(node.inode, offset, &mut buf) {
                    Some(0) | None => break,
                    Some(n) => {
                        for i in 0..n as usize {
                            let b = buf[i];
                            w.write_byte(b);
                            last_byte = b;
                        }
                        offset += n;
                    }
                }
            }
            if offset > 0 && last_byte != b'\n' {
                w.write_byte(b'\n');
            }
        }
        None => {
            w.write_str("cat: ");
            w.write_str(path);
            w.write_str(": not found\r\n");
        }
    }
}

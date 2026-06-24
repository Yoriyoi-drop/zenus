#![no_std]

extern crate alloc;

use zutils_common::{Args, Writer};
use zenus_fs::vfs::{self, FileSystem as _, FileType};

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    if args.args().len() < 2 {
        w.write_str("mv: missing operand\r\n");
        return;
    }
    let src = args.get(1).unwrap_or("");
    let dst = args.get(2).unwrap_or("");
    let src_node = match vfs::open(src) {
        Some(n) => n,
        None => {
            w.write_str("mv: ");
            w.write_str(src);
            w.write_str(": not found\r\n");
            return;
        }
    };
    let stat = src_node.fs.stat(src_node.inode);
    if stat.file_type == FileType::Directory {
        w.write_str("mv: cannot move a directory\r\n");
        return;
    }
    let mut buf = alloc::vec![0u8; stat.size as usize];
    if src_node.fs.read(src_node.inode, 0, &mut buf).is_none() {
        w.write_str("mv: read failed\r\n");
        return;
    }
    if !vfs::create_file(dst) {
        w.write_str("mv: cannot create ");
        w.write_str(dst);
        w.write_str("\r\n");
        return;
    }
    let dst_node = match vfs::open(dst) {
        Some(n) => n,
        None => {
            w.write_str("mv: cannot open destination\r\n");
            return;
        }
    };
    dst_node.fs.write(dst_node.inode, 0, &buf);
    if vfs::remove(src) {
        w.write_str("mv: ok\r\n");
    } else {
        w.write_str("mv: could not remove source\r\n");
    }
}

#![no_std]

use zutils_common::{Args, Writer};
use zenus_fs::vfs;

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    if args.args().len() < 2 {
        w.write_str("Usage: chmod <mode> <file>\r\n");
        return;
    }
    let mode_str = args.get(1).unwrap_or("");
    let path = args.get(2).unwrap_or("");
    let mode = match usize::from_str_radix(mode_str, 8) {
        Ok(m) => m as u16,
        Err(_) => {
            w.write_str("chmod: invalid mode\r\n");
            return;
        }
    };
    match vfs::open(path) {
        Some(node) => {
            if node.fs.chmod(node.inode, mode) {
                w.write_str("chmod: ok\r\n");
            } else {
                w.write_str("chmod: failed\r\n");
            }
        }
        None => {
            w.write_str("chmod: ");
            w.write_str(path);
            w.write_str(": not found\r\n");
        }
    }
}

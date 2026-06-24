#![no_std]

use zutils_common::{Args, Writer};
use zenus_fs::vfs;

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    if args.args().len() < 2 {
        w.write_str("Usage: chown <uid> <file>\r\n");
        return;
    }
    let uid_str = args.get(1).unwrap_or("");
    let path = args.get(2).unwrap_or("");
    let uid: u32 = match uid_str.parse() {
        Ok(u) => u,
        Err(_) => {
            w.write_str("chown: invalid uid\r\n");
            return;
        }
    };
    match vfs::open(path) {
        Some(node) => {
            if node.fs.chown(node.inode, uid, 0) {
                w.write_str("chown: ok\r\n");
            } else {
                w.write_str("chown: failed\r\n");
            }
        }
        None => {
            w.write_str("chown: ");
            w.write_str(path);
            w.write_str(": not found\r\n");
        }
    }
}

#![no_std]

use zutils_common::{Args, Writer};
use zenus_fs::vfs;

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let path = match args.args().iter().find(|a| !a.is_empty()) {
        Some(p) => p,
        None => {
            w.write_str("rm: missing operand\r\n");
            return;
        }
    };
    if vfs::remove(path) {
        w.write_str("ok\r\n");
    } else {
        w.write_str("rm: failed to remove\r\n");
    }
}

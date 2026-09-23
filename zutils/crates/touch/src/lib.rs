#![no_std]

use zenus_fs::vfs;
use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let path = match args.args().iter().find(|a| !a.is_empty()) {
        Some(p) => p,
        None => {
            w.write_str("touch: missing operand\r\n");
            return;
        }
    };
    if vfs::create_file(path) {
        w.write_str("ok\r\n");
    } else {
        w.write_str("touch: failed to create file\r\n");
    }
}

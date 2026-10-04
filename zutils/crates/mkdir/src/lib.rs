#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


use zenus_fs::vfs;
use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let path = match args.args().iter().find(|a| !a.is_empty()) {
        Some(p) => p,
        None => {
            w.write_str("mkdir: missing operand\r\n");
            return;
        }
    };
    if vfs::create_dir(path) {
        w.write_str("ok\r\n");
    } else {
        w.write_str("mkdir: failed to create directory\r\n");
    }
}

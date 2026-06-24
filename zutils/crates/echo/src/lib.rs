#![no_std]

use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    for (i, arg) in args.args().iter().enumerate() {
        if arg.is_empty() { continue; }
        if i > 0 { w.write_byte(b' '); }
        w.write_str(arg);
    }
    w.write_str("\r\n");
}

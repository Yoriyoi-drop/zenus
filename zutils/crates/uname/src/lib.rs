#![no_std]

use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("Zenus OS v0.1.0 x86_64\r\n");
}

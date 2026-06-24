#![no_std]

use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("\x1B[2J\x1B[H");
}

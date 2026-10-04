#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("Zenus OS v0.1.0 x86_64\r\n");
}

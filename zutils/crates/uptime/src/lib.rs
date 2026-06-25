#![no_std]

use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let ticks = zenus_arch::interrupts::pit::get_ticks();
    let seconds = ticks / 100;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    w.write_u64(days);
    w.write_str(" days, ");
    w.write_u64(hours % 24);
    w.write_str(":");
    if minutes % 60 < 10 { w.write_byte(b'0'); }
    w.write_u64(minutes % 60);
    w.write_str(":");
    if seconds % 60 < 10 { w.write_byte(b'0'); }
    w.write_u64(seconds % 60);
    w.write_str("\r\n");
}

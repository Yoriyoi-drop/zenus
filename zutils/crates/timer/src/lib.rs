#![no_std]

use zutils_common::{Args, Writer};
use zenus_arch::interrupts::handler;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let ticks = handler::get_timer_tick();
    w.write_str("Timer ticks: ");
    w.write_u64(ticks);
    w.write_str("\r\n");
}

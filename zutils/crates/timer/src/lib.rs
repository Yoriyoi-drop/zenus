#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


use zenus_arch::interrupts::handler;
use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let ticks = handler::get_timer_tick();
    w.write_str("Timer ticks: ");
    w.write_u64(ticks);
    w.write_str("\r\n");
}

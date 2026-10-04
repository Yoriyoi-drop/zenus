#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


use zenus_arch::acpi;
use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("Shutting down...\r\n");
    acpi::shutdown_via_acpi();
}

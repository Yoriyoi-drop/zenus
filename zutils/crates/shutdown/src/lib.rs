#![no_std]

use zutils_common::{Args, Writer};
use zenus_arch::acpi;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("Shutting down...\r\n");
    acpi::shutdown_via_acpi();
}

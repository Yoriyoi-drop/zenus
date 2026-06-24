#![no_std]

use zutils_common::{Args, Writer};
use zenus_arch::acpi;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("Rebooting...\r\n");
    acpi::reboot_via_keyboard();
}

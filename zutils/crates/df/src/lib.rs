#![no_std]

use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("Filesystem\tType\t\tSize\tUsed\tFree\r\n");
    w.write_str("rootfs\t\ttmpfs\t\t128M\t-\t-\r\n");
    w.write_str("/dev\t\tdevfs\t\t-\t-\t-\r\n");
    w.write_str("/initrd\t\ttarfs\t\t10K\t-\t-\r\n");
}

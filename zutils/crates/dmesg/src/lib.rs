#![no_std]

use zenus_console::log;
use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let snap = log::dmesg_snapshot();
    for i in 0..snap.count {
        let entry = &snap.entries[i];
        let len = core::cmp::min(entry.len as usize, entry.msg.len());
        let msg = core::str::from_utf8(&entry.msg[..len]).unwrap_or("");
        w.write_str(entry.level.prefix());
        w.write_str(" ");
        w.write_str(msg);
        w.write_str("\r\n");
    }
    if snap.count == 0 {
        w.write_str("(no messages)\r\n");
    }
}

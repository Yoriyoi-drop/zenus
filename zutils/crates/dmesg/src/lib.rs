#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


extern crate alloc;

use alloc::vec::Vec;
use zenus_console::log;
use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    // Buffer on the HEAP, never on the task stack.
    //
    // The old code did `let snap = log::dmesg_snapshot();` — a 33 KB struct
    // returned by value, which a debug build materialises more than once.
    // The shell task only has ~32 KB of usable kernel stack, so running
    // `dmesg` underflowed the stack by ~69 KB: the zero-initialisation of
    // that struct wiped the gap below the shell's stack AND the idle task's
    // saved interrupt frame, and the next `yield_now()` tripped the frame
    // check (`BADFRAME src=0`) and parked the CPU.
    //
    // Reserve capacity BEFORE taking the ring-buffer lock so no allocation
    // happens while it is held.
    let n = log::dmesg_count();
    let mut data: Vec<u8> = Vec::with_capacity(n * 140 + 16);
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(n + 1);

    log::dmesg_for_each(|level, msg| {
        let start = data.len();
        data.extend_from_slice(level.prefix().as_bytes());
        data.push(b' ');
        data.extend_from_slice(msg.as_bytes());
        data.extend_from_slice(b"\r\n");
        spans.push((start, data.len()));
    });

    if spans.is_empty() {
        w.write_str("(no messages)\r\n");
        return;
    }
    // Write one line at a time: the serial output buffer is 4 KB, a single
    // 33 KB write would have been silently truncated by it.
    for &(start, end) in &spans {
        w.write_str(core::str::from_utf8(&data[start..end]).unwrap_or(""));
    }
}

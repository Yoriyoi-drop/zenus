#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


use zenus_sched::scheduler;
use zutils_common::{Args, Writer};

const STATE_W: usize = 10;
const NAME_W: usize = 20;
const ID_W: usize = 6;

/// Emit `n` spaces up to `width` columns.
///
/// The old code padded with hardcoded literals (`16 - state.len()`, `24 -
/// name_len`) that did not match the header it printed, so every column after
/// STATE was misaligned against the header. `for _ in name_len..24` also
/// emitted *nothing* when the name was longer than 24 bytes, pushing UID/GID
/// out of their columns entirely.
fn pad<W: Writer + ?Sized>(w: &mut W, n: usize, width: usize) {
    for _ in n..width {
        w.write_byte(b' ');
    }
}

/// Write `text` left-aligned in `width` columns.
fn field<W: Writer + ?Sized>(w: &mut W, text: &str, width: usize) {
    w.write_str(text);
    pad(w, text.len(), width);
}

/// Write `v` right-aligned in `width` columns.
fn num_field<W: Writer + ?Sized>(w: &mut W, v: u64, width: usize) {
    let mut buf = [0u8; 20];
    let mut i = 20;
    let mut n = v;
    if n == 0 {
        i -= 1;
        buf[i] = b'0';
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let s = core::str::from_utf8(&buf[i..]).unwrap_or("0");
    pad(w, s.len(), width);
    w.write_str(s);
}

fn dashes(n: usize) -> usize {
    n
}

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    field(w, "PID", ID_W);
    w.write_byte(b'\t');
    field(w, "STATE", STATE_W);
    w.write_byte(b'\t');
    field(w, "NAME", NAME_W);
    w.write_byte(b'\t');
    field(w, "UID", ID_W);
    w.write_byte(b'\t');
    w.write_str("GID");
    w.write_str("\r\n");

    field(w, &"-".repeat(3), ID_W);
    w.write_byte(b'\t');
    field(w, &"-".repeat(dashes(5)), STATE_W);
    w.write_byte(b'\t');
    field(w, &"-".repeat(4), NAME_W);
    w.write_byte(b'\t');
    field(w, &"-".repeat(3), ID_W);
    w.write_byte(b'\t');
    w.write_str("---");
    w.write_str("\r\n");

    let tasks = scheduler::list_tasks();
    let current = scheduler::current_task_id();
    for info in tasks.iter().flatten() {
        num_field(w, info.id, ID_W);
        w.write_byte(b'\t');
        field(w, info.state.to_str(), STATE_W);
        w.write_byte(b'\t');
        let name_len = info.name.iter().position(|&b| b == 0).unwrap_or(info.name.len());
        let name = core::str::from_utf8(&info.name[..name_len]).unwrap_or("?");
        let name = if name.is_empty() { "-" } else { name };
        field(w, name, NAME_W);
        w.write_byte(b'\t');
        num_field(w, info.uid as u64, ID_W);
        w.write_byte(b'\t');
        w.write_u64(info.gid as u64);
        if info.id == current {
            w.write_str(" *");
        }
        w.write_str("\r\n");
    }
}

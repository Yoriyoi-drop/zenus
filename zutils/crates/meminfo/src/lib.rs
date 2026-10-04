#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


use zenus_mem::allocator::ALLOCATOR;
use zenus_mem::frame_allocator;
use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let free_head = ALLOCATOR.free_head_addr();
    let total = ALLOCATOR.total_size();
    let free = ALLOCATOR.free_size();
    // Report the real numbers: this used to hardcode "4MB" while the
    // kernel boots an 8MB heap (and never showed how much is free).
    w.write_str("Heap: ");
    w.write_u64((total / (1024 * 1024)) as u64);
    w.write_str("MB free-list allocator (free: ");
    w.write_u64((free / 1024) as u64);
    w.write_str(" KB)\r\n");
    // No manual "0x": `write_hex` already prefixes one, so the literal
    // duplicated it (`0x0xFFFFFFFF...`).
    w.write_str("  Free list head: ");
    w.write_hex(free_head as u64);
    w.write_str("\r\n");

    let fa = frame_allocator::FRAME_ALLOCATOR.lock();
    w.write_str("Physical frames:\r\n");
    w.write_str("  Total: ");
    w.write_u64(fa.total_memory() / 4096);
    w.write_str(" frames (");
    w.write_u64(fa.total_memory() / (1024 * 1024));
    w.write_str(" MB)\r\n");
    w.write_str("  Used:  ");
    w.write_u64(fa.used_memory() / 4096);
    w.write_str(" frames (");
    w.write_u64(fa.used_memory() / (1024 * 1024));
    w.write_str(" MB)\r\n");
    w.write_str("  Free stack: ");
    w.write_u64(fa.free_frames_count() as u64);
    w.write_str(" frames\r\n");
}

#![no_std]

use zutils_common::{Args, Writer};
use zenus_fs::vfs;
use zenus_sched::scheduler;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let mnt_ns = scheduler::current_mnt_ns();
    w.write_str("Mount points (ns ");
    w.write_u64(mnt_ns as u64);
    w.write_str("):\r\n");
    w.write_str("  /       tmpfs (root)\r\n");
    if vfs::open_in_ns(mnt_ns, "/dev").is_some() {
        w.write_str("  /dev    devfs\r\n");
    }
    if vfs::open_in_ns(mnt_ns, "/mnt").is_some() {
        w.write_str("  /mnt    ext2 (if mounted)\r\n");
    }
    if vfs::open_in_ns(mnt_ns, "/initrd").is_some() {
        w.write_str("  /initrd initrd (tarfs)\r\n");
    }
    let (hits, misses) = zenus_fs::block_cache::bc_stats();
    w.write_str("Block cache: ");
    w.write_u64(hits);
    w.write_str(" hits, ");
    w.write_u64(misses);
    w.write_str(" misses\r\n");
}

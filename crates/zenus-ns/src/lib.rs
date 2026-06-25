#![no_std]

extern crate alloc;

pub mod uts;
pub mod pid;
pub mod mnt;
pub mod net;
pub mod user;
pub mod ipc;

use core::sync::atomic::{AtomicU32, Ordering};

static NEXT_NS_ID: AtomicU32 = AtomicU32::new(1);

pub type NsId = u32;

pub const NS_ROOT: NsId = 0;

pub const CLONE_NEWUTS: u64 = 0x04000000;
pub const CLONE_NEWPID: u64 = 0x20000000;
pub const CLONE_NEWNS: u64 = 0x00020000;
pub const CLONE_NEWNET: u64 = 0x40000000;
pub const CLONE_NEWUSER: u64 = 0x10000000;
pub const CLONE_NEWIPC: u64 = 0x08000000;

pub fn alloc_ns_id() -> NsId {
    NEXT_NS_ID.fetch_add(1, Ordering::SeqCst)
}

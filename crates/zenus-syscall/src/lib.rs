#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


extern crate alloc;

pub mod elf;
pub mod syscall;

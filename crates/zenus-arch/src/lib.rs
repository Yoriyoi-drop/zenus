#![no_std]
#![feature(abi_x86_interrupt)]
#![allow(static_mut_refs)]
#![allow(bad_asm_style)]
extern crate alloc;

pub mod acpi;
pub mod ata;
pub mod cpu;
pub mod crash;
pub mod gdt;
pub mod interrupts;
pub mod keyboard;
pub mod limine;
pub mod pci;
pub mod random;
pub mod rtc;
pub mod smp;
pub mod user;
pub mod watchdog;

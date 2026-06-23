#![no_std]
#![feature(abi_x86_interrupt)]
#![allow(static_mut_refs)]
#![allow(bad_asm_style)]
extern crate alloc;

pub mod limine;
pub mod cpu;
pub mod gdt;
pub mod interrupts;
pub mod smp;
pub mod pci;
pub mod acpi;
pub mod keyboard;
pub mod ata;
pub mod rtc;
pub mod user;
pub mod random;
pub mod crash;
pub mod watchdog;

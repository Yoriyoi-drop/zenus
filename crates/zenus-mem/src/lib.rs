#![no_std]
#![allow(static_mut_refs)]

pub mod allocator;
pub mod frame_allocator;
pub mod paging;
pub mod vma;
pub use vma::VmaTable;

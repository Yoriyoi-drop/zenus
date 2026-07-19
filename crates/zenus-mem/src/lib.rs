#![no_std]
#![allow(static_mut_refs)]

pub mod frame_allocator;
pub mod paging;
pub mod allocator;
pub mod vma;
pub use vma::VmaTable;

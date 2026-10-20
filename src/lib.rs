#![deny(unsafe_op_in_unsafe_fn)]

pub mod config;
pub mod geometry;
pub mod pixels;
pub mod state;
pub mod timeline;

#[cfg(all(windows, target_arch = "x86_64"))]
pub mod windows;

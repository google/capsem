//! Small, owned Unix primitives shared by host-side Capsem crates.

pub mod contained;
mod errno;
pub mod fd;
pub mod fs;
pub mod lock;
pub mod process;

//! Small, owned Unix primitives shared by host-side Capsem crates.

pub mod change_watch;
pub mod contained;
mod errno;
pub mod fd;
pub mod fs;
#[cfg(target_os = "linux")]
pub mod guest_memory;
pub mod lock;
pub mod peer;
pub mod process;
pub mod router_channel;
pub mod router_sandbox;
pub mod router_stream;
pub mod tcp;
pub mod tree_clone;

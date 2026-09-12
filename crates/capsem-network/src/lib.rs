//! The private link between VMs: length-prefixed ethernet frames on a
//! stream, and the verdict a switch gives each one.
//!
//! [`frames`] is the codec both ends of the VSOCK link speak. [`switch`] is
//! the whole decision of the per-network switch, written without I/O so a
//! frame's fate is a unit test: forwarded to one member, answered (ARP), or
//! dropped for a named reason.

pub mod frames;
pub mod switch;

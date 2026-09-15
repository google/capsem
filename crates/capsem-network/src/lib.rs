//! Private networks between VMs: length-prefixed ethernet frames on a
//! cable, and where a switch sends each one.
//!
//! [`frames`] is the codec both ends of a VSOCK cable speak. [`switch`] is
//! the whole forwarding decision of a network's switch, written without I/O
//! so a frame's fate is a unit test: one port, every other port, or dropped
//! for a named reason.

pub mod frames;
pub mod switch;

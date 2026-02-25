pub mod host_state;
pub mod net;
pub mod vm;

pub use capsem_proto;
pub use capsem_proto::{
    GuestToHost, HostToGuest, MAX_FRAME_SIZE, decode_guest_msg, decode_host_msg, encode_guest_msg,
    encode_host_msg,
};
pub use host_state::{
    HostState, HostStateMachine, StateMachine, Transition, validate_guest_msg, validate_host_msg,
};
pub use vm::config::VmConfig;
pub use vm::machine::VirtualMachine;
pub use vm::vsock::{
    self, CoalesceBuffer, VsockConnection, VsockManager, VSOCK_PORT_CONTROL, VSOCK_PORT_SNI_PROXY,
    VSOCK_PORT_TERMINAL,
};

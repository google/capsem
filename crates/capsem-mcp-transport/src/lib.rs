//! Connected MCP capabilities shared by the service and confined native workers.
//! This crate owns transport and role bootstrap, never policy, secrets or VM state.

pub mod http_transport;
pub mod native_worker;

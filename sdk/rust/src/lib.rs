#![doc = include_str!("../README.md")]

pub use capsem_api as models;
pub mod browser;
mod client;
pub use client::{create_call_options, prepare_create, prepare_fork, prepare_start};
mod connections;
mod credentials;
mod environments;
mod error;
pub use environments::Environments;
mod hypervisor;
mod images;
mod local;
mod managed;
pub use connections::{Connections, CredentialGrants};
pub use credentials::Credentials;
pub use managed::ManagedSession;
pub use tokio_util::sync::CancellationToken;
pub mod operations;
mod options;
mod requests;
pub use requests::Requests;
pub mod resources;
pub mod transport;
mod vm;
pub use error::{Error, Result};
pub use hypervisor::Hypervisor;
pub use images::Images;
pub use options::{
    CreateOptions, DiagnosticOptions, HistoryOptions, LogOptions, NetworkLogOptions, Registry, RunOptions,
    TimelineOptions, TriageOptions, VmSelector,
};
pub use resources::{Debug, Files, Mcp, McpServer, McpTools, Port, PortOptions, VmNetworks};
pub use vm::VM;

#[cfg(test)]
mod test_gateway;

#[cfg(test)]
mod test_contract;

#[cfg(test)]
mod tests;

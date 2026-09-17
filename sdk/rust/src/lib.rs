#![doc = include_str!("../README.md")]

pub use capsem_api as models;
mod client;
mod error;
mod hypervisor;
pub mod operations;
mod options;
pub mod resources;
pub mod transport;
mod vm;
pub use error::{Error, Result};
pub use hypervisor::Hypervisor;
pub use options::{
    CreateOptions, DiagnosticOptions, HistoryOptions, LogOptions, NetworkLogOptions, PageOptions, Registry, RunOptions,
    TimelineOptions, TriageOptions, VmSelector,
};
pub use resources::{Debug, Files, Port, PortOptions, VmNetworks};
pub use vm::VM;

#[cfg(test)]
mod test_gateway;

#[cfg(test)]
mod test_contract;

#[cfg(test)]
mod tests;

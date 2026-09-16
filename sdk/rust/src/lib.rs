#![doc = include_str!("../README.md")]

pub use capsem_api as models;
mod client;
mod error;
mod hypervisor;
mod memory;
pub mod operations;
mod options;
pub mod resources;
pub mod transport;
mod vm;
pub use error::{Error, Result};
pub use hypervisor::Hypervisor;
pub use memory::Memory;
pub use options::{
    ContainerOptions, CreateOptions, DiagnosticOptions, HistoryOptions, LogOptions, NetworkLogOptions, PageOptions,
    RunOptions, TimelineOptions, TriageOptions, VmSelector,
};
pub use vm::VM;

#[cfg(test)]
mod test_gateway;

#[cfg(test)]
mod test_contract;

#[cfg(test)]
mod tests;

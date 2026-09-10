//! HTTP wire contracts shared by the gateway and service.
//! No runtime, filesystem, or service dependencies belong here.

mod document;
pub use document::openapi;
mod hypervisor;
pub use hypervisor::*;
mod profile_status;
pub use profile_status::*;

mod lifecycle;
pub use lifecycle::*;
mod execution;
pub use execution::*;
mod files;
pub use files::*;
mod logs;
pub use logs::*;
mod updates;
pub use updates::*;
mod profiles;
pub use profiles::*;

#[cfg(test)]
mod tests;

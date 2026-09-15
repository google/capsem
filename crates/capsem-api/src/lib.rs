//! HTTP wire contracts shared by the gateway and service.
//! No runtime, filesystem, or service dependencies belong here.

mod document;
pub use document::openapi;
mod hypervisor;
pub use hypervisor::*;
mod vm_info;
pub use vm_info::*;
mod profile_status;
pub use profile_status::*;

mod containers;
pub use containers::*;
mod exposures;
pub use exposures::*;
mod lifecycle;
pub mod stream;
pub use lifecycle::*;
mod execution;
pub use execution::*;
mod files;
pub use files::*;
mod snapshots;
pub use snapshots::*;
mod timeline;
pub use timeline::*;
mod history;
pub use history::*;
mod stats_detail;
pub use stats_detail::*;
mod interactions;
pub use interactions::*;
mod logs;
pub use logs::*;
mod updates;
pub use updates::*;
mod restart;
pub use restart::*;
mod profiles;
pub use profiles::*;
mod networks;
pub use networks::*;
mod diagnostics;
pub use diagnostics::*;
mod mcp;
pub use mcp::*;

#[cfg(test)]
mod tests;

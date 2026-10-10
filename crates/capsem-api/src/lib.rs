//! HTTP wire contracts shared by the gateway and service.
//! No runtime, filesystem, or service dependencies belong here.

mod credentials;
mod document;
pub use credentials::*;
pub use document::{openapi, CONTRACT_VERSION};
mod hypervisor;
pub use hypervisor::*;
mod vm_info;
pub use vm_info::*;
mod asset_status;
pub use asset_status::*;

mod containers;
pub use containers::*;
mod exposures;
pub use exposures::*;
mod images;
pub use images::*;
mod lifecycle;
pub mod stream;
pub use lifecycle::*;
mod execution;
pub use execution::*;
mod files;
pub use files::*;
mod timeline;
pub use timeline::*;
mod history;
pub use history::*;
mod stats_detail;
pub use stats_detail::*;
pub mod bodies;
pub use bodies::{BodyEncoding, EventBodiesQuery, EventBodiesResponse};
mod interactions;
pub use interactions::*;
mod logs;
pub use logs::*;
mod updates;
pub use updates::*;
mod restart;
pub use restart::*;
mod proxy;
pub use proxy::*;
mod networks;
pub use networks::*;
mod diagnostics;
pub use diagnostics::*;
mod mcp;
pub use mcp::*;

#[cfg(test)]
mod tests;

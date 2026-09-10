//! HTTP wire contracts shared by the gateway and service.
//! No runtime, filesystem, or service dependencies belong here.

mod lifecycle;
pub use lifecycle::*;
mod execution;
pub use execution::*;
mod files;
pub use files::*;
mod updates;
pub use updates::*;
mod profiles;
pub use profiles::*;

#[cfg(test)]
mod tests;

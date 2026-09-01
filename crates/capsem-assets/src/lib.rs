//! Asset manifests, verification, compatibility, and resolution for Capsem.
//!
//! The crate owns immutable asset identity and materialization without taking
//! dependencies on VM, policy, database, MCP, or application layers.

pub mod asset_manager;
pub mod manifest_compat;

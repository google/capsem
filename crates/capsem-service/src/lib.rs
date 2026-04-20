//! `capsem-service` library surface.
//!
//! `main.rs` is the daemon entry point and still owns the bulk of the service
//! (`ServiceState`, `PersistentRegistry`, every axum handler, IPC plumbing).
//! This module exists so pure helpers can be unit-tested without spinning up
//! the full daemon and so a follow-up sprint can move handlers into
//! route-grouped modules without a second `Cargo.toml` change.

pub mod api;
pub mod errors;
pub mod fs_utils;
pub mod naming;

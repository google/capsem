//! Generic typed settings system with corp override.
//!
//! Each setting has an id, name, description, type, category, default value,
//! and optional `enabled_by` pointer to a parent toggle. Settings are stored
//! in TOML files at:
//!   - User: ~/.capsem/user.toml
//!   - Corporate: /etc/capsem/corp.toml
//!
//! Merge semantics: corp settings override user settings per-key.
//! User can only write user.toml. Corp file is read-only (MDM-distributed).

mod types;
mod registry;
mod loader;
mod presets;
mod resolver;
mod builder;
mod lint;
mod tree;
pub mod corp_provision;

// Re-export everything to preserve the existing public API.
pub use types::*;
pub use registry::{setting_definitions, default_settings_file};
pub use loader::*;
pub use presets::*;
pub use resolver::*;
pub use builder::*;
pub use lint::*;
pub use tree::*;

// Re-export sibling types used by tests and downstream code.
pub use super::domain_policy::{Action, DomainPolicy};
pub use super::http_policy::{HttpPolicy, HttpRule};

#[cfg(test)]
#[allow(unused_imports)]
mod tests;

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

mod builder;
mod condition;
pub mod corp_provision;
mod lint;
mod loader;
mod presets;
mod registry;
mod resolver;
mod tree;
mod types;

// Re-export everything to preserve the existing public API.
pub use builder::*;
pub use lint::*;
pub use loader::*;
pub use presets::*;
pub use registry::{default_settings_file, setting_definitions};
pub use resolver::*;
pub use tree::*;
pub use types::*;

// Re-export sibling types used by tests and downstream code.
pub use super::domain_policy::{Action, DomainPolicy};
pub use super::http_policy::{HttpPolicy, HttpRule};

#[cfg(test)]
#[allow(unused_imports)]
mod tests;

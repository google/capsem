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
mod ownership;
mod presets;
mod provider_profile;
mod registry;
mod resolver;
mod security_rule_profile;
mod tree;
mod types;

pub use builder::*;
pub use lint::*;
pub use loader::*;
pub use ownership::*;
pub use presets::*;
pub use provider_profile::*;
pub use registry::{default_settings_file, setting_definitions};
pub use resolver::*;
pub use security_rule_profile::*;
pub use tree::*;
pub use types::*;

#[cfg(test)]
#[allow(unused_imports)]
mod tests;

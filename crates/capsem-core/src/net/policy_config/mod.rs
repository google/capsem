//! Generic typed UI settings system with corp constraints.
//!
//! Each setting has an id, name, description, type, category, default value,
//! and optional `enabled_by` pointer to a parent toggle. Local UI settings are
//! stored in `settings.toml`. Corporate constraints live in `corp.toml`.
//!
//! Merge semantics: corp settings override local settings per-key.

mod builder;
mod condition;
pub mod corp_provision;
mod lint;
mod loader;
mod ownership;
mod profile_contract;
mod provider_profile;
mod resolver;
mod security_rule_profile;
mod settings_metadata;
mod tree;
mod types;

pub use builder::*;
pub use lint::*;
pub use loader::*;
pub use ownership::*;
pub use profile_contract::*;
pub use provider_profile::*;
pub use resolver::*;
pub use security_rule_profile::*;
pub use settings_metadata::{default_settings_file, setting_definitions};
pub use tree::*;
pub use types::*;

#[cfg(test)]
#[allow(unused_imports)]
mod tests;

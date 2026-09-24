//! Generic typed UI settings system with corp constraints.
//!
//! Each setting has an id, name, description, type, category, default value,
//! and optional `enabled_by` pointer to a parent toggle. Local UI settings are
//! stored in `settings.toml`. Corporate constraints live in `corp.toml`.
//!
//! Merge semantics: corp settings override local settings per-key.

mod active_profile_digest;
mod builder;
pub mod corp_provision;
mod lint;
mod loader;
mod ownership;
mod profile_catalog;
mod profile_contract;
mod provider_profile;
mod resolver;
mod security_rule_profile;
mod settings_metadata;
mod tree;
mod types;
mod validation;

pub use active_profile_digest::active_profile_digest;
pub use builder::*;
pub use capsem_config::*;
pub use lint::load_merged_lint;
pub use loader::*;
pub use ownership::*;
pub use profile_catalog::*;
pub use profile_contract::*;
pub use tree::load_settings_tree;

/// Immutable plugin configuration selected for one runtime generation.
pub type PluginPolicy = std::collections::BTreeMap<String, SecurityPluginConfig>;
pub type PluginPolicySnapshot = std::sync::Arc<PluginPolicy>;
/// Live policy handle whose writers replace a snapshot and readers clone its Arc.
pub type SharedPluginPolicy = std::sync::Arc<std::sync::RwLock<PluginPolicySnapshot>>;

pub fn snapshot_plugin_policy(policy: &SharedPluginPolicy) -> PluginPolicySnapshot {
    policy.read().unwrap().clone()
}

#[cfg(test)]
#[allow(unused_imports)]
mod tests;

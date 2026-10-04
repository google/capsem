//! Generic typed UI settings system with corp constraints.
//!
//! Each setting has an id, name, description, type, category, default value,
//! and optional `enabled_by` pointer to a parent toggle. The user's settings
//! live in `settings.toml`, which may set any registry id. Corporate
//! constraints live in `corp.toml`, which may set any id too.
//!
//! Merge semantics: corp settings override local settings per-key, and an id
//! corp sets is locked against `/settings/edit`.

mod active_policy;
mod builder;
pub mod corp_provision;
mod lint;
mod loader;
mod ownership;
mod provider_profile;
mod resolver;
mod security_rule_profile;
mod settings_metadata;
mod settings_policy;
mod tree;
mod types;

pub use active_policy::{active_policy_digest, ActivePolicyFile};
pub use builder::*;
pub use capsem_config::*;
pub use lint::load_merged_lint;
pub use loader::*;
pub use ownership::*;
pub use settings_policy::*;
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

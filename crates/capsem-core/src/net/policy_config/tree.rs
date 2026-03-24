use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use super::types::*;
use super::registry::{setting_definitions, DEFAULTS_TOML};
use super::loader::load_settings_files;
use super::resolver::resolve_settings;

/// A settings tree node: either a group of children or a leaf setting.
///
/// Serialized with `tag = "kind"` so JSON includes `{"kind": "group", ...}` or
/// `{"kind": "leaf", ...}`.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum SettingsNode {
    #[serde(rename = "group")]
    Group {
        key: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        enabled_by: Option<String>,
        collapsed: bool,
        children: Vec<SettingsNode>,
    },
    #[serde(rename = "leaf")]
    Leaf(Box<ResolvedSetting>),
}

/// Build a settings tree mirroring the TOML hierarchy with resolved values at leaves.
///
/// Walks the TOML structure like `collect_settings` but produces nested
/// `SettingsNode::Group` / `SettingsNode::Leaf` instead of flattening.
fn build_tree_from_table(
    path: &str,
    table: &toml::value::Table,
    parent_enabled_by: &Option<String>,
    parent_collapsed: bool,
    resolved_map: &HashMap<String, ResolvedSetting>,
) -> Vec<SettingsNode> {
    // Check if this is a leaf (has "type" key)
    if table.contains_key("type") {
        if let Some(resolved) = resolved_map.get(path) {
            return vec![SettingsNode::Leaf(Box::new(resolved.clone()))];
        }
        return vec![];
    }

    // Group node
    let group_name = table
        .get("name")
        .and_then(|v| v.as_str())
        .map(String::from);
    let group_description = table
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);
    let group_enabled_by = table
        .get("enabled_by")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| parent_enabled_by.clone());
    let group_collapsed = table
        .get("collapsed")
        .and_then(|v| v.as_bool())
        .unwrap_or(parent_collapsed);

    let mut children = Vec::new();
    for (key, val) in table {
        if matches!(
            key.as_str(),
            "name" | "description" | "enabled_by" | "collapsed"
        ) {
            continue;
        }
        if let Some(child_table) = val.as_table() {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            let child_nodes = build_tree_from_table(
                &child_path,
                child_table,
                &group_enabled_by,
                group_collapsed,
                resolved_map,
            );
            children.extend(child_nodes);
        }
    }

    // If we have a group name (this is a named group), wrap children.
    // Top-level call (path is empty) skips wrapping.
    if let Some(name) = group_name {
        if !path.is_empty() {
            return vec![SettingsNode::Group {
                key: path.to_string(),
                name,
                description: group_description,
                enabled_by: if parent_enabled_by.is_some() {
                    // Sub-group inherits parent enabled_by but the group node
                    // itself should show its own enabled_by.
                    group_enabled_by
                } else {
                    table
                        .get("enabled_by")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                },
                collapsed: group_collapsed,
                children,
            }];
        }
    }

    children
}

/// Build the full settings tree from defaults.toml + resolved values.
///
/// Returns top-level groups (AI Providers, Package Registries, etc.).
/// Dynamic `guest.env.*` settings are appended to the Guest Environment group.
pub fn build_settings_tree(resolved: &[ResolvedSetting]) -> Vec<SettingsNode> {
    let root: toml::Value =
        toml::from_str(DEFAULTS_TOML).expect("built-in defaults.toml is invalid");
    let settings = root
        .get("settings")
        .and_then(|v| v.as_table())
        .expect("defaults.toml missing [settings]");

    // Build a lookup from ID to resolved setting.
    let resolved_map: HashMap<String, ResolvedSetting> = resolved
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();

    let mut tree = Vec::new();
    for (key, val) in settings {
        if let Some(child_table) = val.as_table() {
            let nodes = build_tree_from_table(
                key,
                child_table,
                &None,
                false,
                &resolved_map,
            );
            tree.extend(nodes);
        }
    }

    // Append dynamic guest.env.* settings to the Environment group (under VM).
    let dynamic_envs: Vec<&ResolvedSetting> = resolved
        .iter()
        .filter(|s| s.id.starts_with("guest.env.") && !resolved_map.contains_key(&s.id)
            || (s.id.starts_with("guest.env.") && s.category == "VM" && setting_definitions().iter().all(|d| d.id != s.id)))
        .collect();

    if !dynamic_envs.is_empty() {
        // Find the Environment group (child of VM) and append
        fn append_dynamic(nodes: &mut [SettingsNode], envs: &[&ResolvedSetting]) {
            for node in nodes.iter_mut() {
                if let SettingsNode::Group { name, children, .. } = node {
                    if name == "Environment" {
                        for env in envs {
                            children.push(SettingsNode::Leaf(Box::new((*env).clone())));
                        }
                        return;
                    }
                    append_dynamic(children, envs);
                }
            }
        }
        append_dynamic(&mut tree, &dynamic_envs);
    }

    tree
}

/// Load settings tree from standard locations.
pub fn load_settings_tree() -> Vec<SettingsNode> {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    build_settings_tree(&resolved)
}

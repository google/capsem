use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use super::types::*;
use super::registry::{setting_definitions, DEFAULTS_JSON};
use super::loader::load_settings_files;
use super::resolver::resolve_settings;

/// A settings tree node: group, leaf setting, action button, or MCP server.
///
/// Serialized with `tag = "kind"` so JSON includes `{"kind": "group", ...}` etc.
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
        enabled: bool,
        collapsed: bool,
        children: Vec<SettingsNode>,
    },
    #[serde(rename = "leaf")]
    Leaf(Box<ResolvedSetting>),
    /// A grammar-driven action node (button/widget, no stored value).
    #[serde(rename = "action")]
    Action {
        key: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        action: ActionKind,
    },
    /// A declarative MCP server definition.
    #[serde(rename = "mcp_server")]
    McpServer(Box<McpServerDef>),
}

/// Build a settings tree mirroring the JSON hierarchy with resolved values at leaves.
///
/// Walks the JSON structure like `collect_settings` but produces nested
/// `SettingsNode::Group` / `SettingsNode::Leaf` instead of flattening.
fn build_tree_from_object(
    path: &str,
    table: &serde_json::Map<String, serde_json::Value>,
    parent_enabled_by: &Option<String>,
    parent_collapsed: bool,
    resolved_map: &HashMap<String, ResolvedSetting>,
) -> Vec<SettingsNode> {
    // Check if this is a leaf (has "type" key)
    if table.contains_key("type") {
        if let Some(resolved) = resolved_map.get(path) {
            if resolved.metadata.hidden {
                return vec![];
            }
            return vec![SettingsNode::Leaf(Box::new(resolved.clone()))];
        }
        return vec![];
    }

    // Check if this is an action node (has "action" key)
    if let Some(action_val) = table.get("action").and_then(|v| v.as_str()) {
        let action: ActionKind = match serde_json::from_value(
            serde_json::Value::String(action_val.to_string()),
        ) {
            Ok(a) => a,
            Err(_) => {
                tracing::warn!("unknown action kind '{action_val}' at {path}");
                return vec![];
            }
        };
        let hidden = table
            .get("hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if hidden {
            return vec![];
        }
        return vec![SettingsNode::Action {
            key: path.to_string(),
            name: table
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            description: table
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
            action,
        }];
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

    let group_hidden = table
        .get("hidden")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if group_hidden && !path.is_empty() {
        return vec![];
    }

    let mut children = Vec::new();
    for (key, val) in table {
        if matches!(
            key.as_str(),
            "name" | "description" | "enabled_by" | "collapsed" | "enabled" | "hidden"
        ) {
            continue;
        }
        if let Some(child_table) = val.as_object() {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            let child_nodes = build_tree_from_object(
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
            let group_enabled = table
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
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
                enabled: group_enabled,
                collapsed: group_collapsed,
                children,
            }];
        }
    }

    children
}

/// Build the full settings tree from defaults.json + resolved values.
///
/// Returns top-level groups (AI Providers, Package Registries, etc.).
/// Dynamic `guest.env.*` settings are appended to the Guest Environment group.
pub fn build_settings_tree(resolved: &[ResolvedSetting]) -> Vec<SettingsNode> {
    let root: serde_json::Value =
        serde_json::from_str(DEFAULTS_JSON).expect("built-in defaults.json is invalid");
    let settings = root
        .get("settings")
        .and_then(|v| v.as_object())
        .expect("defaults.json missing settings");

    // Build a lookup from ID to resolved setting.
    let resolved_map: HashMap<String, ResolvedSetting> = resolved
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();

    let mut tree = Vec::new();
    for (key, val) in settings {
        if let Some(child_table) = val.as_object() {
            let nodes = build_tree_from_object(
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

/// Build a settings tree including MCP server nodes.
///
/// MCP servers are appended as a top-level "MCP Servers" group if any exist.
pub fn build_settings_tree_with_mcp(
    resolved: &[ResolvedSetting],
    mcp_servers: &[McpServerDef],
) -> Vec<SettingsNode> {
    let mut tree = build_settings_tree(resolved);

    if !mcp_servers.is_empty() {
        let mcp_children: Vec<SettingsNode> = mcp_servers
            .iter()
            .filter(|s| s.enabled)
            .map(|s| SettingsNode::McpServer(Box::new(s.clone())))
            .collect();
        if !mcp_children.is_empty() {
            tree.push(SettingsNode::Group {
                key: "mcp".to_string(),
                name: "MCP Servers".to_string(),
                description: Some(
                    "Model Context Protocol servers available to AI agents".to_string(),
                ),
                enabled_by: None,
                enabled: true,
                collapsed: false,
                children: mcp_children,
            });
        }
    }

    tree
}

/// Load settings tree from standard locations.
pub fn load_settings_tree() -> Vec<SettingsNode> {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    let mcp_servers = super::loader::load_mcp_servers();
    build_settings_tree_with_mcp(&resolved, &mcp_servers)
}

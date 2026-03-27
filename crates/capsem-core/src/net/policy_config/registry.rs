use std::collections::HashMap;

use serde::Deserialize;

use super::types::*;

// ---------------------------------------------------------------------------
// JSON registry parser
// ---------------------------------------------------------------------------

/// A setting leaf as it appears in the defaults JSON. Core fields at top level,
/// metadata under `meta` sub-table.
#[derive(Deserialize, Debug)]
struct SettingDefRaw {
    name: String,
    description: String,
    #[serde(rename = "type")]
    setting_type: SettingType,
    default: SettingValue,
    #[serde(default)]
    collapsed: bool,
    #[serde(default)]
    meta: SettingMetaRaw,
}

#[derive(Deserialize, Debug, Default)]
struct SettingMetaRaw {
    #[serde(default)]
    domains: Vec<String>,
    #[serde(default)]
    choices: Vec<String>,
    #[serde(default)]
    min: Option<i64>,
    #[serde(default)]
    max: Option<i64>,
    #[serde(default)]
    rules: HashMap<String, HttpMethodPermissions>,
    #[serde(default)]
    env_vars: Vec<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    docs_url: Option<String>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    filetype: Option<String>,
    #[serde(default)]
    widget: Option<Widget>,
    #[serde(default)]
    side_effect: Option<SideEffect>,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    builtin: bool,
}

/// Category/group metadata from grouping nodes.
#[derive(Debug, Clone, Default)]
struct GroupMeta {
    /// Display name from nearest ancestor group with a `name` key.
    category: String,
    /// Parent toggle ID -- propagated to all child settings except the toggle.
    enabled_by: Option<String>,
    /// Whether the group starts collapsed in the UI.
    collapsed: bool,
}

/// Recursively walk the JSON object, collecting setting leaves.
///
/// An object with a `type` key is a leaf setting; otherwise it is a group node
/// whose `name`, `description`, `enabled_by`, and `collapsed` are group metadata.
fn collect_settings(
    path: &str,
    table: &serde_json::Map<String, serde_json::Value>,
    parent: &GroupMeta,
    out: &mut Vec<SettingDef>,
) {
    // Action nodes have `action` key -- skip them in the setting registry
    if table.contains_key("action") {
        return;
    }

    if table.contains_key("type") {
        // Leaf setting -- deserialize the object into SettingDefRaw
        let val = serde_json::Value::Object(table.clone());
        let def: SettingDefRaw = serde_json::from_value(val)
            .unwrap_or_else(|e| panic!("bad setting '{path}': {e}"));
        // Inherit enabled_by from parent group, unless this IS the toggle itself
        let enabled_by = if parent.enabled_by.as_deref() == Some(path) {
            None
        } else {
            parent.enabled_by.clone()
        };
        out.push(SettingDef {
            id: path.to_string(),
            category: parent.category.clone(),
            name: def.name,
            description: def.description,
            setting_type: def.setting_type,
            default_value: def.default,
            enabled_by,
            metadata: SettingMetadata {
                domains: def.meta.domains,
                choices: def.meta.choices,
                min: def.meta.min,
                max: def.meta.max,
                rules: def.meta.rules,
                env_vars: def.meta.env_vars,
                collapsed: def.collapsed,
                format: def.meta.format,
                docs_url: def.meta.docs_url,
                prefix: def.meta.prefix,
                filetype: def.meta.filetype,
                widget: def.meta.widget,
                side_effect: def.meta.side_effect,
                hidden: def.meta.hidden,
                builtin: def.meta.builtin,
                ..Default::default()
            },
        });
        return;
    }

    // Group node -- extract category metadata, recurse into children
    let group = GroupMeta {
        category: table
            .get("name")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| parent.category.clone()),
        enabled_by: table
            .get("enabled_by")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| parent.enabled_by.clone()),
        collapsed: table
            .get("collapsed")
            .and_then(|v| v.as_bool())
            .unwrap_or(parent.collapsed),
    };

    for (key, val) in table {
        // Skip group metadata keys -- they are not child settings
        if matches!(
            key.as_str(),
            "name" | "description" | "enabled_by" | "collapsed"
        ) {
            continue;
        }
        if let Some(child) = val.as_object() {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            collect_settings(&child_path, child, &group, out);
        }
    }
}

pub(super) const DEFAULTS_JSON: &str = include_str!("../../../../../config/defaults.json");

/// Returns the setting definitions parsed from the embedded defaults.json.
pub fn setting_definitions() -> Vec<SettingDef> {
    let root: serde_json::Value =
        serde_json::from_str(DEFAULTS_JSON).expect("built-in defaults.json is invalid");
    let settings = root
        .get("settings")
        .and_then(|v| v.as_object())
        .expect("defaults.json missing settings");
    let mut defs = Vec::new();
    let root_group = GroupMeta::default();
    collect_settings("", settings, &root_group, &mut defs);
    defs
}

/// Returns an empty settings file (all defaults).
pub fn default_settings_file() -> SettingsFile {
    SettingsFile::default()
}

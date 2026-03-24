use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::loader::{load_settings_file, write_settings_file};
use super::types::*;

const MEDIUM_PRESET_TOML: &str = include_str!("../../../../../config/presets/medium.toml");
const HIGH_PRESET_TOML: &str = include_str!("../../../../../config/presets/high.toml");

/// Parsed preset TOML file format.
#[derive(Deserialize, Debug)]
struct PresetToml {
    name: String,
    description: String,
    #[serde(default)]
    settings: HashMap<String, toml::Value>,
    #[serde(default)]
    mcp: Option<PresetMcpConfig>,
}

/// MCP configuration within a preset.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PresetMcpConfig {
    pub default_tool_permission: Option<crate::mcp::policy::ToolDecision>,
}

/// A security preset with its settings and MCP config.
#[derive(Serialize, Debug, Clone)]
pub struct SecurityPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub settings: HashMap<String, SettingValue>,
    pub mcp: Option<PresetMcpConfig>,
}

fn parse_preset(id: &str, toml_str: &str) -> SecurityPreset {
    let parsed: PresetToml =
        toml::from_str(toml_str).unwrap_or_else(|e| panic!("bad preset '{id}': {e}"));
    let mut settings = HashMap::new();
    for (key, val) in parsed.settings {
        let sv = match val {
            toml::Value::Boolean(b) => SettingValue::Bool(b),
            toml::Value::Integer(n) => SettingValue::Number(n),
            toml::Value::String(s) => SettingValue::Text(s),
            _ => continue,
        };
        settings.insert(key, sv);
    }
    SecurityPreset {
        id: id.to_string(),
        name: parsed.name,
        description: parsed.description,
        settings,
        mcp: parsed.mcp,
    }
}

/// Returns all available security presets (compile-time embedded).
pub fn security_presets() -> Vec<SecurityPreset> {
    vec![
        parse_preset("medium", MEDIUM_PRESET_TOML),
        parse_preset("high", HIGH_PRESET_TOML),
    ]
}

/// Apply a security preset by ID. Batch-writes settings to user.toml,
/// skipping any corp-locked keys. Returns the list of skipped setting IDs.
/// Also sets `mcp.default_tool_permission` if the preset specifies one.
pub fn apply_preset(preset_id: &str) -> Result<Vec<String>, String> {
    let user_path = super::user_config_path().ok_or("HOME not set")?;
    let corp_path = super::corp_config_path();
    apply_preset_to(preset_id, &user_path, &corp_path)
}

/// Internal: apply a preset with explicit file paths (testable without env vars).
pub fn apply_preset_to(
    preset_id: &str,
    user_path: &Path,
    corp_path: &Path,
) -> Result<Vec<String>, String> {
    let presets = security_presets();
    let preset = presets
        .iter()
        .find(|p| p.id == preset_id)
        .ok_or_else(|| format!("unknown preset: {preset_id}"))?;

    let mut file = load_settings_file(user_path)?;
    let corp = load_settings_file(corp_path)?;

    let mut skipped = Vec::new();
    let now = crate::session::now_iso();

    for (key, value) in &preset.settings {
        if corp.settings.contains_key(key) {
            skipped.push(key.clone());
            continue;
        }
        file.settings.insert(
            key.clone(),
            SettingEntry {
                value: value.clone(),
                modified: now.clone(),
            },
        );
    }

    // Apply MCP default_tool_permission if specified and not corp-locked.
    if let Some(ref mcp_config) = preset.mcp {
        if let Some(perm) = mcp_config.default_tool_permission {
            let corp_mcp = corp.mcp.unwrap_or_default();
            if corp_mcp.default_tool_permission.is_some() {
                skipped.push("mcp.default_tool_permission".to_string());
            } else {
                let mut user_mcp = file.mcp.clone().unwrap_or_default();
                user_mcp.default_tool_permission = Some(perm);
                file.mcp = Some(user_mcp);
            }
        }
    }

    write_settings_file(user_path, &file)?;
    Ok(skipped)
}

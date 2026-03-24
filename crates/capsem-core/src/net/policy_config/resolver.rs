use std::collections::HashMap;
use super::types::*;
use super::registry::setting_definitions;

/// Check if a setting is locked by corp.
pub fn is_setting_corp_locked(id: &str, corp: &SettingsFile) -> bool {
    corp.settings.contains_key(id)
}

/// Resolve all settings from user + corp files against the registry.
///
/// For each registered definition + any dynamic keys (guest.env.*),
/// corp overrides user, user overrides default.
/// Computes `enabled` from parent toggle.
pub fn resolve_settings(user: &SettingsFile, corp: &SettingsFile) -> Vec<ResolvedSetting> {
    let defs = setting_definitions();
    let mut resolved = Vec::new();

    for def in &defs {
        let (effective_value, source, modified) = resolve_value(&def.id, &def.default_value, user, corp);
        let corp_locked = corp.settings.contains_key(&def.id);

        resolved.push(ResolvedSetting {
            id: def.id.clone(),
            category: def.category.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
            setting_type: def.setting_type,
            default_value: def.default_value.clone(),
            effective_value,
            source,
            modified,
            corp_locked,
            enabled_by: def.enabled_by.clone(),
            enabled: true, // computed below
            metadata: def.metadata.clone(),
            collapsed: def.metadata.collapsed,
        });
    }

    // Dynamic settings: guest.env.* (not in registry)
    let dynamic_keys = collect_dynamic_keys(user, corp);
    for key in dynamic_keys {
        let default = SettingValue::Text(String::new());
        let (effective_value, source, modified) = resolve_value(&key, &default, user, corp);
        let corp_locked = corp.settings.contains_key(&key);

        resolved.push(ResolvedSetting {
            id: key.clone(),
            category: "VM".to_string(),
            name: key.strip_prefix("guest.env.").unwrap_or(&key).to_string(),
            description: format!("Guest environment variable: {}", key.strip_prefix("guest.env.").unwrap_or(&key)),
            setting_type: SettingType::Text,
            default_value: default,
            effective_value,
            source,
            modified,
            corp_locked,
            enabled_by: None,
            enabled: true,
            metadata: SettingMetadata::default(),
            collapsed: false,
        });
    }

    // Compute enabled_by: look up parent toggle value
    compute_enabled(&mut resolved);

    resolved
}

/// Resolve a single setting value: corp > user > default.
fn resolve_value(
    id: &str,
    default: &SettingValue,
    user: &SettingsFile,
    corp: &SettingsFile,
) -> (SettingValue, PolicySource, Option<String>) {
    if let Some(entry) = corp.settings.get(id) {
        (entry.value.clone(), PolicySource::Corp, Some(entry.modified.clone()))
    } else if let Some(entry) = user.settings.get(id) {
        (entry.value.clone(), PolicySource::User, Some(entry.modified.clone()))
    } else {
        (default.clone(), PolicySource::Default, None)
    }
}

/// Collect all dynamic keys (guest.env.*) from both files.
fn collect_dynamic_keys(user: &SettingsFile, corp: &SettingsFile) -> Vec<String> {
    let mut keys: Vec<String> = user
        .settings
        .keys()
        .chain(corp.settings.keys())
        .filter(|k| k.starts_with("guest.env."))
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Compute the `enabled` flag for each setting based on its parent toggle.
fn compute_enabled(settings: &mut [ResolvedSetting]) {
    // Build a lookup of id -> effective bool value
    let values: HashMap<String, bool> = settings
        .iter()
        .filter_map(|s| s.effective_value.as_bool().map(|b| (s.id.clone(), b)))
        .collect();

    for s in settings.iter_mut() {
        if let Some(ref parent_id) = s.enabled_by {
            s.enabled = values.get(parent_id.as_str()).copied().unwrap_or(false);
        }
        // else enabled stays true (set during construction)
    }
}

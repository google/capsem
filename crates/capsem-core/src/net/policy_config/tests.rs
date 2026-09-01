use super::*;
use std::collections::HashMap;

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn empty_file() -> SettingsFile {
    SettingsFile::default()
}

fn now_str() -> String {
    "2026-02-25T00:00:00Z".to_string()
}

fn file_with(entries: Vec<(&str, SettingValue)>) -> SettingsFile {
    let mut settings = HashMap::new();
    for (id, value) in entries {
        settings.insert(
            id.to_string(),
            SettingEntry {
                value,
                modified: now_str(),
            },
        );
    }
    SettingsFile {
        settings,
        ..Default::default()
    }
}

fn security_rule_ids(policies: &MergedPolicies) -> Vec<&str> {
    policies
        .security_rules
        .rules()
        .iter()
        .map(|rule| rule.rule_id.as_str())
        .collect()
}

fn has_security_rule(policies: &MergedPolicies, rule_id: &str) -> bool {
    security_rule_ids(policies).contains(&rule_id)
}

#[path = "tests/config_validation.rs"]
mod config_validation;
#[path = "tests/guest_runtime.rs"]
mod guest_runtime;
#[path = "tests/merged_policies.rs"]
mod merged_policies;
#[path = "tests/resolution.rs"]
mod resolution;
#[path = "tests/settings_management.rs"]
mod settings_management;
use settings_management::with_temp_configs;
#[path = "tests/vm_credentials.rs"]
mod vm_credentials;

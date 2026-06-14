use super::*;
use crate::net::policy_config::{setting_definitions, SettingEntry, SettingValue, SettingsFile};

fn entry(value: SettingValue) -> SettingEntry {
    SettingEntry {
        value,
        modified: "2026-06-07T00:00:00Z".to_string(),
    }
}

fn parse(input: &str) -> SettingsFile {
    toml::from_str(input).expect("settings carrier parses")
}

#[test]
fn setting_id_ownership_matches_current_registry_contract() {
    for definition in setting_definitions() {
        let owner = setting_id_owner(&definition.id);
        if definition.id.starts_with("app.") || definition.id.starts_with("appearance.") {
            assert_eq!(owner, ConfigOwner::Settings, "{}", definition.id);
        } else {
            assert_eq!(owner, ConfigOwner::Profile, "{}", definition.id);
        }
    }
}

#[test]
fn settings_toml_accepts_only_ui_application_preferences() {
    let mut file = SettingsFile::default();
    file.settings.insert(
        "appearance.dark_mode".to_string(),
        entry(SettingValue::Bool(true)),
    );
    file.settings.insert(
        "app.auto_update".to_string(),
        entry(SettingValue::Bool(false)),
    );

    validate_settings_toml_contract(&file).expect("ui settings are valid settings.toml");
}

#[test]
fn settings_toml_rejects_profile_behavior_settings() {
    for id in [
        "vm.resources.cpu_count",
        "security.web.http_upstream_ports",
        "ai.openai.api_key",
        "repository.providers.github.token",
    ] {
        let mut file = SettingsFile::default();
        file.settings
            .insert(id.to_string(), entry(SettingValue::Text("x".to_string())));

        let error = match validate_settings_toml_contract(&file) {
            Ok(()) => panic!("{id} must not belong to settings.toml"),
            Err(error) => error,
        };
        assert!(
            error.contains("owned by profile"),
            "{id} produced wrong error: {error}"
        );
    }
}

#[test]
fn settings_toml_rejects_behavior_sections() {
    for (label, input) in [
        (
            "rule_files",
            r#"
[rule_files]
enforcement = "enforcement.toml"
"#,
        ),
        (
            "profiles",
            r#"
[profiles.rules.block_http]
name = "block_http"
action = "block"
match = 'has(http.host)'
"#,
        ),
        (
            "default",
            r#"
[default.http]
name = "http"
action = "allow"
priority = "default"
match = 'has(http.host)'
"#,
        ),
        (
            "corp",
            r#"
[corp.rules.block_http]
name = "block_http"
action = "block"
match = 'has(http.host)'
"#,
        ),
        (
            "ai",
            r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_api"
action = "allow"
match = 'http.host == "api.openai.com"'
"#,
        ),
        (
            "plugins",
            r#"
[plugins.dummy_pre_eicar]
mode = "block"
"#,
        ),
        (
            "network",
            r#"
[network.dns]
upstreams = ["127.0.0.1:5353"]
"#,
        ),
    ] {
        let file = parse(input);
        assert!(
            validate_settings_toml_contract(&file).is_err(),
            "{label} must not belong to settings.toml"
        );
    }
}

#[test]
fn profile_toml_accepts_profile_behavior_and_rejects_ui_and_corp_fields() {
    let valid = parse(
        r#"
[settings."vm.resources.cpu_count"]
value = 8
modified = "2026-06-07T00:00:00Z"

[settings."security.web.http_upstream_ports"]
value = [80, 11434]
modified = "2026-06-07T00:00:00Z"

[rule_files]
enforcement = "rules/enforcement.toml"
sigma = "rules/detection.yaml"

[default.http]
name = "default_http"
action = "allow"
priority = "default"
match = 'has(http.host)'

[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_api"
action = "allow"
match = 'http.host == "api.openai.com"'

[plugins.dummy_pre_eicar]
mode = "block"
"#,
    );
    validate_profile_toml_contract(&valid).expect("profile behavior is profile-owned");

    let mut ui = SettingsFile::default();
    ui.settings.insert(
        "appearance.dark_mode".to_string(),
        entry(SettingValue::Bool(true)),
    );
    assert!(validate_profile_toml_contract(&ui)
        .unwrap_err()
        .contains("owned by settings"));

    let corp = parse(
        r#"
refresh_policy = "24h"

[corp_rule_files]
sigma_output_endpoint = "https://security.example.invalid/sigma"
"#,
    );
    assert!(validate_profile_toml_contract(&corp).is_err());

    let network = parse(
        r#"
[network.dns]
upstreams = ["127.0.0.1:5353"]
"#,
    );
    assert!(validate_profile_toml_contract(&network)
        .unwrap_err()
        .contains("network mechanics"));
}

#[test]
fn corp_toml_accepts_constraints_and_rejects_ui_preferences() {
    let valid = parse(
        r#"
refresh_policy = "24h"

[settings."vm.resources.cpu_count"]
value = 8
modified = "2026-06-07T00:00:00Z"

[corp.rules.block_external_http]
name = "block_external_http"
action = "block"
corp_locked = true
priority = -10
match = 'http.host == "external.example"'

[corp_rule_files]
sigma_output_endpoint = "https://security.example.invalid/sigma"

[network.dns]
upstreams = ["127.0.0.1:5353"]
"#,
    );
    validate_corp_toml_contract(&valid).expect("corp constraints are corp-owned");

    let mut ui = SettingsFile::default();
    ui.settings.insert(
        "app.auto_update".to_string(),
        entry(SettingValue::Bool(true)),
    );
    assert!(validate_corp_toml_contract(&ui)
        .unwrap_err()
        .contains("owned by settings"));
}

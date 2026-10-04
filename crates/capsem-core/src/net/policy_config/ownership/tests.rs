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
fn settings_toml_accepts_every_registry_setting() {
    // Every registry id is the user's (#289). With VM profiles gone, an id
    // settings.toml refused could be set by corp alone: the user's git
    // identity and service toggles among them.
    let mut file = SettingsFile::default();
    for definition in setting_definitions() {
        file.settings
            .insert(definition.id.clone(), entry(definition.default_value.clone()));
    }
    for id in [
        "appearance.dark_mode",
        "repository.git.identity.author_name",
        "repository.providers.github.allow",
        "security.services.registry.npm.allow",
        "vm.resources.cpu_count",
    ] {
        assert!(file.settings.contains_key(id), "{id} left the registry");
    }

    validate_settings_toml_contract(&file).expect("every registry setting belongs to settings.toml");
}

#[test]
fn settings_toml_carries_the_user_policy() {
    let file = parse(
        r#"
[rule_files]
enforcement = "enforcement.toml"

[profiles.rules.block_http]
name = "block_http"
action = "block"
match = 'has(http.host)'

[default.http]
name = "http"
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

[[mcp.servers]]
name = "wiki"
url = "https://wiki.example.invalid/mcp"
"#,
    );
    validate_settings_toml_contract(&file).expect("settings.toml owns the user's policy");
}

#[test]
fn settings_toml_rejects_corp_only_sections() {
    for (label, input) in [
        (
            "corp",
            r#"
[corp.rules.block_http]
name = "block_http"
action = "block"
match = 'has(http.host)'
"#,
        ),
        ("refresh_policy", r#"refresh_policy = "24h""#),
        (
            "corp_rule_files",
            r#"
[corp_rule_files]
sigma_output_endpoint = "https://security.example.invalid/sigma"
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

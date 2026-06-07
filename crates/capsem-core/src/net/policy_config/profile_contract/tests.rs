use super::*;

fn parse_profile(input: &str) -> ProfileConfigFile {
    toml::from_str(input).expect("profile TOML parses")
}

#[test]
fn profile_config_file_owns_full_profile_behavior_contract() {
    let profile = parse_profile(
        r#"
id = "developer"
name = "Developer"
description = "Default developer VM profile."
icon_svg = "<svg viewBox=\"0 0 16 16\"></svg>"

[availability]
web = true
shell = true
mobile = false

[assets]
channel = "stable"
kernel = "vmlinuz"
initrd = "initrd.img"
rootfs = "rootfs.erofs"

[vm]
cpu_count = 6
ram_gb = 8
scratch_disk_size_gb = 32

[rule_files]
enforcement = "rules/enforcement.toml"
sigma = "rules/detection.yaml"

[profiles.defaults.default_http_requests]
name = "default_http_requests"
action = "allow"
priority = "default"
reason = "Default allow for HTTP requests."
match = 'has(http.host)'

[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
match = 'file.read.path.contains("skills/")'

[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"
aliases = ["api.openai.com"]
listen_ports = [443]
credential_setting_id = "ai.openai.api_key"
allowed_remote_targets = ["api.openai.com:443"]
files = ["/root/.codex/config.toml"]

[ai.openai.rules.http_api]
name = "openai_http_api"
action = "allow"
match = 'http.host == "api.openai.com"'

[plugins.dummy_pre_eicar]
mode = "block"
detection_level = "critical"

[mcp]
health_check_interval_secs = 60

[[mcp.servers]]
name = "filesystem"
url = "http://127.0.0.1:9000"
enabled = true

[skills]
paths = ["/root/.codex/skills/security/SKILL.md"]

[tool_config_sources.codex]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
observed_hash = "blake3:2222222222222222222222222222222222222222222222222222222222222222"
inferred_endpoint_ref = "ai.openai"
credential_refs = ["credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"]
allowed_overlays = ["mcp_injection", "broker_placeholders", "endpoint_selection"]
"#,
    );

    profile.validate().expect("profile contract validates");
    assert_eq!(profile.id, "developer");
    assert_eq!(profile.assets.rootfs, "rootfs.erofs");
    assert_eq!(profile.vm.cpu_count, 6);
    assert!(profile
        .profiles
        .defaults
        .contains_key("default_http_requests"));
    assert!(profile.profiles.rules.contains_key("skill_loaded"));
    assert!(profile.ai.contains_key("openai"));
    assert!(profile.plugins.contains_key("dummy_pre_eicar"));
    assert_eq!(profile.mcp.unwrap().servers[0].name, "filesystem");
}

#[test]
fn builtin_default_profile_manifest_is_valid_and_erofs_backed() {
    let profile = ProfileConfigFile::builtin_default();

    profile
        .validate()
        .expect("builtin default profile validates");
    assert_eq!(profile.id, "default");
    assert_eq!(profile.name, "Default");
    assert_eq!(profile.assets.rootfs, "rootfs.erofs");
    assert!(profile.availability.web);
    assert!(profile.availability.shell);
    assert!(profile
        .profiles
        .defaults
        .contains_key("default_http_requests"));
    assert!(profile.plugins.contains_key("credential_broker"));
}

#[test]
fn profile_config_rejects_credential_broker_settings() {
    let error = toml::from_str::<ProfileConfigFile>(
        r#"
id = "developer"
name = "Developer"
description = "Default developer VM profile."

[credentials]
broker_enabled = true
"#,
    )
    .expect_err("credential broker config is plugin-owned, not a profile credential block");
    assert!(error.to_string().contains("unknown field `credentials`"));
}

#[test]
fn profile_config_rejects_ui_settings_soup() {
    let error = toml::from_str::<ProfileConfigFile>(
        r#"
id = "developer"
name = "Developer"
description = "Default developer VM profile."

[settings."appearance.dark_mode"]
value = true
modified = "2026-06-07T00:00:00Z"
"#,
    )
    .expect_err("profile files must not accept settings.toml sections");
    assert!(error.to_string().contains("unknown field `settings`"));
}

#[test]
fn profile_config_validation_rejects_bad_identity_assets_and_vm_defaults() {
    let mut profile = parse_profile(
        r#"
id = "Bad Profile"
name = "Developer"
description = "Default developer VM profile."
"#,
    );
    assert!(profile.validate().unwrap_err().contains("lowercase ascii"));

    profile.id = "developer".to_string();
    profile.icon_svg = Some("<div></div>".to_string());
    assert!(profile.validate().unwrap_err().contains("icon_svg"));

    profile.icon_svg = Some("<svg></svg>".to_string());
    profile.vm.cpu_count = 0;
    assert!(profile.validate().unwrap_err().contains("cpu_count"));

    profile.vm.cpu_count = 4;
    profile.assets.rootfs.clear();
    assert!(profile.validate().unwrap_err().contains("rootfs"));
}

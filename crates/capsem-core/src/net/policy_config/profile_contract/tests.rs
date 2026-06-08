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
revision = "2026.0607.1"
refresh_policy = "24h"

[availability]
web = true
shell = true
mobile = false

[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"
filesystem = "erofs"
compression = "lz4hc"
compression_level = 12

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "https://example.invalid/arm64-vmlinuz"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
signature = "minisig:test"
size = 1
content_type = "application/octet-stream"

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "https://example.invalid/arm64-initrd.img"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
signature = "minisig:test"
size = 1
content_type = "application/octet-stream"

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://example.invalid/arm64-rootfs.erofs"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
signature = "minisig:test"
size = 1
content_type = "application/vnd.capsem.erofs"
filesystem = "erofs"
compression = "lz4hc"
compression_level = 12

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
    assert_eq!(profile.assets.arch["arm64"].rootfs.name, "rootfs.erofs");
    assert_eq!(profile.vm.cpu_count, 6);
    assert_eq!(
        profile.rule_files.enforcement.as_deref(),
        Some("rules/enforcement.toml")
    );
    assert_eq!(
        profile.rule_files.sigma.as_deref(),
        Some("rules/detection.yaml")
    );
    assert!(profile.profiles.rules.contains_key("skill_loaded"));
    assert!(profile.ai.contains_key("openai"));
    assert!(profile.plugins.contains_key("dummy_pre_eicar"));
    assert_eq!(profile.mcp.unwrap().servers[0].name, "filesystem");
}

#[test]
fn builtin_code_profile_manifest_is_valid_and_erofs_backed() {
    let profile = ProfileConfigFile::builtin_code();

    profile.validate().expect("builtin code profile validates");
    assert_eq!(profile.id, "code");
    assert_eq!(profile.name, "Code");
    assert_eq!(
        profile
            .assets
            .current_arch_assets()
            .expect("current architecture assets")
            .rootfs
            .name,
        "rootfs.erofs"
    );
    assert!(profile.availability.web);
    assert!(profile.availability.shell);
    assert_eq!(
        profile.rule_files.enforcement.as_deref(),
        Some("profiles/code/enforcement.toml")
    );
    assert_eq!(
        profile.rule_files.sigma.as_deref(),
        Some("profiles/code/detection.yaml")
    );
    assert!(profile.plugins.contains_key("credential_broker"));
}

#[test]
fn profile_config_rejects_credential_broker_settings() {
    let error = toml::from_str::<ProfileConfigFile>(
        r#"
id = "developer"
name = "Developer"
description = "Default developer VM profile."
revision = "2026.0607.1"
refresh_policy = "24h"

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
revision = "2026.0607.1"
refresh_policy = "24h"

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
    let mut profile = ProfileConfigFile::builtin_code();
    profile.id = "Bad Profile".to_string();
    assert!(profile.validate().unwrap_err().contains("lowercase ascii"));

    profile.id = "developer".to_string();
    profile.icon_svg = Some("<div></div>".to_string());
    assert!(profile.validate().unwrap_err().contains("icon_svg"));

    profile.icon_svg = Some("<svg></svg>".to_string());
    profile.vm.cpu_count = 0;
    assert!(profile.validate().unwrap_err().contains("cpu_count"));

    profile.vm.cpu_count = 4;
    profile.assets.arch.clear();
    assert!(profile.validate().unwrap_err().contains("assets.arch"));
}

#[test]
fn checked_in_code_profile_parses_and_validates() {
    let profile = toml::from_str::<ProfileConfigFile>(include_str!(
        "../../../../../../config/profiles/code.toml"
    ))
    .expect("checked-in code profile parses");

    profile
        .validate()
        .expect("checked-in code profile validates");
    assert_eq!(profile.id, "code");
    assert_eq!(profile.assets.filesystem, "erofs");
    assert_eq!(profile.assets.compression, "lz4hc");
    assert_eq!(profile.assets.compression_level, 12);
    assert!(profile.assets.arch.contains_key("arm64"));
    assert!(profile.assets.arch.contains_key("x86_64"));
    assert!(profile.plugins.contains_key("credential_broker"));
}

#[test]
fn profile_catalog_loads_directory_profiles_and_rejects_id_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("code.toml"),
        include_str!("../../../../../../config/profiles/code.toml"),
    )
    .unwrap();

    let catalog = ProfileCatalog::load_from_dir(dir.path()).expect("catalog loads");
    let profile = catalog.get("code").expect("code profile exists");
    assert_eq!(profile.name, "Code");
    assert_eq!(catalog.profiles().count(), 1);

    std::fs::write(
        dir.path().join("wrong.toml"),
        include_str!("../../../../../../config/profiles/code.toml"),
    )
    .unwrap();
    let error = ProfileCatalog::load_from_dir(dir.path()).unwrap_err();
    assert!(error.contains("id mismatch"), "{error}");
}

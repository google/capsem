use super::*;

#[test]
fn load_settings_file_missing_returns_default() {
    let result = load_settings_file(Path::new("/nonexistent/path/settings.toml"));
    assert!(result.is_ok());
    let file = result.unwrap();
    assert!(file.settings.is_empty());
}

#[test]
fn load_settings_file_invalid_toml() {
    let tmp = std::env::temp_dir().join("capsem-test-invalid.toml");
    std::fs::write(&tmp, "this is not valid { toml !!!").unwrap();
    let result = load_settings_file(&tmp);
    assert!(result.is_err());
    std::fs::remove_file(&tmp).ok();
}

#[test]
fn load_settings_file_empty_file() {
    let tmp = std::env::temp_dir().join("capsem-test-empty.toml");
    std::fs::write(&tmp, "").unwrap();
    let result = load_settings_file(&tmp);
    assert!(result.is_ok());
    std::fs::remove_file(&tmp).ok();
}

#[test]
fn write_then_load_roundtrip() {
    let tmp = std::env::temp_dir().join("capsem-test-roundtrip.toml");
    let mut file = SettingsFile::default();
    file.settings.insert(
        "test.key".into(),
        crate::net::policy_config::types::SettingEntry {
            value: SettingValue::Text("hello".into()),
            modified: "2024-01-01T00:00:00Z".into(),
        },
    );
    write_settings_file(&tmp, &file).unwrap();
    let loaded = load_settings_file(&tmp).unwrap();
    assert!(loaded.settings.contains_key("test.key"));
    let val = &loaded.settings["test.key"].value;
    assert_eq!(val.as_text(), Some("hello"));
    std::fs::remove_file(&tmp).ok();
}

#[test]
fn load_local_settings_file_rejects_profile_behavior() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        r#"
[settings."vm.resources.cpu_count"]
value = 8
modified = "2026-06-11T00:00:00Z"
"#,
    )
    .unwrap();

    let error = load_local_settings_file(tmp.path()).expect_err("profile behavior rejected");
    assert!(error.contains("owned by profile"), "{error}");
}

#[test]
fn load_local_settings_file_accepts_ui_preferences() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        r#"
[settings."appearance.dark_mode"]
value = true
modified = "2026-06-11T00:00:00Z"
"#,
    )
    .unwrap();

    let file = load_local_settings_file(tmp.path()).expect("ui settings load");
    assert!(file.settings.contains_key("appearance.dark_mode"));
}

#[test]
fn load_corp_settings_file_rejects_ui_preferences() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        r#"
[settings."app.auto_update"]
value = true
modified = "2026-06-11T00:00:00Z"
"#,
    )
    .unwrap();

    let error = load_corp_settings_file(tmp.path()).expect_err("ui setting rejected");
    assert!(error.contains("owned by settings"), "{error}");
}

#[test]
fn load_corp_settings_file_accepts_constraints() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        r#"
refresh_policy = "24h"

[settings."vm.resources.cpu_count"]
value = 8
modified = "2026-06-11T00:00:00Z"

[corp.rules.block_example]
name = "block_example"
action = "block"
priority = -10
match = 'http.host == "example.invalid"'
"#,
    )
    .unwrap();

    let file = load_corp_settings_file(tmp.path()).expect("corp constraints load");
    assert!(file.settings.contains_key("vm.resources.cpu_count"));
    assert!(file.corp.rules.contains_key("block_example"));
}

#[test]
fn settings_file_parses_rule_file_references() {
    let file: SettingsFile = toml::from_str(
        r#"
[rule_files]
enforcement = "profiles/base/enforcement.toml"
sigma = "profiles/base/detection.yaml"

[corp_rule_files]
sigma_output_endpoint = "https://security.example.invalid/capsem/sigma"
"#,
    )
    .expect("rule file references parse");

    assert_eq!(
        file.rule_files.enforcement.as_deref(),
        Some("profiles/base/enforcement.toml")
    );
    assert_eq!(
        file.rule_files.sigma.as_deref(),
        Some("profiles/base/detection.yaml")
    );
    assert_eq!(
        file.corp_rule_files.sigma_output_endpoint.as_deref(),
        Some("https://security.example.invalid/capsem/sigma")
    );
}

#[test]
fn load_referenced_enforcement_rules_resolves_relative_to_settings_file() {
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("settings.toml");
    let rules_dir = dir.path().join("profiles").join("base");
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(
        rules_dir.join("enforcement.toml"),
        r#"
[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
match = 'file.read.path.matches("(^|.*/)skills/.+\\.md$") && file.read.ext == "md"'
"#,
    )
    .unwrap();
    std::fs::write(
        &settings_path,
        r#"
[rule_files]
enforcement = "profiles/base/enforcement.toml"
"#,
    )
    .unwrap();

    let file = load_settings_file(&settings_path).expect("settings load");
    let profile =
        load_referenced_enforcement_rules(&settings_path, &file).expect("enforcement loads");
    assert!(profile
        .expect("profile exists")
        .profiles
        .rules
        .contains_key("skill_loaded"));
}

#[test]
fn load_referenced_sigma_rules_resolves_relative_to_settings_file() {
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("settings.toml");
    let rules_dir = dir.path().join("profiles").join("base");
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(
        rules_dir.join("detection.yaml"),
        r#"
title: OpenAI Traffic To Unexpected Endpoint
id: 11111111-1111-4111-8111-111111111111
logsource:
  product: capsem
  service: security_event
detection:
  selection_model:
    model.provider: openai
  filter_approved_endpoint:
    http.host: api.openai.com
  condition: selection_model and not filter_approved_endpoint
level: high
capsem:
  action: block
  reason: OpenAI traffic must use the approved endpoint.
"#,
    )
    .unwrap();
    std::fs::write(
        &settings_path,
        r#"
[rule_files]
sigma = "profiles/base/detection.yaml"
"#,
    )
    .unwrap();

    let file = load_settings_file(&settings_path).expect("settings load");
    let profile = load_referenced_sigma_rules(&settings_path, &file).expect("sigma loads");
    let profile = profile.expect("profile exists");
    let rule = profile
        .profiles
        .rules
        .get("openai_traffic_to_unexpected_endpoint")
        .expect("derived Sigma rule");
    assert_eq!(rule.action, super::super::SecurityRuleAction::Block);
    assert_eq!(
        rule.detection_level,
        Some(super::super::DetectionLevel::High)
    );
    assert_eq!(
        rule.condition,
        r#"model.provider == "openai" && http.host != "api.openai.com""#
    );
}

#[test]
fn migrate_setting_ids_does_not_resurrect_retired_web_decision_keys() {
    let mut file = SettingsFile::default();
    file.settings.insert(
        "web.defaults.allow_read".into(),
        crate::net::policy_config::types::SettingEntry {
            value: SettingValue::Bool(true),
            modified: "2024-01-01".into(),
        },
    );
    migrate_setting_ids(&mut file);
    assert!(file.settings.contains_key("web.defaults.allow_read"));
    assert!(!file.settings.contains_key("security.web.allow_read"));
}

#[test]
fn migrate_setting_ids_still_renames_live_service_keys() {
    let mut file = SettingsFile::default();
    file.settings.insert(
        "web.search.google.allow".into(),
        crate::net::policy_config::types::SettingEntry {
            value: SettingValue::Bool(false),
            modified: "old".into(),
        },
    );
    migrate_setting_ids(&mut file);
    assert!(!file.settings.contains_key("web.search.google.allow"));
    let val = file.settings["security.services.search.google.allow"]
        .value
        .as_bool()
        .unwrap();
    assert!(!val);
}

#[test]
fn can_write_corp_settings_always_false() {
    assert!(!can_write_corp_settings());
}

/// Env-var resolution tests run serially in a single test to avoid races with
/// other tests mutating the same process-global env vars under parallel
/// execution.
#[test]
fn env_var_path_resolution() {
    let _guard = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();

    // Snapshot prior values so we can restore them at the end.
    let prev_home_override = std::env::var("CAPSEM_HOME").ok();
    let prev_corp = std::env::var("CAPSEM_CORP_CONFIG").ok();

    // Local settings are rooted by CAPSEM_HOME.
    std::env::set_var("CAPSEM_HOME", "/tmp/custom-capsem-home");
    assert_eq!(
        settings_config_path(),
        Some(std::path::PathBuf::from(
            "/tmp/custom-capsem-home/settings.toml"
        ))
    );
    std::env::remove_var("CAPSEM_HOME");

    // Corp override via env.
    std::env::set_var("CAPSEM_CORP_CONFIG", "/tmp/custom-corp.toml");
    assert_eq!(
        corp_config_path(),
        std::path::PathBuf::from("/tmp/custom-corp.toml")
    );
    std::env::remove_var("CAPSEM_CORP_CONFIG");

    // Corp default (env unset).
    assert_eq!(
        corp_config_path(),
        std::path::PathBuf::from("/etc/capsem/corp.toml")
    );

    // Restore any prior values.
    match prev_home_override {
        Some(v) => std::env::set_var("CAPSEM_HOME", v),
        None => std::env::remove_var("CAPSEM_HOME"),
    }
    match prev_corp {
        Some(v) => std::env::set_var("CAPSEM_CORP_CONFIG", v),
        None => std::env::remove_var("CAPSEM_CORP_CONFIG"),
    }
}

#[test]
fn load_settings_and_corp_files_preserves_direct_corp_rule_groups_from_env_config() {
    let _guard = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let tmp = tempfile::tempdir().unwrap();
    let settings_home = tmp.path().join("capsem-home");
    let settings_path = settings_home.join("settings.toml");
    let corp_path = tmp.path().join("corp.toml");
    std::fs::create_dir_all(&settings_home).unwrap();
    std::fs::write(&settings_path, "").unwrap();
    std::fs::write(
        &corp_path,
        r#"
[corp.rules.block_local_deny_target]
name = "block_local_deny_target"
action = "block"
priority = -100
detection_level = "high"
reason = "Loader regression proof."
match = 'http.host == "127.0.0.1" && http.path == "/deny-target"'

[plugins.credential_broker]
mode = "rewrite"

[network.dns]
upstreams = ["127.0.0.1:5353"]
        "#,
    )
    .unwrap();

    let prev_home_override = std::env::var("CAPSEM_HOME").ok();
    let prev_corp = std::env::var("CAPSEM_CORP_CONFIG").ok();
    std::env::set_var("CAPSEM_HOME", &settings_home);
    std::env::set_var("CAPSEM_CORP_CONFIG", &corp_path);
    let (_, corp) = load_settings_and_corp_files();
    match prev_home_override {
        Some(v) => std::env::set_var("CAPSEM_HOME", v),
        None => std::env::remove_var("CAPSEM_HOME"),
    }
    match prev_corp {
        Some(v) => std::env::set_var("CAPSEM_CORP_CONFIG", v),
        None => std::env::remove_var("CAPSEM_CORP_CONFIG"),
    }

    assert!(
        corp.corp.rules.contains_key("block_local_deny_target"),
        "direct corp rules must not be dropped by load_settings_and_corp_files"
    );
    assert!(
        corp.plugins.contains_key("credential_broker"),
        "corp plugin policy must not be dropped by load_settings_and_corp_files"
    );
    assert_eq!(corp.network.dns.upstreams, vec!["127.0.0.1:5353"]);
}

#[test]
fn parse_mcp_section_ignores_missing_section() {
    let toml = "[settings]\n";
    assert!(parse_mcp_section(toml, PolicySource::User).is_empty());
}

#[test]
fn parse_mcp_section_ignores_invalid_toml() {
    assert!(parse_mcp_section("{{{not toml", PolicySource::User).is_empty());
}

#[test]
fn parse_mcp_section_skips_non_server_mechanics_keys() {
    let toml = r#"
[mcp]
health_check_interval_secs = 60

[mcp.my_server]
name = "Example"
transport = "stdio"
command = "example-mcp"
"#;
    let servers = parse_mcp_section(toml, PolicySource::User);
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].key, "my_server");
    assert_eq!(servers[0].name, "Example");
    assert_eq!(servers[0].command.as_deref(), Some("example-mcp"));
    assert_eq!(servers[0].source, PolicySource::User);
    // enabled defaults to true via the `default_true` helper.
    assert!(servers[0].enabled);
    assert!(!servers[0].corp_locked);
}

#[test]
fn load_settings_file_rejects_retired_mcp_policy_keys() {
    let dir = tempfile::tempdir().unwrap();
    for retired in [
        r#"[mcp]
global_policy = "block"
"#,
        r#"[mcp]
default_tool_permission = "warn"
"#,
        r#"[mcp.tool_permissions]
local__echo = "block"
"#,
    ] {
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, retired).unwrap();
        let error = load_settings_file(&path).unwrap_err();
        assert!(
            error.contains("retired MCP policy key"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn parse_mcp_section_skips_malformed_server_entries() {
    let toml = r#"
[mcp.bad_server]
# missing required `name` field
transport = "stdio"

[mcp.good_server]
name = "Good"
transport = "sse"
url = "https://example.com/mcp"
"#;
    let servers = parse_mcp_section(toml, PolicySource::Corp);
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].key, "good_server");
    assert_eq!(servers[0].url.as_deref(), Some("https://example.com/mcp"));
}

#[test]
fn parse_mcp_section_json_ignores_missing_section() {
    assert!(parse_mcp_section_json("{}", PolicySource::Default).is_empty());
    // Also handles invalid JSON silently.
    assert!(parse_mcp_section_json("not json", PolicySource::Default).is_empty());
}

#[test]
fn parse_mcp_section_json_parses_builtin_server() {
    let json = r#"{
      "mcp": {
        "my_tool": {
          "name": "My Tool",
          "transport": "stdio",
          "command": "mytool",
          "builtin": true,
          "enabled": false
        }
      }
    }"#;
    let servers = parse_mcp_section_json(json, PolicySource::Default);
    assert_eq!(servers.len(), 1);
    let s = &servers[0];
    assert_eq!(s.key, "my_tool");
    assert!(s.builtin);
    assert!(!s.enabled);
    assert_eq!(s.source, PolicySource::Default);
}

#[test]
fn parse_mcp_section_json_skips_malformed_entries() {
    let json = r#"{
      "mcp": {
        "broken": {},
        "ok": {"name": "OK", "transport": "stdio"}
      }
    }"#;
    let servers = parse_mcp_section_json(json, PolicySource::User);
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].key, "ok");
}

#[test]
fn validate_setting_value_allows_non_file_values() {
    assert!(validate_setting_value("any.id", &SettingValue::Bool(true)).is_ok());
    assert!(validate_setting_value("any.id", &SettingValue::Number(1)).is_ok());
    assert!(validate_setting_value("any.id", &SettingValue::Text("x".into())).is_ok());
}

#[test]
fn validate_setting_value_accepts_empty_json_file() {
    let v = SettingValue::File {
        path: "/tmp/out.json".into(),
        content: String::new(),
    };
    // Empty content is allowed for .json paths (no JSON parse performed).
    assert!(validate_setting_value("cfg.id", &v).is_ok());
}

#[test]
fn validate_setting_value_rejects_bad_json_content() {
    let v = SettingValue::File {
        path: "/tmp/out.json".into(),
        content: "not json at all".into(),
    };
    let err = validate_setting_value("cfg.id", &v).unwrap_err();
    assert!(err.contains("invalid JSON for cfg.id"));
}

#[test]
fn validate_setting_value_accepts_non_json_file_content() {
    // Non-.json paths skip JSON validation.
    let v = SettingValue::File {
        path: "/tmp/out.conf".into(),
        content: "arbitrary text".into(),
    };
    assert!(validate_setting_value("cfg.id", &v).is_ok());
}

#[test]
fn validate_setting_value_rejects_invalid_path() {
    // capsem_proto::validate_file_path rejects traversal/relative paths.
    let v = SettingValue::File {
        path: "../etc/passwd".into(),
        content: "x".into(),
    };
    let err = validate_setting_value("cfg.id", &v).unwrap_err();
    assert!(err.contains("invalid path for cfg.id"));
}

#[test]
fn batch_update_settings_empty_changes_is_noop() {
    let changes: HashMap<String, SettingValue> = HashMap::new();
    let applied = batch_update_settings(&changes).unwrap();
    assert!(applied.is_empty());
}

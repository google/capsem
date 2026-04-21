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
    file.settings.insert("test.key".into(), crate::net::policy_config::types::SettingEntry {
        value: SettingValue::Text("hello".into()),
        modified: "2024-01-01T00:00:00Z".into(),
    });
    write_settings_file(&tmp, &file).unwrap();
    let loaded = load_settings_file(&tmp).unwrap();
    assert!(loaded.settings.contains_key("test.key"));
    let val = &loaded.settings["test.key"].value;
    assert_eq!(val.as_text(), Some("hello"));
    std::fs::remove_file(&tmp).ok();
}

#[test]
fn migrate_setting_ids_renames_old_keys() {
    let mut file = SettingsFile::default();
    file.settings.insert("web.defaults.allow_read".into(), crate::net::policy_config::types::SettingEntry {
        value: SettingValue::Bool(true),
        modified: "2024-01-01".into(),
    });
    migrate_setting_ids(&mut file);
    assert!(!file.settings.contains_key("web.defaults.allow_read"));
    assert!(file.settings.contains_key("security.web.allow_read"));
}

#[test]
fn migrate_setting_ids_does_not_clobber_new() {
    let mut file = SettingsFile::default();
    // Both old and new key exist -- new key should be preserved
    file.settings.insert("web.defaults.allow_read".into(), crate::net::policy_config::types::SettingEntry {
        value: SettingValue::Bool(false),
        modified: "old".into(),
    });
    file.settings.insert("security.web.allow_read".into(), crate::net::policy_config::types::SettingEntry {
        value: SettingValue::Bool(true),
        modified: "new".into(),
    });
    migrate_setting_ids(&mut file);
    // New key retains its value
    let val = file.settings["security.web.allow_read"].value.as_bool().unwrap();
    assert!(val); // true from the new key, not false from old
}

#[test]
fn can_write_corp_settings_always_false() {
    assert!(!can_write_corp_settings());
}

/// Env-var resolution tests run serially in a single test to avoid races
/// with other tests mutating the same process-global env vars under
/// parallel execution.
#[test]
fn env_var_path_resolution() {
    // Snapshot prior values so we can restore them at the end.
    let prev_user = std::env::var("CAPSEM_USER_CONFIG").ok();
    let prev_corp = std::env::var("CAPSEM_CORP_CONFIG").ok();

    // User override via env.
    std::env::set_var("CAPSEM_USER_CONFIG", "/tmp/custom-user.toml");
    assert_eq!(
        user_config_path(),
        Some(std::path::PathBuf::from("/tmp/custom-user.toml"))
    );
    std::env::remove_var("CAPSEM_USER_CONFIG");

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
    match prev_user {
        Some(v) => std::env::set_var("CAPSEM_USER_CONFIG", v),
        None => std::env::remove_var("CAPSEM_USER_CONFIG"),
    }
    match prev_corp {
        Some(v) => std::env::set_var("CAPSEM_CORP_CONFIG", v),
        None => std::env::remove_var("CAPSEM_CORP_CONFIG"),
    }
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
fn parse_mcp_section_skips_global_keys() {
    let toml = r#"
[mcp]
global_policy = "any"
default_tool_permission = "deny"
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
        "global_policy": "any",
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

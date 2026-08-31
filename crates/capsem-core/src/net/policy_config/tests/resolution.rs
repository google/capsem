use super::*;

#[test]
fn repeated_plugin_policy_snapshots_share_one_allocation() {
    let policy = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(PluginPolicy::new())));

    let first = snapshot_plugin_policy(&policy);
    let second = snapshot_plugin_policy(&policy);

    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "reading an unchanged policy must only increment an Arc refcount"
    );
}

#[test]
fn replacing_plugin_policy_snapshot_is_visible_to_later_readers() {
    let policy = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(PluginPolicy::new())));
    let before = snapshot_plugin_policy(&policy);
    let mut replacement = PluginPolicy::new();
    replacement.insert(
        "log_sanitizer".to_string(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Rewrite,
            detection_level: DetectionLevel::High,
        },
    );

    *policy.write().unwrap() = std::sync::Arc::new(replacement);
    let after = snapshot_plugin_policy(&policy);

    assert!(!std::sync::Arc::ptr_eq(&before, &after));
    assert_eq!(after["log_sanitizer"].mode, SecurityPluginMode::Rewrite);
}

// -----------------------------------------------------------------------
// A: Corp override (7)
// -----------------------------------------------------------------------

#[test]
fn corp_override_bool() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let corp = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved.iter().find(|s| s.id == SETTING_GITHUB_ALLOW).unwrap();
    assert_eq!(s.effective_value, SettingValue::Bool(false));
    assert_eq!(s.source, PolicySource::Corp);
}

#[test]
fn corp_override_network_mechanics_ports() {
    let user = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80, 3128, 3713, 8080, 11434]),
    )]);
    let corp = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80]),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved
        .iter()
        .find(|s| s.id == "security.web.http_upstream_ports")
        .unwrap();
    assert_eq!(s.effective_value, SettingValue::IntList(vec![80]));
    assert_eq!(s.source, PolicySource::Corp);
}

#[test]
fn corp_override_number() {
    let user = file_with(vec![("vm.resources.max_body_capture", SettingValue::Number(8192))]);
    let corp = file_with(vec![("vm.resources.max_body_capture", SettingValue::Number(1024))]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved
        .iter()
        .find(|s| s.id == "vm.resources.max_body_capture")
        .unwrap();
    assert_eq!(s.effective_value, SettingValue::Number(1024));
    assert_eq!(s.source, PolicySource::Corp);
}

#[test]
fn corp_override_api_key() {
    let user = file_with(vec![(
        SETTING_GITHUB_TOKEN,
        SettingValue::Text("credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into()),
    )]);
    let corp = file_with(vec![(
        SETTING_GITHUB_TOKEN,
        SettingValue::Text("credential:blake3:2222222222222222222222222222222222222222222222222222222222222222".into()),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved.iter().find(|s| s.id == SETTING_GITHUB_TOKEN).unwrap();
    assert_eq!(
        s.effective_value,
        SettingValue::Text("credential:blake3:2222222222222222222222222222222222222222222222222222222222222222".into())
    );
    assert_eq!(s.source, PolicySource::Corp);
}

#[test]
fn corp_override_guest_env() {
    let user = file_with(vec![("guest.env.EDITOR", SettingValue::Text("vim".into()))]);
    let corp = file_with(vec![("guest.env.EDITOR", SettingValue::Text("nano".into()))]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved.iter().find(|s| s.id == "guest.env.EDITOR").unwrap();
    assert_eq!(s.effective_value, SettingValue::Text("nano".into()));
    assert_eq!(s.source, PolicySource::Corp);
}

#[test]
fn corp_override_mixed_categories() {
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        ("vm.resources.log_bodies", SettingValue::Bool(true)),
        ("appearance.dark_mode", SettingValue::Bool(false)),
    ]);
    let corp = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(false)),
        ("vm.resources.log_bodies", SettingValue::Bool(false)),
    ]);
    let resolved = resolve_settings(&user, &corp);

    let repo = resolved.iter().find(|s| s.id == SETTING_GITHUB_ALLOW).unwrap();
    assert_eq!(repo.effective_value, SettingValue::Bool(false));
    assert_eq!(repo.source, PolicySource::Corp);

    let log = resolved.iter().find(|s| s.id == "vm.resources.log_bodies").unwrap();
    assert_eq!(log.effective_value, SettingValue::Bool(false));
    assert_eq!(log.source, PolicySource::Corp);

    // appearance.dark_mode not in corp -> user value
    let dark = resolved.iter().find(|s| s.id == "appearance.dark_mode").unwrap();
    assert_eq!(dark.effective_value, SettingValue::Bool(false));
    assert_eq!(dark.source, PolicySource::User);
}

#[test]
fn corp_overrides_all_registry_and_repository_toggles() {
    let corp = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(false)),
        (SETTING_GITLAB_ALLOW, SettingValue::Bool(false)),
        ("security.services.registry.npm.allow", SettingValue::Bool(false)),
        ("security.services.registry.pypi.allow", SettingValue::Bool(false)),
        ("security.services.registry.crates.allow", SettingValue::Bool(false)),
        ("security.services.registry.debian.allow", SettingValue::Bool(false)),
    ]);
    let resolved = resolve_settings(&empty_file(), &corp);
    for s in &resolved {
        let is_registry_toggle = s.id.starts_with("security.services.registry.") && s.id.ends_with(".allow");
        let is_repo_toggle = s.id == SETTING_GITHUB_ALLOW || s.id == SETTING_GITLAB_ALLOW;
        if is_registry_toggle || is_repo_toggle {
            assert_eq!(s.effective_value, SettingValue::Bool(false), "failed for {}", s.id);
            assert_eq!(s.source, PolicySource::Corp);
        }
    }
}

// -----------------------------------------------------------------------
// B: User cannot expand (3)
// -----------------------------------------------------------------------

#[test]
fn user_cannot_enable_blocked_provider() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let corp = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved.iter().find(|s| s.id == SETTING_GITHUB_ALLOW).unwrap();
    assert_eq!(s.effective_value, SettingValue::Bool(false));
    assert!(s.corp_locked);
}

#[test]
fn user_cannot_change_corp_network_mechanics_ports() {
    let user = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80, 3128, 3713, 8080, 11434]),
    )]);
    let corp = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80]),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved
        .iter()
        .find(|s| s.id == "security.web.http_upstream_ports")
        .unwrap();
    assert_eq!(s.effective_value, SettingValue::IntList(vec![80]));
    assert!(s.corp_locked);
}

#[test]
fn user_cannot_override_corp_api_key() {
    let user = file_with(vec![(
        SETTING_GITHUB_TOKEN,
        SettingValue::Text("credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into()),
    )]);
    let corp = file_with(vec![(
        SETTING_GITHUB_TOKEN,
        SettingValue::Text("credential:blake3:2222222222222222222222222222222222222222222222222222222222222222".into()),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved.iter().find(|s| s.id == SETTING_GITHUB_TOKEN).unwrap();
    assert_eq!(
        s.effective_value,
        SettingValue::Text("credential:blake3:2222222222222222222222222222222222222222222222222222222222222222".into())
    );
    assert!(s.corp_locked);
}

// -----------------------------------------------------------------------
// C: User isolation (4)
// -----------------------------------------------------------------------

#[test]
fn can_write_corp_is_always_false() {
    assert!(!can_write_corp_settings());
}

#[test]
fn write_local_settings_creates_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_settings.toml");
    let file = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(true))]);
    write_settings_file(&path, &file).unwrap();
    assert!(path.exists());
}

#[test]
fn write_local_settings_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roundtrip.toml");
    let file = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        ("vm.resources.max_body_capture", SettingValue::Number(8192)),
        ("guest.env.EDITOR", SettingValue::Text("vim".into())),
    ]);
    write_settings_file(&path, &file).unwrap();
    let loaded = load_settings_file(&path).unwrap();
    assert_eq!(file.settings.len(), loaded.settings.len());
    for (key, entry) in &file.settings {
        let loaded_entry = loaded.settings.get(key).unwrap();
        assert_eq!(entry.value, loaded_entry.value, "mismatch for {key}");
    }
}

#[test]
fn write_local_settings_preserves_other_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preserve.toml");
    let mut file = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        ("vm.resources.log_bodies", SettingValue::Bool(false)),
    ]);
    write_settings_file(&path, &file).unwrap();

    // Update one setting
    file.settings.get_mut("vm.resources.log_bodies").unwrap().value = SettingValue::Bool(true);
    write_settings_file(&path, &file).unwrap();

    let loaded = load_settings_file(&path).unwrap();
    assert_eq!(
        loaded.settings.get(SETTING_GITHUB_ALLOW).unwrap().value,
        SettingValue::Bool(true),
    );
    assert_eq!(
        loaded.settings.get("vm.resources.log_bodies").unwrap().value,
        SettingValue::Bool(true),
    );
}

// -----------------------------------------------------------------------
// D: Defaults (5)
// -----------------------------------------------------------------------

#[test]
fn default_settings_file_is_empty() {
    let file = default_settings_file();
    assert!(file.settings.is_empty());
}

#[test]
fn default_resolve_has_all_definitions() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let defs = setting_definitions();
    for def in &defs {
        assert!(
            resolved.iter().any(|s| s.id == def.id),
            "missing definition: {}",
            def.id,
        );
    }
}

#[test]
fn default_ai_providers_all_enabled() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    for id in &["ai.anthropic.allow", "ai.openai.allow", "ai.google.allow"] {
        assert_eq!(
            resolved.iter().find(|s| s.id == *id),
            None,
            "{id} must not be a settings-owned provider toggle"
        );
    }
}

#[test]
fn default_registries_allowed() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    for id in &[
        SETTING_GITHUB_ALLOW,
        "security.services.registry.npm.allow",
        "security.services.registry.pypi.allow",
        "security.services.registry.crates.allow",
    ] {
        let s = resolved.iter().find(|s| s.id == *id).unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(true), "expected {id} to be true");
    }
}

#[test]
fn default_web_session_appearance() {
    let resolved = resolve_settings(&empty_file(), &empty_file());

    let ports = resolved
        .iter()
        .find(|s| s.id == "security.web.http_upstream_ports")
        .unwrap();
    assert_eq!(
        ports.effective_value,
        SettingValue::IntList(vec![80, 3128, 3713, 8080, 11434])
    );

    let lb = resolved.iter().find(|s| s.id == "vm.resources.log_bodies").unwrap();
    assert_eq!(lb.effective_value, SettingValue::Bool(false));

    let mbc = resolved
        .iter()
        .find(|s| s.id == "vm.resources.max_body_capture")
        .unwrap();
    assert_eq!(mbc.effective_value, SettingValue::Number(4096));

    let rd = resolved.iter().find(|s| s.id == "vm.resources.retention_days").unwrap();
    assert_eq!(rd.effective_value, SettingValue::Number(30));

    let dm = resolved.iter().find(|s| s.id == "appearance.dark_mode").unwrap();
    assert_eq!(dm.effective_value, SettingValue::Bool(true));

    let fs = resolved.iter().find(|s| s.id == "appearance.font_size").unwrap();
    assert_eq!(fs.effective_value, SettingValue::Number(14));
}

// -----------------------------------------------------------------------
// E: Definitions (4)
// -----------------------------------------------------------------------

#[test]
fn definitions_have_unique_ids() {
    let defs = setting_definitions();
    let mut ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    let original_len = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), original_len, "duplicate setting IDs found");
}

#[test]
fn definitions_have_nonempty_descriptions() {
    for def in setting_definitions() {
        assert!(!def.description.is_empty(), "empty description for {}", def.id);
        assert!(!def.name.is_empty(), "empty name for {}", def.id);
    }
}

#[test]
fn registry_toggles_have_domain_metadata() {
    let defs = setting_definitions();
    for def in &defs {
        if def.id.starts_with("security.services.registry.") && def.id.ends_with(".allow") {
            assert!(
                !def.metadata.domains.is_empty(),
                "toggle {} has no domain metadata",
                def.id,
            );
        }
    }
}

#[test]
fn ai_providers_have_domains_settings() {
    let defs = setting_definitions();
    for prefix in &["ai.anthropic", "ai.openai", "ai.google"] {
        let domains_id = format!("{prefix}.domains");
        let def = defs.iter().find(|d| d.id == domains_id);
        assert!(
            def.is_none(),
            "{domains_id} must not be a settings-owned provider domain setting"
        );
    }
}

#[test]
fn web_mechanics_ports_are_int_list_setting() {
    let defs = setting_definitions();
    let ports = defs
        .iter()
        .find(|d| d.id == "security.web.http_upstream_ports")
        .unwrap();
    assert_eq!(ports.setting_type, SettingType::IntList);
}

// -----------------------------------------------------------------------
// F: Source tracking (6)
// -----------------------------------------------------------------------

#[test]
fn source_default() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let s = resolved.iter().find(|s| s.id == "vm.resources.log_bodies").unwrap();
    assert_eq!(s.source, PolicySource::Default);
    assert!(s.modified.is_none());
}

#[test]
fn source_user() {
    let user = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let s = resolved.iter().find(|s| s.id == "vm.resources.log_bodies").unwrap();
    assert_eq!(s.source, PolicySource::User);
    assert!(s.modified.is_some());
}

#[test]
fn source_corp() {
    let corp = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(true))]);
    let resolved = resolve_settings(&empty_file(), &corp);
    let s = resolved.iter().find(|s| s.id == "vm.resources.log_bodies").unwrap();
    assert_eq!(s.source, PolicySource::Corp);
    assert!(s.modified.is_some());
}

#[test]
fn source_corp_beats_user() {
    let user = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(true))]);
    let corp = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved.iter().find(|s| s.id == "vm.resources.log_bodies").unwrap();
    assert_eq!(s.source, PolicySource::Corp);
    assert_eq!(s.effective_value, SettingValue::Bool(false));
}

#[test]
fn source_dynamic_guest_env() {
    let user = file_with(vec![("guest.env.FOO", SettingValue::Text("bar".into()))]);
    let resolved = resolve_settings(&user, &empty_file());
    let s = resolved.iter().find(|s| s.id == "guest.env.FOO").unwrap();
    assert_eq!(s.source, PolicySource::User);
    assert_eq!(s.category, "VM");
}

#[test]
fn is_setting_corp_locked_test() {
    let corp = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    assert!(is_setting_corp_locked(SETTING_GITHUB_ALLOW, &corp));
    assert!(!is_setting_corp_locked(SETTING_GITLAB_ALLOW, &corp));
}

// -----------------------------------------------------------------------
// G: enabled_by (4)
// -----------------------------------------------------------------------

#[test]
fn enabled_by_parent_on_child_enabled() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let child = resolved.iter().find(|s| s.id == SETTING_GITHUB_TOKEN).unwrap();
    assert!(child.enabled);
    assert_eq!(child.enabled_by, Some(SETTING_GITHUB_ALLOW.to_string()));
}

#[test]
fn enabled_by_parent_off_child_disabled() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &empty_file());
    let child = resolved.iter().find(|s| s.id == SETTING_GITHUB_TOKEN).unwrap();
    assert!(!child.enabled);
}

#[test]
fn enabled_by_none_always_enabled() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let s = resolved.iter().find(|s| s.id == "vm.resources.log_bodies").unwrap();
    assert!(s.enabled);
    assert!(s.enabled_by.is_none());
}

#[test]
fn enabled_by_chain_not_supported() {
    let mut user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &empty_file());
    let key = resolved.iter().find(|s| s.id == SETTING_GITHUB_TOKEN).unwrap();
    assert!(!key.enabled);

    // Turn on the toggle -> key is enabled
    user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let key = resolved.iter().find(|s| s.id == SETTING_GITHUB_TOKEN).unwrap();
    assert!(key.enabled);
}

#[test]
fn settings_to_guest_config_from_dynamic() {
    let user = file_with(vec![
        ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ("guest.env.TERM", SettingValue::Text("xterm".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert_eq!(env.get("EDITOR").unwrap(), "vim");
    assert_eq!(env.get("TERM").unwrap(), "xterm");
}

// -----------------------------------------------------------------------
// I: Roundtrip + edge cases (4)
// -----------------------------------------------------------------------

#[test]
fn settings_file_toml_roundtrip() {
    let file = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(true)),
        ("vm.resources.max_body_capture", SettingValue::Number(8192)),
        ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        (
            "ai.google.gemini.settings_json",
            SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: r#"{"key":"value"}"#.into(),
            },
        ),
    ]);
    let toml_str = toml::to_string_pretty(&file).unwrap();
    let parsed: SettingsFile = toml::from_str(&toml_str).unwrap();
    assert_eq!(file.settings.len(), parsed.settings.len());
    for (key, entry) in &file.settings {
        assert_eq!(&entry.value, &parsed.settings[key].value, "mismatch for {key}");
    }
}

#[test]
fn settings_file_disk_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk_roundtrip.toml");
    let file = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        ("appearance.font_size", SettingValue::Number(16)),
    ]);
    write_settings_file(&path, &file).unwrap();
    let loaded = load_settings_file(&path).unwrap();
    assert_eq!(file, loaded);
}

#[test]
fn empty_files_use_defaults() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    for s in &resolved {
        assert_eq!(s.source, PolicySource::Default, "non-default source for {}", s.id);
    }
}

#[test]
fn invalid_toml_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "{{{{not valid").unwrap();
    let result = load_settings_file(&path);
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// TOML parsing from raw strings (M)
// -----------------------------------------------------------------------

#[test]
fn parse_real_user_toml_format() {
    // This is the exact format a real settings.toml has on disk.
    let toml_str = r#"
[settings]
"ai.google.api_key" = { value = "AIzaSyTest1234", modified = "2026-02-25T00:00:00Z" }
"ai.anthropic.allow" = { value = true, modified = "2026-02-25T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-ant-test-key", modified = "2026-02-25T00:00:00Z" }
"#;
    let file: SettingsFile = toml::from_str(toml_str).expect("should parse real settings.toml format");
    assert_eq!(file.settings.len(), 3);
    assert_eq!(
        file.settings["ai.google.api_key"].value,
        SettingValue::Text("AIzaSyTest1234".into()),
    );
    assert_eq!(file.settings["ai.anthropic.allow"].value, SettingValue::Bool(true),);
    assert_eq!(
        file.settings["ai.anthropic.api_key"].value,
        SettingValue::Text("sk-ant-test-key".into()),
    );
}

#[test]
fn parse_toml_mixed_value_types() {
    let toml_str = r#"
[settings]
"vm.resources.log_bodies" = { value = true, modified = "2026-01-01T00:00:00Z" }
"vm.resources.max_body_capture" = { value = 8192, modified = "2026-01-01T00:00:00Z" }
"security.web.http_upstream_ports" = { value = [80, 3128, 3713, 8080, 11434], modified = "2026-01-01T00:00:00Z" }
"appearance.font_size" = { value = 16, modified = "2026-01-01T00:00:00Z" }
"#;
    let file: SettingsFile = toml::from_str(toml_str).expect("should parse mixed types");
    assert_eq!(file.settings["vm.resources.log_bodies"].value, SettingValue::Bool(true));
    assert_eq!(
        file.settings["vm.resources.max_body_capture"].value,
        SettingValue::Number(8192)
    );
    assert_eq!(
        file.settings["security.web.http_upstream_ports"].value,
        SettingValue::IntList(vec![80, 3128, 3713, 8080, 11434])
    );
    assert_eq!(file.settings["appearance.font_size"].value, SettingValue::Number(16));
}

#[test]
fn parse_toml_empty_settings_table() {
    let toml_str = "[settings]\n";
    let file: SettingsFile = toml::from_str(toml_str).expect("should parse empty table");
    assert!(file.settings.is_empty());
}

#[test]
fn parse_toml_completely_empty() {
    let file: SettingsFile = toml::from_str("").expect("should parse empty string");
    assert!(file.settings.is_empty());
}

#[test]
fn parse_toml_missing_modified_fails() {
    // SettingEntry requires both value and modified
    let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true }
"#;
    let result: Result<SettingsFile, _> = toml::from_str(toml_str);
    assert!(result.is_err(), "missing 'modified' field should fail");
}

#[test]
fn parse_toml_missing_value_fails() {
    let toml_str = r#"
[settings]
"ai.anthropic.allow" = { modified = "2026-01-01T00:00:00Z" }
"#;
    let result: Result<SettingsFile, _> = toml::from_str(toml_str);
    assert!(result.is_err(), "missing 'value' field should fail");
}

#[test]
fn parse_toml_extra_fields_ignored() {
    // TOML with extra unknown fields in the entry should still parse
    // (serde default behavior: ignore unknown fields)
    let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z", extra = "ignored" }
"#;
    let result: Result<SettingsFile, _> = toml::from_str(toml_str);
    // By default serde does NOT deny unknown fields, so this should succeed.
    // If it fails, SettingEntry is using deny_unknown_fields.
    assert!(result.is_ok(), "extra fields should be ignored: {:?}", result.err());
}

#[test]
fn parse_toml_wrong_value_type_fails() {
    // value is a nested table that doesn't match any SettingValue variant
    let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = { nested = { deep = true } }, modified = "2026-01-01T00:00:00Z" }
"#;
    let result: Result<SettingsFile, _> = toml::from_str(toml_str);
    assert!(result.is_err(), "nested table value should fail deserialization");
}

#[test]
fn parse_toml_list_values() {
    // Lists are now valid SettingValue variants.
    let toml_str = r#"
[settings]
"domains" = { value = ["a.com", "b.com"], modified = "2026-01-01T00:00:00Z" }
"counts" = { value = [1, 2, 3], modified = "2026-01-01T00:00:00Z" }
"#;
    let file: SettingsFile = toml::from_str(toml_str).unwrap();
    assert_eq!(
        file.settings["domains"].value,
        SettingValue::StringList(vec!["a.com".into(), "b.com".into()])
    );
    assert_eq!(file.settings["counts"].value, SettingValue::IntList(vec![1, 2, 3]));
}

#[test]
fn parse_toml_unquoted_dotted_keys() {
    // In TOML, unquoted dotted keys create nested tables, not flat keys.
    // This is a common mistake: ai.anthropic.allow = { ... } creates
    // [ai] -> [anthropic] -> allow = { ... }, NOT a flat key "ai.anthropic.allow".
    let toml_str = r#"
[settings]
ai.anthropic.allow = { value = true, modified = "2026-01-01T00:00:00Z" }
"#;
    let result: Result<SettingsFile, _> = toml::from_str(toml_str);
    // This should fail because the nested table structure does not match
    // HashMap<String, SettingEntry>.
    assert!(
        result.is_err(),
        "unquoted dotted keys should fail (creates nested tables)"
    );
}

#[test]
fn parse_toml_guest_env_keys() {
    let toml_str = r#"
[settings]
"guest.env.EDITOR" = { value = "vim", modified = "2026-01-01T00:00:00Z" }
"guest.env.TERM" = { value = "xterm-256color", modified = "2026-01-01T00:00:00Z" }
"#;
    let file: SettingsFile = toml::from_str(toml_str).expect("should parse guest env");
    assert_eq!(file.settings.len(), 2);
    assert_eq!(
        file.settings["guest.env.EDITOR"].value,
        SettingValue::Text("vim".into()),
    );
}

#[test]
fn parse_toml_api_key_with_special_chars() {
    // API keys often have dashes, underscores, and mixed case
    let toml_str = r#"
[settings]
"ai.anthropic.api_key" = { value = "sk-ant-api03-ABCD_1234-efgh-5678", modified = "2026-01-01T00:00:00Z" }
"#;
    let file: SettingsFile = toml::from_str(toml_str).expect("should parse API key with special chars");
    assert_eq!(
        file.settings["ai.anthropic.api_key"].value,
        SettingValue::Text("sk-ant-api03-ABCD_1234-efgh-5678".into()),
    );
}

#[test]
fn parse_toml_resolves_with_api_key_type() {
    // Parse from raw TOML, then resolve -- token settings must have
    // setting_type == ApiKey, not Text.
    let toml_str = r#"
[settings]
"repository.providers.github.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.token" = { value = "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111", modified = "2026-01-01T00:00:00Z" }
"#;
    let user: SettingsFile = toml::from_str(toml_str).unwrap();
    let resolved = resolve_settings(&user, &empty_file());
    let s = resolved.iter().find(|s| s.id == SETTING_GITHUB_TOKEN).unwrap();
    assert_eq!(
        s.setting_type,
        SettingType::ApiKey,
        "token settings must have ApiKey type"
    );
    assert_eq!(
        s.effective_value,
        SettingValue::Text("credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into())
    );
}

#[test]
fn parse_toml_serialized_format_roundtrips() {
    // Verify that toml::to_string_pretty output parses back correctly
    let file = file_with(vec![
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into(),
            ),
        ),
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        ("vm.resources.max_body_capture", SettingValue::Number(4096)),
    ]);
    let serialized = toml::to_string_pretty(&file).unwrap();
    let parsed: SettingsFile = toml::from_str(&serialized)
        .unwrap_or_else(|e| panic!("failed to re-parse serialized TOML:\n{serialized}\nerror: {e}"));
    assert_eq!(file.settings.len(), parsed.settings.len());
    for (key, entry) in &file.settings {
        assert_eq!(&entry.value, &parsed.settings[key].value, "mismatch for {key}");
    }
}

#[test]
fn json_metadata_fields_present_when_empty() {
    // SettingMetadata uses skip_serializing_if = "Vec::is_empty" etc.
    // If empty fields are omitted from JSON, the JS frontend will crash
    // because it accesses metadata.choices.length (undefined.length -> TypeError).
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let json = serde_json::to_string(&resolved).unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();

    // Find a setting with sparse metadata (e.g., a token setting)
    let api_key = parsed.iter().find(|v| v["id"] == SETTING_GITHUB_TOKEN).unwrap();
    let meta = &api_key["metadata"];

    // These fields MUST be present in JSON (even when empty) or the
    // frontend will crash with undefined.length errors.
    assert!(
        meta.get("choices").is_some(),
        "metadata.choices must be present in JSON (got: {meta})"
    );
    assert!(
        meta.get("domains").is_some(),
        "metadata.domains must be present in JSON (got: {meta})"
    );
}

#[test]
fn resolved_settings_json_serialization() {
    // Tauri sends settings as JSON to the frontend. Verify the full
    // pipeline: parse TOML -> resolve -> serialize to JSON -> has setting_type.
    let toml_str = r#"
[settings]
"repository.providers.github.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.token" = { value = "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111", modified = "2026-01-01T00:00:00Z" }
"#;
    let user: SettingsFile = toml::from_str(toml_str).unwrap();
    let resolved = resolve_settings(&user, &empty_file());
    let json = serde_json::to_string(&resolved).expect("should serialize to JSON");

    // Verify key fields are present in the JSON
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = parsed.as_array().unwrap();

    // Find the token setting
    let api_key = arr
        .iter()
        .find(|v| v["id"] == SETTING_GITHUB_TOKEN)
        .expect("should have repository.providers.github.token in JSON");
    assert_eq!(
        api_key["setting_type"], "apikey",
        "setting_type must be 'apikey' in JSON"
    );
    assert_eq!(
        api_key["effective_value"],
        "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
    );
    assert_eq!(api_key["enabled"], true);

    // Find a bool setting
    let allow = arr
        .iter()
        .find(|v| v["id"] == SETTING_GITHUB_ALLOW)
        .expect("should have repository.providers.github.allow in JSON");
    assert_eq!(allow["setting_type"], "bool");
    assert_eq!(allow["effective_value"], true);

    // Verify all settings have a setting_type field
    for item in arr {
        assert!(
            item.get("setting_type").is_some(),
            "setting {} missing setting_type in JSON",
            item["id"],
        );
    }
}

#[test]
fn load_settings_file_missing_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent.toml");
    let file = load_settings_file(&path).unwrap();
    assert!(file.settings.is_empty());
}

#[test]
fn load_settings_file_garbage_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("garbage.toml");
    std::fs::write(&path, "not = [valid { toml }").unwrap();
    assert!(load_settings_file(&path).is_err());
}

#[test]
fn load_settings_file_wrong_schema_returns_error() {
    // Valid TOML but wrong structure (settings is a string, not a table)
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wrong_schema.toml");
    std::fs::write(&path, "settings = \"not a table\"").unwrap();
    assert!(load_settings_file(&path).is_err());
}

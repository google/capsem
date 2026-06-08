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

// -----------------------------------------------------------------------
// A: Corp override (7)
// -----------------------------------------------------------------------

#[test]
fn corp_override_bool() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let corp = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_ALLOW)
        .unwrap();
    assert_eq!(s.effective_value, SettingValue::Bool(false));
    assert_eq!(s.source, PolicySource::Corp);
}

#[test]
fn corp_override_network_mechanics_ports() {
    let user = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80, 11434]),
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
    let user = file_with(vec![(
        "vm.resources.max_body_capture",
        SettingValue::Number(8192),
    )]);
    let corp = file_with(vec![(
        "vm.resources.max_body_capture",
        SettingValue::Number(1024),
    )]);
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
        SettingValue::Text(
            "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                .into(),
        ),
    )]);
    let corp = file_with(vec![(
        SETTING_GITHUB_TOKEN,
        SettingValue::Text(
            "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222"
                .into(),
        ),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_TOKEN)
        .unwrap();
    assert_eq!(
        s.effective_value,
        SettingValue::Text(
            "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222"
                .into()
        )
    );
    assert_eq!(s.source, PolicySource::Corp);
}

#[test]
fn corp_override_guest_env() {
    let user = file_with(vec![("guest.env.EDITOR", SettingValue::Text("vim".into()))]);
    let corp = file_with(vec![(
        "guest.env.EDITOR",
        SettingValue::Text("nano".into()),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved
        .iter()
        .find(|s| s.id == "guest.env.EDITOR")
        .unwrap();
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

    let repo = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_ALLOW)
        .unwrap();
    assert_eq!(repo.effective_value, SettingValue::Bool(false));
    assert_eq!(repo.source, PolicySource::Corp);

    let log = resolved
        .iter()
        .find(|s| s.id == "vm.resources.log_bodies")
        .unwrap();
    assert_eq!(log.effective_value, SettingValue::Bool(false));
    assert_eq!(log.source, PolicySource::Corp);

    // appearance.dark_mode not in corp -> user value
    let dark = resolved
        .iter()
        .find(|s| s.id == "appearance.dark_mode")
        .unwrap();
    assert_eq!(dark.effective_value, SettingValue::Bool(false));
    assert_eq!(dark.source, PolicySource::User);
}

#[test]
fn corp_overrides_all_registry_and_repository_toggles() {
    let corp = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(false)),
        (SETTING_GITLAB_ALLOW, SettingValue::Bool(false)),
        (
            "security.services.registry.npm.allow",
            SettingValue::Bool(false),
        ),
        (
            "security.services.registry.pypi.allow",
            SettingValue::Bool(false),
        ),
        (
            "security.services.registry.crates.allow",
            SettingValue::Bool(false),
        ),
        (
            "security.services.registry.debian.allow",
            SettingValue::Bool(false),
        ),
    ]);
    let resolved = resolve_settings(&empty_file(), &corp);
    for s in &resolved {
        let is_registry_toggle =
            s.id.starts_with("security.services.registry.") && s.id.ends_with(".allow");
        let is_repo_toggle = s.id == SETTING_GITHUB_ALLOW || s.id == SETTING_GITLAB_ALLOW;
        if is_registry_toggle || is_repo_toggle {
            assert_eq!(
                s.effective_value,
                SettingValue::Bool(false),
                "failed for {}",
                s.id
            );
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
    let s = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_ALLOW)
        .unwrap();
    assert_eq!(s.effective_value, SettingValue::Bool(false));
    assert!(s.corp_locked);
}

#[test]
fn user_cannot_change_corp_network_mechanics_ports() {
    let user = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80, 11434]),
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
        SettingValue::Text(
            "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                .into(),
        ),
    )]);
    let corp = file_with(vec![(
        SETTING_GITHUB_TOKEN,
        SettingValue::Text(
            "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222"
                .into(),
        ),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_TOKEN)
        .unwrap();
    assert_eq!(
        s.effective_value,
        SettingValue::Text(
            "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222"
                .into()
        )
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
fn write_user_settings_creates_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_user.toml");
    let file = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(true))]);
    write_settings_file(&path, &file).unwrap();
    assert!(path.exists());
}

#[test]
fn write_user_settings_roundtrip() {
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
fn write_user_settings_preserves_other_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preserve.toml");
    let mut file = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        ("vm.resources.log_bodies", SettingValue::Bool(false)),
    ]);
    write_settings_file(&path, &file).unwrap();

    // Update one setting
    file.settings
        .get_mut("vm.resources.log_bodies")
        .unwrap()
        .value = SettingValue::Bool(true);
    write_settings_file(&path, &file).unwrap();

    let loaded = load_settings_file(&path).unwrap();
    assert_eq!(
        loaded.settings.get(SETTING_GITHUB_ALLOW).unwrap().value,
        SettingValue::Bool(true),
    );
    assert_eq!(
        loaded
            .settings
            .get("vm.resources.log_bodies")
            .unwrap()
            .value,
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
        assert_eq!(
            s.effective_value,
            SettingValue::Bool(true),
            "expected {id} to be true"
        );
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
        SettingValue::IntList(vec![80, 11434])
    );

    let lb = resolved
        .iter()
        .find(|s| s.id == "vm.resources.log_bodies")
        .unwrap();
    assert_eq!(lb.effective_value, SettingValue::Bool(false));

    let mbc = resolved
        .iter()
        .find(|s| s.id == "vm.resources.max_body_capture")
        .unwrap();
    assert_eq!(mbc.effective_value, SettingValue::Number(4096));

    let rd = resolved
        .iter()
        .find(|s| s.id == "vm.resources.retention_days")
        .unwrap();
    assert_eq!(rd.effective_value, SettingValue::Number(30));

    let dm = resolved
        .iter()
        .find(|s| s.id == "appearance.dark_mode")
        .unwrap();
    assert_eq!(dm.effective_value, SettingValue::Bool(true));

    let fs = resolved
        .iter()
        .find(|s| s.id == "appearance.font_size")
        .unwrap();
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
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), original_len, "duplicate setting IDs found");
}

#[test]
fn definitions_have_nonempty_descriptions() {
    for def in setting_definitions() {
        assert!(
            !def.description.is_empty(),
            "empty description for {}",
            def.id
        );
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
    let s = resolved
        .iter()
        .find(|s| s.id == "vm.resources.log_bodies")
        .unwrap();
    assert_eq!(s.source, PolicySource::Default);
    assert!(s.modified.is_none());
}

#[test]
fn source_user() {
    let user = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let s = resolved
        .iter()
        .find(|s| s.id == "vm.resources.log_bodies")
        .unwrap();
    assert_eq!(s.source, PolicySource::User);
    assert!(s.modified.is_some());
}

#[test]
fn source_corp() {
    let corp = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(true))]);
    let resolved = resolve_settings(&empty_file(), &corp);
    let s = resolved
        .iter()
        .find(|s| s.id == "vm.resources.log_bodies")
        .unwrap();
    assert_eq!(s.source, PolicySource::Corp);
    assert!(s.modified.is_some());
}

#[test]
fn source_corp_beats_user() {
    let user = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(true))]);
    let corp = file_with(vec![("vm.resources.log_bodies", SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &corp);
    let s = resolved
        .iter()
        .find(|s| s.id == "vm.resources.log_bodies")
        .unwrap();
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
    let child = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_TOKEN)
        .unwrap();
    assert!(child.enabled);
    assert_eq!(child.enabled_by, Some(SETTING_GITHUB_ALLOW.to_string()));
}

#[test]
fn enabled_by_parent_off_child_disabled() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &empty_file());
    let child = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_TOKEN)
        .unwrap();
    assert!(!child.enabled);
}

#[test]
fn enabled_by_none_always_enabled() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let s = resolved
        .iter()
        .find(|s| s.id == "vm.resources.log_bodies")
        .unwrap();
    assert!(s.enabled);
    assert!(s.enabled_by.is_none());
}

#[test]
fn enabled_by_chain_not_supported() {
    let mut user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &empty_file());
    let key = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_TOKEN)
        .unwrap();
    assert!(!key.enabled);

    // Turn on the toggle -> key is enabled
    user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let key = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_TOKEN)
        .unwrap();
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
        assert_eq!(
            &entry.value, &parsed.settings[key].value,
            "mismatch for {key}"
        );
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
        assert_eq!(
            s.source,
            PolicySource::Default,
            "non-default source for {}",
            s.id
        );
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
    // This is the exact format a real user.toml has on disk.
    let toml_str = r#"
[settings]
"ai.google.api_key" = { value = "AIzaSyTest1234", modified = "2026-02-25T00:00:00Z" }
"ai.anthropic.allow" = { value = true, modified = "2026-02-25T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-ant-test-key", modified = "2026-02-25T00:00:00Z" }
"#;
    let file: SettingsFile = toml::from_str(toml_str).expect("should parse real user.toml format");
    assert_eq!(file.settings.len(), 3);
    assert_eq!(
        file.settings["ai.google.api_key"].value,
        SettingValue::Text("AIzaSyTest1234".into()),
    );
    assert_eq!(
        file.settings["ai.anthropic.allow"].value,
        SettingValue::Bool(true),
    );
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
"security.web.http_upstream_ports" = { value = [80, 11434], modified = "2026-01-01T00:00:00Z" }
"appearance.font_size" = { value = 16, modified = "2026-01-01T00:00:00Z" }
"#;
    let file: SettingsFile = toml::from_str(toml_str).expect("should parse mixed types");
    assert_eq!(
        file.settings["vm.resources.log_bodies"].value,
        SettingValue::Bool(true)
    );
    assert_eq!(
        file.settings["vm.resources.max_body_capture"].value,
        SettingValue::Number(8192)
    );
    assert_eq!(
        file.settings["security.web.http_upstream_ports"].value,
        SettingValue::IntList(vec![80, 11434])
    );
    assert_eq!(
        file.settings["appearance.font_size"].value,
        SettingValue::Number(16)
    );
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
    assert!(
        result.is_ok(),
        "extra fields should be ignored: {:?}",
        result.err()
    );
}

#[test]
fn parse_toml_wrong_value_type_fails() {
    // value is a nested table that doesn't match any SettingValue variant
    let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = { nested = { deep = true } }, modified = "2026-01-01T00:00:00Z" }
"#;
    let result: Result<SettingsFile, _> = toml::from_str(toml_str);
    assert!(
        result.is_err(),
        "nested table value should fail deserialization"
    );
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
    assert_eq!(
        file.settings["counts"].value,
        SettingValue::IntList(vec![1, 2, 3])
    );
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
    let file: SettingsFile =
        toml::from_str(toml_str).expect("should parse API key with special chars");
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
    let s = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_TOKEN)
        .unwrap();
    assert_eq!(
        s.setting_type,
        SettingType::ApiKey,
        "token settings must have ApiKey type"
    );
    assert_eq!(
        s.effective_value,
        SettingValue::Text(
            "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                .into()
        )
    );
}

#[test]
fn parse_toml_serialized_format_roundtrips() {
    // Verify that toml::to_string_pretty output parses back correctly
    let file = file_with(vec![
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                    .into(),
            ),
        ),
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        ("vm.resources.max_body_capture", SettingValue::Number(4096)),
    ]);
    let serialized = toml::to_string_pretty(&file).unwrap();
    let parsed: SettingsFile = toml::from_str(&serialized).unwrap_or_else(|e| {
        panic!("failed to re-parse serialized TOML:\n{serialized}\nerror: {e}")
    });
    assert_eq!(file.settings.len(), parsed.settings.len());
    for (key, entry) in &file.settings {
        assert_eq!(
            &entry.value, &parsed.settings[key].value,
            "mismatch for {key}"
        );
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
    let api_key = parsed
        .iter()
        .find(|v| v["id"] == SETTING_GITHUB_TOKEN)
        .unwrap();
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

// -----------------------------------------------------------------------
// VM settings
// -----------------------------------------------------------------------

#[test]
fn vm_settings_default_cpu_count() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let vs = settings_to_vm_settings(&resolved);
    assert_eq!(vs.cpu_count, Some(4));
}

#[test]
fn vm_settings_default_scratch_size() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let vs = settings_to_vm_settings(&resolved);
    assert_eq!(vs.scratch_disk_size_gb, Some(16));
}

#[test]
fn vm_settings_default_ram() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let vs = settings_to_vm_settings(&resolved);
    assert_eq!(vs.ram_gb, Some(4));
}

#[test]
fn vm_settings_from_user() {
    let user = file_with(vec![(
        "vm.resources.scratch_disk_size_gb",
        SettingValue::Number(32),
    )]);
    let resolved = resolve_settings(&user, &empty_file());
    let vs = settings_to_vm_settings(&resolved);
    assert_eq!(vs.scratch_disk_size_gb, Some(32));
}

#[test]
fn vm_settings_ram_from_user() {
    let user = file_with(vec![("vm.resources.ram_gb", SettingValue::Number(8))]);
    let resolved = resolve_settings(&user, &empty_file());
    let vs = settings_to_vm_settings(&resolved);
    assert_eq!(vs.ram_gb, Some(8));
}

#[test]
fn vm_settings_corp_overrides_user() {
    let user = file_with(vec![(
        "vm.resources.scratch_disk_size_gb",
        SettingValue::Number(32),
    )]);
    let corp = file_with(vec![(
        "vm.resources.scratch_disk_size_gb",
        SettingValue::Number(4),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let vs = settings_to_vm_settings(&resolved);
    assert_eq!(vs.scratch_disk_size_gb, Some(4));
}

#[test]
fn vm_settings_ram_corp_overrides_user() {
    let user = file_with(vec![("vm.resources.ram_gb", SettingValue::Number(8))]);
    let corp = file_with(vec![("vm.resources.ram_gb", SettingValue::Number(2))]);
    let resolved = resolve_settings(&user, &corp);
    let vs = settings_to_vm_settings(&resolved);
    assert_eq!(vs.ram_gb, Some(2));
}

#[test]
fn vm_settings_cpu_from_user() {
    let user = file_with(vec![("vm.resources.cpu_count", SettingValue::Number(2))]);
    let resolved = resolve_settings(&user, &empty_file());
    let vs = settings_to_vm_settings(&resolved);
    assert_eq!(vs.cpu_count, Some(2));
}

#[test]
fn vm_settings_cpu_corp_overrides_user() {
    let user = file_with(vec![("vm.resources.cpu_count", SettingValue::Number(8))]);
    let corp = file_with(vec![("vm.resources.cpu_count", SettingValue::Number(2))]);
    let resolved = resolve_settings(&user, &corp);
    let vs = settings_to_vm_settings(&resolved);
    assert_eq!(vs.cpu_count, Some(2));
}

// -----------------------------------------------------------------------
// L: API key materialization guards
// -----------------------------------------------------------------------

#[test]
fn api_key_not_materialized_when_toggle_on() {
    let user = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(true)),
        (
            "ai.anthropic.api_key",
            SettingValue::Text("sk-test-123".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
}

#[test]
fn brokered_api_key_ref_stays_out_of_guest_env() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("user.toml");
    let store_path = dir.path().join("credential-store.json");
    let _user_guard = EnvVarGuard::set("CAPSEM_USER_CONFIG", &user_path);
    let _home_guard = EnvVarGuard::set("HOME", dir.path());
    let _store_guard = EnvVarGuard::set(crate::credential_broker::TEST_STORE_ENV, &store_path);

    let obs = crate::credential_broker::CredentialObservation {
        provider: crate::credential_broker::CredentialProvider::Anthropic,
        raw_value: "sk-ant-keychain-env".to_string(),
        source: ".env:ANTHROPIC_API_KEY".to_string(),
        event_type: Some("file.content".to_string()),
        confidence: 1.0,
        trace_id: None,
        context_json: None,
    };
    crate::credential_broker::broker_observed_credential(&obs).unwrap();
    let user = load_settings_file(&user_path).unwrap();
    assert!(!user.settings.contains_key("ai.anthropic.api_key"));
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();

    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
    let user_toml = std::fs::read_to_string(&user_path).unwrap();
    assert!(user_toml.contains("[ai.anthropic.discovery]"));
    assert!(user_toml.contains("credential_ref = \"credential:blake3:"));
    assert!(!user_toml.contains("sk-ant-keychain-env"));
    assert!(!user_toml.contains("ai.anthropic.api_key"));
}

#[test]
fn brokered_google_api_key_ref_stays_out_of_guest_env() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("user.toml");
    let store_path = dir.path().join("credential-store.json");
    let _user_guard = EnvVarGuard::set("CAPSEM_USER_CONFIG", &user_path);
    let _home_guard = EnvVarGuard::set("HOME", dir.path());
    let _store_guard = EnvVarGuard::set(crate::credential_broker::TEST_STORE_ENV, &store_path);

    let obs = crate::credential_broker::CredentialObservation {
        provider: crate::credential_broker::CredentialProvider::Google,
        raw_value: "AIza-keychain-env".to_string(),
        source: ".env:GEMINI_API_KEY".to_string(),
        event_type: Some("file.content".to_string()),
        confidence: 1.0,
        trace_id: None,
        context_json: None,
    };
    crate::credential_broker::broker_observed_credential(&obs).unwrap();
    let user = load_settings_file(&user_path).unwrap();
    assert!(!user.settings.contains_key("ai.google.api_key"));
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();

    assert!(!env.contains_key("GEMINI_API_KEY"));
    assert!(!env.contains_key("GOOGLE_API_KEY"));
    let user_toml = std::fs::read_to_string(&user_path).unwrap();
    assert!(user_toml.contains("[ai.google.discovery]"));
    assert!(user_toml.contains("credential_ref = \"credential:blake3:"));
    assert!(!user_toml.contains("AIza-keychain-env"));
    assert!(!user_toml.contains("ai.google.api_key"));
}

#[test]
fn brokered_openai_key_writes_provider_discovery_without_raw_secret() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("user.toml");
    let store_path = dir.path().join("credential-store.json");
    let _user_guard = EnvVarGuard::set("CAPSEM_USER_CONFIG", &user_path);
    let _home_guard = EnvVarGuard::set("HOME", dir.path());
    let _store_guard = EnvVarGuard::set(crate::credential_broker::TEST_STORE_ENV, &store_path);

    let obs = crate::credential_broker::CredentialObservation {
        provider: crate::credential_broker::CredentialProvider::OpenAi,
        raw_value: "sk-openai-discovery-secret".to_string(),
        source: "http.header.authorization".to_string(),
        event_type: Some("http.request".to_string()),
        confidence: 0.95,
        trace_id: Some("trace-discovery".to_string()),
        context_json: None,
    };

    let brokered = crate::credential_broker::broker_observed_credential(&obs).unwrap();
    let loaded = load_settings_file(&user_path).unwrap();
    assert!(
        !loaded.settings.contains_key("ai.openai.api_key"),
        "credential broker must not materialize broker refs into settings"
    );

    let discovery = loaded
        .ai
        .get("openai")
        .and_then(|provider| provider.discovery.as_ref())
        .expect("OpenAI discovery record should be written");
    assert_eq!(discovery.source, "http.header.authorization");
    assert_eq!(discovery.event_type.as_deref(), Some("http.request"));
    assert_eq!(discovery.confidence, 0.95);
    assert_eq!(discovery.trace_id.as_deref(), Some("trace-discovery"));
    assert_eq!(
        discovery.credential_ref.as_deref(),
        Some(brokered.credential_ref.as_str())
    );

    let user_toml = std::fs::read_to_string(&user_path).unwrap();
    assert!(user_toml.contains("[ai.openai.discovery]"));
    assert!(user_toml.contains("credential_ref = \"credential:blake3:"));
    assert!(!user_toml.contains("sk-openai-discovery-secret"));
}

#[test]
fn brokered_provider_discovery_does_not_write_corp_locked_credential_setting() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("user.toml");
    let store_path = dir.path().join("credential-store.json");
    write_settings_file(&user_path, &SettingsFile::default()).unwrap();

    let _user_guard = EnvVarGuard::set("CAPSEM_USER_CONFIG", &user_path);
    let _home_guard = EnvVarGuard::set("HOME", dir.path());
    let _store_guard = EnvVarGuard::set(crate::credential_broker::TEST_STORE_ENV, &store_path);

    let obs = crate::credential_broker::CredentialObservation {
        provider: crate::credential_broker::CredentialProvider::OpenAi,
        raw_value: "sk-openai-corp-locked".to_string(),
        source: ".env:OPENAI_API_KEY".to_string(),
        event_type: Some("file.event".to_string()),
        confidence: 1.0,
        trace_id: None,
        context_json: None,
    };

    let result = crate::credential_broker::broker_observed_credential(&obs);
    assert!(
        result.is_ok(),
        "provider discovery must not touch stale credential setting ids"
    );

    let loaded = load_settings_file(&user_path).unwrap();
    assert!(
        !loaded.settings.contains_key("ai.openai.api_key"),
        "credential setting must never be written by the broker"
    );
    assert!(
        loaded
            .ai
            .get("openai")
            .and_then(|provider| provider.discovery.as_ref())
            .is_some(),
        "provider discovery should still be recorded"
    );
}

#[test]
fn api_key_not_materialized_when_toggle_off() {
    let user = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(false)),
        (
            "ai.anthropic.api_key",
            SettingValue::Text("sk-test-123".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
}

#[test]
fn api_key_not_injected_when_empty() {
    let user = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(true)),
        ("ai.anthropic.api_key", SettingValue::Text("".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_key = gc
        .env
        .as_ref()
        .is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
    assert!(!has_key, "empty API key should not be injected");
}

#[test]
fn google_api_key_does_not_set_gemini_env_var() {
    let user = file_with(vec![
        ("ai.google.allow", SettingValue::Bool(true)),
        ("ai.google.api_key", SettingValue::Text("AIza-test".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("GEMINI_API_KEY"));
    assert!(!env.contains_key("GOOGLE_API_KEY"));
}

#[test]
fn openai_api_key_not_materialized_when_toggle_off() {
    let user = file_with(vec![
        ("ai.openai.allow", SettingValue::Bool(false)),
        (
            "ai.openai.api_key",
            SettingValue::Text("sk-oai-test".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("OPENAI_API_KEY"));
}

#[test]
fn google_api_key_not_materialized_when_toggle_off() {
    let user = file_with(vec![
        ("ai.google.allow", SettingValue::Bool(false)),
        ("ai.google.api_key", SettingValue::Text("AIza-off".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("GEMINI_API_KEY"));
}

#[test]
fn all_three_provider_keys_stay_out_of_guest_env() {
    let user = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(true)),
        ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
        ("ai.openai.allow", SettingValue::Bool(true)),
        ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
        ("ai.google.allow", SettingValue::Bool(true)),
        ("ai.google.api_key", SettingValue::Text("AIza".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
    assert!(!env.contains_key("OPENAI_API_KEY"));
    assert!(!env.contains_key("GEMINI_API_KEY"));
}

#[test]
fn brokered_provider_credentials_never_materialize_as_boot_env() {
    let user = file_with(vec![
        (
            "ai.anthropic.api_key",
            SettingValue::Text("credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into()),
        ),
        (
            "ai.openai.api_key",
            SettingValue::Text("credential:blake3:2222222222222222222222222222222222222222222222222222222222222222".into()),
        ),
        ("ai.google.allow", SettingValue::Bool(false)),
        (
            "ai.google.api_key",
            SettingValue::Text("credential:blake3:3333333333333333333333333333333333333333333333333333333333333333".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
    assert!(!env.contains_key("OPENAI_API_KEY"));
    assert!(!env.contains_key("GEMINI_API_KEY"));
}

#[test]
fn raw_provider_credentials_do_not_materialize_as_boot_env_even_before_validation() {
    let user = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(true)),
        ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
        ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
        ("ai.google.allow", SettingValue::Bool(false)),
        ("ai.google.api_key", SettingValue::Text("AIza".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
    assert!(!env.contains_key("OPENAI_API_KEY"));
    assert!(!env.contains_key("GEMINI_API_KEY"));
}

#[test]
fn provider_allowed_toggles_are_not_guest_authority_env_vars() {
    let user = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(true)),
        ("ai.openai.allow", SettingValue::Bool(false)),
        ("ai.google.allow", SettingValue::Bool(true)),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("CAPSEM_ANTHROPIC_ALLOWED"));
    assert!(!env.contains_key("CAPSEM_OPENAI_ALLOWED"));
    assert!(!env.contains_key("CAPSEM_GOOGLE_ALLOWED"));
}

#[test]
fn provider_allowed_defaults_are_not_guest_authority_env_vars() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("CAPSEM_ANTHROPIC_ALLOWED"));
    assert!(!env.contains_key("CAPSEM_OPENAI_ALLOWED"));
    assert!(!env.contains_key("CAPSEM_GOOGLE_ALLOWED"));
}

#[test]
fn web_default_toggles_not_exposed_as_guest_authority() {
    let defaults = resolve_settings(&empty_file(), &empty_file());
    let gc_defaults = settings_to_guest_config(&defaults);
    let env_defaults = gc_defaults.env.unwrap();
    assert!(!env_defaults.contains_key("CAPSEM_WEB_ALLOW_READ"));
    assert!(!env_defaults.contains_key("CAPSEM_WEB_ALLOW_WRITE"));

    let user = file_with(vec![
        ("security.web.allow_read", SettingValue::Bool(true)),
        ("security.web.allow_write", SettingValue::Bool(true)),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert!(!env.contains_key("CAPSEM_WEB_ALLOW_READ"));
    assert!(!env.contains_key("CAPSEM_WEB_ALLOW_WRITE"));
}

#[test]
fn empty_keys_skipped_regardless_of_toggle() {
    // Toggle on/off must not matter; credential settings never materialize
    // into guest env.
    let user = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(true)),
        ("ai.anthropic.api_key", SettingValue::Text("".into())),
        ("ai.openai.api_key", SettingValue::Text("".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    // Only dynamic env vars from defaults might exist, but no API keys.
    let has_ant = gc
        .env
        .as_ref()
        .is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
    let has_oai = gc
        .env
        .as_ref()
        .is_some_and(|e| e.contains_key("OPENAI_API_KEY"));
    assert!(!has_ant, "empty anthropic key should not be injected");
    assert!(!has_oai, "empty openai key should not be injected");
}

// -----------------------------------------------------------------------
// M: AI CLI boot file burn guards
// -----------------------------------------------------------------------

#[test]
fn ai_cli_boot_files_are_not_materialized_from_settings_defaults() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap_or_default();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    for path in [
        "/root/.gemini/settings.json",
        "/root/.gemini/projects.json",
        "/root/.gemini/trustedFolders.json",
        "/root/.gemini/installation_id",
        "/root/.claude/settings.json",
        "/root/.claude.json",
        "/root/.codex/config.toml",
    ] {
        assert!(!paths.contains(&path), "{path} must not come from settings");
    }
}

#[test]
fn ai_cli_boot_file_user_overrides_are_not_materialized_from_settings() {
    let user = file_with(vec![
        (
            "ai.google.gemini.settings_json",
            SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: r#"{"mcpServers":{"custom":{}}}"#.into(),
            },
        ),
        (
            "ai.openai.codex.config_toml",
            SettingValue::File {
                path: "/root/.codex/config.toml".into(),
                content: "[mcp_servers.custom]\ncommand = \"custom\"".into(),
            },
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap_or_default();
    assert!(!files
        .iter()
        .any(|f| f.path == "/root/.gemini/settings.json"));
    assert!(!files.iter().any(|f| f.path == "/root/.codex/config.toml"));
}

#[test]
fn ai_keys_and_boot_files_both_stay_out_when_toggle_off() {
    let user = file_with(vec![
        ("ai.google.allow", SettingValue::Bool(false)),
        ("ai.google.api_key", SettingValue::Text("AIza-key".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("GEMINI_API_KEY"));
    let files = gc.files.unwrap_or_default();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(!paths.contains(&"/root/.gemini/settings.json"));
    assert!(!paths.contains(&"/root/.gemini/projects.json"));
    assert!(!paths.contains(&"/root/.gemini/trustedFolders.json"));
    assert!(!paths.contains(&"/root/.gemini/installation_id"));
}

// -----------------------------------------------------------------------
// Shell config boot files (bashrc + tmux.conf)
// -----------------------------------------------------------------------

#[test]
fn bashrc_boot_file_injected() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap();
    let bashrc = files.iter().find(|f| f.path == "/root/.bashrc");
    assert!(bashrc.is_some(), "bashrc boot file should be injected");
    assert!(
        bashrc.unwrap().content.contains("PS1="),
        "bashrc should contain PS1 prompt"
    );
}

#[test]
fn tmux_conf_boot_file_injected() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap();
    let tmux = files.iter().find(|f| f.path == "/root/.tmux.conf");
    assert!(tmux.is_some(), "tmux.conf boot file should be injected");
    assert!(
        tmux.unwrap().content.contains("default-terminal"),
        "tmux.conf should contain terminal setting"
    );
}

#[test]
fn bashrc_user_override() {
    let custom = "PS1='custom> '\nalias foo='bar'\n";
    let user = file_with(vec![(
        "vm.environment.shell.bashrc",
        SettingValue::File {
            path: "/root/.bashrc".into(),
            content: custom.into(),
        },
    )]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap();
    let bashrc = files.iter().find(|f| f.path == "/root/.bashrc").unwrap();
    assert!(
        bashrc.content.contains("custom>"),
        "user override should replace default bashrc content"
    );
}

#[test]
fn shell_boot_files_have_correct_mode() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap();
    for path in &["/root/.bashrc", "/root/.tmux.conf"] {
        let f = files.iter().find(|f| f.path == *path).unwrap();
        assert_eq!(f.mode, 0o600, "boot file {} should have mode 0600", path);
    }
}

// -----------------------------------------------------------------------
// Filetype metadata
// -----------------------------------------------------------------------

#[test]
fn filetype_metadata_propagated() {
    let defs = setting_definitions();
    let bashrc = defs
        .iter()
        .find(|d| d.id == "vm.environment.shell.bashrc")
        .unwrap();
    assert_eq!(bashrc.metadata.filetype.as_deref(), Some("bash"));
    let tmux = defs
        .iter()
        .find(|d| d.id == "vm.environment.shell.tmux_conf")
        .unwrap();
    assert_eq!(tmux.metadata.filetype.as_deref(), Some("conf"));
}

// -----------------------------------------------------------------------
// N: File setting type
// -----------------------------------------------------------------------

#[test]
fn file_type_exists_in_setting_type_enum() {
    // The File variant should serialize to "file".
    let st = SettingType::File;
    let json = serde_json::to_string(&st).unwrap();
    assert_eq!(json, r#""file""#);
}

#[test]
fn ai_cli_json_settings_are_not_settings() {
    let defs = setting_definitions();
    for id in &[
        "ai.google.gemini.settings_json",
        "ai.google.gemini.projects_json",
        "ai.google.gemini.trusted_folders_json",
    ] {
        assert!(
            defs.iter().all(|d| d.id != *id),
            "{id} must not be settings-owned AI CLI state"
        );
    }
}

#[test]
fn shell_boot_files_are_file_type() {
    let defs = setting_definitions();
    let def = defs
        .iter()
        .find(|d| d.id == "vm.environment.shell.bashrc")
        .unwrap();
    assert_eq!(def.setting_type, SettingType::File);
    let (path, content) = def.default_value.as_file().expect("should be File value");
    assert_eq!(path, "/root/.bashrc");
    assert!(content.contains("alias "));
}

#[test]
fn file_settings_have_path_in_default_value() {
    // Every File-type setting must have a File default with a valid path.
    let defs = setting_definitions();
    for def in &defs {
        if def.setting_type == SettingType::File {
            let (path, _) = def
                .default_value
                .as_file()
                .unwrap_or_else(|| panic!("File setting {} must have File default value", def.id));
            assert!(
                path.starts_with('/'),
                "path must be absolute: {path} (setting {})",
                def.id
            );
        }
    }
}

#[test]
fn guest_config_does_not_materialize_ai_file_settings() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap_or_default();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(!paths.contains(&"/root/.gemini/settings.json"));
    assert!(!paths.contains(&"/root/.gemini/projects.json"));
    assert!(!paths.contains(&"/root/.gemini/trustedFolders.json"));
    assert!(!paths.contains(&"/root/.gemini/installation_id"));
    assert!(!paths.contains(&"/root/.claude/settings.json"));
    assert!(!paths.contains(&"/root/.claude.json"));
    assert!(!paths.contains(&"/root/.codex/config.toml"));
}

// -----------------------------------------------------------------------
// O: Setting value validation
// -----------------------------------------------------------------------

#[test]
fn validate_file_setting_rejects_invalid_json() {
    let err = validate_setting_value(
        "ai.google.gemini.settings_json",
        &SettingValue::File {
            path: "/root/.gemini/settings.json".into(),
            content: "{not valid json".into(),
        },
    );
    assert!(err.is_err(), "invalid JSON should be rejected");
    assert!(err.unwrap_err().contains("invalid JSON"));
}

#[test]
fn validate_file_setting_accepts_valid_json() {
    let result = validate_setting_value(
        "ai.google.gemini.settings_json",
        &SettingValue::File {
            path: "/root/.gemini/settings.json".into(),
            content: r#"{"key":"value"}"#.into(),
        },
    );
    assert!(result.is_ok());
}

#[test]
fn validate_file_setting_accepts_empty_content() {
    // Empty content is fine -- means "use default" or "don't inject".
    let result = validate_setting_value(
        "ai.google.gemini.settings_json",
        &SettingValue::File {
            path: "/root/.gemini/settings.json".into(),
            content: "".into(),
        },
    );
    assert!(result.is_ok());
}

#[test]
fn validate_non_json_file_accepts_anything() {
    // installation_id path doesn't end in .json -- no JSON validation.
    let result = validate_setting_value(
        "ai.google.gemini.installation_id",
        &SettingValue::File {
            path: "/root/.gemini/installation_id".into(),
            content: "not json at all".into(),
        },
    );
    assert!(result.is_ok());
}

#[test]
fn validate_non_file_settings_pass_through() {
    // Bool, Number, etc. settings always pass validation.
    let result = validate_setting_value(SETTING_GITHUB_ALLOW, &SettingValue::Bool(true));
    assert!(result.is_ok());
}

#[test]
fn file_type_resolved_setting_has_file_value() {
    // The resolved setting for a File type should have a File value with path.
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let s = resolved
        .iter()
        .find(|s| s.id == "vm.environment.shell.bashrc")
        .unwrap();
    assert_eq!(s.setting_type, SettingType::File);
    let (path, _content) = s.effective_value.as_file().expect("should be a File value");
    assert_eq!(path, "/root/.bashrc");
}

// -----------------------------------------------------------------------
// P: Metadata-driven env var injection
// -----------------------------------------------------------------------

#[test]
fn api_key_settings_do_not_drive_guest_env_vars() {
    let defs = setting_definitions();
    for id in [
        "ai.anthropic.api_key",
        "ai.openai.api_key",
        "ai.google.api_key",
    ] {
        assert!(
            defs.iter().all(|d| d.id != id),
            "{id} must not be a settings-owned provider credential"
        );
    }
}

#[test]
fn builtin_env_settings_exist() {
    // Built-in guest env vars (TERM, HOME, PATH, LANG) must be registered
    // settings, not hardcoded in build_boot_config.
    let defs = setting_definitions();
    let required = ["TERM", "HOME", "PATH", "LANG"];
    for var in &required {
        let found = defs
            .iter()
            .any(|d| d.metadata.env_vars.contains(&var.to_string()));
        assert!(found, "no setting definition injects env var {var}");
    }
}

#[test]
fn ca_bundle_setting_injects_three_env_vars() {
    // A single CA bundle setting should inject REQUESTS_CA_BUNDLE,
    // NODE_EXTRA_CA_CERTS, and SSL_CERT_FILE.
    let defs = setting_definitions();
    let ca_vars = ["REQUESTS_CA_BUNDLE", "NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE"];
    for var in &ca_vars {
        let found = defs
            .iter()
            .any(|d| d.metadata.env_vars.contains(&var.to_string()));
        assert!(found, "no setting definition injects env var {var}");
    }
}

#[test]
fn brokered_credential_setting_metadata_does_not_materialize_guest_env() {
    let user = file_with(vec![(
        "ai.anthropic.api_key",
        SettingValue::Text(
            "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                .into(),
        ),
    )]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
}

#[test]
fn builtin_env_defaults_in_guest_config() {
    // With no user/corp overrides, the built-in env vars should have
    // their default values from the setting definitions.
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert_eq!(env.get("TERM").unwrap(), "xterm-256color");
    assert_eq!(env.get("HOME").unwrap(), "/root");
    assert!(env.get("PATH").unwrap().contains("/usr/bin"));
    assert_eq!(env.get("LANG").unwrap(), "C");
}

#[test]
fn ca_bundle_injected_as_three_env_vars() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    let ca_path = "/etc/ssl/certs/ca-certificates.crt";
    assert_eq!(env.get("REQUESTS_CA_BUNDLE").unwrap(), ca_path);
    assert_eq!(env.get("NODE_EXTRA_CA_CERTS").unwrap(), ca_path);
    assert_eq!(env.get("SSL_CERT_FILE").unwrap(), ca_path);
}

#[test]
fn corp_can_override_builtin_env() {
    // Corp should be able to lock down built-in env settings.
    let defs = setting_definitions();
    let term_def = defs
        .iter()
        .find(|d| d.metadata.env_vars.contains(&"TERM".to_string()))
        .unwrap();
    let corp = file_with(vec![(&term_def.id, SettingValue::Text("dumb".into()))]);
    let resolved = resolve_settings(&empty_file(), &corp);
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert_eq!(env.get("TERM").unwrap(), "dumb");
}

#[test]
fn user_can_override_builtin_env() {
    let defs = setting_definitions();
    let path_def = defs
        .iter()
        .find(|d| d.metadata.env_vars.contains(&"PATH".to_string()))
        .unwrap();
    let user = file_with(vec![(
        &path_def.id,
        SettingValue::Text("/custom/bin".into()),
    )]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert_eq!(env.get("PATH").unwrap(), "/custom/bin");
}

#[test]
fn empty_env_var_setting_not_injected() {
    // A setting with env_vars metadata but empty value should not be injected.
    let user = file_with(vec![(
        "ai.anthropic.api_key",
        SettingValue::Text("".into()),
    )]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_key = gc
        .env
        .as_ref()
        .is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
    assert!(!has_key, "empty API key should not be injected");
}

#[test]
fn dynamic_guest_env_still_works() {
    // Dynamic guest.env.* settings should still be injected alongside
    // metadata-driven env vars.
    let user = file_with(vec![("guest.env.EDITOR", SettingValue::Text("vim".into()))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert_eq!(env.get("EDITOR").unwrap(), "vim");
    // Built-in env vars should also be present.
    assert!(env.contains_key("TERM"));
}

#[test]
fn each_boot_message_fits_in_frame() {
    // Each individual boot message (SetEnv, FileWrite) must fit in
    // MAX_FRAME_SIZE. The old single-BootConfig frame limit is gone.
    use capsem_proto::{encode_host_msg, MAX_FRAME_SIZE};

    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);

    // Each env var as a SetEnv message
    for (key, value) in gc.env.unwrap_or_default() {
        let msg = capsem_proto::HostToGuest::SetEnv {
            key: key.clone(),
            value: value.clone(),
        };
        let frame = encode_host_msg(&msg).unwrap();
        assert!(
            frame.len() - 4 <= MAX_FRAME_SIZE as usize,
            "SetEnv({key}) too large: {} bytes",
            frame.len() - 4,
        );
    }

    // Each file as a FileWrite message
    for f in gc.files.unwrap_or_default() {
        let msg = capsem_proto::HostToGuest::FileWrite {
            id: 1,
            path: f.path.clone(),
            data: f.content.into_bytes(),
            mode: f.mode,
        };
        let frame = encode_host_msg(&msg).unwrap();
        assert!(
            frame.len() - 4 <= MAX_FRAME_SIZE as usize,
            "FileWrite({}) too large: {} bytes",
            f.path,
            frame.len() - 4,
        );
    }
}

#[test]
fn all_env_vars_metadata_refers_to_text_settings() {
    // Every setting with env_vars metadata must have a text-like type
    // (Text, ApiKey, Url, Email).
    let defs = setting_definitions();
    for def in &defs {
        if !def.metadata.env_vars.is_empty() {
            assert!(
                matches!(
                    def.setting_type,
                    SettingType::Text | SettingType::ApiKey | SettingType::Url | SettingType::Email
                ),
                "setting {} has env_vars but type {:?} (should be text-like)",
                def.id,
                def.setting_type,
            );
        }
    }
}

// -------------------------------------------------------------------
// Boot handshake validation in settings layer
// -------------------------------------------------------------------

#[test]
fn settings_rejects_blocked_env_var() {
    // guest.env.LD_PRELOAD in user.toml should be silently dropped.
    let user = file_with(vec![(
        "guest.env.LD_PRELOAD",
        SettingValue::Text("/evil/lib.so".into()),
    )]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_key = gc
        .env
        .as_ref()
        .is_some_and(|e| e.contains_key("LD_PRELOAD"));
    assert!(!has_key, "LD_PRELOAD should be dropped by validation");
}

#[test]
fn settings_rejects_ld_library_path() {
    let user = file_with(vec![(
        "guest.env.LD_LIBRARY_PATH",
        SettingValue::Text("/evil".into()),
    )]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_key = gc
        .env
        .as_ref()
        .is_some_and(|e| e.contains_key("LD_LIBRARY_PATH"));
    assert!(!has_key, "LD_LIBRARY_PATH should be dropped by validation");
}

#[test]
fn settings_accepts_normal_dynamic_env() {
    let user = file_with(vec![("guest.env.EDITOR", SettingValue::Text("vim".into()))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert_eq!(env.get("EDITOR").unwrap(), "vim");
}

// -----------------------------------------------------------------------
// Web search category
// -----------------------------------------------------------------------

#[test]
fn web_search_google_allowed_by_default() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let s = resolved
        .iter()
        .find(|s| s.id == "security.services.search.google.allow")
        .unwrap();
    assert_eq!(s.effective_value, SettingValue::Bool(true));
    assert_eq!(s.category, "Google");
}

#[test]
fn web_search_bing_duckduckgo_blocked_by_default() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    for id in &[
        "security.services.search.bing.allow",
        "security.services.search.duckduckgo.allow",
    ] {
        let s = resolved.iter().find(|s| s.id == *id).unwrap();
        assert_eq!(
            s.effective_value,
            SettingValue::Bool(false),
            "expected {id} to be false"
        );
    }
}

#[test]
fn default_http_allow_is_security_rule_not_network_policy() {
    let m = MergedPolicies::from_files(&empty_file(), &empty_file());
    assert!(
        has_security_rule(&m, "profiles.rules.default_http"),
        "default HTTP behavior must be a visible security rule"
    );
}

#[test]
fn default_http_upstream_ports_in_network_policy() {
    let m = MergedPolicies::from_files(&empty_file(), &empty_file());
    assert_eq!(m.network.http_upstream_ports, vec![80, 11434]);
}

#[test]
fn user_http_upstream_ports_override_network_policy() {
    let user = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80, 50233]),
    )]);
    let m = MergedPolicies::from_files(&user, &empty_file());
    assert_eq!(m.network.http_upstream_ports, vec![80, 50233]);
}

#[test]
fn corp_http_upstream_ports_override_user_network_policy() {
    let user = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80, 50233]),
    )]);
    let corp = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80, 11434]),
    )]);
    let m = MergedPolicies::from_files(&user, &corp);
    assert_eq!(m.network.http_upstream_ports, vec![80, 11434]);
}

#[test]
fn settings_guest_config_does_not_inject_mcp_into_ai_cli_files() {
    let user = file_with(vec![(
        "ai.google.gemini.settings_json",
        SettingValue::File {
            path: "/root/.gemini/settings.json".into(),
            content: r#"{"mcpServers":{"myserver":{"command":"my-tool"}}}"#.into(),
        },
    )]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap_or_default();
    for path in [
        "/root/.claude/settings.json",
        "/root/.gemini/settings.json",
        "/root/.gemini/projects.json",
        "/root/.claude.json",
        "/root/.codex/config.toml",
    ] {
        assert!(!files.iter().any(|f| f.path == path));
    }
}

// -----------------------------------------------------------------------
// TOML registry tests
// -----------------------------------------------------------------------

#[test]
fn toml_registry_parses() {
    // The embedded defaults.toml must parse without panicking.
    let defs = setting_definitions();
    assert!(
        !defs.is_empty(),
        "defaults.toml must produce at least one setting"
    );
}

#[test]
fn toml_registry_setting_count() {
    // Guard against accidental deletions. Update this if settings are
    // intentionally added or removed.
    let defs = setting_definitions();
    assert!(
        defs.len() >= 20,
        "expected at least 20 settings from defaults.toml, got {}",
        defs.len(),
    );
}

#[test]
fn toml_registry_ids_from_path() {
    // IDs are dot-separated paths derived from the TOML table nesting.
    let defs = setting_definitions();
    for def in &defs {
        assert!(
            def.id.contains('.'),
            "setting id '{}' should be a dotted path",
            def.id,
        );
    }
}

#[test]
fn toml_registry_category_inherited() {
    // Category is inherited from the nearest ancestor group with a `name`.
    let defs = setting_definitions();
    let github_allow = defs.iter().find(|d| d.id == SETTING_GITHUB_ALLOW).unwrap();
    assert!(
        !github_allow.category.is_empty(),
        "repository.providers.github.allow should have a category inherited from its group",
    );
}

#[test]
fn toml_registry_enabled_by_inherited() {
    // enabled_by is inherited from the group and applied to children
    // but NOT to the toggle setting itself.
    let defs = setting_definitions();
    let allow = defs.iter().find(|d| d.id == SETTING_GITHUB_ALLOW).unwrap();
    assert!(
        allow.enabled_by.is_none(),
        "the toggle itself should not have enabled_by",
    );
    let api_key = defs.iter().find(|d| d.id == SETTING_GITHUB_TOKEN).unwrap();
    assert_eq!(
        api_key.enabled_by.as_deref(),
        Some(SETTING_GITHUB_ALLOW),
        "token should inherit enabled_by from its group",
    );
}

#[test]
fn toml_registry_meta_fields() {
    // Metadata fields (domains, choices, rules, env_vars)
    // are correctly parsed from the `meta` sub-table.
    let defs = setting_definitions();

    // Registry toggles should have domains in metadata
    let github = defs.iter().find(|d| d.id == SETTING_GITHUB_ALLOW).unwrap();
    assert!(
        !github.metadata.domains.is_empty(),
        "github toggle should have domain metadata"
    );

    // security.web.http_upstream_ports should be network mechanics, not a decision toggle.
    let ports = defs
        .iter()
        .find(|d| d.id == "security.web.http_upstream_ports")
        .unwrap();
    assert_eq!(
        ports.setting_type,
        SettingType::IntList,
        "http_upstream_ports should be an int list"
    );

    assert!(
        defs.iter().all(|d| !d.id.starts_with("ai.")),
        "AI provider controls must not be settings-owned"
    );
}

// -----------------------------------------------------------------------
// Config lint tests
// -----------------------------------------------------------------------

fn make_resolved(
    id: &str,
    stype: SettingType,
    value: SettingValue,
    meta: SettingMetadata,
    enabled_by: Option<&str>,
) -> ResolvedSetting {
    ResolvedSetting {
        id: id.to_string(),
        category: "Test".to_string(),
        name: id.to_string(),
        description: "test".to_string(),
        setting_type: stype,
        default_value: value.clone(),
        effective_value: value,
        source: PolicySource::Default,
        modified: None,
        corp_locked: false,
        enabled_by: enabled_by.map(String::from),
        enabled: true,
        metadata: meta,
        collapsed: false,
        history: Vec::new(),
    }
}

// -- JSON validation (File values) --

fn file_val(path: &str, content: &str) -> SettingValue {
    SettingValue::File {
        path: path.into(),
        content: content.into(),
    }
}

#[test]
fn config_lint_valid_json_passes() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", r#"{"key":"val"}"#),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_malformed_json_gives_clear_error() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", "{bad json}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "error" && i.message.contains("invalid JSON")));
}

#[test]
fn config_lint_json_not_object_warns() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", "42"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("not an object")));
}

#[test]
fn config_lint_empty_json_file_ok() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", ""),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_json_with_trailing_comma_gives_error() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", r#"{"a":1,}"#),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.iter().any(|i| i.severity == "error"));
}

#[test]
fn config_lint_json_with_unicode_passes() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", r#"{"name":"cafe\u0301"}"#),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_json_deeply_nested_passes() {
    let json = r#"{"a":{"b":{"c":{"d":{"e":"deep"}}}}}"#;
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", json),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_json_huge_payload_passes() {
    let big_val = "x".repeat(1_000_000);
    let json = format!(r#"{{"data":"{}"}}"#, big_val);
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/test.json", &json),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_file_path_must_be_absolute() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("relative/path.json", "{}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "error" && i.message.contains("absolute")));
}

#[test]
fn config_lint_file_path_no_traversal() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/root/../etc/passwd", "{}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "error" && i.message.contains("..")));
}

#[test]
fn config_lint_file_unusual_path_warns() {
    let s = make_resolved(
        "test.file",
        SettingType::File,
        file_val("/tmp/test.json", "{}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("unusual")));
}

// -- Number validation --

#[test]
fn config_lint_number_in_range_ok() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(128),
        ..Default::default()
    };
    let s = make_resolved(
        "vm.cpu",
        SettingType::Number,
        SettingValue::Number(4),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_number_below_min_error() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(128),
        ..Default::default()
    };
    let s = make_resolved(
        "vm.cpu",
        SettingType::Number,
        SettingValue::Number(0),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
    assert!(issues[0].message.contains("below minimum"));
}

#[test]
fn config_lint_number_above_max_error() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(128),
        ..Default::default()
    };
    let s = make_resolved(
        "vm.disk",
        SettingType::Number,
        SettingValue::Number(256),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
    assert!(issues[0].message.contains("exceeds maximum"));
}

#[test]
fn config_lint_number_at_boundary_ok() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(128),
        ..Default::default()
    };
    let s1 = make_resolved(
        "vm.min",
        SettingType::Number,
        SettingValue::Number(1),
        meta.clone(),
        None,
    );
    let s2 = make_resolved(
        "vm.max",
        SettingType::Number,
        SettingValue::Number(128),
        meta,
        None,
    );
    let issues = config_lint(&[s1, s2]);
    assert!(issues.is_empty());
}

// -- Choice validation --

#[test]
fn config_lint_valid_choice_ok() {
    let meta = SettingMetadata {
        choices: vec!["allow".into(), "deny".into()],
        ..Default::default()
    };
    let s = make_resolved(
        "net.action",
        SettingType::Text,
        SettingValue::Text("deny".into()),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_invalid_choice_error() {
    let meta = SettingMetadata {
        choices: vec!["allow".into(), "deny".into()],
        ..Default::default()
    };
    let s = make_resolved(
        "net.action",
        SettingType::Text,
        SettingValue::Text("block".into()),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
    assert!(issues[0].message.contains("not a valid choice"));
}

#[test]
fn config_lint_empty_choice_when_choices_defined_error() {
    let meta = SettingMetadata {
        choices: vec!["allow".into(), "deny".into()],
        ..Default::default()
    };
    let s = make_resolved(
        "net.action",
        SettingType::Text,
        SettingValue::Text("".into()),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
}

#[test]
fn config_lint_case_sensitive_choice() {
    let meta = SettingMetadata {
        choices: vec!["allow".into(), "deny".into()],
        ..Default::default()
    };
    let s = make_resolved(
        "net.action",
        SettingType::Text,
        SettingValue::Text("Allow".into()),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1, "'Allow' != 'allow' -- case sensitive");
}

// -- API key validation --

#[test]
fn config_lint_apikey_with_whitespace_warns() {
    let s = make_resolved(
        "ai.key",
        SettingType::ApiKey,
        SettingValue::Text("sk-ant key".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("whitespace")));
}

#[test]
fn config_lint_apikey_with_newline_warns() {
    let s = make_resolved(
        "ai.key",
        SettingType::ApiKey,
        SettingValue::Text("sk-ant\n".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("whitespace")));
}

#[test]
fn config_lint_apikey_empty_when_enabled_warns() {
    let toggle = make_resolved(
        "ai.provider.allow",
        SettingType::Bool,
        SettingValue::Bool(true),
        SettingMetadata::default(),
        None,
    );
    let key = make_resolved(
        "ai.provider.key",
        SettingType::ApiKey,
        SettingValue::Text("".into()),
        SettingMetadata::default(),
        Some("ai.provider.allow"),
    );
    let issues = config_lint(&[toggle, key]);
    assert!(issues
        .iter()
        .any(|i| i.severity == "warning" && i.message.contains("not set")));
}

#[test]
fn config_lint_apikey_empty_when_disabled_ok() {
    let toggle = make_resolved(
        "ai.provider.allow",
        SettingType::Bool,
        SettingValue::Bool(false),
        SettingMetadata::default(),
        None,
    );
    let key = make_resolved(
        "ai.provider.key",
        SettingType::ApiKey,
        SettingValue::Text("".into()),
        SettingMetadata::default(),
        Some("ai.provider.allow"),
    );
    let issues = config_lint(&[toggle, key]);
    assert!(
        issues.is_empty(),
        "disabled provider with empty key is fine"
    );
}

#[test]
fn config_lint_apikey_normal_value_ok() {
    let s = make_resolved(
        "ai.key",
        SettingType::ApiKey,
        SettingValue::Text("sk-ant-api03-valid".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

// -- Text validation --

#[test]
fn config_lint_text_with_nul_byte_error() {
    let s = make_resolved(
        "t.val",
        SettingType::Text,
        SettingValue::Text("hello\0world".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, "error");
    assert!(issues[0].message.contains("invalid characters"));
}

#[test]
fn config_lint_text_normal_ok() {
    let s = make_resolved(
        "t.val",
        SettingType::Text,
        SettingValue::Text("hello".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_text_unicode_ok() {
    let s = make_resolved(
        "t.val",
        SettingType::Text,
        SettingValue::Text("cafe\u{0301}".into()),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

#[test]
fn config_lint_text_very_long_ok() {
    let long_val = "x".repeat(10_000);
    let s = make_resolved(
        "t.val",
        SettingType::Text,
        SettingValue::Text(long_val),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s]);
    assert!(issues.is_empty());
}

// -- Serialization roundtrip --

#[test]
fn config_lint_all_issues_serialize_deserialize() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s = make_resolved(
        "v.n",
        SettingType::Number,
        SettingValue::Number(99),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    let json = serde_json::to_string(&issues).unwrap();
    let roundtrip: Vec<ConfigIssue> = serde_json::from_str(&json).unwrap();
    assert_eq!(issues, roundtrip);
}

#[test]
fn config_lint_issue_messages_are_nonempty() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s = make_resolved(
        "v.n",
        SettingType::Number,
        SettingValue::Number(99),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    for issue in &issues {
        assert!(!issue.message.is_empty());
        assert!(!issue.id.is_empty());
    }
}

#[test]
fn config_lint_issue_ids_are_valid_setting_ids() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s = make_resolved(
        "vm.resources.cpu_count",
        SettingType::Number,
        SettingValue::Number(99),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    for issue in &issues {
        assert_eq!(issue.id, "vm.resources.cpu_count");
    }
}

// -- Integration --

#[test]
fn config_lint_default_config_has_no_errors() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let issues = config_lint(&resolved);
    let errors: Vec<_> = issues.iter().filter(|i| i.severity == "error").collect();
    assert!(
        errors.is_empty(),
        "default config should have no errors: {errors:?}"
    );
}

#[test]
fn config_lint_returns_multiple_issues() {
    let meta_num = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s1 = make_resolved(
        "v.n",
        SettingType::Number,
        SettingValue::Number(99),
        meta_num,
        None,
    );
    let s2 = make_resolved(
        "v.f",
        SettingType::File,
        file_val("/root/test.json", "{bad}"),
        SettingMetadata::default(),
        None,
    );
    let issues = config_lint(&[s1, s2]);
    assert!(issues.len() >= 2, "expected multiple issues: {issues:?}");
}

// -- docs_url --

#[test]
fn config_lint_empty_key_has_docs_url() {
    let meta = SettingMetadata {
        docs_url: Some("https://example.com/keys".into()),
        ..Default::default()
    };
    let toggle = make_resolved(
        "ai.provider.allow",
        SettingType::Bool,
        SettingValue::Bool(true),
        SettingMetadata::default(),
        None,
    );
    let key = make_resolved(
        "ai.provider.key",
        SettingType::ApiKey,
        SettingValue::Text("".into()),
        meta,
        Some("ai.provider.allow"),
    );
    let issues = config_lint(&[toggle, key]);
    let empty_key_issue = issues
        .iter()
        .find(|i| i.message.contains("not set"))
        .unwrap();
    assert_eq!(
        empty_key_issue.docs_url.as_deref(),
        Some("https://example.com/keys")
    );
}

#[test]
fn config_lint_non_key_issue_no_docs_url() {
    let meta = SettingMetadata {
        min: Some(1),
        max: Some(10),
        ..Default::default()
    };
    let s = make_resolved(
        "v.n",
        SettingType::Number,
        SettingValue::Number(99),
        meta,
        None,
    );
    let issues = config_lint(&[s]);
    assert!(!issues.is_empty());
    for issue in &issues {
        assert!(
            issue.docs_url.is_none(),
            "non-key issues should not have docs_url"
        );
    }
}

#[test]
fn docs_url_parsed_from_toml() {
    let defs = setting_definitions();
    let github_token = defs.iter().find(|d| d.id == SETTING_GITHUB_TOKEN).unwrap();
    assert_eq!(
        github_token.metadata.docs_url.as_deref(),
        Some("https://github.com/settings/tokens")
    );
}

// -----------------------------------------------------------------------
// Settings tree tests
// -----------------------------------------------------------------------

#[test]
fn settings_tree_has_top_level_groups() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);
    assert!(!tree.is_empty(), "tree should have top-level nodes");
    // All top-level nodes should be groups
    for node in &tree {
        match node {
            SettingsNode::Group { name, .. } => {
                assert!(!name.is_empty());
            }
            SettingsNode::Leaf(_) => {
                panic!("top-level nodes should be groups, not leaves");
            }
            SettingsNode::Action { .. } | SettingsNode::McpServer(_) => {
                // Action and MCP nodes can appear at top level
            }
        }
    }
}

#[test]
fn settings_tree_contains_all_definitions() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);
    let defs = setting_definitions();

    fn collect_leaf_ids(nodes: &[SettingsNode]) -> Vec<String> {
        let mut ids = Vec::new();
        for node in nodes {
            match node {
                SettingsNode::Leaf(s) => ids.push(s.id.clone()),
                SettingsNode::Group { children, .. } => {
                    ids.extend(collect_leaf_ids(children));
                }
                SettingsNode::Action { .. } | SettingsNode::McpServer(_) => {}
            }
        }
        ids
    }

    let leaf_ids = collect_leaf_ids(&tree);
    for def in &defs {
        assert!(
            leaf_ids.contains(&def.id),
            "tree missing definition: {}",
            def.id,
        );
    }
}

#[test]
fn settings_tree_groups_have_expected_names() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);

    fn collect_group_names(nodes: &[SettingsNode]) -> Vec<String> {
        let mut names = Vec::new();
        for node in nodes {
            if let SettingsNode::Group { name, children, .. } = node {
                names.push(name.clone());
                names.extend(collect_group_names(children));
            }
        }
        names
    }

    let names = collect_group_names(&tree);
    for expected in &[
        "Security",
        "Network Mechanics",
        "Services",
        "Search Engines",
        "Package Registries",
        "Appearance",
        "VM",
        "Environment",
        "Resources",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "tree missing group: {expected}",
        );
    }
}

#[test]
fn settings_tree_serializes_to_json() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);
    let json = serde_json::to_string(&tree).unwrap();
    // Verify it round-trips
    let _: Vec<SettingsNode> = serde_json::from_str(&json).unwrap();
    assert!(json.contains("\"kind\":\"group\""));
    assert!(json.contains("\"kind\":\"leaf\""));
}

#[test]
fn settings_tree_dynamic_env_appended_to_guest() {
    let user = file_with(vec![("guest.env.EDITOR", SettingValue::Text("vim".into()))]);
    let resolved = resolve_settings(&user, &empty_file());
    let tree = build_settings_tree(&resolved);

    fn find_leaf_in_group(nodes: &[SettingsNode], group_name: &str, leaf_id: &str) -> bool {
        for node in nodes {
            if let SettingsNode::Group { name, children, .. } = node {
                if name == group_name {
                    return children.iter().any(|c| match c {
                        SettingsNode::Leaf(s) => s.id == leaf_id,
                        SettingsNode::Group { children, .. } => {
                            children.iter().any(|cc| match cc {
                                SettingsNode::Leaf(s) => s.id == leaf_id,
                                _ => false,
                            })
                        }
                        _ => false,
                    });
                }
                if find_leaf_in_group(children, group_name, leaf_id) {
                    return true;
                }
            }
        }
        false
    }

    assert!(
        find_leaf_in_group(&tree, "Environment", "guest.env.EDITOR"),
        "dynamic guest.env.EDITOR should appear in Environment group (under VM)",
    );
}

#[test]
fn settings_tree_enabled_by_on_groups() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);

    fn find_group(nodes: &[SettingsNode], key: &str) -> Option<SettingsNode> {
        for node in nodes {
            if let SettingsNode::Group {
                key: k, children, ..
            } = node
            {
                if k == key {
                    return Some(node.clone());
                }
                if let Some(found) = find_group(children, key) {
                    return Some(found);
                }
            }
        }
        None
    }

    let github = find_group(&tree, "repository.providers.github");
    assert!(
        github.is_some(),
        "should find repository.providers.github group"
    );
    if let Some(SettingsNode::Group { enabled_by, .. }) = github {
        assert_eq!(enabled_by, Some(SETTING_GITHUB_ALLOW.to_string()));
    }
}

// -----------------------------------------------------------------------
// Grammar: action nodes in tree
// -----------------------------------------------------------------------

#[test]
fn settings_tree_contains_action_nodes() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);

    fn find_action(nodes: &[SettingsNode], action: ActionKind) -> bool {
        for node in nodes {
            match node {
                SettingsNode::Action { action: a, .. } if *a == action => return true,
                SettingsNode::Group { children, .. } => {
                    if find_action(children, action) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    assert!(
        find_action(&tree, ActionKind::CheckUpdate),
        "tree should contain check_update action"
    );
}

#[test]
fn action_nodes_not_in_setting_definitions() {
    let defs = setting_definitions();
    // Action node keys should NOT appear as setting definitions
    assert!(
        defs.iter().all(|d| d.id != "app.check_update"),
        "action nodes should not be in setting_definitions"
    );
}

// -----------------------------------------------------------------------
// Grammar: side_effect metadata
// -----------------------------------------------------------------------

#[test]
fn dark_mode_has_side_effect() {
    let defs = setting_definitions();
    let dark_mode = defs
        .iter()
        .find(|d| d.id == "appearance.dark_mode")
        .unwrap();
    assert_eq!(
        dark_mode.metadata.side_effect,
        Some(SideEffect::ToggleTheme)
    );
}

// -----------------------------------------------------------------------
// Grammar: MCP server loading
// -----------------------------------------------------------------------

#[test]
fn mcp_section_parsed_from_defaults() {
    // guest/config/mcp/local.toml declares [local]
    let servers = super::loader::load_mcp_servers();
    let local = servers.iter().find(|s| s.key == "local");
    assert!(local.is_some(), "local MCP server should be in defaults");
    let local = local.unwrap();
    assert_eq!(local.name, "Local");
    assert_eq!(local.transport, McpTransport::Stdio);
    assert_eq!(local.command.as_deref(), Some("/run/capsem-mcp-server"));
    assert!(local.builtin);
    assert!(local.enabled);
    assert_eq!(local.source, PolicySource::Default);
}

#[test]
fn mcp_servers_in_tree() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let servers = super::loader::load_mcp_servers();
    let tree = build_settings_tree_with_mcp(&resolved, &servers);

    // Find the MCP Servers group
    let mcp_group = tree
        .iter()
        .find(|n| matches!(n, SettingsNode::Group { name, .. } if name == "MCP Servers"));
    assert!(mcp_group.is_some(), "tree should have MCP Servers group");

    if let Some(SettingsNode::Group { children, .. }) = mcp_group {
        let has_local = children
            .iter()
            .any(|c| matches!(c, SettingsNode::McpServer(s) if s.key == "local"));
        assert!(has_local, "MCP Servers group should contain local");
    }
}

// -----------------------------------------------------------------------
// Grammar: list value types
// -----------------------------------------------------------------------

#[test]
fn setting_value_string_list_roundtrip() {
    let val = SettingValue::StringList(vec!["a.com".into(), "b.com".into()]);
    let json = serde_json::to_string(&val).unwrap();
    let back: SettingValue = serde_json::from_str(&json).unwrap();
    assert_eq!(val, back);
}

#[test]
fn setting_value_int_list_roundtrip() {
    let val = SettingValue::IntList(vec![1, 2, 3]);
    let json = serde_json::to_string(&val).unwrap();
    let back: SettingValue = serde_json::from_str(&json).unwrap();
    assert_eq!(val, back);
}

#[test]
fn setting_value_float_list_roundtrip() {
    let val = SettingValue::FloatList(vec![1.5, 2.5]);
    let json = serde_json::to_string(&val).unwrap();
    let back: SettingValue = serde_json::from_str(&json).unwrap();
    assert_eq!(val, back);
}

// -----------------------------------------------------------------------
// Batch update + corp enforcement
// -----------------------------------------------------------------------

fn with_temp_configs<F: FnOnce(&std::path::Path, &std::path::Path)>(
    user_entries: Vec<(&str, SettingValue)>,
    corp_entries: Vec<(&str, SettingValue)>,
    f: F,
) {
    // This helper mutates process-wide env vars that the loader reads.
    // Serialize across the whole test binary so parallel tests don't
    // stomp each other's CAPSEM_*_CONFIG (caused flaky batch_update_*
    // failures before this lock).
    let _guard = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();

    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("user.toml");
    let corp_path = dir.path().join("corp.toml");
    let user_file = file_with(user_entries);
    let corp_file = file_with(corp_entries);
    loader::write_settings_file(&user_path, &user_file).unwrap();
    loader::write_settings_file(&corp_path, &corp_file).unwrap();
    // Point env vars to temp files
    std::env::set_var("CAPSEM_USER_CONFIG", &user_path);
    std::env::set_var("CAPSEM_CORP_CONFIG", &corp_path);
    f(&user_path, &corp_path);
    std::env::remove_var("CAPSEM_USER_CONFIG");
    std::env::remove_var("CAPSEM_CORP_CONFIG");
}

#[test]
fn batch_update_accepts_valid_changes() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        changes.insert(
            SETTING_GITHUB_TOKEN.to_string(),
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                    .into(),
            ),
        );
        let result = loader::batch_update_profile_settings(&changes);
        assert!(result.is_ok(), "valid changes should succeed: {:?}", result);
        let applied = result.unwrap();
        assert_eq!(applied, vec![SETTING_GITHUB_TOKEN]);
    });
}

#[test]
fn batch_update_rejects_corp_locked() {
    with_temp_configs(
        vec![],
        vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))],
        |_, _| {
            let mut changes = HashMap::new();
            changes.insert(SETTING_GITHUB_ALLOW.to_string(), SettingValue::Bool(true));
            let result = loader::batch_update_profile_settings(&changes);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("corp-locked"));
        },
    );
}

#[test]
fn batch_update_rejects_mixed_batch_atomically() {
    with_temp_configs(
        vec![],
        vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))],
        |user_path, _| {
            let mut changes = HashMap::new();
            // One valid change
            changes.insert(
                SETTING_GITHUB_TOKEN.to_string(),
                SettingValue::Text(
                    "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                        .into(),
                ),
            );
            // One corp-locked change
            changes.insert(SETTING_GITHUB_ALLOW.to_string(), SettingValue::Bool(true));
            let result = loader::batch_update_profile_settings(&changes);
            assert!(result.is_err(), "mixed batch should be rejected");

            // Verify nothing was written (atomic rejection)
            let file = loader::load_settings_file(user_path).unwrap();
            assert!(
                !file.settings.contains_key(SETTING_GITHUB_TOKEN),
                "valid change should NOT be written when batch is rejected"
            );
        },
    );
}

#[test]
fn batch_update_rejects_unknown_setting_id() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        changes.insert("nonexistent.setting".to_string(), SettingValue::Bool(true));
        let result = loader::batch_update_settings(&changes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown setting"));
    });
}

#[test]
fn batch_update_settings_rejects_profile_owned_setting_ids() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        changes.insert(
            "vm.resources.cpu_count".to_string(),
            SettingValue::Number(8),
        );
        let result = loader::batch_update_settings(&changes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("profile-owned setting"));
    });
}

#[test]
fn batch_update_rejects_retired_web_decision_setting_ids() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        for retired_id in [
            "security.web.allow_read",
            "security.web.allow_write",
            "security.web.custom_allow",
            "security.web.custom_block",
        ] {
            changes.insert(retired_id.to_string(), SettingValue::Bool(true));
            let result = loader::batch_update_settings(&changes);
            assert!(result.is_err(), "{retired_id} should be rejected");
            assert!(result.unwrap_err().contains("unknown setting"));
            changes.clear();
        }
    });
}

#[test]
fn batch_update_allows_dynamic_guest_env() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        changes.insert(
            "guest.env.MY_VAR".to_string(),
            SettingValue::Text("hello".into()),
        );
        let result = loader::batch_update_profile_settings(&changes);
        assert!(result.is_ok(), "dynamic guest.env.* should be allowed");
    });
}

#[test]
fn batch_update_empty_is_noop() {
    with_temp_configs(vec![], vec![], |_, _| {
        let changes = HashMap::new();
        let result = loader::batch_update_settings(&changes);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    });
}

#[test]
fn load_settings_response_returns_all_fields() {
    with_temp_configs(vec![], vec![], |_, _| {
        let response = loader::load_settings_response();
        assert!(!response.tree.is_empty(), "tree should not be empty");
        assert!(response
            .issues
            .iter()
            .all(|issue| !issue.id.is_empty() && !issue.message.is_empty()));
    });
}

// -----------------------------------------------------------------------
// .git-credentials generation tests
// -----------------------------------------------------------------------

#[test]
fn git_credentials_not_generated_from_github_token_settings() {
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                    .into(),
            ),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap_or_default();
    assert!(!files.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(!files.iter().any(|f| f.path == "/root/.gitconfig"));
}

#[test]
fn git_credentials_not_generated_from_multiple_provider_settings() {
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                    .into(),
            ),
        ),
        (SETTING_GITLAB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITLAB_TOKEN,
            SettingValue::Text("glpat-test456".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap_or_default();
    assert!(!files.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(!files.iter().any(|f| f.path == "/root/.gitconfig"));
}

#[test]
fn git_credentials_not_generated_when_allow_false() {
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(false)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text("ghp_test123".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when allow=false"
    );
}

#[test]
fn git_credentials_not_generated_when_token_empty() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when token is empty"
    );
}

#[test]
fn git_credentials_not_generated_when_corp_blocks() {
    let user = file_with(vec![(
        SETTING_GITHUB_TOKEN,
        SettingValue::Text("ghp_test123".into()),
    )]);
    let corp = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &corp);
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when corp blocks provider"
    );
}

#[test]
fn git_credentials_rejects_token_with_special_chars() {
    // Newlines
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text("ghp_test\ninjected".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when token contains newlines"
    );

    // @ sign (could inject a different host)
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text("ghp_test@evil.com".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when token contains @"
    );

    // : colon (could break URL structure)
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text("ghp_test:injected".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when token contains :"
    );
}

#[test]
fn git_credentials_gitconfig_not_generated_without_tokens() {
    // No tokens at all -- neither .git-credentials nor .gitconfig should exist
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    let has_gitconfig = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.gitconfig"));
    assert!(
        !has_creds,
        ".git-credentials should not exist without tokens"
    );
    assert!(!has_gitconfig, ".gitconfig should not exist without tokens");
}

// -----------------------------------------------------------------------
// Git identity env var tests
// -----------------------------------------------------------------------

#[test]
fn git_identity_env_vars_injected() {
    let user = file_with(vec![
        (
            "repository.git.identity.author_name",
            SettingValue::Text("Test User".into()),
        ),
        (
            "repository.git.identity.author_email",
            SettingValue::Text("test@example.com".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert_eq!(env.get("GIT_AUTHOR_NAME").unwrap(), "Test User");
    assert_eq!(env.get("GIT_COMMITTER_NAME").unwrap(), "Test User");
    assert_eq!(env.get("GIT_AUTHOR_EMAIL").unwrap(), "test@example.com");
    assert_eq!(env.get("GIT_COMMITTER_EMAIL").unwrap(), "test@example.com");
}

#[test]
fn git_identity_env_vars_absent_when_empty() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(
        !env.contains_key("GIT_AUTHOR_NAME"),
        "GIT_AUTHOR_NAME should not be set when empty"
    );
    assert!(
        !env.contains_key("GIT_COMMITTER_NAME"),
        "GIT_COMMITTER_NAME should not be set when empty"
    );
    assert!(
        !env.contains_key("GIT_AUTHOR_EMAIL"),
        "GIT_AUTHOR_EMAIL should not be set when empty"
    );
    assert!(
        !env.contains_key("GIT_COMMITTER_EMAIL"),
        "GIT_COMMITTER_EMAIL should not be set when empty"
    );
}

// -----------------------------------------------------------------------
// Repository section definitions tests
// -----------------------------------------------------------------------

#[test]
fn repository_settings_exist_in_definitions() {
    let defs = setting_definitions();
    let ids = [
        "repository.git.identity.author_name",
        "repository.git.identity.author_email",
        SETTING_GITHUB_ALLOW,
        "repository.providers.github.domains",
        SETTING_GITHUB_TOKEN,
        SETTING_GITLAB_ALLOW,
        "repository.providers.gitlab.domains",
        SETTING_GITLAB_TOKEN,
    ];
    for id in &ids {
        assert!(
            defs.iter().any(|d| d.id == *id),
            "missing setting definition: {id}"
        );
    }
}

#[test]
fn default_github_allowed_gitlab_not() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gh = resolved
        .iter()
        .find(|s| s.id == SETTING_GITHUB_ALLOW)
        .unwrap();
    assert_eq!(gh.effective_value, SettingValue::Bool(true));
    let gl = resolved
        .iter()
        .find(|s| s.id == SETTING_GITLAB_ALLOW)
        .unwrap();
    assert_eq!(gl.effective_value, SettingValue::Bool(false));
}

#[test]
fn setting_id_constants_exist_in_registry() {
    let defs = setting_definitions();
    let ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    for constant in [
        SETTING_GITHUB_ALLOW,
        SETTING_GITHUB_TOKEN,
        SETTING_GITLAB_ALLOW,
        SETTING_GITLAB_TOKEN,
    ] {
        assert!(
            ids.contains(&constant),
            "constant '{constant}' not found in setting_definitions()"
        );
    }
}

// -----------------------------------------------------------------------
// GH_TOKEN / GITLAB_TOKEN materialization guards
// -----------------------------------------------------------------------

#[test]
fn gh_token_not_materialized_when_github_enabled() {
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                    .into(),
            ),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("GH_TOKEN"));
    assert!(!env.contains_key("GITHUB_TOKEN"));
}

#[test]
fn gitlab_token_not_materialized_when_gitlab_enabled() {
    let user = file_with(vec![
        (SETTING_GITLAB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITLAB_TOKEN,
            SettingValue::Text(
                "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222"
                    .into(),
            ),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("GITLAB_TOKEN"));
}

#[test]
fn gh_token_not_injected_when_token_empty() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(
        !env.contains_key("GH_TOKEN"),
        "GH_TOKEN should not be set when token is empty"
    );
    assert!(
        !env.contains_key("GITHUB_TOKEN"),
        "GITHUB_TOKEN should not be set when token is empty"
    );
}

// -----------------------------------------------------------------------
// Prefix metadata tests
// -----------------------------------------------------------------------

#[test]
fn token_settings_have_prefix_metadata() {
    let defs = setting_definitions();
    let gh = defs.iter().find(|d| d.id == SETTING_GITHUB_TOKEN).unwrap();
    assert_eq!(gh.metadata.prefix.as_deref(), Some("ghp_"));
    let gl = defs.iter().find(|d| d.id == SETTING_GITLAB_TOKEN).unwrap();
    assert_eq!(gl.metadata.prefix.as_deref(), Some("glpat-"));
}

// -----------------------------------------------------------------------
// Setting ID migration
// -----------------------------------------------------------------------

#[test]
fn migrate_old_setting_ids() {
    let mut file = file_with(vec![
        ("web.defaults.allow_read", SettingValue::Bool(true)),
        ("web.custom_allow", SettingValue::Text("example.com".into())),
        ("registry.npm.allow", SettingValue::Bool(false)),
        ("web.search.google.allow", SettingValue::Bool(true)),
    ]);
    migrate_setting_ids(&mut file);

    // Old keys removed
    assert!(file.settings.contains_key("web.defaults.allow_read"));
    assert!(file.settings.contains_key("web.custom_allow"));
    assert!(!file.settings.contains_key("registry.npm.allow"));
    assert!(!file.settings.contains_key("web.search.google.allow"));

    // Live service keys still migrate; retired web decision keys do not.
    assert!(!file.settings.contains_key("security.web.allow_read"));
    assert!(!file.settings.contains_key("security.web.custom_allow"));
    assert_eq!(
        file.settings["security.services.registry.npm.allow"].value,
        SettingValue::Bool(false)
    );
    assert_eq!(
        file.settings["security.services.search.google.allow"].value,
        SettingValue::Bool(true)
    );
}

#[test]
fn migrate_does_not_clobber_existing_new_keys() {
    let mut file = SettingsFile::default();
    file.settings.insert(
        "web.search.google.allow".to_string(),
        SettingEntry {
            value: SettingValue::Bool(true),
            modified: now_str(),
        },
    );
    file.settings.insert(
        "security.services.search.google.allow".to_string(),
        SettingEntry {
            value: SettingValue::Bool(false),
            modified: now_str(),
        },
    );
    migrate_setting_ids(&mut file);

    // New key keeps its value, old key is dropped
    assert_eq!(
        file.settings["security.services.search.google.allow"].value,
        SettingValue::Bool(false)
    );
    assert!(!file.settings.contains_key("web.search.google.allow"));
}

// -----------------------------------------------------------------------
// Q: MergedPolicies basic construction (6)
// -----------------------------------------------------------------------

fn file_with_mcp(
    entries: Vec<(&str, SettingValue)>,
    mcp: crate::mcp::policy::McpUserConfig,
) -> SettingsFile {
    let mut f = file_with(entries);
    f.mcp = Some(mcp);
    f
}

#[test]
fn merged_defaults_only() {
    let m = MergedPolicies::from_files(&empty_file(), &empty_file());
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
    assert!(has_security_rule(&m, "profiles.rules.default_dns"));
}

#[test]
fn merged_user_enables_provider() {
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &empty_file());
    assert!(has_security_rule(
        &m,
        "profiles.rules.ai_anthropic_http_api"
    ));
}

#[test]
fn merged_user_enables_search() {
    let user = file_with(vec![(
        "security.services.search.google.allow",
        SettingValue::Bool(true),
    )]);
    let m = MergedPolicies::from_files(&user, &empty_file());
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_all_policies_populated() {
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &empty_file());
    assert!(!m.security_rules.rules().is_empty());
    // Guest config still carries non-secret built-in shell env defaults.
    assert!(m.guest.env.is_some());
    // VM settings have defaults
    assert!(m.vm.cpu_count.is_some());
}

// -----------------------------------------------------------------------
// S: Corp override persistence (11)
// -----------------------------------------------------------------------

#[test]
fn corp_forces_provider_on() {
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
    let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &corp);
    assert!(has_security_rule(
        &m,
        "profiles.rules.ai_anthropic_http_api"
    ));
}

#[test]
fn corp_forces_provider_off() {
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
    let m = MergedPolicies::from_files(&user, &corp);
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn corp_sets_api_key() {
    let user = file_with(vec![(
        "ai.openai.api_key",
        SettingValue::Text(
            "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
                .into(),
        ),
    )]);
    let corp = file_with(vec![(
        "ai.openai.api_key",
        SettingValue::Text(
            "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222"
                .into(),
        ),
    )]);
    let m = MergedPolicies::from_files(&user, &corp);
    let env = m.guest.env.unwrap_or_default();
    assert!(!env.contains_key("OPENAI_API_KEY"));
}

#[test]
fn corp_sets_network_mechanics_ports() {
    let user = empty_file();
    let corp = file_with(vec![(
        "security.web.http_upstream_ports",
        SettingValue::IntList(vec![80]),
    )]);
    let resolved = resolve_settings(&user, &corp);
    let ports = resolved
        .iter()
        .find(|setting| setting.id == "security.web.http_upstream_ports")
        .unwrap();
    assert_eq!(ports.effective_value, SettingValue::IntList(vec![80]));
    assert_eq!(ports.source, PolicySource::Corp);
}

#[test]
fn retired_web_decision_settings_are_not_resolved() {
    let user = file_with(vec![
        ("security.web.allow_read", SettingValue::Bool(true)),
        ("security.web.allow_write", SettingValue::Bool(true)),
        (
            "security.web.custom_allow",
            SettingValue::Text("internal.corp.com".into()),
        ),
        (
            "security.web.custom_block",
            SettingValue::Text("evil.com".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    for retired_id in [
        "security.web.allow_read",
        "security.web.allow_write",
        "security.web.custom_allow",
        "security.web.custom_block",
    ] {
        assert!(
            resolved.iter().all(|setting| setting.id != retired_id),
            "{retired_id} must not be a resolved setting"
        );
    }
}

// -----------------------------------------------------------------------
// T: Invalid / missing / corrupt inputs (13)
// -----------------------------------------------------------------------

#[test]
fn merged_from_missing_user_toml() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent = dir.path().join("missing_user.toml");
    let user = load_settings_file(&nonexistent).unwrap_or_default();
    let m = MergedPolicies::from_files(&user, &empty_file());
    // Should produce valid defaults without panicking
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_from_missing_corp_toml() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent = dir.path().join("missing_corp.toml");
    let corp = load_settings_file(&nonexistent).unwrap_or_default();
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &corp);
    assert!(has_security_rule(
        &m,
        "profiles.rules.ai_anthropic_http_api"
    ));
}

#[test]
fn merged_from_both_missing() {
    let dir = tempfile::tempdir().unwrap();
    let u = load_settings_file(&dir.path().join("u.toml")).unwrap_or_default();
    let c = load_settings_file(&dir.path().join("c.toml")).unwrap_or_default();
    let m = MergedPolicies::from_files(&u, &c);
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_from_invalid_user_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "not valid {{{{ toml").unwrap();
    let result = load_settings_file(&path);
    assert!(result.is_err());
    // Fallback to default still works
    let user = result.unwrap_or_default();
    let m = MergedPolicies::from_files(&user, &empty_file());
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_from_invalid_corp_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad_corp.toml");
    std::fs::write(&path, "garbage!!!!").unwrap();
    let result = load_settings_file(&path);
    assert!(result.is_err());
    let corp = result.unwrap_or_default();
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    let m = MergedPolicies::from_files(&user, &corp);
    assert!(has_security_rule(
        &m,
        "profiles.rules.ai_anthropic_http_api"
    ));
}

#[test]
fn merged_ignores_unknown_setting_ids() {
    let user = file_with(vec![
        ("nonexistent.setting.foo", SettingValue::Bool(true)),
        ("ai.anthropic.allow", SettingValue::Bool(true)),
    ]);
    let m = MergedPolicies::from_files(&user, &empty_file());
    // Should not crash, anthropic should still work
    assert!(has_security_rule(
        &m,
        "profiles.rules.ai_anthropic_http_api"
    ));
}

#[test]
fn merged_wrong_type_for_bool_setting() {
    // SettingValue::Text for a Bool-type setting -- resolve will use default
    let user = file_with(vec![(
        "ai.anthropic.allow",
        SettingValue::Text("yes".into()),
    )]);
    let m = MergedPolicies::from_files(&user, &empty_file());
    // Provider detection/default rules are independent from legacy allow
    // toggles; malformed toggle values do not create network decisions.
    assert!(has_security_rule(
        &m,
        "profiles.rules.ai_anthropic_http_api"
    ));
}

#[test]
fn merged_wrong_type_for_number_setting() {
    let user = file_with(vec![(
        "vm.resources.cpu_count",
        SettingValue::Text("four".into()),
    )]);
    let m = MergedPolicies::from_files(&user, &empty_file());
    // as_number() returns None -> falls back to default (4)
    assert_eq!(m.vm.cpu_count, Some(4));
}

#[test]
fn merged_retired_custom_allow_setting_is_ignored() {
    let user = file_with(vec![(
        "security.web.custom_allow",
        SettingValue::Text("".into()),
    )]);
    let m = MergedPolicies::from_files(&user, &empty_file());
    // Should not crash, empty string -> no domains added
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_empty_mcp_section() {
    use crate::mcp::policy::McpUserConfig;
    let user = file_with_mcp(vec![], McpUserConfig::default());
    let m = MergedPolicies::from_files(&user, &empty_file());
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

// -----------------------------------------------------------------------
// retired callback policy compatibility
// -----------------------------------------------------------------------

#[test]
fn settings_file_rejects_old_policy_tables() {
    let error = toml::from_str::<SettingsFile>(
        r#"
[policy.http.block_openai_github]
on = "http.request"
if = 'http.host == "github.com"'
decision = "block"
priority = 10
"#,
    )
    .expect_err("old policy tables must not deserialize");

    assert!(
        error.to_string().contains("unknown field") || error.to_string().contains("policy"),
        "{error}"
    );
}

#[test]
fn batch_update_settings_json_rejects_old_policy_rule_shape_atomically() {
    with_temp_configs(vec![], vec![], |user_path, _| {
        let mut changes = HashMap::new();
        changes.insert(
            SETTING_GITHUB_TOKEN.to_string(),
            serde_json::json!("credential:blake3:0000000000000000000000000000000000000000000000000000000000000000"),
        );
        changes.insert(
            "policy.http.block_openai_github".to_string(),
            serde_json::json!({
                "on": "http.request",
                "if": "http.host == 'github.com'",
                "decision": "block",
                "priority": 10
            }),
        );

        let error = loader::batch_update_profile_settings_json(&changes)
            .expect_err("old policy writes must reject");
        assert!(
            error.contains("unknown setting: policy.http.block_openai_github"),
            "{error}"
        );
        let loaded = loader::load_settings_file(user_path).unwrap();
        assert!(
            loaded.settings.is_empty(),
            "batch rejection must leave the settings file unchanged"
        );
    });
}

#[test]
fn settings_file_parses_provider_security_rules_under_ai_provider_sections() {
    let file: SettingsFile = toml::from_str(
        r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_api_observed"
action = "allow"
detection_level = "informational"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .expect("provider security rules parse inside settings file");

    assert!(file.ai.contains_key("openai"));
    let rules = ProviderRuleProfile {
        ai: file.ai.clone(),
    }
    .compile_rule_set(SecurityRuleSource::User)
    .expect("provider security rules compile");
    assert!(rules
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api"));

    let policies = MergedPolicies::from_files(&file, &SettingsFile::default());
    assert!(policies
        .security_rules
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api"));
}

#[test]
fn settings_file_parses_discovery_only_provider_record() {
    let file: SettingsFile = toml::from_str(
        r#"
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "http.header.authorization"
event_type = "http.request"
confidence = 1.0
credential_ref = "credential:blake3:0000000000000000000000000000000000000000000000000000000000000000"
trace_id = "trace-openai"
"#,
    )
    .expect("discovery-only provider records are valid settings TOML");

    let discovery = file.ai["openai"].discovery.as_ref().unwrap();
    assert_eq!(discovery.event_type.as_deref(), Some("http.request"));
    assert_eq!(
        discovery.credential_ref.as_deref(),
        Some("credential:blake3:0000000000000000000000000000000000000000000000000000000000000000")
    );

    let policies = MergedPolicies::from_files(&file, &SettingsFile::default());
    assert_eq!(
        policies.model_endpoints.protocol_for_host("api.openai.com"),
        Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi)
    );
    assert!(policies
        .security_rules
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api"));
}

#[test]
fn provider_discovery_rejects_unknown_event_type_and_raw_secret_reference() {
    let stale_event_type = toml::from_str::<SettingsFile>(
        r#"
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "old-observer"
event_type = "mcp.request"
confidence = 1.0
"#,
    )
    .expect("serde accepts the shape before provider validation");
    let profile = ProviderRuleProfile {
        ai: stale_event_type.ai,
    };
    assert!(
        profile.validate().is_err(),
        "provider discovery must use canonical runtime event types"
    );

    let raw_secret = toml::from_str::<SettingsFile>(
        r#"
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "old-observer"
event_type = "http.request"
confidence = 1.0
credential_ref = "sk-raw-secret"
"#,
    )
    .expect("serde accepts the shape before provider validation");
    let profile = ProviderRuleProfile { ai: raw_secret.ai };
    assert!(
        profile.validate().is_err(),
        "provider discovery must never accept raw credentials"
    );
}

#[test]
fn tool_config_sources_are_rejected_from_settings_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("user.toml");
    std::fs::write(
        &path,
        r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
observed_hash = "blake3:0000000000000000000000000000000000000000000000000000000000000000"
observed_version = "2026-06-06"
inferred_endpoint_ref = "ai.openai"
credential_refs = ["credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"]
allowed_overlays = ["mcp_injection", "broker_placeholders"]
"#,
    )
    .unwrap();

    let error = load_settings_file(&path).expect_err("tool_config_sources is runtime evidence");
    assert!(error.contains("tool_config_sources"), "{error}");
}

#[test]
fn tool_config_sources_are_not_a_static_credential_escape_hatch() {
    let cases = [
        (
            "raw credential ref",
            r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
credential_refs = ["sk-raw-secret"]
"#,
        ),
        (
            "rendered content field",
            r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
content = "api_key = 'sk-raw-secret'"
"#,
        ),
        (
            "bad hash",
            r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
observed_hash = "abc123"
"#,
        ),
        (
            "bad endpoint ref",
            r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
inferred_endpoint_ref = "openai"
"#,
        ),
    ];

    for (name, toml_text) in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user.toml");
        std::fs::write(&path, toml_text).unwrap();
        let error = load_settings_file(&path).expect_err("tool_config_sources is retired");
        assert!(error.contains("tool_config_sources"), "{name}: {error}");
    }
}

#[test]
fn settings_loader_rejects_raw_provider_credentials_but_accepts_broker_refs() {
    let dir = tempfile::tempdir().unwrap();
    let valid_path = dir.path().join("valid.toml");
    std::fs::write(
        &valid_path,
        r#"
[settings]
"repository.providers.github.token" = { value = "", modified = "2026-06-06T10:00:00Z" }
"#,
    )
    .unwrap();
    let valid_result = load_settings_file(&valid_path);
    assert!(
        valid_result.is_ok(),
        "broker refs and empty credential settings are allowed: {valid_result:?}"
    );

    let raw_path = dir.path().join("raw.toml");
    std::fs::write(
        &raw_path,
        r#"
[settings]
"ai.openai.api_key" = { value = "sk-raw-openai", modified = "2026-06-06T10:00:00Z" }
"#,
    )
    .unwrap();
    let error = load_settings_file(&raw_path).expect_err("raw provider credential must fail");
    assert!(
        error.contains("retired AI setting id ai.openai.api_key"),
        "error should reject retired AI setting ids: {error}"
    );
}

#[test]
fn batch_update_settings_rejects_raw_provider_credentials_atomically() {
    with_temp_configs(vec![], vec![], |user_path, _| {
        let mut changes = HashMap::new();
        changes.insert(
            "ai.openai.api_key".to_string(),
            serde_json::json!("sk-raw-openai"),
        );

        let result = loader::batch_update_profile_settings_json(&changes);
        let error = result.expect_err("retired API key writes must be rejected");
        assert!(error.contains("unknown setting"), "{error}");
        let loaded = loader::load_settings_file(user_path).unwrap();
        assert!(
            !loaded.settings.contains_key("ai.openai.api_key"),
            "raw rejected setting must not be written"
        );
    });
}

#[test]
fn builtin_provider_rules_compile_only_into_security_rules() {
    let policies = MergedPolicies::from_files(&SettingsFile::default(), &SettingsFile::default());
    let rule_ids = policies
        .security_rules
        .rules()
        .iter()
        .map(|rule| rule.rule_id.as_str())
        .collect::<Vec<_>>();

    assert!(rule_ids.contains(&"profiles.rules.ai_openai_http_api"));
    assert!(rule_ids.contains(&"profiles.rules.ai_ollama_http_local_host"));
    assert!(rule_ids.contains(&"profiles.rules.ai_google_dns_googleapis"));
    assert!(
        rule_ids.iter().all(|id| !id.starts_with("policy.")),
        "provider rules must not be mirrored into the retired callback policy rail"
    );
}

#[test]
fn merged_policies_compile_profile_and_corp_security_rules() {
    let user = SettingsFile {
        profiles: SecurityRuleProfile::parse_toml(
            r#"
[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
match = 'file.read.path.contains("skills/")'
"#,
        )
        .unwrap()
        .profiles,
        ..Default::default()
    };
    let corp = SettingsFile {
        corp: SecurityRuleProfile::parse_toml(
            r#"
[corp.rules.block_openai]
name = "block_openai"
action = "block"
detection_level = "critical"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
        )
        .unwrap()
        .corp,
        ..Default::default()
    };

    let policies = MergedPolicies::from_files(&user, &corp);
    let ids: Vec<_> = policies
        .security_rules
        .rules()
        .iter()
        .map(|rule| (rule.rule_id.as_str(), rule.priority))
        .collect();

    assert!(ids.contains(&("profiles.rules.skill_loaded", 10)));
    assert!(ids.contains(&("corp.rules.block_openai", -10)));
}

#[test]
fn merged_policies_carry_live_model_endpoint_registry() {
    let user: SettingsFile = toml::from_str(
        r#"
[ai.private_gateway]
name = "Private Gateway"
protocol = "openai-compatible"
url = "https://llm.internal.example/v1"
aliases = ["company-openai"]
listen_ports = [443, 8443]
allowed_remote_targets = ["llm.internal.example:443", "company-openai:8443"]

[ai.private_gateway.rules.http_api]
name = "private_gateway_http_seen"
action = "allow"
match = 'http.host == "llm.internal.example"'
"#,
    )
    .expect("settings parse");

    let policies = MergedPolicies::from_files(&user, &SettingsFile::default());

    assert_eq!(
        policies
            .model_endpoints
            .protocol_for_host("llm.internal.example"),
        Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi)
    );
    assert_eq!(
        policies.model_endpoints.protocol_for_host("api.openai.com"),
        Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi)
    );
    assert_eq!(
        policies
            .model_endpoints
            .protocol_for_target("company-openai", 8443),
        Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi)
    );
    assert_eq!(
        policies
            .model_endpoints
            .protocol_for_target("company-openai", 11434),
        None
    );
    let endpoint = policies
        .model_endpoints
        .get("private_gateway")
        .expect("private endpoint");
    assert_eq!(endpoint.provider_id, "private_gateway");
    assert_eq!(
        endpoint.allowed_remote_targets,
        vec!["llm.internal.example:443", "company-openai:8443"]
    );
}

#[test]
fn load_settings_file_merges_referenced_sigma_into_security_rules() {
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("user.toml");
    std::fs::write(
        dir.path().join("detection.yaml"),
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
"#,
    )
    .unwrap();
    std::fs::write(
        &settings_path,
        r#"
[rule_files]
sigma = "detection.yaml"
"#,
    )
    .unwrap();

    let user = load_settings_file(&settings_path).expect("settings load");
    let policies = MergedPolicies::from_files(&user, &SettingsFile::default());
    let rule = policies
        .security_rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.openai_traffic_to_unexpected_endpoint")
        .expect("referenced Sigma rule compiles into runtime rules");

    assert_eq!(rule.action, SecurityRuleAction::Block);
    assert_eq!(rule.detection_level, Some(DetectionLevel::High));
}

#[test]
fn provider_security_rules_merge_corp_block_with_rule_priority() {
    let corp: SettingsFile = toml::from_str(
        r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_api_corp_block"
action = "block"
detection_level = "critical"
priority = -100
corp_locked = true
reason = "OpenAI blocked by corporate policy"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .unwrap();

    let merged = ProviderRuleProfile::merge_defaults_user_and_corp(
        &ProviderRuleProfile::default(),
        &ProviderRuleProfile {
            ai: corp.ai.clone(),
        },
    )
    .expect("provider rules merge");
    let rules = merged
        .compile_rule_set(SecurityRuleSource::Corp)
        .expect("merged provider rules compile");
    let rule = rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api")
        .expect("corp provider rule exists");
    assert_eq!(rule.name, "openai_http_api_corp_block");
    assert_eq!(rule.action, SecurityRuleAction::Block);
    assert_eq!(rule.priority, -100);
    assert_eq!(rule.detection_level, Some(DetectionLevel::Critical));
}

#[test]
fn provider_discovery_and_user_allow_cannot_reenable_corp_blocked_provider() {
    let user: SettingsFile = toml::from_str(
        r#"
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "http.header.authorization"
event_type = "http.request"
confidence = 1.0
credential_ref = "credential:blake3:0000000000000000000000000000000000000000000000000000000000000000"

[ai.openai.rules.http_api]
name = "openai_http_api_user_allow"
action = "allow"
priority = 100
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .unwrap();
    let corp: SettingsFile = toml::from_str(
        r#"
[ai.openai.rules.http_api]
name = "openai_http_api_corp_block"
action = "block"
detection_level = "critical"
priority = -100
corp_locked = true
reason = "OpenAI blocked by corporate policy"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .unwrap();

    let policies = MergedPolicies::from_files(&user, &corp);
    let rule = policies
        .security_rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api")
        .expect("provider rule id should exist");
    assert_eq!(rule.name, "openai_http_api_corp_block");
    assert_eq!(rule.action, SecurityRuleAction::Block);
    assert_eq!(rule.priority, -100);
    assert!(rule.corp_locked);

    let event = serde_json::json!({
        "http": {
            "host": "api.openai.com"
        }
    });
    let evaluation = policies
        .security_rules
        .evaluate(&event)
        .expect("security event evaluates");
    assert!(
        evaluation
            .rules_for_action(SecurityRuleAction::Allow)
            .iter()
            .all(|rule| rule.rule_id != "profiles.rules.ai_openai_http_api"),
        "user provider allow rule must be replaced by the corp block, not matched alongside it"
    );
    assert_eq!(
        evaluation.enforcement_rules()[0].rule_id,
        "profiles.rules.ai_openai_http_api"
    );
}

#[test]
fn load_settings_response_does_not_expose_provider_status() {
    let _guard = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();

    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("user.toml");
    let corp_path = dir.path().join("corp.toml");
    std::fs::write(
        &user_path,
        r#"
[settings]
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "http.header.authorization"
event_type = "http.request"
confidence = 1.0
credential_ref = "credential:blake3:0000000000000000000000000000000000000000000000000000000000000000"
"#,
    )
    .unwrap();
    std::fs::write(
        &corp_path,
        r#"
[ai.openai.rules.http_api]
name = "openai_http_api_corp_block"
action = "block"
priority = -100
corp_locked = true
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .unwrap();
    let _user_config = EnvVarGuard::set("CAPSEM_USER_CONFIG", &user_path);
    let _corp_config = EnvVarGuard::set("CAPSEM_CORP_CONFIG", &corp_path);

    let serialized =
        serde_json::to_value(load_settings_response()).expect("settings response serializes");
    assert!(
        serialized.get("providers").is_none(),
        "settings response must not expose provider status"
    );
    assert!(
        serialized.get("tool_config_sources").is_none(),
        "settings response must not expose runtime tool config observations"
    );
    assert!(
        serialized.get("policy").is_none(),
        "settings response must not expose retired policy payloads"
    );
}

#[test]
fn load_settings_response_exposes_settings_tree_only() {
    let _guard = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();

    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("user.toml");
    let corp_path = dir.path().join("corp.toml");
    write_settings_file(&user_path, &SettingsFile::default()).unwrap();
    write_settings_file(&corp_path, &SettingsFile::default()).unwrap();
    let _user_config = EnvVarGuard::set("CAPSEM_USER_CONFIG", &user_path);
    let _corp_config = EnvVarGuard::set("CAPSEM_CORP_CONFIG", &corp_path);

    let serialized =
        serde_json::to_value(load_settings_response()).expect("settings response serializes");
    assert!(
        serialized.get("tree").is_some(),
        "settings response must expose the settings tree"
    );
    assert!(
        serialized.get("issues").is_some(),
        "settings response must expose config issues"
    );
    assert!(
        serialized.get("providers").is_none(),
        "provider state belongs to profile rules and plugin/runtime status, not settings"
    );
    assert!(
        serialized.get("policy").is_none(),
        "retired policy maps must stay out of settings response"
    );
}

#[test]
fn merged_partial_settings_file() {
    // TOML with only [mcp] section, no [settings]
    use crate::mcp::policy::McpUserConfig;
    let user = SettingsFile {
        settings: HashMap::new(),
        mcp: Some(McpUserConfig {
            health_check_interval_secs: Some(30),
            ..Default::default()
        }),
        ..Default::default()
    };
    let m = MergedPolicies::from_files(&user, &empty_file());
    // No settings -> defaults for everything else
    assert!(has_security_rule(&m, "profiles.rules.default_http"));
}

#[test]
fn merged_partial_settings_only() {
    // Settings but no MCP section
    let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
    assert!(user.mcp.is_none());
    let m = MergedPolicies::from_files(&user, &empty_file());
    // Settings applied
    assert!(has_security_rule(
        &m,
        "profiles.rules.ai_anthropic_http_api"
    ));
}

#[test]
fn merged_settings_expose_typed_plugin_policy_with_corp_override() {
    let user: SettingsFile = toml::from_str(
        r#"
[plugins]
[plugins.dummy_pre]
mode = "rewrite"
detection_level = "medium"

[plugins.dummy_post]
mode = "allow"
"#,
    )
    .expect("user plugin policy parses");
    let corp: SettingsFile = toml::from_str(
        r#"
[plugins.dummy_post]
mode = "block"
detection_level = "critical"

[plugins.dummy_disabled]
mode = "disable"
"#,
    )
    .expect("corp plugin policy parses");

    let merged = MergedPolicies::from_files(&user, &corp);

    assert_eq!(
        merged.plugins["dummy_pre"].mode,
        SecurityPluginMode::Rewrite
    );
    assert_eq!(
        merged.plugins["dummy_pre"].detection_level,
        DetectionLevel::Medium
    );
    assert_eq!(merged.plugins["dummy_post"].mode, SecurityPluginMode::Block);
    assert_eq!(
        merged.plugins["dummy_post"].detection_level,
        DetectionLevel::Critical
    );
    assert_eq!(
        merged.plugins["dummy_disabled"].mode,
        SecurityPluginMode::Disable
    );
    assert_eq!(
        merged.plugins["dummy_disabled"].active_detection_level(),
        None
    );
}

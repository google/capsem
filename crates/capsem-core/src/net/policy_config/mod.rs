//! Generic typed settings system with corp override.
//!
//! Each setting has an id, name, description, type, category, default value,
//! and optional `enabled_by` pointer to a parent toggle. Settings are stored
//! in TOML files at:
//!   - User: ~/.capsem/user.toml
//!   - Corporate: /etc/capsem/corp.toml
//!
//! Merge semantics: corp settings override user settings per-key.
//! User can only write user.toml. Corp file is read-only (MDM-distributed).

mod types;
mod registry;
mod loader;
mod presets;
mod resolver;
mod builder;
mod lint;
mod tree;

// Re-export everything to preserve the existing public API.
pub use types::*;
pub use registry::{setting_definitions, default_settings_file};
pub use loader::*;
pub use presets::*;
pub use resolver::*;
pub use builder::*;
pub use lint::*;
pub use tree::*;

// Re-export sibling types used by tests and downstream code.
pub use super::domain_policy::{Action, DomainPolicy};
pub use super::http_policy::{HttpPolicy, HttpRule};

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    use super::*;
    use super::builder::{inject_capsem_mcp_server, inject_capsem_mcp_server_toml, inject_api_key_approval};
    use std::collections::HashMap;

    fn empty_file() -> SettingsFile {
        SettingsFile::default()
    }

    fn now_str() -> String {
        "2026-02-25T00:00:00Z".to_string()
    }

    fn file_with(entries: Vec<(&str, SettingValue)>) -> SettingsFile {
        let mut settings = HashMap::new();
        for (id, value) in entries {
            settings.insert(id.to_string(), SettingEntry {
                value,
                modified: now_str(),
            });
        }
        SettingsFile { settings, mcp: None }
    }

    // -----------------------------------------------------------------------
    // A: Corp override (7)
    // -----------------------------------------------------------------------

    #[test]
    fn corp_override_bool() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(false));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_bool_web_defaults() {
        let user = file_with(vec![("security.web.allow_read", SettingValue::Bool(true))]);
        let corp = file_with(vec![("security.web.allow_read", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "security.web.allow_read").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(false));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_number() {
        let user = file_with(vec![("vm.resources.max_body_capture", SettingValue::Number(8192))]);
        let corp = file_with(vec![("vm.resources.max_body_capture", SettingValue::Number(1024))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "vm.resources.max_body_capture").unwrap();
        assert_eq!(s.effective_value, SettingValue::Number(1024));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_api_key() {
        let user = file_with(vec![("ai.anthropic.api_key", SettingValue::Text("user-key".into()))]);
        let corp = file_with(vec![("ai.anthropic.api_key", SettingValue::Text("corp-key".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("corp-key".into()));
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
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("vm.resources.log_bodies", SettingValue::Bool(true)),
            ("appearance.dark_mode", SettingValue::Bool(false)),
        ]);
        let corp = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("vm.resources.log_bodies", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &corp);

        let ai = resolved.iter().find(|s| s.id == "ai.anthropic.allow").unwrap();
        assert_eq!(ai.effective_value, SettingValue::Bool(false));
        assert_eq!(ai.source, PolicySource::Corp);

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
        // Corp blocks anthropic, user tries to enable
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(false));
        assert!(s.corp_locked);
    }

    #[test]
    fn user_cannot_change_corp_web_defaults() {
        let user = file_with(vec![("security.web.allow_read", SettingValue::Bool(true))]);
        let corp = file_with(vec![("security.web.allow_read", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "security.web.allow_read").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(false));
        assert!(s.corp_locked);
    }

    #[test]
    fn user_cannot_override_corp_api_key() {
        let user = file_with(vec![("ai.openai.api_key", SettingValue::Text("user-key".into()))]);
        let corp = file_with(vec![("ai.openai.api_key", SettingValue::Text("corp-key".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.openai.api_key").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("corp-key".into()));
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
            ("ai.anthropic.allow", SettingValue::Bool(true)),
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
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("vm.resources.log_bodies", SettingValue::Bool(false)),
        ]);
        write_settings_file(&path, &file).unwrap();

        // Update one setting
        file.settings.get_mut("vm.resources.log_bodies").unwrap().value = SettingValue::Bool(true);
        write_settings_file(&path, &file).unwrap();

        let loaded = load_settings_file(&path).unwrap();
        assert_eq!(
            loaded.settings.get("ai.anthropic.allow").unwrap().value,
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
            let s = resolved.iter().find(|s| s.id == *id).unwrap();
            assert_eq!(s.effective_value, SettingValue::Bool(true), "expected {id} to be true");
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

        let ar = resolved.iter().find(|s| s.id == "security.web.allow_read").unwrap();
        assert_eq!(ar.effective_value, SettingValue::Bool(false));

        let aw = resolved.iter().find(|s| s.id == "security.web.allow_write").unwrap();
        assert_eq!(aw.effective_value, SettingValue::Bool(false));

        let lb = resolved.iter().find(|s| s.id == "vm.resources.log_bodies").unwrap();
        assert_eq!(lb.effective_value, SettingValue::Bool(false));

        let mbc = resolved.iter().find(|s| s.id == "vm.resources.max_body_capture").unwrap();
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
        ids.sort();
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
            assert!(def.is_some(), "missing {domains_id} setting");
            let def = def.unwrap();
            assert_eq!(def.setting_type, SettingType::Text);
            assert!(def.enabled_by.is_some());
        }
    }

    #[test]
    fn web_defaults_are_bool_settings() {
        let defs = setting_definitions();
        let ar = defs.iter().find(|d| d.id == "security.web.allow_read").unwrap();
        assert_eq!(ar.setting_type, SettingType::Bool);
        let aw = defs.iter().find(|d| d.id == "security.web.allow_write").unwrap();
        assert_eq!(aw.setting_type, SettingType::Bool);
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
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        assert!(is_setting_corp_locked("ai.anthropic.allow", &corp));
        assert!(!is_setting_corp_locked("ai.openai.allow", &corp));
    }

    // -----------------------------------------------------------------------
    // G: enabled_by (4)
    // -----------------------------------------------------------------------

    #[test]
    fn enabled_by_parent_on_child_enabled() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let child = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert!(child.enabled);
        assert_eq!(child.enabled_by, Some("ai.anthropic.allow".to_string()));
    }

    #[test]
    fn enabled_by_parent_off_child_disabled() {
        // User explicitly disables anthropic
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let child = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
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
        // Only one level of enabled_by is supported.
        // When the toggle is off, api_key is disabled.
        let mut user = file_with(vec![("ai.openai.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let key = resolved.iter().find(|s| s.id == "ai.openai.api_key").unwrap();
        assert!(!key.enabled);

        // Turn on the toggle -> key is enabled
        user = file_with(vec![("ai.openai.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let key = resolved.iter().find(|s| s.id == "ai.openai.api_key").unwrap();
        assert!(key.enabled);
    }

    // -----------------------------------------------------------------------
    // H: Translation (5)
    // -----------------------------------------------------------------------

    #[test]
    fn settings_to_domain_policy_defaults() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);

        // Registries enabled by default -> domains allowed
        let (action, _) = dp.evaluate("github.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("pypi.org");
        assert_eq!(action, Action::Allow);

        // All AI providers enabled by default -> domains allowed
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Allow);

        // Google AI enabled by default -> domains allowed
        let (action, _) = dp.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Allow);

        // Unknown domains denied
        let (action, _) = dp.evaluate("example.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn settings_to_domain_policy_toggle_off_registry() {
        let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);

        let (action, _) = dp.evaluate("github.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn settings_to_domain_policy_toggle_on_provider() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);

        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
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

    #[test]
    fn settings_to_http_policy_from_metadata_rules() {
        let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let hp = settings_to_http_policy(&resolved);

        // github.com is allowed at domain level
        let d = hp.evaluate_domain("github.com");
        assert_eq!(d.action, Action::Allow);

        // GET should be allowed (from metadata rules)
        let d = hp.evaluate_request("github.com", "GET", "/repos/foo");
        assert_eq!(d.action, Action::Allow);
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
            ("ai.google.gemini.settings_json", SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: r#"{"key":"value"}"#.into(),
            }),
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
"security.web.allow_read" = { value = false, modified = "2026-01-01T00:00:00Z" }
"appearance.font_size" = { value = 16, modified = "2026-01-01T00:00:00Z" }
"#;
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse mixed types");
        assert_eq!(file.settings["vm.resources.log_bodies"].value, SettingValue::Bool(true));
        assert_eq!(file.settings["vm.resources.max_body_capture"].value, SettingValue::Number(8192));
        assert_eq!(file.settings["security.web.allow_read"].value, SettingValue::Bool(false));
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
        // value is an array -- not a valid SettingValue variant
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = [1, 2, 3], modified = "2026-01-01T00:00:00Z" }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "array value should fail deserialization");
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
        assert!(result.is_err(), "unquoted dotted keys should fail (creates nested tables)");
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
        // Parse from raw TOML, then resolve -- api_key settings must have
        // setting_type == ApiKey, not Text.
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-test", modified = "2026-01-01T00:00:00Z" }
"#;
        let user: SettingsFile = toml::from_str(toml_str).unwrap();
        let resolved = resolve_settings(&user, &empty_file());
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert_eq!(s.setting_type, SettingType::ApiKey, "api_key settings must have ApiKey type");
        assert_eq!(s.effective_value, SettingValue::Text("sk-test".into()));
    }

    #[test]
    fn parse_toml_serialized_format_roundtrips() {
        // Verify that toml::to_string_pretty output parses back correctly
        let file = file_with(vec![
            ("ai.google.api_key", SettingValue::Text("AIzaTest".into())),
            ("ai.anthropic.allow", SettingValue::Bool(true)),
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

        // Find a setting with empty metadata (e.g., api_key settings)
        let api_key = parsed.iter()
            .find(|v| v["id"] == "ai.anthropic.api_key")
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
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-test", modified = "2026-01-01T00:00:00Z" }
"#;
        let user: SettingsFile = toml::from_str(toml_str).unwrap();
        let resolved = resolve_settings(&user, &empty_file());
        let json = serde_json::to_string(&resolved).expect("should serialize to JSON");

        // Verify key fields are present in the JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();

        // Find the api_key setting
        let api_key = arr.iter()
            .find(|v| v["id"] == "ai.anthropic.api_key")
            .expect("should have ai.anthropic.api_key in JSON");
        assert_eq!(api_key["setting_type"], "apikey", "setting_type must be 'apikey' in JSON");
        assert_eq!(api_key["effective_value"], "sk-test");
        assert_eq!(api_key["enabled"], true);

        // Find a bool setting
        let allow = arr.iter()
            .find(|v| v["id"] == "ai.anthropic.allow")
            .expect("should have ai.anthropic.allow in JSON");
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
        let user = file_with(vec![("vm.resources.scratch_disk_size_gb", SettingValue::Number(32))]);
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
        let user = file_with(vec![("vm.resources.scratch_disk_size_gb", SettingValue::Number(32))]);
        let corp = file_with(vec![("vm.resources.scratch_disk_size_gb", SettingValue::Number(4))]);
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
    // J: Domain settings (4)
    // -----------------------------------------------------------------------

    #[test]
    fn domains_setting_drives_allow_list() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.domains", SettingValue::Text("*.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn domains_setting_drives_block_list() {
        // User disables anthropic, so domains go to block list
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn domains_setting_parsed_correctly() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.domains", SettingValue::Text("api.anthropic.com , console.anthropic.com , *.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("console.anthropic.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("new.anthropic.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn domains_setting_empty_skipped() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.domains", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // Empty domains text means nothing added to allow list
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "empty domains should not allow anything");
    }

    // -----------------------------------------------------------------------
    // K: Corp block enforcement (3)
    // -----------------------------------------------------------------------

    #[test]
    fn corp_blocked_domains_always_in_block_list() {
        // Corp locks ai.anthropic.allow = false
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        // User tries to empty the domains
        let user = file_with(vec![
            ("ai.anthropic.domains", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // Default domains (*.anthropic.com) should still be blocked
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked domains must stay blocked");
    }

    #[test]
    fn corp_blocked_domain_not_allowed_via_other_service() {
        // Corp blocks anthropic
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        // User adds api.anthropic.com to google domains and enables google
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(true)),
            ("ai.google.domains", SettingValue::Text("*.googleapis.com,api.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // api.anthropic.com should be blocked even though it's in google domains
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked domain must not be allowed via other service");
        // google domains should still work
        let (action, _) = dp.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn user_disabled_service_domains_in_block_list() {
        // User (not corp) disables a service
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Deny);
    }

    // -----------------------------------------------------------------------
    // K2: Stress tests -- block > allow > default invariants
    // -----------------------------------------------------------------------

    #[test]
    fn stress_disabled_provider_always_blocked_regardless_of_default() {
        // Provider explicitly off + default allow_read/write => domains must still be blocked.
        let user = file_with(vec![
            ("security.web.allow_read", SettingValue::Bool(true)),
            ("security.web.allow_write", SettingValue::Bool(true)),
            ("ai.anthropic.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "disabled provider must be blocked even with defaults=allow");
    }

    #[test]
    fn stress_enabled_provider_always_allowed_regardless_of_default() {
        // Provider on + default_action=deny => domains must still be allowed.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow, "enabled provider must be allowed even with default=deny");
    }

    #[test]
    fn stress_corp_block_beats_user_allow() {
        // Corp blocks anthropic, user enables it -- block must win.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp block must beat user allow");
    }

    #[test]
    fn stress_corp_block_beats_user_allow_with_default_allow() {
        // Corp blocks, user enables, default=allow -- still blocked.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("security.web.allow_read", SettingValue::Bool(true)),
            ("security.web.allow_write", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp block must beat user allow + default allow");
    }

    #[test]
    fn stress_corp_block_via_other_provider_wildcard() {
        // Corp blocks *.anthropic.com via anthropic toggle.
        // User adds *.anthropic.com to openai domains and enables openai.
        // Corp-blocked wildcard must still deny.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.openai.domains", SettingValue::Text("*.openai.com, *.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // anthropic subdomain must be blocked despite being in openai domains
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked wildcard must not be allowed via other provider");
        // openai subdomain should be allowed (not corp-blocked)
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn stress_corp_block_cannot_be_circumvented_by_emptying_domains() {
        // Corp blocks anthropic. User empties the domains field to try
        // removing the domains from the block list.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.anthropic.domains", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // Default domains should still be blocked (union of default + effective)
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp block must survive user emptying domains");
    }

    #[test]
    fn stress_corp_block_cannot_be_circumvented_by_changing_domains() {
        // Corp blocks anthropic. User changes domains to something else.
        // Both old defaults AND new effective domains must be blocked.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.anthropic.domains", SettingValue::Text("custom.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // Default wildcard still blocked
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "default domains must remain blocked");
        // User's custom domain also blocked (corp said no anthropic)
        let (action, _) = dp.evaluate("custom.anthropic.com");
        assert_eq!(action, Action::Deny, "user-added domains must also be blocked when corp says no");
    }

    #[test]
    fn stress_user_disable_blocks_even_with_default_allow() {
        // User disables a provider. Even with defaults=allow,
        // that provider's domains must be explicitly blocked.
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("security.web.allow_read", SettingValue::Bool(true)),
            ("security.web.allow_write", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Deny, "user-disabled provider must be blocked even with defaults=allow");
    }

    #[test]
    fn stress_registry_disable_blocks_all_domains() {
        // Disabling a registry blocks ALL its domains, not just some.
        let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("github.com");
        assert_eq!(action, Action::Deny);
        let (action, _) = dp.evaluate("api.github.com");
        assert_eq!(action, Action::Deny);
        let (action, _) = dp.evaluate("raw.githubusercontent.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn stress_all_providers_disabled_all_blocked() {
        // Disable every provider and registry. All their domains must be blocked.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.google.allow", SettingValue::Bool(false)),
            (SETTING_GITHUB_ALLOW, SettingValue::Bool(false)),
            ("security.services.registry.pypi.allow", SettingValue::Bool(false)),
            ("security.services.registry.npm.allow", SettingValue::Bool(false)),
            ("security.services.registry.crates.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // Every known domain should be denied
        for domain in &[
            "api.anthropic.com", "api.openai.com",
            "generativelanguage.googleapis.com",
            "github.com", "api.github.com",
            "pypi.org", "registry.npmjs.org",
        ] {
            let (action, _) = dp.evaluate(domain);
            assert_eq!(action, Action::Deny, "{domain} must be blocked when all services disabled");
        }
    }

    #[test]
    fn stress_all_providers_enabled_all_allowed() {
        // Enable every provider. All their domains must be allowed.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.google.allow", SettingValue::Bool(true)),
            (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        for domain in &[
            "api.anthropic.com", "api.openai.com",
            "generativelanguage.googleapis.com",
            "github.com", "api.github.com",
            "pypi.org",
        ] {
            let (action, _) = dp.evaluate(domain);
            assert_eq!(action, Action::Allow, "{domain} must be allowed when all services enabled");
        }
    }

    #[test]
    fn stress_unknown_domain_follows_default_deny() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // default_action defaults to "deny"
        let (action, _) = dp.evaluate("totally-unknown.example.org");
        assert_eq!(action, Action::Deny, "unknown domain must follow default deny");
    }

    #[test]
    fn stress_unknown_domain_follows_default_allow() {
        let user = file_with(vec![
            ("security.web.allow_read", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("totally-unknown.example.org");
        assert_eq!(action, Action::Allow, "unknown domain must follow default allow");
    }

    #[test]
    fn stress_corp_block_all_providers_user_enables_all() {
        // Corp blocks every AI provider. User enables them all.
        // Corp must win for all.
        let corp = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.google.allow", SettingValue::Bool(false)),
        ]);
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.google.allow", SettingValue::Bool(true)),
            ("security.web.allow_read", SettingValue::Bool(true)),
            ("security.web.allow_write", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        for domain in &[
            "api.anthropic.com", "api.openai.com",
            "generativelanguage.googleapis.com",
        ] {
            let (action, _) = dp.evaluate(domain);
            assert_eq!(action, Action::Deny, "{domain} must be blocked when corp blocks all providers");
        }
    }

    #[test]
    fn stress_mixed_corp_and_user_decisions() {
        // Corp blocks anthropic only. User enables openai, disables google.
        // anthropic: corp-blocked (deny)
        // openai: user-enabled (allow)
        // google: user-disabled (deny)
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.google.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);

        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked anthropic must be denied");

        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Allow, "user-enabled openai must be allowed");

        let (action, _) = dp.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Deny, "user-disabled google must be denied");
    }

    // -----------------------------------------------------------------------
    // L: API key injection
    // -----------------------------------------------------------------------

    #[test]
    fn api_key_injected_when_toggle_on() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("sk-test-123".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test-123");
    }

    #[test]
    fn api_key_injected_even_when_toggle_off() {
        // API keys are always injected so user can enable the provider at
        // runtime without rebooting the VM.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("ai.anthropic.api_key", SettingValue::Text("sk-test-123".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test-123");
    }

    #[test]
    fn api_key_not_injected_when_empty() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
        assert!(!has_key, "empty API key should not be injected");
    }

    #[test]
    fn google_api_key_sets_gemini_env_var() {
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(true)),
            ("ai.google.api_key", SettingValue::Text("AIza-test".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza-test");
        // Only GEMINI_API_KEY is set (not GOOGLE_API_KEY) to avoid
        // gemini CLI warning: "Both GOOGLE_API_KEY and GEMINI_API_KEY are set"
        assert!(env.get("GOOGLE_API_KEY").is_none());
    }

    #[test]
    fn openai_api_key_injected_when_toggle_off() {
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.openai.api_key", SettingValue::Text("sk-oai-test".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai-test");
    }

    #[test]
    fn google_api_key_injected_when_toggle_off() {
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza-off".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza-off");
    }

    #[test]
    fn all_three_providers_injected() {
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
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-ant");
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai");
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza");
        // 3 API keys + 7 built-in env vars (TERM, HOME, PATH, LANG, 3x CA)
        // + 3 CAPSEM_*_ALLOWED provider flags
        assert_eq!(env.len(), 13);
    }

    #[test]
    fn all_three_providers_injected_all_toggles_off() {
        // All toggles off but keys set -- all should still be injected.
        let user = file_with(vec![
            // anthropic defaults to off
            ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
            // openai defaults to off
            ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
            // google: explicitly disable
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-ant");
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai");
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza");
    }

    #[test]
    fn mixed_toggles_all_keys_injected() {
        // One provider on, two off -- all keys should be injected.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
            // openai defaults to off
            ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-ant");
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai");
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza");
    }

    #[test]
    fn provider_allowed_env_vars_injected() {
        // CAPSEM_*_ALLOWED env vars reflect the provider allow toggles.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.google.allow", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("CAPSEM_ANTHROPIC_ALLOWED").unwrap(), "1");
        assert_eq!(env.get("CAPSEM_OPENAI_ALLOWED").unwrap(), "0");
        assert_eq!(env.get("CAPSEM_GOOGLE_ALLOWED").unwrap(), "1");
    }

    #[test]
    fn provider_allowed_defaults_to_one() {
        // Default allow values: all providers enabled.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("CAPSEM_ANTHROPIC_ALLOWED").unwrap(), "1");
        assert_eq!(env.get("CAPSEM_OPENAI_ALLOWED").unwrap(), "1");
        assert_eq!(env.get("CAPSEM_GOOGLE_ALLOWED").unwrap(), "1");
    }

    #[test]
    fn empty_keys_skipped_regardless_of_toggle() {
        // Toggle on but key empty -- should NOT be injected.
        // Toggle off and key empty -- should NOT be injected.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("".into())),
            ("ai.openai.api_key", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        // Only dynamic env vars from defaults might exist, but no API keys.
        let has_ant = gc.env.as_ref().is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
        let has_oai = gc.env.as_ref().is_some_and(|e| e.contains_key("OPENAI_API_KEY"));
        assert!(!has_ant, "empty anthropic key should not be injected");
        assert!(!has_oai, "empty openai key should not be injected");
    }

    // -----------------------------------------------------------------------
    // M: Gemini CLI boot files
    // -----------------------------------------------------------------------

    #[test]
    fn gemini_boot_files_injected_when_google_enabled() {
        // Google AI is enabled by default, so gemini files should be injected
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    #[test]
    fn gemini_boot_files_injected_even_when_google_disabled() {
        // Boot files are always injected so user can enable the provider at
        // runtime without rebooting the VM.
        let user = file_with(vec![("ai.google.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    #[test]
    fn gemini_settings_json_user_override() {
        let custom = r#"{"homeDirectoryWarningDismissed":true,"mcpServers":{"myserver":{}}}"#;
        let user = file_with(vec![
            ("ai.google.gemini.settings_json", SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: custom.into(),
            }),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini_settings = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        assert!(gemini_settings.content.contains("mcpServers"));
    }

    #[test]
    fn gemini_boot_files_have_correct_paths() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    #[test]
    fn gemini_boot_files_user_override_with_toggle_off() {
        // Custom file content should be injected even when google is disabled.
        let custom = r#"{"mcpServers":{"custom":{}}}"#;
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.gemini.settings_json", SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: custom.into(),
            }),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini_settings = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        assert!(gemini_settings.content.contains("mcpServers"), "custom content should be present");
    }

    #[test]
    fn gemini_boot_files_empty_value_skipped() {
        // If a file setting is explicitly set to empty content, it should not be injected.
        let user = file_with(vec![
            ("ai.google.gemini.settings_json", SettingValue::File { path: "/root/.gemini/settings.json".into(), content: "".into() }),
            ("ai.google.gemini.projects_json", SettingValue::File { path: "/root/.gemini/projects.json".into(), content: "".into() }),
            ("ai.google.gemini.trusted_folders_json", SettingValue::File { path: "/root/.gemini/trustedFolders.json".into(), content: "".into() }),
            ("ai.google.gemini.installation_id", SettingValue::File { path: "/root/.gemini/installation_id".into(), content: "".into() }),
            ("ai.anthropic.claude.settings_json", SettingValue::File { path: "/root/.claude/settings.json".into(), content: "".into() }),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let file_paths: Vec<&str> = gc.files.as_ref().map_or(vec![], |f| f.iter().map(|x| x.path.as_str()).collect());
        assert!(!file_paths.contains(&"/root/.gemini/settings.json"));
        assert!(!file_paths.contains(&"/root/.claude/settings.json"));
    }

    #[test]
    fn gemini_boot_files_have_correct_mode() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        for f in &files {
            assert_eq!(f.mode, 0o600, "boot file {} should have mode 0600 (owner-only)", f.path);
        }
    }

    #[test]
    fn api_keys_and_boot_files_both_injected_toggle_off() {
        // End-to-end: toggle off, but key + files should all be present.
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza-key".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        // API key should be injected
        let env = gc.env.unwrap();
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza-key");
        // Boot files (from defaults) should also be injected
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
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
        assert!(bashrc.unwrap().content.contains("PS1="), "bashrc should contain PS1 prompt");
    }

    #[test]
    fn tmux_conf_boot_file_injected() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let tmux = files.iter().find(|f| f.path == "/root/.tmux.conf");
        assert!(tmux.is_some(), "tmux.conf boot file should be injected");
        assert!(tmux.unwrap().content.contains("default-terminal"), "tmux.conf should contain terminal setting");
    }

    #[test]
    fn bashrc_user_override() {
        let custom = "PS1='custom> '\nalias foo='bar'\n";
        let user = file_with(vec![
            ("vm.environment.shell.bashrc", SettingValue::File {
                path: "/root/.bashrc".into(),
                content: custom.into(),
            }),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let bashrc = files.iter().find(|f| f.path == "/root/.bashrc").unwrap();
        assert!(bashrc.content.contains("custom>"), "user override should replace default bashrc content");
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
        let bashrc = defs.iter().find(|d| d.id == "vm.environment.shell.bashrc").unwrap();
        assert_eq!(bashrc.metadata.filetype.as_deref(), Some("bash"));
        let tmux = defs.iter().find(|d| d.id == "vm.environment.shell.tmux_conf").unwrap();
        assert_eq!(tmux.metadata.filetype.as_deref(), Some("conf"));
        let claude = defs.iter().find(|d| d.id == "ai.anthropic.claude.settings_json").unwrap();
        assert_eq!(claude.metadata.filetype.as_deref(), Some("json"));
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
    fn gemini_json_settings_use_file_type() {
        // All .json Gemini settings should be SettingType::File, not Text.
        let defs = setting_definitions();
        for id in &[
            "ai.google.gemini.settings_json",
            "ai.google.gemini.projects_json",
            "ai.google.gemini.trusted_folders_json",
        ] {
            let def = defs.iter().find(|d| d.id == *id).unwrap();
            assert_eq!(
                def.setting_type,
                SettingType::File,
                "{id} should be File type"
            );
        }
    }

    #[test]
    fn gemini_installation_id_is_file_type() {
        // installation_id is now a File type (path + content).
        let defs = setting_definitions();
        let def = defs.iter().find(|d| d.id == "ai.google.gemini.installation_id").unwrap();
        assert_eq!(def.setting_type, SettingType::File);
        let (path, content) = def.default_value.as_file().expect("should be File value");
        assert_eq!(path, "/root/.gemini/installation_id");
        assert!(content.starts_with("capsem-sandbox-"));
    }

    #[test]
    fn file_settings_have_path_in_default_value() {
        // Every File-type setting must have a File default with a valid path.
        let defs = setting_definitions();
        for def in &defs {
            if def.setting_type == SettingType::File {
                let (path, _) = def.default_value.as_file().unwrap_or_else(|| {
                    panic!("File setting {} must have File default value", def.id)
                });
                assert!(path.starts_with('/'), "path must be absolute: {path} (setting {})", def.id);
            }
        }
    }

    #[test]
    fn guest_config_collects_file_type_settings() {
        // settings_to_guest_config should pick up File values directly.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        // All file settings come from SettingValue::File
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
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
        let result = validate_setting_value(
            "ai.anthropic.allow",
            &SettingValue::Bool(true),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn file_type_resolved_setting_has_file_value() {
        // The resolved setting for a File type should have a File value with path.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "ai.google.gemini.settings_json").unwrap();
        assert_eq!(s.setting_type, SettingType::File);
        let (path, _content) = s.effective_value.as_file().expect("should be a File value");
        assert_eq!(path, "/root/.gemini/settings.json");
    }

    // -----------------------------------------------------------------------
    // P: Metadata-driven env var injection
    // -----------------------------------------------------------------------

    #[test]
    fn api_key_settings_have_env_vars_metadata() {
        // API key settings must declare their env var name in metadata.env_vars
        // instead of relying on a hardcoded API_KEY_MAP.
        let defs = setting_definitions();
        let cases = [
            ("ai.anthropic.api_key", "ANTHROPIC_API_KEY"),
            ("ai.openai.api_key", "OPENAI_API_KEY"),
            ("ai.google.api_key", "GEMINI_API_KEY"),
        ];
        for (id, expected_var) in &cases {
            let def = defs.iter().find(|d| d.id == *id)
                .unwrap_or_else(|| panic!("missing setting {id}"));
            assert!(
                def.metadata.env_vars.contains(&expected_var.to_string()),
                "{id} should have env_vars containing {expected_var}, got {:?}",
                def.metadata.env_vars,
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
            let found = defs.iter().any(|d| d.metadata.env_vars.contains(&var.to_string()));
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
            let found = defs.iter().any(|d| d.metadata.env_vars.contains(&var.to_string()));
            assert!(found, "no setting definition injects env var {var}");
        }
    }

    #[test]
    fn guest_config_env_from_metadata_env_vars() {
        // settings_to_guest_config should inject env vars based on
        // metadata.env_vars, not hardcoded API_KEY_MAP.
        let user = file_with(vec![
            ("ai.anthropic.api_key", SettingValue::Text("sk-test".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test");
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
        let term_def = defs.iter().find(|d| d.metadata.env_vars.contains(&"TERM".to_string())).unwrap();
        let corp = file_with(vec![(&term_def.id, SettingValue::Text("dumb".into()))]);
        let resolved = resolve_settings(&empty_file(), &corp);
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("TERM").unwrap(), "dumb");
    }

    #[test]
    fn user_can_override_builtin_env() {
        let defs = setting_definitions();
        let path_def = defs.iter().find(|d| d.metadata.env_vars.contains(&"PATH".to_string())).unwrap();
        let user = file_with(vec![(&path_def.id, SettingValue::Text("/custom/bin".into()))]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("PATH").unwrap(), "/custom/bin");
    }

    #[test]
    fn empty_env_var_setting_not_injected() {
        // A setting with env_vars metadata but empty value should not be injected.
        let user = file_with(vec![
            ("ai.anthropic.api_key", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
        assert!(!has_key, "empty API key should not be injected");
    }

    #[test]
    fn dynamic_guest_env_still_works() {
        // Dynamic guest.env.* settings should still be injected alongside
        // metadata-driven env vars.
        let user = file_with(vec![
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ]);
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
            let msg = capsem_proto::HostToGuest::SetEnv { key: key.clone(), value: value.clone() };
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
        // (Text, ApiKey, Password, Url, Email).
        let defs = setting_definitions();
        for def in &defs {
            if !def.metadata.env_vars.is_empty() {
                assert!(
                    matches!(def.setting_type, SettingType::Text | SettingType::ApiKey | SettingType::Password | SettingType::Url | SettingType::Email),
                    "setting {} has env_vars but type {:?} (should be text-like)",
                    def.id, def.setting_type,
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
        let user = file_with(vec![
            ("guest.env.LD_PRELOAD", SettingValue::Text("/evil/lib.so".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("LD_PRELOAD"));
        assert!(!has_key, "LD_PRELOAD should be dropped by validation");
    }

    #[test]
    fn settings_rejects_ld_library_path() {
        let user = file_with(vec![
            ("guest.env.LD_LIBRARY_PATH", SettingValue::Text("/evil".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("LD_LIBRARY_PATH"));
        assert!(!has_key, "LD_LIBRARY_PATH should be dropped by validation");
    }

    #[test]
    fn settings_accepts_normal_dynamic_env() {
        let user = file_with(vec![
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ]);
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
        let s = resolved.iter().find(|s| s.id == "security.services.search.google.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(true));
        assert_eq!(s.category, "Google");
    }

    #[test]
    fn web_search_bing_duckduckgo_blocked_by_default() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        for id in &["security.services.search.bing.allow", "security.services.search.duckduckgo.allow"] {
            let s = resolved.iter().find(|s| s.id == *id).unwrap();
            assert_eq!(s.effective_value, SettingValue::Bool(false), "expected {id} to be false");
        }
    }

    #[test]
    fn web_search_google_domains_in_policy() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("www.google.com");
        assert_eq!(action, Action::Allow, "google.com should be allowed by default");
    }

    // -----------------------------------------------------------------------
    // Custom allow/block
    // -----------------------------------------------------------------------

    #[test]
    fn custom_allow_allows_domains() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // elie.net is in the default custom_allow
        let (action, _) = dp.evaluate("elie.net");
        assert_eq!(action, Action::Allow, "elie.net should be allowed via custom_allow");
    }

    #[test]
    fn custom_allow_wildcard_allows_subdomains() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("www.elie.net");
        assert_eq!(action, Action::Allow, "*.elie.net should allow subdomains");
    }

    #[test]
    fn custom_block_blocks_domains() {
        let user = file_with(vec![
            ("security.web.custom_block", SettingValue::Text("evil.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("evil.com");
        assert_eq!(action, Action::Deny, "custom_block should block domains");
    }

    #[test]
    fn custom_block_beats_custom_allow_on_overlap() {
        let user = file_with(vec![
            ("security.web.custom_allow", SettingValue::Text("overlap.com".into())),
            ("security.web.custom_block", SettingValue::Text("overlap.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("overlap.com");
        assert_eq!(action, Action::Deny, "block must beat allow for overlapping domains");
    }

    #[test]
    fn custom_allow_empty_entries_tolerated() {
        let user = file_with(vec![
            ("security.web.custom_allow", SettingValue::Text(",, , foo.com , ,".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("foo.com");
        assert_eq!(action, Action::Allow, "empty entries should be ignored");
    }

    #[test]
    fn custom_block_empty_is_noop() {
        let user = file_with(vec![
            ("security.web.custom_block", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // Default custom_allow domains (elie.net) still allowed
        let (action, _) = dp.evaluate("elie.net");
        assert_eq!(action, Action::Allow, "empty custom_block should not block anything");
    }

    #[test]
    fn custom_allow_corp_override() {
        // Corp sets custom_allow to empty -> user's default elie.net is gone
        let corp = file_with(vec![
            ("security.web.custom_allow", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&empty_file(), &corp);
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("elie.net");
        assert_eq!(action, Action::Deny, "corp should be able to override custom_allow");
    }

    #[test]
    fn custom_allow_in_network_policy() {
        // Verify custom domains also appear in the NetworkPolicy path
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let allowed = dp.allowed_patterns();
        assert!(
            allowed.iter().any(|d| d == "elie.net"),
            "elie.net should be in allowed patterns: {allowed:?}"
        );
    }

    // -----------------------------------------------------------------------
    // MCP server injection into settings.json
    // -----------------------------------------------------------------------

    #[test]
    fn inject_capsem_mcp_server_into_empty_json() {
        let result = inject_capsem_mcp_server(r#"{}"#);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server"
        );
    }

    #[test]
    fn inject_capsem_mcp_server_preserves_existing_servers() {
        let input = r#"{"mcpServers":{"github":{"command":"npx","args":["-y","@github/mcp"]}}}"#;
        let result = inject_capsem_mcp_server(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["mcpServers"]["github"]["command"], "npx");
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server"
        );
    }

    #[test]
    fn inject_capsem_mcp_server_preserves_other_keys() {
        let input = r#"{"permissions":{"defaultMode":"bypassPermissions"}}"#;
        let result = inject_capsem_mcp_server(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["permissions"]["defaultMode"], "bypassPermissions");
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server"
        );
    }

    #[test]
    fn inject_capsem_mcp_server_invalid_json_passthrough() {
        let input = "not json at all";
        let result = inject_capsem_mcp_server(input);
        assert_eq!(result, input);
    }

    #[test]
    fn claude_default_settings_has_capsem_mcp_server() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let claude = files.iter().find(|f| f.path == "/root/.claude/settings.json").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&claude.content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server",
            "capsem MCP server should be injected into Claude settings.json"
        );
        // Original permissions should still be there
        assert_eq!(parsed["permissions"]["defaultMode"], "bypassPermissions");
    }

    #[test]
    fn gemini_default_settings_has_capsem_mcp_server() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&gemini.content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server",
            "capsem MCP server should be injected into Gemini settings.json"
        );
    }

    #[test]
    fn user_mcp_servers_preserved_alongside_capsem() {
        let custom = r#"{"mcpServers":{"myserver":{"command":"my-tool"}}}"#;
        let user = file_with(vec![
            ("ai.google.gemini.settings_json", SettingValue::File {
                path: "/root/.gemini/settings.json".into(),
                content: custom.into(),
            }),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&gemini.content).unwrap();
        assert_eq!(parsed["mcpServers"]["myserver"]["command"], "my-tool");
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server"
        );
    }

    #[test]
    fn capsem_mcp_not_in_non_settings_json_files() {
        // Other boot files (projects.json, etc.) should NOT get mcpServers injected
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let projects = files.iter().find(|f| f.path == "/root/.gemini/projects.json").unwrap();
        assert!(!projects.content.contains("capsem"), "projects.json should not have capsem injected");
    }

    #[test]
    fn claude_state_json_has_capsem_mcp_server() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let claude = files
            .iter()
            .find(|f| f.path == "/root/.claude.json")
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&claude.content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["capsem"]["command"],
            "/run/capsem-mcp-server",
            "capsem MCP server should be injected into .claude.json"
        );
    }

    #[test]
    fn codex_default_config_has_capsem_mcp_server() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let codex = files
            .iter()
            .find(|f| f.path == "/root/.codex/config.toml")
            .unwrap();
        assert!(
            codex.content.contains("capsem"),
            "codex config.toml should contain capsem MCP server"
        );
        assert!(
            codex.content.contains("/run/capsem-mcp-server"),
            "codex config.toml should reference /run/capsem-mcp-server"
        );
    }

    // -----------------------------------------------------------------------
    // TOML MCP server injection
    // -----------------------------------------------------------------------

    #[test]
    fn inject_capsem_mcp_server_toml_empty() {
        let result = inject_capsem_mcp_server_toml("");
        let parsed: toml::Value = toml::from_str(&result).unwrap();
        let cmd = parsed["mcp_servers"]["capsem"]["command"].as_str().unwrap();
        assert_eq!(cmd, "/run/capsem-mcp-server");
    }

    #[test]
    fn inject_capsem_mcp_server_toml_preserves_existing() {
        let input = "[mcp_servers.github]\ncommand = \"npx\"\nargs = [\"-y\", \"@github/mcp\"]\n";
        let result = inject_capsem_mcp_server_toml(input);
        let parsed: toml::Value = toml::from_str(&result).unwrap();
        assert_eq!(
            parsed["mcp_servers"]["github"]["command"].as_str().unwrap(),
            "npx"
        );
        assert_eq!(
            parsed["mcp_servers"]["capsem"]["command"].as_str().unwrap(),
            "/run/capsem-mcp-server"
        );
    }

    #[test]
    fn inject_capsem_mcp_server_toml_invalid_passthrough() {
        let input = "not valid toml [[[";
        let result = inject_capsem_mcp_server_toml(input);
        assert_eq!(result, input);
    }

    // -----------------------------------------------------------------------
    // TOML registry tests
    // -----------------------------------------------------------------------

    #[test]
    fn toml_registry_parses() {
        // The embedded defaults.toml must parse without panicking.
        let defs = setting_definitions();
        assert!(!defs.is_empty(), "defaults.toml must produce at least one setting");
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
        let anthropic_allow = defs.iter().find(|d| d.id == "ai.anthropic.allow").unwrap();
        assert!(
            !anthropic_allow.category.is_empty(),
            "ai.anthropic.allow should have a category inherited from its group",
        );
    }

    #[test]
    fn toml_registry_enabled_by_inherited() {
        // enabled_by is inherited from the group and applied to children
        // but NOT to the toggle setting itself.
        let defs = setting_definitions();
        let allow = defs.iter().find(|d| d.id == "ai.anthropic.allow").unwrap();
        assert!(
            allow.enabled_by.is_none(),
            "the toggle itself should not have enabled_by",
        );
        let api_key = defs.iter().find(|d| d.id == "ai.anthropic.api_key").unwrap();
        assert_eq!(
            api_key.enabled_by.as_deref(),
            Some("ai.anthropic.allow"),
            "api_key should inherit enabled_by from its group",
        );
    }

    #[test]
    fn toml_registry_meta_fields() {
        // Metadata fields (domains, choices, rules, env_vars)
        // are correctly parsed from the `meta` sub-table.
        let defs = setting_definitions();

        // Registry toggles should have domains in metadata
        let github = defs.iter().find(|d| d.id == SETTING_GITHUB_ALLOW).unwrap();
        assert!(!github.metadata.domains.is_empty(), "github toggle should have domain metadata");

        // security.web.allow_read should be a bool
        let ar = defs.iter().find(|d| d.id == "security.web.allow_read").unwrap();
        assert_eq!(ar.setting_type, SettingType::Bool, "allow_read should be bool");

        // API key settings should have env_vars
        let key = defs.iter().find(|d| d.id == "ai.anthropic.api_key").unwrap();
        assert!(
            !key.metadata.env_vars.is_empty(),
            "api_key settings should have env_vars metadata",
        );
    }

    // -----------------------------------------------------------------------
    // Config lint tests
    // -----------------------------------------------------------------------

    fn make_resolved(id: &str, stype: SettingType, value: SettingValue, meta: SettingMetadata, enabled_by: Option<&str>) -> ResolvedSetting {
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
        }
    }

    // -- JSON validation (File values) --

    fn file_val(path: &str, content: &str) -> SettingValue {
        SettingValue::File { path: path.into(), content: content.into() }
    }

    #[test]
    fn config_lint_valid_json_passes() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", r#"{"key":"val"}"#), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_malformed_json_gives_clear_error() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", "{bad json}"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "error" && i.message.contains("invalid JSON")));
    }

    #[test]
    fn config_lint_json_not_object_warns() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", "42"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("not an object")));
    }

    #[test]
    fn config_lint_empty_json_file_ok() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", ""), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_json_with_trailing_comma_gives_error() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", r#"{"a":1,}"#), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "error"));
    }

    #[test]
    fn config_lint_json_with_unicode_passes() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", r#"{"name":"cafe\u0301"}"#), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_json_deeply_nested_passes() {
        let json = r#"{"a":{"b":{"c":{"d":{"e":"deep"}}}}}"#;
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", json), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_json_huge_payload_passes() {
        let big_val = "x".repeat(1_000_000);
        let json = format!(r#"{{"data":"{}"}}"#, big_val);
        let s = make_resolved("test.file", SettingType::File, file_val("/root/test.json", &json), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_file_path_must_be_absolute() {
        let s = make_resolved("test.file", SettingType::File, file_val("relative/path.json", "{}"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "error" && i.message.contains("absolute")));
    }

    #[test]
    fn config_lint_file_path_no_traversal() {
        let s = make_resolved("test.file", SettingType::File, file_val("/root/../etc/passwd", "{}"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "error" && i.message.contains("..")));
    }

    #[test]
    fn config_lint_file_unusual_path_warns() {
        let s = make_resolved("test.file", SettingType::File, file_val("/tmp/test.json", "{}"), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("unusual")));
    }

    // -- Number validation --

    #[test]
    fn config_lint_number_in_range_ok() {
        let meta = SettingMetadata { min: Some(1), max: Some(128), ..Default::default() };
        let s = make_resolved("vm.cpu", SettingType::Number, SettingValue::Number(4), meta, None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_number_below_min_error() {
        let meta = SettingMetadata { min: Some(1), max: Some(128), ..Default::default() };
        let s = make_resolved("vm.cpu", SettingType::Number, SettingValue::Number(0), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
        assert!(issues[0].message.contains("below minimum"));
    }

    #[test]
    fn config_lint_number_above_max_error() {
        let meta = SettingMetadata { min: Some(1), max: Some(128), ..Default::default() };
        let s = make_resolved("vm.disk", SettingType::Number, SettingValue::Number(256), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
        assert!(issues[0].message.contains("exceeds maximum"));
    }

    #[test]
    fn config_lint_number_at_boundary_ok() {
        let meta = SettingMetadata { min: Some(1), max: Some(128), ..Default::default() };
        let s1 = make_resolved("vm.min", SettingType::Number, SettingValue::Number(1), meta.clone(), None);
        let s2 = make_resolved("vm.max", SettingType::Number, SettingValue::Number(128), meta, None);
        let issues = config_lint(&[s1, s2]);
        assert!(issues.is_empty());
    }

    // -- Choice validation --

    #[test]
    fn config_lint_valid_choice_ok() {
        let meta = SettingMetadata { choices: vec!["allow".into(), "deny".into()], ..Default::default() };
        let s = make_resolved("net.action", SettingType::Text, SettingValue::Text("deny".into()), meta, None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_invalid_choice_error() {
        let meta = SettingMetadata { choices: vec!["allow".into(), "deny".into()], ..Default::default() };
        let s = make_resolved("net.action", SettingType::Text, SettingValue::Text("block".into()), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
        assert!(issues[0].message.contains("not a valid choice"));
    }

    #[test]
    fn config_lint_empty_choice_when_choices_defined_error() {
        let meta = SettingMetadata { choices: vec!["allow".into(), "deny".into()], ..Default::default() };
        let s = make_resolved("net.action", SettingType::Text, SettingValue::Text("".into()), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
    }

    #[test]
    fn config_lint_case_sensitive_choice() {
        let meta = SettingMetadata { choices: vec!["allow".into(), "deny".into()], ..Default::default() };
        let s = make_resolved("net.action", SettingType::Text, SettingValue::Text("Allow".into()), meta, None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1, "'Allow' != 'allow' -- case sensitive");
    }

    // -- API key validation --

    #[test]
    fn config_lint_apikey_with_whitespace_warns() {
        let s = make_resolved("ai.key", SettingType::ApiKey, SettingValue::Text("sk-ant key".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("whitespace")));
    }

    #[test]
    fn config_lint_apikey_with_newline_warns() {
        let s = make_resolved("ai.key", SettingType::ApiKey, SettingValue::Text("sk-ant\n".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("whitespace")));
    }

    #[test]
    fn config_lint_apikey_empty_when_enabled_warns() {
        let toggle = make_resolved("ai.provider.allow", SettingType::Bool, SettingValue::Bool(true), SettingMetadata::default(), None);
        let key = make_resolved("ai.provider.key", SettingType::ApiKey, SettingValue::Text("".into()), SettingMetadata::default(), Some("ai.provider.allow"));
        let issues = config_lint(&[toggle, key]);
        assert!(issues.iter().any(|i| i.severity == "warning" && i.message.contains("not set")));
    }

    #[test]
    fn config_lint_apikey_empty_when_disabled_ok() {
        let toggle = make_resolved("ai.provider.allow", SettingType::Bool, SettingValue::Bool(false), SettingMetadata::default(), None);
        let key = make_resolved("ai.provider.key", SettingType::ApiKey, SettingValue::Text("".into()), SettingMetadata::default(), Some("ai.provider.allow"));
        let issues = config_lint(&[toggle, key]);
        assert!(issues.is_empty(), "disabled provider with empty key is fine");
    }

    #[test]
    fn config_lint_apikey_normal_value_ok() {
        let s = make_resolved("ai.key", SettingType::ApiKey, SettingValue::Text("sk-ant-api03-valid".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    // -- Text validation --

    #[test]
    fn config_lint_text_with_nul_byte_error() {
        let s = make_resolved("t.val", SettingType::Text, SettingValue::Text("hello\0world".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "error");
        assert!(issues[0].message.contains("invalid characters"));
    }

    #[test]
    fn config_lint_text_normal_ok() {
        let s = make_resolved("t.val", SettingType::Text, SettingValue::Text("hello".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_text_unicode_ok() {
        let s = make_resolved("t.val", SettingType::Text, SettingValue::Text("cafe\u{0301}".into()), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn config_lint_text_very_long_ok() {
        let long_val = "x".repeat(10_000);
        let s = make_resolved("t.val", SettingType::Text, SettingValue::Text(long_val), SettingMetadata::default(), None);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    // -- Serialization roundtrip --

    #[test]
    fn config_lint_all_issues_serialize_deserialize() {
        let meta = SettingMetadata { min: Some(1), max: Some(10), ..Default::default() };
        let s = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta, None);
        let issues = config_lint(&[s]);
        let json = serde_json::to_string(&issues).unwrap();
        let roundtrip: Vec<ConfigIssue> = serde_json::from_str(&json).unwrap();
        assert_eq!(issues, roundtrip);
    }

    #[test]
    fn config_lint_issue_messages_are_nonempty() {
        let meta = SettingMetadata { min: Some(1), max: Some(10), ..Default::default() };
        let s = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta, None);
        let issues = config_lint(&[s]);
        for issue in &issues {
            assert!(!issue.message.is_empty());
            assert!(!issue.id.is_empty());
        }
    }

    #[test]
    fn config_lint_issue_ids_are_valid_setting_ids() {
        let meta = SettingMetadata { min: Some(1), max: Some(10), ..Default::default() };
        let s = make_resolved("vm.resources.cpu_count", SettingType::Number, SettingValue::Number(99), meta, None);
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
        assert!(errors.is_empty(), "default config should have no errors: {errors:?}");
    }

    #[test]
    fn config_lint_returns_multiple_issues() {
        let meta_num = SettingMetadata { min: Some(1), max: Some(10), ..Default::default() };
        let s1 = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta_num, None);
        let s2 = make_resolved("v.f", SettingType::File, file_val("/root/test.json", "{bad}"), SettingMetadata::default(), None);
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
        let toggle = make_resolved("ai.provider.allow", SettingType::Bool, SettingValue::Bool(true), SettingMetadata::default(), None);
        let key = make_resolved("ai.provider.key", SettingType::ApiKey, SettingValue::Text("".into()), meta, Some("ai.provider.allow"));
        let issues = config_lint(&[toggle, key]);
        let empty_key_issue = issues.iter().find(|i| i.message.contains("not set")).unwrap();
        assert_eq!(empty_key_issue.docs_url.as_deref(), Some("https://example.com/keys"));
    }

    #[test]
    fn config_lint_non_key_issue_no_docs_url() {
        let meta = SettingMetadata { min: Some(1), max: Some(10), ..Default::default() };
        let s = make_resolved("v.n", SettingType::Number, SettingValue::Number(99), meta, None);
        let issues = config_lint(&[s]);
        assert!(!issues.is_empty());
        for issue in &issues {
            assert!(issue.docs_url.is_none(), "non-key issues should not have docs_url");
        }
    }

    #[test]
    fn docs_url_parsed_from_toml() {
        let defs = setting_definitions();
        let anthropic_key = defs.iter().find(|d| d.id == "ai.anthropic.api_key").unwrap();
        assert_eq!(anthropic_key.metadata.docs_url.as_deref(), Some("https://console.anthropic.com/settings/keys"));
        let github_token = defs.iter().find(|d| d.id == SETTING_GITHUB_TOKEN).unwrap();
        assert_eq!(github_token.metadata.docs_url.as_deref(), Some("https://github.com/settings/tokens"));
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
        for expected in &["AI Providers", "Security", "Web", "Services", "Search Engines", "Package Registries", "Appearance", "VM", "Environment", "Resources"] {
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
                if let SettingsNode::Group { key: k, children, .. } = node {
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

        // ai.anthropic group should have enabled_by = "ai.anthropic.allow"
        let anthropic = find_group(&tree, "ai.anthropic");
        assert!(anthropic.is_some(), "should find ai.anthropic group");
        if let Some(SettingsNode::Group { enabled_by, .. }) = anthropic {
            assert_eq!(enabled_by, Some("ai.anthropic.allow".to_string()));
        }
    }

    // -----------------------------------------------------------------------
    // .git-credentials generation tests
    // -----------------------------------------------------------------------

    #[test]
    fn git_credentials_generated_with_github_token() {
        let user = file_with(vec![
            (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
            (SETTING_GITHUB_TOKEN, SettingValue::Text("ghp_test123".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let creds = files
            .iter()
            .find(|f| f.path == "/root/.git-credentials")
            .expect(".git-credentials should be generated");
        assert_eq!(creds.mode, 0o600);
        assert!(creds
            .content
            .contains("https://oauth2:ghp_test123@github.com"));
        // .gitconfig must also be generated with credential.helper = store
        let gitconfig = files
            .iter()
            .find(|f| f.path == "/root/.gitconfig")
            .expect(".gitconfig should be generated");
        assert_eq!(gitconfig.mode, 0o644);
        assert!(gitconfig.content.contains("helper = store"));
    }

    #[test]
    fn git_credentials_generated_with_multiple_providers() {
        let user = file_with(vec![
            (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
            (SETTING_GITHUB_TOKEN, SettingValue::Text("ghp_test123".into())),
            (SETTING_GITLAB_ALLOW, SettingValue::Bool(true)),
            (SETTING_GITLAB_TOKEN, SettingValue::Text("glpat-test456".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let creds = files
            .iter()
            .find(|f| f.path == "/root/.git-credentials")
            .expect(".git-credentials should be generated");
        assert!(creds
            .content
            .contains("https://oauth2:ghp_test123@github.com"));
        assert!(creds
            .content
            .contains("https://oauth2:glpat-test456@gitlab.com"));
    }

    #[test]
    fn git_credentials_not_generated_when_allow_false() {
        let user = file_with(vec![
            (SETTING_GITHUB_ALLOW, SettingValue::Bool(false)),
            (SETTING_GITHUB_TOKEN, SettingValue::Text("ghp_test123".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_creds = gc
            .files
            .as_ref()
            .map_or(false, |f| f.iter().any(|f| f.path == "/root/.git-credentials"));
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
            .map_or(false, |f| f.iter().any(|f| f.path == "/root/.git-credentials"));
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
            .map_or(false, |f| f.iter().any(|f| f.path == "/root/.git-credentials"));
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
            .map_or(false, |f| f.iter().any(|f| f.path == "/root/.git-credentials"));
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
            .map_or(false, |f| f.iter().any(|f| f.path == "/root/.git-credentials"));
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
            .map_or(false, |f| f.iter().any(|f| f.path == "/root/.git-credentials"));
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
            .map_or(false, |f| f.iter().any(|f| f.path == "/root/.git-credentials"));
        let has_gitconfig = gc
            .files
            .as_ref()
            .map_or(false, |f| f.iter().any(|f| f.path == "/root/.gitconfig"));
        assert!(!has_creds, ".git-credentials should not exist without tokens");
        assert!(!has_gitconfig, ".gitconfig should not exist without tokens");
    }

    // -----------------------------------------------------------------------
    // Git identity env var tests
    // -----------------------------------------------------------------------

    #[test]
    fn git_identity_env_vars_injected() {
        let user = file_with(vec![
            ("repository.git.identity.author_name", SettingValue::Text("Test User".into())),
            ("repository.git.identity.author_email", SettingValue::Text("test@example.com".into())),
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
        assert!(!env.contains_key("GIT_AUTHOR_NAME"), "GIT_AUTHOR_NAME should not be set when empty");
        assert!(!env.contains_key("GIT_COMMITTER_NAME"), "GIT_COMMITTER_NAME should not be set when empty");
        assert!(!env.contains_key("GIT_AUTHOR_EMAIL"), "GIT_AUTHOR_EMAIL should not be set when empty");
        assert!(!env.contains_key("GIT_COMMITTER_EMAIL"), "GIT_COMMITTER_EMAIL should not be set when empty");
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
            assert!(defs.iter().any(|d| d.id == *id), "missing setting definition: {id}");
        }
    }

    #[test]
    fn default_github_allowed_gitlab_not() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gh = resolved.iter().find(|s| s.id == SETTING_GITHUB_ALLOW).unwrap();
        assert_eq!(gh.effective_value, SettingValue::Bool(true));
        let gl = resolved.iter().find(|s| s.id == SETTING_GITLAB_ALLOW).unwrap();
        assert_eq!(gl.effective_value, SettingValue::Bool(false));
    }

    #[test]
    fn setting_id_constants_exist_in_registry() {
        let defs = setting_definitions();
        let ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
        for constant in [
            SETTING_ANTHROPIC_ALLOW, SETTING_ANTHROPIC_API_KEY,
            SETTING_OPENAI_ALLOW, SETTING_OPENAI_API_KEY,
            SETTING_GOOGLE_ALLOW, SETTING_GOOGLE_API_KEY,
            SETTING_GITHUB_ALLOW, SETTING_GITHUB_TOKEN,
            SETTING_GITLAB_ALLOW, SETTING_GITLAB_TOKEN,
        ] {
            assert!(ids.contains(&constant), "constant '{constant}' not found in setting_definitions()");
        }
    }

    // -----------------------------------------------------------------------
    // GH_TOKEN / GITLAB_TOKEN env var injection tests
    // -----------------------------------------------------------------------

    #[test]
    fn gh_token_injected_when_github_enabled() {
        let user = file_with(vec![
            (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
            (SETTING_GITHUB_TOKEN, SettingValue::Text("ghp_test123".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("GH_TOKEN").unwrap(), "ghp_test123");
        assert_eq!(env.get("GITHUB_TOKEN").unwrap(), "ghp_test123");
    }

    #[test]
    fn gitlab_token_injected_when_gitlab_enabled() {
        let user = file_with(vec![
            (SETTING_GITLAB_ALLOW, SettingValue::Bool(true)),
            (SETTING_GITLAB_TOKEN, SettingValue::Text("glpat-test456".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("GITLAB_TOKEN").unwrap(), "glpat-test456");
    }

    #[test]
    fn gh_token_not_injected_when_token_empty() {
        let user = file_with(vec![
            (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap_or_default();
        assert!(!env.contains_key("GH_TOKEN"), "GH_TOKEN should not be set when token is empty");
        assert!(!env.contains_key("GITHUB_TOKEN"), "GITHUB_TOKEN should not be set when token is empty");
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
        let anthropic = defs.iter().find(|d| d.id == SETTING_ANTHROPIC_API_KEY).unwrap();
        assert_eq!(anthropic.metadata.prefix.as_deref(), Some("sk-ant-"));
    }

    // -----------------------------------------------------------------------
    // Security presets
    // -----------------------------------------------------------------------

    #[test]
    fn preset_definitions_load_correctly() {
        let presets = security_presets();
        assert_eq!(presets.len(), 2);
        for p in &presets {
            assert!(!p.id.is_empty());
            assert!(!p.name.is_empty());
            assert!(!p.description.is_empty());
        }
    }

    #[test]
    fn preset_medium_has_correct_settings() {
        let presets = security_presets();
        let medium = presets.iter().find(|p| p.id == "medium").unwrap();
        assert_eq!(medium.settings["security.web.allow_read"], SettingValue::Bool(true));
        assert_eq!(medium.settings["security.web.allow_write"], SettingValue::Bool(false));
        assert_eq!(medium.settings["security.services.search.google.allow"], SettingValue::Bool(true));
        assert_eq!(medium.settings["security.services.search.bing.allow"], SettingValue::Bool(true));
        assert_eq!(medium.settings["security.services.search.duckduckgo.allow"], SettingValue::Bool(true));
    }

    #[test]
    fn preset_high_has_correct_settings() {
        let presets = security_presets();
        let high = presets.iter().find(|p| p.id == "high").unwrap();
        assert_eq!(high.settings["security.web.allow_read"], SettingValue::Bool(false));
        assert_eq!(high.settings["security.web.allow_write"], SettingValue::Bool(false));
        assert_eq!(high.settings["security.services.search.google.allow"], SettingValue::Bool(true));
        assert_eq!(high.settings["security.services.search.bing.allow"], SettingValue::Bool(false));
        assert_eq!(high.settings["security.services.search.duckduckgo.allow"], SettingValue::Bool(false));
    }

    #[test]
    fn preset_settings_are_valid_registry_ids() {
        let defs = setting_definitions();
        let def_ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
        for preset in security_presets() {
            for key in preset.settings.keys() {
                assert!(def_ids.contains(&key.as_str()), "preset '{}' has unknown setting: {}", preset.id, key);
            }
        }
    }

    #[test]
    fn apply_preset_medium_writes_user_toml() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();

        let skipped = apply_preset_to("medium", &user_path, &corp_path).unwrap();
        assert!(skipped.is_empty());

        let loaded = load_settings_file(&user_path).unwrap();
        assert_eq!(loaded.settings["security.web.allow_read"].value, SettingValue::Bool(true));
        assert_eq!(loaded.settings["security.web.allow_write"].value, SettingValue::Bool(false));
    }

    #[test]
    fn apply_preset_high_writes_user_toml() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();

        let skipped = apply_preset_to("high", &user_path, &corp_path).unwrap();
        assert!(skipped.is_empty());

        let loaded = load_settings_file(&user_path).unwrap();
        assert_eq!(loaded.settings["security.web.allow_read"].value, SettingValue::Bool(false));
        assert_eq!(loaded.settings["security.services.search.bing.allow"].value, SettingValue::Bool(false));
    }

    #[test]
    fn apply_preset_skips_corp_locked() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        let corp = file_with(vec![("security.web.allow_read", SettingValue::Bool(false))]);
        write_settings_file(&corp_path, &corp).unwrap();

        let skipped = apply_preset_to("medium", &user_path, &corp_path).unwrap();
        assert!(skipped.contains(&"security.web.allow_read".to_string()));

        let loaded = load_settings_file(&user_path).unwrap();
        assert!(!loaded.settings.contains_key("security.web.allow_read"));
    }

    #[test]
    fn apply_preset_does_not_clobber_unrelated_settings() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        let mut initial = SettingsFile::default();
        initial.settings.insert("ai.google.api_key".to_string(), SettingEntry {
            value: SettingValue::Text("AIzaTest".into()),
            modified: now_str(),
        });
        write_settings_file(&user_path, &initial).unwrap();

        apply_preset_to("medium", &user_path, &corp_path).unwrap();

        let loaded = load_settings_file(&user_path).unwrap();
        assert_eq!(loaded.settings["ai.google.api_key"].value, SettingValue::Text("AIzaTest".into()));
        assert_eq!(loaded.settings["security.web.allow_read"].value, SettingValue::Bool(true));
    }

    #[test]
    fn apply_preset_mcp_permission_set() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();

        apply_preset_to("medium", &user_path, &corp_path).unwrap();
        let loaded = load_settings_file(&user_path).unwrap();
        assert_eq!(
            loaded.mcp.as_ref().unwrap().default_tool_permission,
            Some(crate::mcp::policy::ToolDecision::Allow),
        );

        apply_preset_to("high", &user_path, &corp_path).unwrap();
        let loaded = load_settings_file(&user_path).unwrap();
        assert_eq!(
            loaded.mcp.as_ref().unwrap().default_tool_permission,
            Some(crate::mcp::policy::ToolDecision::Warn),
        );
    }

    #[test]
    fn apply_preset_mcp_skips_when_corp_locked() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        let mut corp = SettingsFile::default();
        let mut corp_mcp = crate::mcp::policy::McpUserConfig::default();
        corp_mcp.default_tool_permission = Some(crate::mcp::policy::ToolDecision::Block);
        corp.mcp = Some(corp_mcp);
        write_settings_file(&corp_path, &corp).unwrap();

        let skipped = apply_preset_to("medium", &user_path, &corp_path).unwrap();
        assert!(skipped.contains(&"mcp.default_tool_permission".to_string()));
    }

    #[test]
    fn apply_preset_unknown_id_errors() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();

        let result = apply_preset_to("nonexistent", &user_path, &corp_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown preset"));
    }

    #[test]
    fn apply_preset_overwrites_previous_user_values() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        let initial = file_with(vec![("security.web.allow_read", SettingValue::Bool(true))]);
        write_settings_file(&user_path, &initial).unwrap();

        apply_preset_to("high", &user_path, &corp_path).unwrap();
        let loaded = load_settings_file(&user_path).unwrap();
        assert_eq!(loaded.settings["security.web.allow_read"].value, SettingValue::Bool(false));
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
        assert!(!file.settings.contains_key("web.defaults.allow_read"));
        assert!(!file.settings.contains_key("web.custom_allow"));
        assert!(!file.settings.contains_key("registry.npm.allow"));
        assert!(!file.settings.contains_key("web.search.google.allow"));

        // New keys present with same values
        assert_eq!(file.settings["security.web.allow_read"].value, SettingValue::Bool(true));
        assert_eq!(file.settings["security.web.custom_allow"].value, SettingValue::Text("example.com".into()));
        assert_eq!(file.settings["security.services.registry.npm.allow"].value, SettingValue::Bool(false));
        assert_eq!(file.settings["security.services.search.google.allow"].value, SettingValue::Bool(true));
    }

    #[test]
    fn migrate_does_not_clobber_existing_new_keys() {
        let mut file = SettingsFile::default();
        file.settings.insert("web.defaults.allow_read".to_string(), SettingEntry {
            value: SettingValue::Bool(true),
            modified: now_str(),
        });
        file.settings.insert("security.web.allow_read".to_string(), SettingEntry {
            value: SettingValue::Bool(false),
            modified: now_str(),
        });
        migrate_setting_ids(&mut file);

        // New key keeps its value, old key is dropped
        assert_eq!(file.settings["security.web.allow_read"].value, SettingValue::Bool(false));
        assert!(!file.settings.contains_key("web.defaults.allow_read"));
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
        // Default: no allow rules, network blocks everything
        assert!(!m.network.default_allow_read);
        assert!(!m.network.default_allow_write);
        // MCP default is allow
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Allow);
        // Domain policy denies unknown domains by default
        let (action, _) = m.domain.evaluate("unknown.example.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn merged_user_enables_provider() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let m = MergedPolicies::from_files(&user, &empty_file());
        // Network should have rules for anthropic domains
        assert!(!m.network.rules.is_empty());
        // Domain policy should have anthropic domains in allow
        let has_anthropic = m.network.rules.iter().any(|r| {
            r.allow_read && r.matcher.matches("api.anthropic.com")
        });
        assert!(has_anthropic, "expected anthropic domains in allow rules");
    }

    #[test]
    fn merged_user_enables_search() {
        let user = file_with(vec![
            ("security.services.search.google.allow", SettingValue::Bool(true)),
        ]);
        let m = MergedPolicies::from_files(&user, &empty_file());
        let has_google_search = m.network.rules.iter().any(|r| {
            r.allow_read && r.matcher.matches("www.google.com")
        });
        assert!(has_google_search, "expected google search domains in allow rules");
    }

    #[test]
    fn merged_mcp_default_is_allow() {
        let m = MergedPolicies::from_files(&empty_file(), &empty_file());
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Allow);
    }

    #[test]
    fn merged_user_sets_mcp_warn() {
        use crate::mcp::policy::{McpUserConfig, ToolDecision};
        let user = file_with_mcp(vec![], McpUserConfig {
            default_tool_permission: Some(ToolDecision::Warn),
            ..Default::default()
        });
        let m = MergedPolicies::from_files(&user, &empty_file());
        assert_eq!(m.mcp.default_tool_decision, ToolDecision::Warn);
    }

    #[test]
    fn merged_all_policies_populated() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("security.web.allow_read", SettingValue::Bool(true)),
        ]);
        let m = MergedPolicies::from_files(&user, &empty_file());
        // All 6 fields should be populated (non-default for network at least)
        assert!(!m.network.rules.is_empty());
        assert!(m.network.default_allow_read);
        // Guest config has env vars (provider toggle injects CAPSEM_ANTHROPIC_ALLOWED)
        assert!(m.guest.env.is_some());
        // VM settings have defaults
        assert!(m.vm.cpu_count.is_some());
    }

    // -----------------------------------------------------------------------
    // R: Preset -> MergedPolicies pipeline (6)
    // -----------------------------------------------------------------------

    fn apply_and_merge(preset_id: &str) -> MergedPolicies {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        // Write empty files
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        write_settings_file(&corp_path, &SettingsFile::default()).unwrap();
        // Apply preset
        apply_preset_to(preset_id, &user_path, &corp_path).unwrap();
        // Load and merge
        let user = load_settings_file(&user_path).unwrap();
        let corp = load_settings_file(&corp_path).unwrap();
        MergedPolicies::from_files(&user, &corp)
    }

    #[test]
    fn preset_high_merged_mcp_warn() {
        let m = apply_and_merge("high");
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Warn);
    }

    #[test]
    fn preset_medium_merged_mcp_allow() {
        let m = apply_and_merge("medium");
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Allow);
    }

    #[test]
    fn preset_high_merged_network_blocks_web() {
        let m = apply_and_merge("high");
        assert!(!m.network.default_allow_read);
        assert!(!m.network.default_allow_write);
    }

    #[test]
    fn preset_medium_merged_network_allows_read() {
        let m = apply_and_merge("medium");
        assert!(m.network.default_allow_read);
        assert!(!m.network.default_allow_write);
    }

    #[test]
    fn preset_switch_medium_to_high() {
        use crate::mcp::policy::ToolDecision;
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        write_settings_file(&corp_path, &SettingsFile::default()).unwrap();

        apply_preset_to("medium", &user_path, &corp_path).unwrap();
        let user = load_settings_file(&user_path).unwrap();
        let corp = load_settings_file(&corp_path).unwrap();
        let m = MergedPolicies::from_files(&user, &corp);
        assert_eq!(m.mcp.default_tool_decision, ToolDecision::Allow);
        assert!(m.network.default_allow_read);

        apply_preset_to("high", &user_path, &corp_path).unwrap();
        let user = load_settings_file(&user_path).unwrap();
        let corp = load_settings_file(&corp_path).unwrap();
        let m = MergedPolicies::from_files(&user, &corp);
        assert_eq!(m.mcp.default_tool_decision, ToolDecision::Warn);
        assert!(!m.network.default_allow_read);
    }

    #[test]
    fn preset_switch_high_to_medium() {
        use crate::mcp::policy::ToolDecision;
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        write_settings_file(&corp_path, &SettingsFile::default()).unwrap();

        apply_preset_to("high", &user_path, &corp_path).unwrap();
        let user = load_settings_file(&user_path).unwrap();
        let corp = load_settings_file(&corp_path).unwrap();
        let m = MergedPolicies::from_files(&user, &corp);
        assert_eq!(m.mcp.default_tool_decision, ToolDecision::Warn);

        apply_preset_to("medium", &user_path, &corp_path).unwrap();
        let user = load_settings_file(&user_path).unwrap();
        let corp = load_settings_file(&corp_path).unwrap();
        let m = MergedPolicies::from_files(&user, &corp);
        assert_eq!(m.mcp.default_tool_decision, ToolDecision::Allow);
        assert!(m.network.default_allow_read);
    }

    // -----------------------------------------------------------------------
    // S: Corp override persistence (11)
    // -----------------------------------------------------------------------

    #[test]
    fn corp_forces_provider_on() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let m = MergedPolicies::from_files(&user, &corp);
        let has_anthropic_allowed = m.network.rules.iter().any(|r| {
            r.allow_read && r.matcher.matches("api.anthropic.com")
        });
        assert!(has_anthropic_allowed);
    }

    #[test]
    fn corp_forces_provider_off() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let m = MergedPolicies::from_files(&user, &corp);
        // The toggle is off due to corp override, so anthropic should be blocked
        let anthropic_allowed = m.network.rules.iter().any(|r| {
            r.allow_read && r.matcher.matches("api.anthropic.com")
        });
        assert!(!anthropic_allowed);
    }

    #[test]
    fn corp_sets_api_key() {
        let user = file_with(vec![("ai.openai.api_key", SettingValue::Text("user-key".into()))]);
        let corp = file_with(vec![("ai.openai.api_key", SettingValue::Text("corp-key".into()))]);
        let m = MergedPolicies::from_files(&user, &corp);
        let env = m.guest.env.unwrap();
        assert_eq!(env.get("OPENAI_API_KEY").map(|s| s.as_str()), Some("corp-key"));
    }

    #[test]
    fn corp_sets_custom_allow_list() {
        let user = empty_file();
        let corp = file_with(vec![("security.web.custom_allow", SettingValue::Text("internal.corp.com".into()))]);
        let m = MergedPolicies::from_files(&user, &corp);
        let has_corp_domain = m.network.rules.iter().any(|r| {
            r.allow_read && r.matcher.matches("internal.corp.com")
        });
        assert!(has_corp_domain);
    }

    #[test]
    fn corp_sets_custom_block_list() {
        let user = file_with(vec![("security.web.allow_read", SettingValue::Bool(true))]);
        let corp = file_with(vec![("security.web.custom_block", SettingValue::Text("evil.com".into()))]);
        let m = MergedPolicies::from_files(&user, &corp);
        let evil_blocked = m.network.rules.iter().any(|r| {
            !r.allow_read && r.matcher.matches("evil.com")
        });
        assert!(evil_blocked);
    }

    #[test]
    fn corp_mcp_overrides_preset() {
        use crate::mcp::policy::{McpUserConfig, ToolDecision};
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        let corp = SettingsFile {
            settings: HashMap::new(),
            mcp: Some(McpUserConfig {
                default_tool_permission: Some(ToolDecision::Block),
                ..Default::default()
            }),
        };
        write_settings_file(&corp_path, &corp).unwrap();

        let skipped = apply_preset_to("high", &user_path, &corp_path).unwrap();
        assert!(skipped.contains(&"mcp.default_tool_permission".to_string()));

        let user = load_settings_file(&user_path).unwrap();
        let corp = load_settings_file(&corp_path).unwrap();
        let m = MergedPolicies::from_files(&user, &corp);
        assert_eq!(m.mcp.default_tool_decision, ToolDecision::Block);
    }

    #[test]
    fn corp_mcp_survives_both_presets() {
        use crate::mcp::policy::{McpUserConfig, ToolDecision};
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        let corp = SettingsFile {
            settings: HashMap::new(),
            mcp: Some(McpUserConfig {
                default_tool_permission: Some(ToolDecision::Block),
                ..Default::default()
            }),
        };
        write_settings_file(&corp_path, &corp).unwrap();

        apply_preset_to("medium", &user_path, &corp_path).unwrap();
        let u = load_settings_file(&user_path).unwrap();
        let c = load_settings_file(&corp_path).unwrap();
        assert_eq!(MergedPolicies::from_files(&u, &c).mcp.default_tool_decision, ToolDecision::Block);

        apply_preset_to("high", &user_path, &corp_path).unwrap();
        let u = load_settings_file(&user_path).unwrap();
        let c = load_settings_file(&corp_path).unwrap();
        assert_eq!(MergedPolicies::from_files(&u, &c).mcp.default_tool_decision, ToolDecision::Block);
    }

    #[test]
    fn corp_setting_persists_after_preset() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        let corp = file_with(vec![("security.web.allow_read", SettingValue::Bool(true))]);
        write_settings_file(&corp_path, &corp).unwrap();

        // High preset wants allow_read=false, but corp locks it to true
        let skipped = apply_preset_to("high", &user_path, &corp_path).unwrap();
        assert!(skipped.contains(&"security.web.allow_read".to_string()));

        let user = load_settings_file(&user_path).unwrap();
        let corp = load_settings_file(&corp_path).unwrap();
        let m = MergedPolicies::from_files(&user, &corp);
        assert!(m.network.default_allow_read);
    }

    #[test]
    fn corp_locks_multiple_all_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        // Corp locks 3 of the 5 settings in the high preset
        let corp = file_with(vec![
            ("security.web.allow_read", SettingValue::Bool(true)),
            ("security.web.allow_write", SettingValue::Bool(true)),
            ("security.services.search.google.allow", SettingValue::Bool(false)),
        ]);
        write_settings_file(&corp_path, &corp).unwrap();

        let skipped = apply_preset_to("high", &user_path, &corp_path).unwrap();
        assert_eq!(skipped.len(), 3);
        assert!(skipped.contains(&"security.web.allow_read".to_string()));
        assert!(skipped.contains(&"security.web.allow_write".to_string()));
        assert!(skipped.contains(&"security.services.search.google.allow".to_string()));
    }

    #[test]
    fn corp_mcp_not_written_to_user_toml() {
        use crate::mcp::policy::{McpUserConfig, ToolDecision};
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        write_settings_file(&user_path, &SettingsFile::default()).unwrap();
        let corp = SettingsFile {
            settings: HashMap::new(),
            mcp: Some(McpUserConfig {
                default_tool_permission: Some(ToolDecision::Block),
                ..Default::default()
            }),
        };
        write_settings_file(&corp_path, &corp).unwrap();

        apply_preset_to("high", &user_path, &corp_path).unwrap();
        let user = load_settings_file(&user_path).unwrap();
        // User TOML should NOT have MCP permission set (corp blocked it)
        let user_perm = user.mcp.as_ref().and_then(|m| m.default_tool_permission);
        assert!(user_perm.is_none(), "user.toml should not have default_tool_permission when corp locks it");
    }

    #[test]
    fn preset_preserves_user_mcp_servers() {
        use crate::mcp::policy::{McpManualServer, McpUserConfig, ToolDecision};
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("user.toml");
        let corp_path = dir.path().join("corp.toml");
        let user = SettingsFile {
            settings: HashMap::new(),
            mcp: Some(McpUserConfig {
                servers: vec![McpManualServer {
                    name: "myserver".into(),
                    url: "http://localhost:8080".into(),
                    headers: HashMap::new(),
                    bearer_token: None,
                    enabled: true,
                }],
                tool_permissions: {
                    let mut m = HashMap::new();
                    m.insert("myserver__danger".into(), ToolDecision::Block);
                    m
                },
                ..Default::default()
            }),
        };
        write_settings_file(&user_path, &user).unwrap();
        write_settings_file(&corp_path, &SettingsFile::default()).unwrap();

        apply_preset_to("high", &user_path, &corp_path).unwrap();
        let user = load_settings_file(&user_path).unwrap();
        let mcp = user.mcp.unwrap();
        assert_eq!(mcp.servers.len(), 1);
        assert_eq!(mcp.servers[0].name, "myserver");
        assert_eq!(mcp.tool_permissions.get("myserver__danger"), Some(&ToolDecision::Block));
        assert_eq!(mcp.default_tool_permission, Some(ToolDecision::Warn));
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
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Allow);
    }

    #[test]
    fn merged_from_missing_corp_toml() {
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("missing_corp.toml");
        let corp = load_settings_file(&nonexistent).unwrap_or_default();
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let m = MergedPolicies::from_files(&user, &corp);
        assert!(!m.network.rules.is_empty());
    }

    #[test]
    fn merged_from_both_missing() {
        let dir = tempfile::tempdir().unwrap();
        let u = load_settings_file(&dir.path().join("u.toml")).unwrap_or_default();
        let c = load_settings_file(&dir.path().join("c.toml")).unwrap_or_default();
        let m = MergedPolicies::from_files(&u, &c);
        assert!(!m.network.default_allow_read);
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Allow);
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
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Allow);
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
        assert!(!m.network.rules.is_empty());
    }

    #[test]
    fn merged_ignores_unknown_setting_ids() {
        let user = file_with(vec![
            ("nonexistent.setting.foo", SettingValue::Bool(true)),
            ("ai.anthropic.allow", SettingValue::Bool(true)),
        ]);
        let m = MergedPolicies::from_files(&user, &empty_file());
        // Should not crash, anthropic should still work
        let has_anthropic = m.network.rules.iter().any(|r| {
            r.allow_read && r.matcher.matches("api.anthropic.com")
        });
        assert!(has_anthropic);
    }

    #[test]
    fn merged_wrong_type_for_bool_setting() {
        // SettingValue::Text for a Bool-type setting -- resolve will use default
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Text("yes".into()))]);
        let m = MergedPolicies::from_files(&user, &empty_file());
        // The bool check should fail gracefully (as_bool returns None -> default false)
        let anthropic_allowed = m.network.rules.iter().any(|r| {
            r.allow_read && r.matcher.matches("api.anthropic.com")
        });
        // With wrong type, the effective value is the user's Text("yes"), but
        // as_bool() returns None so toggle evaluates to false
        assert!(!anthropic_allowed);
    }

    #[test]
    fn merged_wrong_type_for_number_setting() {
        let user = file_with(vec![("vm.resources.cpu_count", SettingValue::Text("four".into()))]);
        let m = MergedPolicies::from_files(&user, &empty_file());
        // as_number() returns None -> falls back to default (4)
        assert_eq!(m.vm.cpu_count, Some(4));
    }

    #[test]
    fn merged_empty_domain_list() {
        let user = file_with(vec![("security.web.custom_allow", SettingValue::Text("".into()))]);
        let m = MergedPolicies::from_files(&user, &empty_file());
        // Should not crash, empty string -> no domains added
        assert!(!m.network.default_allow_read);
    }

    #[test]
    fn merged_empty_mcp_section() {
        use crate::mcp::policy::McpUserConfig;
        let user = file_with_mcp(vec![], McpUserConfig::default());
        let m = MergedPolicies::from_files(&user, &empty_file());
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Allow);
    }

    #[test]
    fn merged_mcp_invalid_permission_string() {
        // ToolDecision serde will reject "yolo" during TOML parsing.
        // If we construct it manually via the struct, the default path handles it.
        // Test that from_files handles a default McpUserConfig gracefully.
        let user = file_with_mcp(vec![], crate::mcp::policy::McpUserConfig {
            default_tool_permission: None, // "yolo" can't be constructed as ToolDecision
            ..Default::default()
        });
        let m = MergedPolicies::from_files(&user, &empty_file());
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Allow);
    }

    #[test]
    fn merged_partial_settings_file() {
        // TOML with only [mcp] section, no [settings]
        use crate::mcp::policy::{McpUserConfig, ToolDecision};
        let user = SettingsFile {
            settings: HashMap::new(),
            mcp: Some(McpUserConfig {
                default_tool_permission: Some(ToolDecision::Block),
                ..Default::default()
            }),
        };
        let m = MergedPolicies::from_files(&user, &empty_file());
        assert_eq!(m.mcp.default_tool_decision, ToolDecision::Block);
        // No settings -> defaults for everything else
        assert!(!m.network.default_allow_read);
    }

    #[test]
    fn merged_partial_settings_only() {
        // Settings but no MCP section
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        assert!(user.mcp.is_none());
        let m = MergedPolicies::from_files(&user, &empty_file());
        // MCP defaults
        assert_eq!(m.mcp.default_tool_decision, crate::mcp::policy::ToolDecision::Allow);
        // Settings applied
        let has_anthropic = m.network.rules.iter().any(|r| {
            r.allow_read && r.matcher.matches("api.anthropic.com")
        });
        assert!(has_anthropic);
    }
}

use super::*;

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
    let bashrc = defs.iter().find(|d| d.id == "vm.environment.shell.bashrc").unwrap();
    assert_eq!(bashrc.metadata.filetype.as_deref(), Some("bash"));
    let tmux = defs.iter().find(|d| d.id == "vm.environment.shell.tmux_conf").unwrap();
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
    let def = defs.iter().find(|d| d.id == "vm.environment.shell.bashrc").unwrap();
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
    let s = resolved.iter().find(|s| s.id == "vm.environment.shell.bashrc").unwrap();
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
    for id in ["ai.anthropic.api_key", "ai.openai.api_key", "ai.google.api_key"] {
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
fn brokered_credential_setting_metadata_does_not_materialize_guest_env() {
    let user = file_with(vec![(
        "ai.anthropic.api_key",
        SettingValue::Text("credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into()),
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
    let user = file_with(vec![(&path_def.id, SettingValue::Text("/custom/bin".into()))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert_eq!(env.get("PATH").unwrap(), "/custom/bin");
}

#[test]
fn empty_env_var_setting_not_injected() {
    // A setting with env_vars metadata but empty value should not be injected.
    let user = file_with(vec![("ai.anthropic.api_key", SettingValue::Text("".into()))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
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
    // guest.env.LD_PRELOAD in settings.toml should be silently dropped.
    let user = file_with(vec![(
        "guest.env.LD_PRELOAD",
        SettingValue::Text("/evil/lib.so".into()),
    )]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("LD_PRELOAD"));
    assert!(!has_key, "LD_PRELOAD should be dropped by validation");
}

#[test]
fn settings_rejects_ld_library_path() {
    let user = file_with(vec![("guest.env.LD_LIBRARY_PATH", SettingValue::Text("/evil".into()))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("LD_LIBRARY_PATH"));
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
    assert_eq!(m.network.http_upstream_ports, vec![80, 3128, 3713, 8080, 11434]);
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
        SettingValue::IntList(vec![80, 3128, 3713, 8080, 11434]),
    )]);
    let m = MergedPolicies::from_files(&user, &corp);
    assert_eq!(m.network.http_upstream_ports, vec![80, 3128, 3713, 8080, 11434]);
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

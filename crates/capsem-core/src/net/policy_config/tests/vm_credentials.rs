use super::*;

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
// L: API key materialization guards
// -----------------------------------------------------------------------

#[test]
fn api_key_not_materialized_when_toggle_on() {
    let user = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(true)),
        ("ai.anthropic.api_key", SettingValue::Text("sk-test-123".into())),
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
    let user_path = dir.path().join("settings.toml");
    let store_path = dir.path().join("credential-store.json");
    let _settings_home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let _home_guard = EnvVarGuard::set("HOME", dir.path());
    let _store_guard = EnvVarGuard::set(crate::credential_broker::STORE_PATH_ENV, &store_path);

    let obs = crate::credential_broker::CredentialObservation {
        provider: crate::credential_broker::CredentialProvider::Anthropic,
        raw_value: "sk-ant-keychain-env".to_string(),
        source: ".env:ANTHROPIC_API_KEY".to_string(),
        event_type: Some("file.content".to_string()),
        trace_id: None,
        context_json: None,
    };
    let brokered = crate::credential_broker::broker_observed_credential(&obs).unwrap();
    assert!(
        !user_path.exists(),
        "credential broker must not write settings.toml for Anthropic discovery"
    );
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();

    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
    assert_eq!(
        crate::credential_broker::resolve_broker_reference_for_provider(
            crate::credential_broker::CredentialProvider::Anthropic,
            &brokered.credential_ref,
        )
        .unwrap()
        .as_deref(),
        Some("sk-ant-keychain-env")
    );
}

#[test]
fn brokered_google_api_key_ref_stays_out_of_guest_env() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("settings.toml");
    let store_path = dir.path().join("credential-store.json");
    let _settings_home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let _home_guard = EnvVarGuard::set("HOME", dir.path());
    let _store_guard = EnvVarGuard::set(crate::credential_broker::STORE_PATH_ENV, &store_path);

    let obs = crate::credential_broker::CredentialObservation {
        provider: crate::credential_broker::CredentialProvider::Google,
        raw_value: "AIza-keychain-env".to_string(),
        source: ".env:GEMINI_API_KEY".to_string(),
        event_type: Some("file.content".to_string()),
        trace_id: None,
        context_json: None,
    };
    let brokered = crate::credential_broker::broker_observed_credential(&obs).unwrap();
    assert!(
        !user_path.exists(),
        "credential broker must not write settings.toml for Google discovery"
    );
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();

    assert!(!env.contains_key("GEMINI_API_KEY"));
    assert!(!env.contains_key("GOOGLE_API_KEY"));
    assert_eq!(
        crate::credential_broker::resolve_broker_reference_for_provider(
            crate::credential_broker::CredentialProvider::Google,
            &brokered.credential_ref,
        )
        .unwrap()
        .as_deref(),
        Some("AIza-keychain-env")
    );
}

#[test]
fn brokered_openai_key_does_not_write_settings_or_raw_secret() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("settings.toml");
    let store_path = dir.path().join("credential-store.json");
    let _settings_home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let _home_guard = EnvVarGuard::set("HOME", dir.path());
    let _store_guard = EnvVarGuard::set(crate::credential_broker::STORE_PATH_ENV, &store_path);

    let obs = crate::credential_broker::CredentialObservation {
        provider: crate::credential_broker::CredentialProvider::OpenAi,
        raw_value: "sk-openai-discovery-secret".to_string(),
        source: "http.header.authorization".to_string(),
        event_type: Some("http.request".to_string()),
        trace_id: Some("trace-discovery".to_string()),
        context_json: None,
    };

    let brokered = crate::credential_broker::broker_observed_credential(&obs).unwrap();
    assert!(brokered.credential_ref.starts_with("credential:blake3:"));
    assert!(
        !user_path.exists(),
        "credential broker must not create settings.toml for provider discovery"
    );
    assert_eq!(
        crate::credential_broker::resolve_broker_reference_for_provider(
            crate::credential_broker::CredentialProvider::OpenAi,
            &brokered.credential_ref,
        )
        .unwrap()
        .as_deref(),
        Some("sk-openai-discovery-secret")
    );
}

#[test]
fn brokered_provider_discovery_does_not_mutate_settings() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("settings.toml");
    let store_path = dir.path().join("credential-store.json");
    write_settings_file(&user_path, &SettingsFile::default()).unwrap();

    let _settings_home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let _home_guard = EnvVarGuard::set("HOME", dir.path());
    let _store_guard = EnvVarGuard::set(crate::credential_broker::STORE_PATH_ENV, &store_path);

    let obs = crate::credential_broker::CredentialObservation {
        provider: crate::credential_broker::CredentialProvider::OpenAi,
        raw_value: "sk-openai-corp-locked".to_string(),
        source: ".env:OPENAI_API_KEY".to_string(),
        event_type: Some("file.event".to_string()),
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
        loaded.ai.is_empty(),
        "provider discovery belongs to broker/plugin status, not settings.toml"
    );
}

#[test]
fn api_key_not_materialized_when_toggle_off() {
    let user = file_with(vec![
        ("ai.anthropic.allow", SettingValue::Bool(false)),
        ("ai.anthropic.api_key", SettingValue::Text("sk-test-123".into())),
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
    let has_key = gc.env.as_ref().is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
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
        ("ai.openai.api_key", SettingValue::Text("sk-oai-test".into())),
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
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into(),
            ),
        ),
        (
            "ai.openai.api_key",
            SettingValue::Text(
                "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222".into(),
            ),
        ),
        ("ai.google.allow", SettingValue::Bool(false)),
        (
            "ai.google.api_key",
            SettingValue::Text(
                "credential:blake3:3333333333333333333333333333333333333333333333333333333333333333".into(),
            ),
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
    let has_ant = gc.env.as_ref().is_some_and(|e| e.contains_key("ANTHROPIC_API_KEY"));
    let has_oai = gc.env.as_ref().is_some_and(|e| e.contains_key("OPENAI_API_KEY"));
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
    assert!(!files.iter().any(|f| f.path == "/root/.gemini/settings.json"));
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

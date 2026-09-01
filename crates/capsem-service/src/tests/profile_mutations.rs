use super::*;

#[tokio::test]
async fn profile_status_rejects_tampered_pinned_profile_files() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    std::fs::write(
        config_root.join("profiles/code/enforcement.toml"),
        "# tampered after profile hash pin\n",
    )
    .unwrap();

    let state = make_asset_state(dir.path().join("assets"));
    let app = build_service_router(state);

    let (status, body) = route_request(app, axum::http::Method::GET, "/profiles/status", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["profile_count"], 1);
    assert_eq!(body["ready_count"], 0);
    assert_eq!(body["profiles"][0]["ready"], false);
    assert!(body["profiles"][0]["invalid_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|file| file["kind"] == "enforcement" && file["valid"] == false));
}

#[tokio::test]
async fn profile_asset_status_download_and_corruption_checks_use_profile_pins() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, profile) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let assets_dir = dir.path().join("assets");
    let state = make_asset_state(assets_dir.clone());
    let app = build_service_router(state);
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let rootfs = &profile.assets.current_arch_assets().unwrap().rootfs;
    let rootfs_target = assets_dir.join(arch).join(capsem_assets::asset_manager::hash_filename(
        &rootfs.name,
        rootfs
            .hash
            .as_deref()
            .expect("rootfs hash")
            .strip_prefix("blake3:")
            .unwrap(),
    ));

    let (status, before) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/assets/status",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(before["ready"], false);
    assert_eq!(before["missing_assets"].as_array().unwrap().len(), 3);

    let (status, ensured) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/assets/ensure",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ensured}");
    assert_eq!(ensured["started"], true);
    assert_eq!(ensured["downloading"], true);
    assert_eq!(ensured["ready"], false);
    let completed = asset_wait::wait_for_profile_assets(&app).await;
    assert_eq!(completed["ready"], true, "{completed}");
    assert_eq!(completed["downloaded"], 3, "{completed}");
    assert!(rootfs_target.exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&rootfs_target, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::fs::write(&rootfs_target, b"corrupted-rootfs").unwrap();
    let (status, cached_after_tamper) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/assets/status",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cached_after_tamper}");
    assert_eq!(
        cached_after_tamper["ready"], true,
        "hot asset status is cache-backed and must not re-hash large assets per route poll"
    );
    assert!(cached_after_tamper["invalid_assets"].as_array().unwrap().is_empty());

    let (status, repairing) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/assets/ensure",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{repairing}");
    assert_eq!(repairing["started"], true);
    assert_eq!(repairing["downloading"], true);
    let repaired = asset_wait::wait_for_profile_assets(&app).await;
    assert_eq!(repaired["ready"], true, "{repaired}");
    assert_eq!(repaired["downloaded"], 1, "{repaired}");
}

#[tokio::test]
async fn profile_mcp_tool_edit_writes_profile_rule_and_mutation_ledger() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let _home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    capsem_core::mcp::save_tool_cache(&[capsem_core::mcp::ToolCacheEntry {
        namespaced_name: "local__fetch_http".to_string(),
        original_name: "fetch_http".to_string(),
        description: Some("Fetch HTTP".to_string()),
        server_name: "local".to_string(),
        annotations: None,
        pin_hash: "tool-pin".to_string(),
        first_seen: "2026-06-10T00:00:00Z".to_string(),
        last_seen: "2026-06-10T00:00:00Z".to_string(),
        approved: true,
    }])
    .expect("write test MCP tool cache");
    let state = make_asset_state(dir.path().join("assets"));
    let app = build_service_router(Arc::clone(&state));

    let (status, edited) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/profiles/code/mcp/servers/local/tools/fetch_http/edit",
        Some(json!({ "action": "ask" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["profile_id"], "code");
    assert_eq!(edited["server_id"], "local");
    assert_eq!(edited["tool_id"], "fetch_http");
    assert_eq!(edited["action"], "ask");
    assert_eq!(edited["mutation"]["category"], "mcp");
    assert_eq!(edited["mutation"]["target_kind"], "mcp_tool");
    assert_eq!(edited["mutation"]["status"], "applied");

    let enforcement =
        std::fs::read_to_string(config_root.join("profiles/code/enforcement.toml")).expect("mutated enforcement file");
    let rule_profile = SecurityRuleProfile::parse_toml(&enforcement).unwrap();
    let rule = rule_profile
        .profiles
        .rules
        .get("mcp_local_fetch_http_permission")
        .expect("profile-managed MCP permission rule");
    assert_eq!(rule.action, capsem_core::net::policy_config::SecurityRuleAction::Ask);
    assert_eq!(
        rule.condition,
        r#"mcp.server.name == "local" && mcp.tool_call.name == "fetch_http""#
    );

    let profile: ProfileConfigFile =
        toml::from_str(&std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap()).unwrap();
    let descriptor = profile.files.enforcement.expect("updated enforcement pin");
    assert_eq!(descriptor.path, "profiles/code/enforcement.toml");
    assert_eq!(
        descriptor.hash,
        Some(format!(
            "blake3:{}",
            capsem_assets::asset_manager::hash_file(&config_root.join("profiles/code/enforcement.toml")).unwrap()
        ))
    );

    state
        .profile_mutation_db
        .flush()
        .await
        .expect("flush profile mutation DB before ledger assertion");
    let main_db = state.main_db_path();
    let reader = capsem_logger::DbReader::open(&main_db).expect("main.db mutation ledger");
    let rows = reader
        .query_raw(
            "SELECT profile_id, category, target_kind, target_key, operation, status \
             FROM profile_mutation_events",
        )
        .expect("query profile mutation events");
    let rows: serde_json::Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(
        rows["rows"][0],
        json!(["code", "mcp", "mcp_tool", "local/fetch_http", "permission", "applied"])
    );

    let (status, tools) = route_request(
        app,
        axum::http::Method::GET,
        "/profiles/code/mcp/servers/local/tools/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tools}");
    assert_eq!(tools[0]["namespaced_name"], "local__fetch_http");
    assert_eq!(tools[0]["permission_action"], "ask");
    assert_eq!(tools[0]["permission_source"], "profile_managed");
    assert!(tools[0].get("approved").is_none(), "{tools}");
}

#[tokio::test]
async fn profile_mcp_default_edit_writes_default_rule_and_mutation_ledger() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let _home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    capsem_core::mcp::save_tool_cache(&[capsem_core::mcp::ToolCacheEntry {
        namespaced_name: "local__fetch_http".to_string(),
        original_name: "fetch_http".to_string(),
        description: Some("Fetch HTTP".to_string()),
        server_name: "local".to_string(),
        annotations: None,
        pin_hash: "tool-pin".to_string(),
        first_seen: "2026-06-10T00:00:00Z".to_string(),
        last_seen: "2026-06-10T00:00:00Z".to_string(),
        approved: true,
    }])
    .expect("write test MCP tool cache");
    let state = make_asset_state(dir.path().join("assets"));
    let app = build_service_router(Arc::clone(&state));

    let (status, initial) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/mcp/default/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{initial}");
    assert_eq!(initial["action"], "allow");
    assert_eq!(initial["source"], "default");
    assert_eq!(initial["rule_id"], "default.mcp");

    let (status, edited) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/profiles/code/mcp/default/edit",
        Some(json!({ "action": "ask" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["profile_id"], "code");
    assert_eq!(edited["action"], "ask");
    assert_eq!(edited["mutation"]["category"], "mcp");
    assert_eq!(edited["mutation"]["target_kind"], "mcp_default");
    assert_eq!(edited["mutation"]["target_key"], "default.mcp");
    assert_eq!(edited["mutation"]["rule_id"], "default.mcp");
    assert_eq!(edited["mutation"]["status"], "applied");

    let enforcement =
        std::fs::read_to_string(config_root.join("profiles/code/enforcement.toml")).expect("mutated enforcement file");
    let rule_profile = SecurityRuleProfile::parse_toml(&enforcement).unwrap();
    let default = rule_profile.default.get("mcp").expect("default mcp rule");
    assert_eq!(default.action, capsem_core::net::policy_config::SecurityRuleAction::Ask);

    let (status, refreshed) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/mcp/default/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{refreshed}");
    assert_eq!(refreshed["action"], "ask");
    assert_eq!(refreshed["source"], "default");
    assert_eq!(refreshed["rule_id"], "default.mcp");

    let profile: ProfileConfigFile =
        toml::from_str(&std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap()).unwrap();
    let descriptor = profile.files.enforcement.expect("updated enforcement pin");
    assert_eq!(descriptor.path, "profiles/code/enforcement.toml");
    assert_eq!(
        descriptor.hash,
        Some(format!(
            "blake3:{}",
            capsem_assets::asset_manager::hash_file(&config_root.join("profiles/code/enforcement.toml")).unwrap()
        ))
    );

    state
        .profile_mutation_db
        .flush()
        .await
        .expect("flush profile mutation DB before ledger assertion");
    let main_db = state.main_db_path();
    let reader = capsem_logger::DbReader::open(&main_db).expect("main.db mutation ledger");
    let rows = reader
        .query_raw(
            "SELECT profile_id, category, target_kind, target_key, operation, status \
             FROM profile_mutation_events",
        )
        .expect("query profile mutation events");
    let rows: serde_json::Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(
        rows["rows"][0],
        json!(["code", "mcp", "mcp_default", "default.mcp", "permission", "applied"])
    );

    let (status, tools) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/mcp/servers/local/tools/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tools}");
    assert_eq!(tools[0]["permission_action"], "ask");
    assert_eq!(tools[0]["permission_source"], "default");
    assert!(tools[0].get("approved").is_none(), "{tools}");

    let (status, default_info) =
        route_request(app, axum::http::Method::GET, "/profiles/code/mcp/default/info", None).await;
    assert_eq!(status, StatusCode::OK, "{default_info}");
    assert_eq!(default_info["action"], "ask");
}

#[tokio::test]
async fn profile_mcp_server_edit_delete_persist_profile_and_mutation_ledger() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let _home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let state = make_asset_state(dir.path().join("assets"));
    let app = build_service_router(Arc::clone(&state));

    let (status, edited) = route_request(
        app.clone(),
        axum::http::Method::PUT,
        "/profiles/code/mcp/servers/github/edit",
        Some(json!({
            "url": "https://mcp.invalid/github",
            "enabled": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["profile_id"], "code");
    assert_eq!(edited["server_id"], "github");
    assert_eq!(edited["url"], "https://mcp.invalid/github");
    assert_eq!(edited["enabled"], true);
    assert_eq!(edited["mutation"]["category"], "mcp");
    assert_eq!(edited["mutation"]["filename"], "profile.toml");
    assert_eq!(edited["mutation"]["affected_path"], "profiles/code/profile.toml");
    assert_eq!(edited["mutation"]["target_kind"], "mcp_server");
    assert_eq!(edited["mutation"]["target_key"], "github");
    assert_eq!(edited["mutation"]["operation"], "upsert");
    assert_eq!(edited["mutation"]["status"], "applied");

    let profile: ProfileConfigFile =
        toml::from_str(&std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap()).unwrap();
    assert!(profile
        .mcp
        .as_ref()
        .unwrap()
        .servers
        .iter()
        .any(|server| server.name == "github" && server.url == "https://mcp.invalid/github" && server.enabled));

    let (status, servers) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/mcp/servers/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{servers}");
    assert!(servers
        .as_array()
        .unwrap()
        .iter()
        .any(|server| server["name"] == "github"
            && server["url"] == "https://mcp.invalid/github"
            && server["enabled"] == true));

    let (status, deleted) = route_request(
        app,
        axum::http::Method::DELETE,
        "/profiles/code/mcp/servers/github/delete",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted["profile_id"], "code");
    assert_eq!(deleted["server_id"], "github");
    assert_eq!(deleted["mutation"]["target_kind"], "mcp_server");
    assert_eq!(deleted["mutation"]["target_key"], "github");
    assert_eq!(deleted["mutation"]["operation"], "delete");
    assert_eq!(deleted["mutation"]["status"], "applied");

    let profile: ProfileConfigFile =
        toml::from_str(&std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap()).unwrap();
    assert!(!profile
        .mcp
        .as_ref()
        .unwrap()
        .servers
        .iter()
        .any(|server| server.name == "github"));

    state
        .profile_mutation_db
        .flush()
        .await
        .expect("flush profile mutation DB before ledger assertion");
    let main_db = state.main_db_path();
    let reader = capsem_logger::DbReader::open(&main_db).expect("main.db mutation ledger");
    let rows = reader
        .query_raw(
            "SELECT profile_id, category, filename, target_kind, target_key, operation, status \
             FROM profile_mutation_events ORDER BY id ASC",
        )
        .expect("query profile mutation events");
    let rows: serde_json::Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(
        rows["rows"],
        json!([
            [
                "code",
                "mcp",
                "profile.toml",
                "mcp_server",
                "github",
                "upsert",
                "applied"
            ],
            [
                "code",
                "mcp",
                "profile.toml",
                "mcp_server",
                "github",
                "delete",
                "applied"
            ]
        ])
    );
}

#[test]
fn profile_mutation_log_fields_match_ledger_contract() {
    let event = capsem_logger::ProfileMutationEvent {
        timestamp_unix_ms: 1_789_000_000_000,
        mutation_id: "abc123def456".into(),
        profile_id: "code".into(),
        actor: "service-api".into(),
        category: "enforcement".into(),
        filename: "enforcement.toml".into(),
        affected_path: "profiles/code/enforcement.toml".into(),
        target_kind: "rule".into(),
        target_key: "eicar_block".into(),
        operation: "upsert".into(),
        rule_id: Some("profiles.rules.eicar_block".into()),
        old_hash: format!("blake3:{}", "1".repeat(64)),
        old_size: 10,
        new_hash: format!("blake3:{}", "2".repeat(64)),
        new_size: 20,
        status: capsem_logger::ProfileMutationStatus::Applied,
        error: None,
        trace_id: Some("trace-profile".into()),
    };

    let fields = profile_mutation_log_fields("enforcement_rule_upsert", &event);

    assert_eq!(fields["route"], "enforcement_rule_upsert");
    assert_eq!(fields["mutation_id"], "abc123def456");
    assert_eq!(fields["profile_id"], "code");
    assert_eq!(fields["actor"], "service-api");
    assert_eq!(fields["category"], "enforcement");
    assert_eq!(fields["filename"], "enforcement.toml");
    assert_eq!(fields["affected_path"], "profiles/code/enforcement.toml");
    assert_eq!(fields["target_kind"], "rule");
    assert_eq!(fields["target_key"], "eicar_block");
    assert_eq!(fields["operation"], "upsert");
    assert_eq!(fields["rule_id"], "profiles.rules.eicar_block");
    assert_eq!(fields["old_size"], 10);
    assert_eq!(fields["new_size"], 20);
    assert_eq!(fields["status"], "applied");
    assert_eq!(fields["trace_id"], "trace-profile");
}

#[tokio::test]
async fn profile_enforcement_list_uses_profile_files_and_corp_not_user_settings() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    add_profile_enforcement_rule(
        &config_root,
        "route_file_probe",
        capsem_core::net::policy_config::SecurityRule {
            name: "route_file_probe".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Allow,
            condition: r#"file.read.path.contains("skills/")"#.to_string(),
            enabled: true,
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Informational),
            priority: None,
            corp_locked: false,
            reason: Some("record skill file reads".to_string()),
            managed: None,
            plugin_config: BTreeMap::new(),
        },
    );
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let (_settings_guard, user_path, corp_path) = install_empty_settings_env(&dir);

    let mut user = capsem_core::net::policy_config::SettingsFile::default();
    user.profiles.rules.insert(
        "settings_only_should_not_load".to_string(),
        capsem_core::net::policy_config::SecurityRule {
            name: "settings_only_should_not_load".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Block,
            condition: r#"http.host.contains("settings-only.invalid")"#.to_string(),
            enabled: true,
            detection_level: None,
            priority: None,
            corp_locked: false,
            reason: Some("old settings route must not leak".to_string()),
            managed: None,
            plugin_config: BTreeMap::new(),
        },
    );
    capsem_core::net::policy_config::write_settings_file(&user_path, &user).unwrap();

    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp.rules.insert(
        "block_evil_example".to_string(),
        capsem_core::net::policy_config::SecurityRule {
            name: "block_evil_example".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Block,
            condition: r#"http.host.contains("evil.example")"#.to_string(),
            enabled: true,
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::High),
            priority: Some(capsem_core::net::policy_config::SecurityRulePriority::Explicit(-100)),
            corp_locked: false,
            reason: Some("corp proof".to_string()),
            managed: None,
            plugin_config: BTreeMap::new(),
        },
    );
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();

    let response: api::EnforcementRuleListResponse = decode_response_json(
        handle_enforcement_rules_list(State(make_test_state()), Path("code".to_string()))
            .await
            .expect("profile and corp rules compile"),
    )
    .await;

    assert!(response
        .rules
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.route_file_probe"
            && rule.source == api::EnforcementRuleSource::Profile));
    assert!(response
        .rules
        .iter()
        .any(|rule| rule.rule_id == "corp.rules.block_evil_example"
            && rule.source == api::EnforcementRuleSource::Corp
            && rule.corp_locked
            && rule.priority == -100));
    assert!(!response
        .rules
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.settings_only_should_not_load"));
}

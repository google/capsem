use super::*;

#[test]
fn profile_update_asset_summary_reflects_effective_contract() {
    let profile = ProfileConfigFile::builtin_primary();
    let summary = build_profile_summary(
        &profile,
        &ProfileCatalogSource::BuiltIn,
        &SettingsFile::default(),
        &SettingsFile::default(),
        3,
    )
    .expect("profile summary should compile profile-owned rules");

    assert_eq!(summary.id, "code");
    assert_eq!(summary.name, "Code");
    assert_eq!(summary.description, "Optimized for coding and long-running agents.");
    assert_eq!(summary.source, "built_in");
    assert_eq!(summary.plugin_count, 3);
    assert_eq!(
        summary.update_semantics.new_sessions,
        api::ProfileNewSessionUpdateSemantics::UseCurrentProfileCatalog
    );
    assert_eq!(
        summary.update_semantics.existing_vms,
        api::ProfileExistingVmUpdateSemantics::PinnedUntilRecreate
    );
    assert_eq!(
        summary.update_semantics.upgrade_action,
        api::ProfileUpgradeAction::RecreateVm
    );
    assert!(
        summary.rule_count >= summary.default_rule_count,
        "total rules cannot be lower than default rules"
    );
}

#[tokio::test]
async fn handle_profiles_list_returns_code_profile_inventory() {
    let state = make_test_state();

    let Json(response) = handle_profiles_list(State(state)).await.unwrap();

    assert_eq!(response.profiles.len(), 2);
    let code = response
        .profiles
        .iter()
        .find(|profile| profile.id == "code")
        .expect("code profile is listed");
    let co_work = response
        .profiles
        .iter()
        .find(|profile| profile.id == "co-work")
        .expect("co-work profile is listed");
    assert!(
        code.icon_svg.is_some(),
        "profile list must expose profile-owned icon_svg for launch surfaces"
    );
    assert!(
        co_work.icon_svg.is_some(),
        "every launchable profile must expose its own icon_svg"
    );
    assert!(
        code.plugin_count > 0,
        "profile inventory should reflect editable plugin policy"
    );
    assert_eq!(
        code.update_semantics.existing_vms,
        api::ProfileExistingVmUpdateSemantics::PinnedUntilRecreate
    );
}

#[tokio::test]
async fn handle_profiles_status_reports_builtin_catalog_and_rejects_fake_assets() {
    let (state, dir) = make_test_state_with_tempdir();

    let status_response = handle_profiles_status(State(state))
        .await
        .expect("profile status should load built-in catalog");
    let status: serde_json::Value = decode_response_json(status_response).await;

    assert_eq!(status["source"], "built_in");
    assert_eq!(status["profile_count"], 2);
    assert_eq!(
        status["ready_count"], 0,
        "S1-b status must verify asset hashes; placeholder files are not ready"
    );
    let code = status["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|profile| profile["id"] == "code")
        .expect("code profile status is present");
    assert_eq!(
        code["profile_payload_hash"],
        profile_payload_hash(&ProfileConfigFile::builtin_primary()).unwrap()
    );
    assert_eq!(
        code["update_semantics"],
        json!({
            "new_sessions": "use_current_profile_catalog",
            "existing_vms": "pinned_until_recreate",
            "upgrade_action": "recreate_vm",
        })
    );
    assert_eq!(code["ready"], false);
    assert!(!code["invalid_assets"].as_array().unwrap().is_empty());
    drop(dir);
}

#[tokio::test]
async fn profiles_status_byte_cache_refreshes_when_asset_manifest_appears() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let stale_response = handle_profiles_status(State(Arc::clone(&state)))
        .await
        .expect("initial profile status should load");
    let stale_status: serde_json::Value = decode_response_json(stale_response).await;
    assert_eq!(stale_status["asset_manifest"]["origin"], "missing");
    assert!(stale_status["asset_manifest"].get("format").is_none());

    std::fs::write(
        dir.path().join("manifest.json"),
        serde_json::json!({
            "format": 2,
            "refresh_policy": "24h",
            "assets": {
                "current": "2099.0101.1",
                "releases": {
                    "2099.0101.1": {
                        "date": "2099-01-01",
                        "deprecated": false,
                        "min_binary": "1.0.0",
                        "arches": {}
                    }
                }
            },
            "binaries": {
                "current": "1.3.1782496403",
                "releases": {
                    "1.3.1782496403": {
                        "date": "2026-06-26",
                        "deprecated": false,
                        "min_assets": "2099.0101.1"
                    }
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    let refreshed_response = handle_profiles_status(State(state))
        .await
        .expect("profile status should refresh when manifest file appears");
    let refreshed_status: serde_json::Value = decode_response_json(refreshed_response).await;

    assert_eq!(refreshed_status["asset_manifest"]["origin"], "installed");
    assert_eq!(refreshed_status["asset_manifest"]["validation_status"], "valid");
    assert_eq!(refreshed_status["asset_manifest"]["format"], 2);
    assert_eq!(refreshed_status["asset_manifest"]["assets_current"], "2099.0101.1");
}

#[test]
fn profile_catalog_status_reports_directory_catalog_readiness() {
    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let state = make_asset_state(dir.path().join("assets"));
    let profile = capsem_core::net::policy_config::Profile::load_from_dir(config_root.join("profiles/code")).unwrap();
    profile
        .download_assets(
            &state.assets_dir,
            capsem_core::net::policy_config::current_profile_arch(),
        )
        .unwrap();
    let profiles_dir = config_root.join("profiles");
    let catalog = ProfileCatalog::load_from_dir(&profiles_dir).unwrap();

    let status = profile_catalog_status_value(&state, &catalog);

    assert_eq!(
        status["source"], "profile",
        "status must not expose host filesystem profile source paths"
    );
    assert_eq!(status["profile_count"], 1);
    assert_eq!(status["ready_count"], 1);
    assert_eq!(status["profiles"][0]["id"], "code");
    assert_eq!(
        status["profiles"][0]["profile_payload_hash"],
        profile_payload_hash(profile.config()).unwrap()
    );
    assert_eq!(status["profiles"][0]["missing_assets"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn vm_list_omits_legacy_global_asset_health_when_profiles_are_authoritative() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let state = make_asset_state(dir.path().join("assets"));
    let profile = capsem_core::net::policy_config::Profile::load_from_dir(config_root.join("profiles/code")).unwrap();
    profile
        .download_assets(
            &state.assets_dir,
            capsem_core::net::policy_config::current_profile_arch(),
        )
        .unwrap();
    let app = build_service_router(state);

    let (status, profiles) = route_request(app.clone(), axum::http::Method::GET, "/profiles/status", None).await;
    assert_eq!(status, StatusCode::OK, "{profiles}");
    assert_eq!(profiles["ready_count"], 1, "{profiles}");
    assert_eq!(profiles["profiles"][0]["missing_assets"], json!([]), "{profiles}");

    let (status, list) = route_request(app, axum::http::Method::GET, "/vms/list", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert!(
        list.get("asset_health").is_none(),
        "/vms/list must not emit retired flat asset health once profiles own assets: {list}"
    );
}

#[test]
fn checked_in_profile_catalog_status_reports_code_and_co_work() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    let profiles_dir = repo_root.join("config/profiles");
    let catalog = ProfileCatalog::load_from_dir(&profiles_dir).expect("checked-in catalog loads");
    // The canonical Mac gate also runs this test inside a Linux container with
    // the checkout mounted read-only. Test state must therefore live in an
    // isolated writable directory instead of the repository's target tree.
    let dir = tempfile::tempdir().expect("writable test root");
    let state = make_asset_state(dir.path().join("assets"));

    let status = profile_catalog_status_value(&state, &catalog);
    let profile_ids = status["profiles"]
        .as_array()
        .expect("profiles array")
        .iter()
        .map(|profile| profile["id"].as_str().expect("profile id").to_string())
        .collect::<Vec<_>>();

    assert_eq!(status["profile_count"], 2);
    assert!(profile_ids.contains(&"code".to_string()), "{profile_ids:?}");
    assert!(profile_ids.contains(&"co-work".to_string()), "{profile_ids:?}");
    for profile in status["profiles"].as_array().expect("profiles array") {
        assert!(
            profile["profile_payload_hash"]
                .as_str()
                .is_some_and(|hash| hash.starts_with("blake3:")),
            "profile status must expose payload hash: {profile}"
        );
    }
}

#[tokio::test]
async fn handle_profiles_reload_reports_active_catalog_status() {
    let (state, _dir) = make_test_state_with_tempdir();

    let Json(response) = handle_profiles_reload(State(state))
        .await
        .expect("profile reload should validate active catalog");

    assert_eq!(response["reloaded"], true);
    assert_eq!(response["catalog"]["source"], "built_in");
    assert_eq!(response["catalog"]["profile_count"], 2);
    assert_eq!(response["catalog"]["ready_count"], 0);
}

#[tokio::test]
async fn reload_refreshes_session_runtime_profile_from_source_profile() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let (state, _dir) = make_test_state_with_tempdir();
    let profile = materialized_test_profile_for("code");
    install_test_profile_catalog(&state, &profile);
    let session_dir = state.run_dir.join("sessions/runtime-refresh");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "runtime-refresh", std::process::id(), session_dir.clone());

    state
        .refresh_active_profiles(Some("code"))
        .expect("initial runtime profile materialization");
    let active_profile = session_dir.join("vm/active_profile.toml");
    assert!(active_profile.exists(), "session must carry one active profile file");
    assert!(
        !std::fs::read_to_string(&active_profile)
            .unwrap()
            .contains("block_local_echo"),
        "fresh active profile should start from the original source profile"
    );

    let source_enforcement = state.run_dir.join("config/profiles/code/enforcement.toml");
    let mut updated = std::fs::read_to_string(&source_enforcement).unwrap();
    updated.push_str(
        r#"

[profiles.rules.block_local_echo]
name = "block_local_echo"
action = "block"
priority = 10
reason = "test blocks local echo through security rules"
match = 'mcp.tool_call.name == "local__echo"'
"#,
    );
    std::fs::write(&source_enforcement, updated).unwrap();

    state
        .refresh_active_profiles(Some("code"))
        .expect("reload must refresh session-local runtime profile config");
    let refreshed = std::fs::read_to_string(&active_profile).unwrap();
    assert!(
        refreshed.contains("block_local_echo"),
        "reload must materialize source profile edits into the active profile"
    );

    let Json(plugin_info) = update_plugin_for_scope(
        &state,
        "dummy_pre_eicar".to_string(),
        profile_plugin_scope(&state, "code".to_string()).unwrap(),
        PluginUpdate {
            mode: Some(capsem_core::net::policy_config::SecurityPluginMode::Block),
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Critical),
        },
    )
    .await
    .expect("plugin edit should update profile override");
    assert_eq!(
        plugin_info.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Block
    );
    assert_eq!(
        plugin_info.config.detection_level,
        capsem_core::net::policy_config::DetectionLevel::Critical
    );
    state
        .refresh_active_profiles(Some("code"))
        .expect("plugin override must refresh runtime profile config");
    let overlay_path = session_dir.join("runtime-config/profiles/code/runtime-overlay.toml");
    assert!(
        !overlay_path.exists(),
        "runtime overlay must not exist after active profile materialization"
    );
    let active_text = std::fs::read_to_string(&active_profile).unwrap();
    assert!(
        active_text.contains("[plugins.dummy_pre_eicar]"),
        "active profile must carry profile plugin overrides into launched VMs"
    );
    assert!(
        active_text.contains("mode = \"block\""),
        "active profile must carry edited plugin mode"
    );
    assert!(
        active_text.contains("detection_level = \"critical\""),
        "active profile must carry edited plugin detection level"
    );
}

#[test]
fn profile_catalog_reload_rejects_invalid_directory_catalog() {
    let state = make_test_state();
    let dir = tempfile::tempdir().unwrap();
    let profiles_dir = dir.path().join("profiles");
    std::fs::create_dir_all(profiles_dir.join("code")).unwrap();
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.id = "strict".to_string();
    std::fs::write(
        profiles_dir.join("code/profile.toml"),
        toml::to_string(&profile).unwrap(),
    )
    .unwrap();
    drop(state);

    let err = ProfileCatalog::load_from_dir(&profiles_dir).unwrap_err();
    assert!(
        err.contains("id mismatch"),
        "expected catalog validation error, got: {err}"
    );
}

#[tokio::test]
async fn handle_profile_info_rejects_unknown_profiles() {
    let state = make_test_state();

    let err = handle_profile_info(State(state), Path("strict".to_string()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("profile not found: strict"));
}

#[tokio::test]
async fn profile_ui_route_matrix_is_registered_for_all_profiles() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let (state, _dir) = make_test_state_with_tempdir();
    let code = materialized_test_profile_for("code");
    let co_work = materialized_test_profile_for("co-work");
    install_test_profile_catalog(&state, &code);
    install_test_profile_catalog(&state, &co_work);
    refresh_profile_route_caches(&state).expect("test profile cache refreshes");
    let routes = [
        (axum::http::Method::GET, "/profiles/{profile}/info"),
        (axum::http::Method::GET, "/profiles/{profile}/assets/status"),
        (axum::http::Method::GET, "/profiles/{profile}/assets/info"),
        (axum::http::Method::GET, "/profiles/{profile}/enforcement/info"),
        (axum::http::Method::GET, "/profiles/{profile}/enforcement/rules/list"),
        (axum::http::Method::GET, "/profiles/{profile}/detection/info"),
        (axum::http::Method::GET, "/profiles/{profile}/detection/rules/list"),
        (axum::http::Method::GET, "/profiles/{profile}/plugins/info"),
        (axum::http::Method::GET, "/profiles/{profile}/plugins/list"),
        (
            axum::http::Method::GET,
            "/profiles/{profile}/plugins/credential_broker/info",
        ),
        (
            axum::http::Method::GET,
            "/profiles/{profile}/plugins/credential_broker/credentials/info",
        ),
        (
            axum::http::Method::POST,
            "/profiles/{profile}/plugins/credential_broker/credentials/reload",
        ),
        (axum::http::Method::GET, "/profiles/{profile}/mcp/info"),
        (axum::http::Method::GET, "/profiles/{profile}/mcp/default/info"),
        (axum::http::Method::GET, "/profiles/{profile}/mcp/servers/list"),
        (axum::http::Method::GET, "/profiles/{profile}/skills/info"),
        (axum::http::Method::GET, "/profiles/{profile}/skills/list"),
    ];

    for profile in ["code", "co-work"] {
        for (method, route) in routes.iter() {
            let path = route.replace("{profile}", profile);
            let (status, body) =
                route_request(build_service_router(Arc::clone(&state)), method.clone(), &path, None).await;
            assert!(
                status.is_success(),
                "{path} should be registered and backed by profile data; got {status} body={body}"
            );
        }
    }
}

#[tokio::test]
async fn handle_profile_validate_accepts_builtin_primary_contract() {
    let response = handle_profile_validate(
        Path("code".to_string()),
        Json(api::ProfileValidateRequest {
            toml: None,
            profile: None,
        }),
    )
    .await
    .expect("builtin code profile should validate")
    .0;

    assert!(response.valid);
    assert_eq!(response.profile_id, "code");
}

#[tokio::test]
async fn handle_profile_validate_rejects_payload_route_mismatch() {
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.id = "strict".to_string();

    let err = handle_profile_validate(
        Path("code".to_string()),
        Json(api::ProfileValidateRequest {
            toml: None,
            profile: Some(profile),
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("profile id mismatch"));
}

#[tokio::test]
async fn profile_skills_routes_persist_profile_and_mutation_ledger() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let _home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let state = make_asset_state(dir.path().join("assets"));
    let app = build_service_router(Arc::clone(&state));

    let unknown_field = serde_json::from_value::<ProfileSkillAddRequest>(json!({
        "path": "/root/.codex/skills/security/SKILL.md",
        "credential_ref": "sk-leak"
    }));
    assert!(
        unknown_field.is_err(),
        "skill mutation payloads must reject credential/provider theater fields"
    );

    let (status, info) = route_request(app.clone(), axum::http::Method::GET, "/profiles/code/skills/info", None).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["profile_id"], "code");
    assert_eq!(info["skill_count"], 0);

    let (status, list) = route_request(app.clone(), axum::http::Method::GET, "/profiles/code/skills/list", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["profile_id"], "code");
    assert!(list["skills"].as_array().unwrap().is_empty());

    let (status, empty_path) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/skills/add",
        Some(json!({ "path": " " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{empty_path}");

    let (status, added) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/skills/add",
        Some(json!({ "path": "/root/.codex/skills/security/SKILL.md" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{added}");
    assert_eq!(added["profile_id"], "code");
    assert_eq!(added["skill_id"], "security");
    assert_eq!(added["mutation"]["category"], "skills");
    assert_eq!(added["mutation"]["filename"], "profile.toml");
    assert_eq!(added["mutation"]["operation"], "add");
    assert_eq!(added["mutation"]["status"], "applied");

    let (status, edited) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/profiles/code/skills/security/edit",
        Some(json!({ "path": "/root/.codex/skills/review/SKILL.md" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["skill_id"], "review");
    assert_eq!(edited["mutation"]["operation"], "edit");

    let (status, list) = route_request(app.clone(), axum::http::Method::GET, "/profiles/code/skills/list", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(
        list["skills"],
        json!([{ "id": "review", "path": "/root/.codex/skills/review/SKILL.md" }])
    );

    let (status, deleted) = route_request(
        app,
        axum::http::Method::DELETE,
        "/profiles/code/skills/review/delete",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted["skill_id"], "review");
    assert_eq!(deleted["mutation"]["operation"], "delete");

    let profile: ProfileConfigFile =
        toml::from_str(&std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap()).unwrap();
    assert!(profile.skills.paths.is_empty());

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
    let rows = rows["rows"].as_array().expect("mutation rows");
    assert_eq!(rows.len(), 3, "{rows:?}");
    for expected in [
        json!(["code", "skills", "profile.toml", "skill", "security", "add", "applied"]),
        json!(["code", "skills", "profile.toml", "skill", "review", "edit", "applied"]),
        json!(["code", "skills", "profile.toml", "skill", "review", "delete", "applied"]),
    ] {
        assert!(rows.contains(&expected), "missing {expected}: {rows:?}");
    }
}

#[tokio::test]
async fn profile_assets_info_reflects_manifest_and_edit_is_gated() {
    let Json(info) = handle_profile_assets_info(Path("code".to_string()))
        .await
        .expect("assets info should reflect profile manifest");
    assert_eq!(info["profile_id"], "code");
    assert_eq!(info["format"], "profile-assets.v1");
    assert_eq!(info["current_assets"]["rootfs"]["name"], "rootfs.erofs");
    assert!(
        info.get("filesystem").is_none(),
        "profile assets info must not expose build filesystem metadata"
    );
    assert!(
        info.get("compression").is_none(),
        "profile assets info must not expose build compression metadata"
    );
}

#[tokio::test]
async fn profile_assets_edit_route_is_not_mounted() {
    let state = make_test_state();
    let app = build_service_router(state);
    let (status, _) = route_request(
        app,
        axum::http::Method::PATCH,
        "/profiles/code/assets/edit",
        Some(json!({})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "profile asset edits have no typed mutation contract; do not mount a fake route"
    );
}

#[tokio::test]
async fn profile_lifecycle_write_routes_are_not_mounted() {
    let state = make_test_state();
    let app = build_service_router(state);
    for (method, uri) in [
        (axum::http::Method::POST, "/profiles/create"),
        (axum::http::Method::PATCH, "/profiles/code/edit"),
        (axum::http::Method::DELETE, "/profiles/code/delete"),
        (axum::http::Method::POST, "/profiles/code/clone"),
    ] {
        let (status, _) = route_request(app.clone(), method, uri, Some(json!({}))).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{uri} must stay unmounted until profile lifecycle writes persist through the typed profile contract"
        );
    }
}

#[tokio::test]
async fn fake_vm_mutation_routes_are_not_mounted() {
    let state = make_test_state();
    insert_fake_instance(&state, "ops-vm", std::process::id());
    let app = build_service_router(state);

    for (method, uri, body) in [
        (
            axum::http::Method::PATCH,
            "/vms/ops-vm/edit",
            Some(json!({ "ram_mb": 8192 })),
        ),
        (axum::http::Method::POST, "/vms/ops-vm/restart", None),
        (axum::http::Method::POST, "/vms/ops-vm/reload-profile", None),
    ] {
        let (status, _) = route_request(app.clone(), method, uri, body).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{uri} must stay unmounted until the VM mutation persists or performs a real operation"
        );
    }
}

#[tokio::test]
async fn profile_plugins_info_summarizes_effective_plugin_policy() {
    let state = make_test_state();

    let Json(info) = handle_profile_plugins_info(State(state), Path("code".to_string()))
        .await
        .expect("plugins info should summarize effective profile plugin policy");

    assert_eq!(info["scope"]["profile_id"], "code");
    assert!(info["plugin_count"].as_u64().unwrap() > 0);
    assert!(info["enabled_count"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn profile_mcp_info_summarizes_profile_mcp_config() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);
    // /profiles/{id}/mcp. Profile MCP routes reflect profile.toml only.
    let settings = capsem_core::net::policy_config::SettingsFile {
        mcp: Some(capsem_core::mcp::policy::McpProfileConfig {
            servers: vec![capsem_core::mcp::policy::McpManualServer {
                name: "settings-only".to_string(),
                url: "https://settings.invalid/mcp".to_string(),
                headers: Default::default(),
                auth: None,
                enabled: true,
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    capsem_core::net::policy_config::write_settings_file(&user_path, &settings).unwrap();

    let state = make_test_state();
    let Json(info) = handle_profile_mcp_info(State(state), Path("code".to_string()))
        .await
        .expect("mcp info should summarize profile mcp config");

    assert_eq!(info["profile_id"], "code");
    assert_eq!(info["server_count"], 1);
    assert_eq!(info["manual_server_count"], 0);
    assert_eq!(info["builtin_local_enabled"], true);
}

#[tokio::test]
async fn profile_mcp_tools_reject_unknown_profile_server() {
    let state = make_test_state();
    let err = handle_profile_mcp_server_tools(State(state), Path(("code".to_string(), "settings-only".to_string())))
        .await
        .expect_err("profile MCP tools must reject servers not configured in the profile");

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("MCP server not found in profile code"));
}

#[tokio::test]
async fn service_wide_ledger_routes_are_db_backed_and_empty_without_session_dbs() {
    let state = make_test_state();

    let Json(latest) = handle_service_security_latest(
        State(Arc::clone(&state)),
        Query(SecurityLedgerQuery { limit: Some(10) }),
    )
    .await
    .expect("service security latest should return an empty ledger");
    assert!(latest.is_empty());

    let Json(status) = handle_service_security_status(State(Arc::clone(&state)))
        .await
        .expect("service security status should return empty DB aggregate");
    assert_eq!(status["total"], 0);
    assert!(status["sessions"].as_array().unwrap().is_empty());

    let Json(detections) = handle_service_detection_latest(
        State(Arc::clone(&state)),
        Query(SecurityLedgerQuery { limit: Some(10) }),
    )
    .await
    .expect("service detection latest should return an empty ledger");
    assert!(detections.is_empty());

    let Json(detection_status) = handle_service_detection_status(State(state))
        .await
        .expect("service detection status should return empty DB aggregate");
    assert_eq!(detection_status["total"], 0);
}

#[tokio::test]
async fn t1_adversarial_route_inputs_fail_closed() {
    let unknown_profile = handle_profile_plugins_info(State(make_test_state()), Path("strict".to_string()))
        .await
        .unwrap_err();
    assert_eq!(unknown_profile.0, StatusCode::NOT_FOUND);

    let bad_rule = capsem_core::net::policy_config::SecurityRule {
        name: "bad_rule".to_string(),
        action: capsem_core::net::policy_config::SecurityRuleAction::Allow,
        condition: "file.read.path.contains(\"tmp\")".to_string(),
        enabled: true,
        detection_level: None,
        priority: None,
        corp_locked: false,
        reason: None,
        managed: None,
        plugin_config: BTreeMap::new(),
    };
    let malformed_rule_id = handle_enforcement_rule_upsert(
        State(make_test_state()),
        Path(("code".to_string(), "Bad Rule".to_string())),
        Json(bad_rule),
    )
    .await
    .unwrap_err();
    assert_eq!(malformed_rule_id.0, StatusCode::BAD_REQUEST);

    let invalid_enum = serde_json::from_value::<PluginUpdate>(json!({
        "mode": "teleport",
    }));
    assert!(invalid_enum.is_err());
    let invalid_detection_level = serde_json::from_value::<PluginUpdate>(json!({
        "detection_level": "panic",
    }));
    assert!(invalid_detection_level.is_err());
    let smuggled_credential_ref = serde_json::from_value::<PluginUpdate>(json!({
        "mode": "rewrite",
        "credential_ref": "sk-leak"
    }));
    assert!(
        smuggled_credential_ref.is_err(),
        "plugin edit payloads must reject credential/provider theater fields"
    );
}

#[tokio::test]
async fn mounted_read_routes_reflect_profile_settings_corp_mcp_and_assets_contracts() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);
    let settings = capsem_core::net::policy_config::SettingsFile {
        mcp: Some(capsem_core::mcp::policy::McpProfileConfig {
            servers: vec![capsem_core::mcp::policy::McpManualServer {
                name: "settings-only".to_string(),
                url: "https://settings.invalid/mcp".to_string(),
                headers: Default::default(),
                auth: None,
                enabled: true,
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    capsem_core::net::policy_config::write_settings_file(&user_path, &settings).unwrap();

    let state = make_test_state();
    let app = build_service_router(state);

    let (status, profiles) = route_request(app.clone(), axum::http::Method::GET, "/profiles/list", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(profiles["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|profile| profile["id"] == "code" && profile["name"].is_string()));

    let (status, profile) = route_request(app.clone(), axum::http::Method::GET, "/profiles/code/info", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["profile"]["id"], "code");
    assert!(profile["profile"]["description"].is_string());

    let (status, status_body) = route_request(app.clone(), axum::http::Method::GET, "/profiles/status", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(status_body["profile_count"].as_u64().unwrap() > 0);

    let (status, validation) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/validate",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(validation["valid"], true);
    assert_eq!(validation["profile_id"], "code");

    let (status, assets_info) =
        route_request(app.clone(), axum::http::Method::GET, "/profiles/code/assets/info", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assets_info["profile_id"], "code");
    assert_eq!(assets_info["format"], "profile-assets.v1");
    assert_eq!(assets_info["current_assets"]["rootfs"]["name"], "rootfs.erofs");
    assert!(
        assets_info.get("filesystem").is_none() && assets_info.get("compression").is_none(),
        "assets route must not expose build-only filesystem/compression metadata: {assets_info}"
    );

    let (status, mcp_info) = route_request(app.clone(), axum::http::Method::GET, "/profiles/code/mcp/info", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mcp_info["profile_id"], "code");
    assert_eq!(mcp_info["manual_server_count"], 0);
    assert_eq!(mcp_info["builtin_local_enabled"], true);

    let (status, settings) = route_request(app.clone(), axum::http::Method::GET, "/settings/info", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        settings.get("tree").is_some() || settings.get("issues").is_some(),
        "settings/info must expose the settings response contract: {settings}"
    );

    let (status, corp_info) = route_request(app, axum::http::Method::GET, "/corp/info", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(corp_info["installed"].is_boolean());
    assert!(corp_info["paths"].is_array());
}

#[tokio::test]
async fn profile_info_and_obom_route_expose_base_image_obom_hash() {
    let dir = tempfile::tempdir().unwrap();
    let profiles_dir = dir.path().join("profiles");
    let profile_dir = profiles_dir.join("code");
    copy_dir_all(checked_in_profile_dir("code").as_path(), &profile_dir);
    let obom_doc = json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "metadata": {
            "component": {
                "name": "capsem-code-rootfs",
                "type": "operating-system"
            }
        },
        "components": [
            {"name": "bash", "version": "5.2", "type": "library"}
        ]
    });
    let obom_bytes = serde_json::to_vec(&obom_doc).unwrap();
    let obom_hash = blake3::hash(&obom_bytes).to_hex().to_string();
    let obom_path = profile_dir.join("obom.cdx.json");
    std::fs::write(&obom_path, &obom_bytes).unwrap();

    let arch = capsem_core::net::policy_config::current_profile_arch().to_string();
    let mut profile = materialized_test_profile();
    profile.obom = Some(ProfileObomConfig {
        format: "cyclonedx-obom.v1".to_string(),
        arch: [(
            arch.clone(),
            ProfileObomDescriptor {
                name: "obom.cdx.json".to_string(),
                url: format!("file://{}", obom_path.display()),
                hash: format!("blake3:{obom_hash}"),
                size: obom_bytes.len() as u64,
                generator: "cdxgen".to_string(),
                generator_version: "11.0.0".to_string(),
            },
        )]
        .into_iter()
        .collect(),
    });
    std::fs::write(profile_dir.join("profile.toml"), toml::to_string(&profile).unwrap()).unwrap();
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", &profiles_dir);

    let state = make_test_state();
    let app = build_service_router(state);

    let (status, info) = route_request(app.clone(), axum::http::Method::GET, "/profiles/code/info", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["obom"]["hash"], format!("blake3:{obom_hash}"));
    assert_eq!(info["obom"]["scope"], "base_image");
    assert_eq!(
        info["obom"]["rootfs_hash"],
        serde_json::json!(profile.assets.current_arch_assets().unwrap().rootfs.hash)
    );
    assert_eq!(info["obom"]["route"], "/profiles/code/obom");

    let (status, obom) = route_request(app, axum::http::Method::GET, "/profiles/code/obom", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(obom["profile_id"], "code");
    assert_eq!(obom["current_arch"], arch);
    assert_eq!(obom["obom"]["hash"], format!("blake3:{obom_hash}"));
    assert_eq!(obom["obom"]["scope"], "base_image");
    assert_eq!(obom["document"], obom_doc);
}

#[tokio::test]
async fn mounted_corp_routes_validate_install_report_and_reload_inline_toml() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_settings_guard, _, _) = install_empty_settings_env(&dir);
    let _home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let app = build_service_router(make_test_state());
    let corp_toml = r#"
refresh_policy = "24h"

[corp_rule_files]
enforcement = "corp/enforcement.toml"
sigma = "corp/detection.yaml"
"#;

    let (status, invalid) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/corp/validate",
        Some(json!({ "toml": "this is [ broken" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(invalid["error"]
        .as_str()
        .unwrap_or_default()
        .contains("invalid corp TOML"));

    let (status, valid) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/corp/validate",
        Some(json!({ "toml": corp_toml })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{valid}");
    assert_eq!(valid["success"], true);

    let (status, installed) = route_request(
        app.clone(),
        axum::http::Method::PUT,
        "/corp/edit",
        Some(json!({ "toml": corp_toml })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{installed}");
    assert_eq!(installed["success"], true);
    let written = std::fs::read_to_string(dir.path().join("corp.toml")).unwrap();
    assert!(written.contains("[corp_rule_files]"));
    assert!(written.contains("enforcement = \"corp/enforcement.toml\""));

    let (status, info) = route_request(app.clone(), axum::http::Method::GET, "/corp/info", None).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["installed"], true);
    assert_eq!(info["source"]["refresh_interval_hours"], 24);
    assert!(info["source"]["content_hash"].is_string());

    let (status, reload) = route_request(app, axum::http::Method::POST, "/corp/reload", None).await;
    assert_eq!(status, StatusCode::OK, "{reload}");
    assert_eq!(reload["success"], true);
    assert_eq!(reload["reloaded"], 0);
}

#[tokio::test]
async fn mounted_plugin_routes_control_profile_evaluation() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let _home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());

    let state = make_test_state();
    let app = build_service_router(state);
    let eval_body = json!({
        "rules_toml": r#"
[profiles.rules.eicar]
name = "eicar"
action = "allow"
detection_level = "high"
match = 'file.import.content.contains("EICAR")'
"#,
        "event": {
            "event_type": "file.import",
            "file_import_content": capsem_core::security_engine::DUMMY_EICAR_TEST_STRING,
        }
    });

    let (status, list) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/plugins/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|plugin| plugin["id"] == "dummy_pre_eicar"));
    let dummy_pre = list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["id"] == "dummy_pre_eicar")
        .expect("dummy_pre_eicar listed");
    assert_eq!(dummy_pre["config"]["mode"], "disable");
    assert_eq!(dummy_pre["runtime"]["enabled"], false);

    let (status, enabled) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/profiles/code/plugins/dummy_pre_eicar/edit",
        Some(json!({ "mode": "rewrite" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(enabled["config"]["mode"], "rewrite");

    let (status, enabled_eval) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/enforcement/evaluate",
        Some(eval_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(enabled_eval["event"]["decision"]["effective"], "allow");
    assert_eq!(
        enabled_eval["event"]["file"]["import_content"],
        "[capsem-rewritten-eicar]"
    );
    assert!(enabled_eval["event"]["detections"]
        .as_array()
        .unwrap()
        .iter()
        .any(|detection| detection["plugin_id"] == "dummy_pre_eicar" && detection["plugin_mode"] == "rewrite"));

    let (status, disabled) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/profiles/code/plugins/dummy_pre_eicar/edit",
        Some(json!({ "mode": "disable" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disabled["config"]["mode"], "disable");

    let (status, after_disable) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/enforcement/evaluate",
        Some(eval_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after_disable["event"]["decision"]["effective"], "allow");

    let (status, reenabled) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/profiles/code/plugins/dummy_pre_eicar/edit",
        Some(json!({ "mode": "block", "detection_level": "critical" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reenabled["config"]["mode"], "block");
    assert_eq!(reenabled["config"]["detection_level"], "critical");

    let (status, after_enable) = route_request(
        app,
        axum::http::Method::POST,
        "/profiles/code/enforcement/evaluate",
        Some(eval_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after_enable["event"]["decision"]["effective"], "block");
    assert!(after_enable["event"]["detections"]
        .as_array()
        .unwrap()
        .iter()
        .any(|detection| detection["plugin_id"] == "dummy_pre_eicar" && detection["detection_level"] == "critical"));
}

#[tokio::test]
async fn mounted_mcp_routes_are_profile_scoped_mechanics_only() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let _builtin_guard = ensure_test_builtin_mcp_binary();

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);
    capsem_core::net::policy_config::write_settings_file(
        &user_path,
        &capsem_core::net::policy_config::SettingsFile {
            mcp: Some(capsem_core::mcp::policy::McpProfileConfig {
                servers: vec![capsem_core::mcp::policy::McpManualServer {
                    name: "settings-only".to_string(),
                    url: "https://settings.invalid/mcp".to_string(),
                    headers: Default::default(),
                    auth: None,
                    enabled: true,
                }],
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();

    let app = build_service_router(make_test_state());

    let (status, servers) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/mcp/servers/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!servers
        .as_array()
        .unwrap()
        .iter()
        .any(|server| server["name"] == "settings-only"));
    let local = servers
        .as_array()
        .unwrap()
        .iter()
        .find(|server| server["name"] == "local")
        .expect("profile route should expose Capsem-owned local builtin MCP");
    assert_eq!(local["source"], "builtin");
    assert_eq!(local["enabled"], true);
    assert_eq!(
        local["running"], false,
        "builtin MCP list entries are static profile capability, not live server lifecycle"
    );

    let (status, mcp_info) = route_request(app.clone(), axum::http::Method::GET, "/profiles/code/mcp/info", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mcp_info["builtin_local_enabled"], true);

    let (status, refresh) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/mcp/servers/local/refresh",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(refresh["success"], true);
    assert_eq!(refresh["server_id"], "local");

    let (status, body) = route_request(
        app,
        axum::http::Method::GET,
        "/profiles/code/mcp/servers/settings-only/tools/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"]
        .as_str()
        .unwrap_or_default()
        .contains("MCP server not found in profile code"));
}

#[tokio::test]
async fn handle_enforcement_rules_list_returns_compiled_profile_rules() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let (_settings_guard, _, _) = install_empty_settings_env(&dir);

    let response: api::EnforcementRuleListResponse = decode_response_json(
        handle_enforcement_rules_list(State(make_test_state()), Path("code".to_string()))
            .await
            .expect("rules list should compile effective profile"),
    )
    .await;

    assert_eq!(response.profile_id, "code");
    assert!(
        response
            .rules
            .iter()
            .any(|rule| rule.rule_id == "profiles.rules.default_http"
                && rule.source == api::EnforcementRuleSource::BuiltinDefault
                && rule.default_rule),
        "list must expose built-in default rules as first-class rows"
    );
    let custom = response
        .rules
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.skill_loaded")
        .expect("custom profile rule should be listed");
    assert_eq!(custom.source, api::EnforcementRuleSource::Profile);
    assert!(!custom.default_rule);
    assert!(custom.enabled);
    assert_eq!(custom.priority, 10);
    assert_eq!(
        custom.detection_level,
        Some(capsem_core::net::policy_config::DetectionLevel::Informational)
    );
}

#[tokio::test]
async fn disabled_rules_are_listed_but_do_not_evaluate() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    add_profile_enforcement_rule(
        &config_root,
        "disabled_tmp_block",
        capsem_core::net::policy_config::SecurityRule {
            name: "disabled_tmp_block".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Block,
            condition: r#"file.read.path.contains("tmp")"#.to_string(),
            enabled: false,
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::High),
            priority: None,
            corp_locked: false,
            reason: Some("disabled rule inventory proof".to_string()),
            managed: None,
            plugin_config: BTreeMap::new(),
        },
    );
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let (_settings_guard, _, _) = install_empty_settings_env(&dir);

    let response: api::EnforcementRuleListResponse = decode_response_json(
        handle_enforcement_rules_list(State(make_test_state()), Path("code".to_string()))
            .await
            .expect("rules list should include disabled rules"),
    )
    .await;
    let disabled = response
        .rules
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.disabled_tmp_block")
        .expect("disabled rule should stay visible in inventory");
    assert!(!disabled.enabled);
    assert_eq!(
        disabled.detection_level,
        Some(capsem_core::net::policy_config::DetectionLevel::High)
    );

    let profile_rules = profile_security_rule_profile_for_route("code").unwrap();
    let rule_set = capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile_rules,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .expect("compile profile rules");
    let event = capsem_core::security_engine::SecurityEvent::new(
        capsem_core::security_engine::RuntimeSecurityEventType::FileEvent,
    )
    .with_file(capsem_core::security_engine::FileSecurityEvent {
        read_path: Some("/tmp/secret.txt".to_string()),
        ..Default::default()
    });
    let evaluation = rule_set.evaluate(&event).expect("evaluate rules");
    assert!(
        evaluation
            .matched_rules()
            .iter()
            .all(|rule| rule.rule_id != "profiles.rules.disabled_tmp_block"),
        "disabled rule must not participate in enforcement or detection"
    );

    let detection_response: api::DetectionRuleListResponse = decode_response_json(
        handle_detection_rules_list(State(make_test_state()), Path("code".to_string()))
            .await
            .expect("detection rules list should include disabled detection rules"),
    )
    .await;
    assert!(detection_response
        .rules
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.disabled_tmp_block" && !rule.enabled));
}

#[tokio::test]
async fn handle_enforcement_rules_list_rejects_unknown_profiles() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let err = handle_enforcement_rules_list(State(make_test_state()), Path("strict".to_string()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("profile not found: strict"));
}

#[tokio::test]
async fn handle_enforcement_info_summarizes_compiled_rules() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let (_settings_guard, _, _) = install_empty_settings_env(&dir);

    let Json(info) = handle_enforcement_info(State(make_test_state()), Path("code".to_string()))
        .await
        .expect("info should summarize effective rules");

    assert_eq!(info.profile_id, "code");
    assert!(info.rule_count > 0);
    assert!(info.default_rule_count > 0);
    assert!(info.custom_rule_count >= 1);
    assert!(info.detection_rule_count >= 1);
    assert!(info.source_counts["profile"] >= 1);
    assert!(info.source_counts["builtin_default"] > 0);
    assert!(info.action_counts.contains_key("allow"));
}

#[tokio::test]
async fn handle_enforcement_info_rejects_unknown_profiles() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let err = handle_enforcement_info(State(make_test_state()), Path("strict".to_string()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("profile not found: strict"));
}

#[tokio::test]
async fn handle_detection_rules_list_returns_detection_rules_only() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    add_profile_enforcement_rule(
        &config_root,
        "pure_block",
        capsem_core::net::policy_config::SecurityRule {
            name: "pure_block".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Block,
            condition: r#"file.read.path.contains("tmp")"#.to_string(),
            enabled: true,
            detection_level: None,
            priority: None,
            corp_locked: false,
            reason: Some("block example without reporting".to_string()),
            managed: None,
            plugin_config: BTreeMap::new(),
        },
    );
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let (_settings_guard, _, _) = install_empty_settings_env(&dir);

    let response: api::DetectionRuleListResponse = decode_response_json(
        handle_detection_rules_list(State(make_test_state()), Path("code".to_string()))
            .await
            .expect("detection rules list should compile effective profile"),
    )
    .await;

    assert_eq!(response.profile_id, "code");
    assert!(
        response.rules.iter().all(|rule| rule.detection_level.is_some()),
        "detection inventory must not include non-reporting enforcement rules"
    );
    assert!(response
        .rules
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.skill_loaded"));
    assert!(!response
        .rules
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.pure_block"));
}

#[tokio::test]
async fn handle_detection_info_summarizes_detection_rules_only() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let (_settings_guard, _, _) = install_empty_settings_env(&dir);

    let Json(info) = handle_detection_info(State(make_test_state()), Path("code".to_string()))
        .await
        .expect("detection info should summarize effective detection rules");

    assert_eq!(info.profile_id, "code");
    assert!(info.rule_count >= 1);
    assert_eq!(info.rule_count, info.detection_rule_count);
    assert!(info.source_counts.contains_key("profile"));
}

#[tokio::test]
async fn handle_detection_rule_upsert_requires_detection_level() {
    let rule = capsem_core::net::policy_config::SecurityRule {
        name: "pure_block".to_string(),
        action: capsem_core::net::policy_config::SecurityRuleAction::Block,
        condition: r#"file.read.path.contains("tmp")"#.to_string(),
        enabled: true,
        detection_level: None,
        priority: None,
        corp_locked: false,
        reason: Some("block without reporting".to_string()),
        managed: None,
        plugin_config: BTreeMap::new(),
    };

    let err = handle_detection_rule_upsert(
        State(make_test_state()),
        Path(("code".to_string(), "pure_block".to_string())),
        Json(rule),
    )
    .await
    .unwrap_err();

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("requires detection_level"));
}

#[tokio::test]
async fn handle_detection_rules_list_rejects_unknown_profiles() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let err = handle_detection_rules_list(State(make_test_state()), Path("strict".to_string()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("profile not found: strict"));
}

#[tokio::test]
async fn profile_plugin_endpoint_matrix_dynamically_controls_enforcement_evaluation() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let _home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let state = make_test_state();

    let list = list_plugins_for_scope(
        &state,
        profile_plugin_scope(&state, "code".to_string()).expect("profile scope"),
    )
    .await
    .expect("list plugins");
    assert_eq!(list.scope.profile_id, "code");
    assert!(
        list.plugins.iter().any(|plugin| plugin.id == "dummy_pre_eicar"),
        "built-in plugin list must include dummy_pre_eicar"
    );
    assert!(
        list.plugins.iter().any(|plugin| plugin.id == "log_sanitizer"),
        "built-in plugin list must include the logging-stage sanitizer"
    );
    assert!(
        list.plugins
            .iter()
            .any(|plugin| plugin.stage == PluginStage::Preprocess),
        "plugin catalog must expose preprocess plugins"
    );
    assert!(
        list.plugins
            .iter()
            .any(|plugin| plugin.stage == PluginStage::Postprocess),
        "plugin catalog must expose postprocess plugins"
    );
    assert!(
        list.plugins.iter().any(|plugin| plugin.stage == PluginStage::Logging),
        "plugin catalog must expose logging plugins"
    );
    let dummy_pre = list
        .plugins
        .iter()
        .find(|plugin| plugin.id == "dummy_pre_eicar")
        .expect("dummy_pre_eicar exists");
    assert_eq!(
        dummy_pre.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Disable,
        "debug plugins must be opt-in test fixtures, not active product defaults"
    );
    assert_eq!(dummy_pre.default_config.mode, dummy_pre.config.mode);
    assert!(!dummy_pre.runtime.enabled);
    let dummy_post = list
        .plugins
        .iter()
        .find(|plugin| plugin.id == "dummy_post_allow")
        .expect("dummy_post_allow exists");
    assert_eq!(
        dummy_post.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Disable,
        "postprocess debug plugin must also be opt-in"
    );
    assert!(!dummy_post.runtime.enabled);
    let broker = list
        .plugins
        .iter()
        .find(|plugin| plugin.id == "credential_broker")
        .expect("built-in plugin list must include credential_broker");
    assert_eq!(broker.stage, PluginStage::Preprocess);
    assert_eq!(broker.version, "1");
    assert_eq!(broker.capabilities.event_families, vec!["http", "file", "mcp"]);
    assert_eq!(
        broker.capabilities.credential_providers,
        vec!["anthropic", "google", "openai", "github", "mcp"]
    );
    assert_eq!(
        broker.capabilities.credential_sources,
        vec![
            "http.authorization",
            "http.body.oauth_token",
            "file.env",
            "mcp.auth_reference"
        ]
    );
    assert_eq!(broker.detail_routes.len(), 2);
    assert_eq!(broker.detail_routes[0].id, "credential_broker_credentials");
    assert_eq!(broker.detail_routes[0].kind, PluginDetailRouteKind::CredentialBroker);
    assert_eq!(
        broker.detail_routes[0].path,
        "/profiles/code/plugins/credential_broker/credentials/info"
    );
    assert_eq!(broker.detail_routes[1].id, "credential_broker_credentials_reload");
    assert_eq!(
        broker.detail_routes[1].path,
        "/profiles/code/plugins/credential_broker/credentials/reload"
    );
    assert!(broker.runtime.enabled);
    assert_eq!(broker.runtime.event_count, 0);
    assert!(
        broker.runtime.brokered_credentials.is_empty(),
        "credential broker refs must be reported from plugin runtime state, not settings/providers"
    );
    let sanitizer = list
        .plugins
        .iter()
        .find(|plugin| plugin.id == "log_sanitizer")
        .expect("log_sanitizer exists");
    assert_eq!(sanitizer.stage, PluginStage::Logging);
    assert_eq!(
        sanitizer.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Rewrite
    );
    assert!(sanitizer.runtime.enabled);
    assert_eq!(
        sanitizer.capabilities.credential_sources,
        vec!["security_event.credential_observations"]
    );
    assert!(
        sanitizer.detail_routes.is_empty(),
        "logging plugins expose the same generic plugin contract unless they own a custom route"
    );

    let Json(info) = handle_profile_plugin_info(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "dummy_pre_eicar".to_string())),
    )
    .await
    .expect("plugin info");
    assert_eq!(info.id, "dummy_pre_eicar");
    assert_eq!(info.scope.profile_id, "code");
    assert_eq!(info.stage, PluginStage::Preprocess);
    assert_eq!(info.version, "1");
    assert!(info.capabilities.credential_providers.is_empty());
    assert!(
        info.detail_routes.is_empty(),
        "debug plugins do not get custom UI routes"
    );
    assert!(!info.runtime.enabled);
    assert!(info.runtime.brokered_credentials.is_empty());
    assert_eq!(
        info.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Disable
    );
    assert_eq!(
        info.config.detection_level,
        capsem_core::net::policy_config::DetectionLevel::Informational
    );

    let request = EnforcementEvaluateRequest::eicar_fixture();
    let default_disabled_response = handle_enforcement_evaluate(
        State(Arc::clone(&state)),
        Path("code".to_string()),
        enforcement_evaluate_body(&request),
    )
    .await
    .expect("default-disabled plugin evaluates");
    let default_disabled: serde_json::Value = decode_response_json(default_disabled_response).await;
    let default_disabled_event = &default_disabled["event"];
    assert_eq!(default_disabled_event["decision"]["effective"], "allow");
    let default_disabled_detections = default_disabled_event["detections"].as_array().unwrap();
    assert!(default_disabled_detections
        .iter()
        .any(|detection| { detection["source"] == "rule" && detection["rule_id"] == "profiles.rules.eicar" }));
    assert!(!default_disabled_detections
        .iter()
        .any(|detection| { detection["source"] == "plugin" && detection["plugin_id"] == "dummy_pre_eicar" }));
    assert!(!default_disabled_detections
        .iter()
        .any(|detection| { detection["source"] == "plugin" && detection["plugin_id"] == "dummy_post_allow" }));
    assert!(
        default_disabled_event.get("http").is_some(),
        "wire DTO must expose every first-party root, even when null"
    );

    let Json(enabled_pre) = handle_profile_plugin_update(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "dummy_pre_eicar".to_string())),
        Json(PluginUpdate {
            mode: Some(capsem_core::net::policy_config::SecurityPluginMode::Rewrite),
            detection_level: None,
        }),
    )
    .await
    .expect("enable pre plugin");
    assert_eq!(
        enabled_pre.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Rewrite
    );
    assert!(enabled_pre.runtime.enabled);
    let Json(enabled_post) = handle_profile_plugin_update(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "dummy_post_allow".to_string())),
        Json(PluginUpdate {
            mode: Some(capsem_core::net::policy_config::SecurityPluginMode::Allow),
            detection_level: None,
        }),
    )
    .await
    .expect("enable post plugin");
    assert_eq!(
        enabled_post.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Allow
    );
    assert!(enabled_post.runtime.enabled);

    let enabled_response = handle_enforcement_evaluate(
        State(Arc::clone(&state)),
        Path("code".to_string()),
        enforcement_evaluate_body(&request),
    )
    .await
    .expect("explicitly enabled plugin evaluates");
    let enabled: serde_json::Value = decode_response_json(enabled_response).await;
    let enabled_event = &enabled["event"];
    assert_eq!(enabled_event["decision"]["effective"], "allow");
    assert_eq!(enabled_event["file"]["import_content"], "[capsem-rewritten-eicar]");
    let enabled_detections = enabled_event["detections"].as_array().unwrap();
    assert!(enabled_detections.iter().any(|detection| {
        detection["source"] == "plugin"
            && detection["plugin_id"] == "dummy_pre_eicar"
            && detection["plugin_mode"] == "rewrite"
    }));
    assert!(enabled_detections
        .iter()
        .any(|detection| { detection["source"] == "plugin" && detection["plugin_id"] == "dummy_post_allow" }));

    let Json(disabled) = handle_profile_plugin_update(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "dummy_pre_eicar".to_string())),
        Json(PluginUpdate {
            mode: Some(capsem_core::net::policy_config::SecurityPluginMode::Disable),
            detection_level: None,
        }),
    )
    .await
    .expect("disable plugin");
    assert_eq!(
        disabled.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Disable
    );

    let after_disable_response = handle_enforcement_evaluate(
        State(Arc::clone(&state)),
        Path("code".to_string()),
        enforcement_evaluate_body(&request),
    )
    .await
    .expect("disabled plugin evaluates");
    let after_disable: serde_json::Value = decode_response_json(after_disable_response).await;
    let after_disable_event = &after_disable["event"];
    assert_eq!(after_disable_event["decision"]["effective"], "allow");
    let after_disable_detections = after_disable_event["detections"].as_array().unwrap();
    assert!(after_disable_detections
        .iter()
        .any(|detection| { detection["source"] == "rule" && detection["rule_id"] == "profiles.rules.eicar" }));
    assert!(!after_disable_detections
        .iter()
        .any(|detection| { detection["source"] == "plugin" && detection["plugin_id"] == "dummy_pre_eicar" }));

    let unknown_plugin_info = handle_profile_plugin_info(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "credential_ref".to_string())),
    )
    .await
    .unwrap_err();
    assert_eq!(unknown_plugin_info.0, StatusCode::NOT_FOUND);
    assert!(unknown_plugin_info.1.contains("unknown plugin"));

    let unknown_plugin_update = handle_profile_plugin_update(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "credential_ref".to_string())),
        Json(PluginUpdate {
            mode: Some(capsem_core::net::policy_config::SecurityPluginMode::Rewrite),
            detection_level: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(unknown_plugin_update.0, StatusCode::NOT_FOUND);
    assert!(unknown_plugin_update.1.contains("unknown plugin"));

    let unknown_profile = handle_profile_plugin_update(
        State(Arc::clone(&state)),
        Path(("strict".to_string(), "dummy_pre_eicar".to_string())),
        Json(PluginUpdate {
            mode: Some(capsem_core::net::policy_config::SecurityPluginMode::Block),
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Medium),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(unknown_profile.0, StatusCode::NOT_FOUND);
    assert!(unknown_profile.1.contains("profile not found: strict"));

    let Json(reenabled) = handle_profile_plugin_update(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "dummy_pre_eicar".to_string())),
        Json(PluginUpdate {
            mode: Some(capsem_core::net::policy_config::SecurityPluginMode::Block),
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Critical),
        }),
    )
    .await
    .expect("reenable plugin");
    assert_eq!(
        reenabled.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Block
    );
    assert_eq!(
        reenabled.config.detection_level,
        capsem_core::net::policy_config::DetectionLevel::Critical
    );

    let after_enable_response = handle_enforcement_evaluate(
        State(state),
        Path("code".to_string()),
        enforcement_evaluate_body(&request),
    )
    .await
    .expect("reenabled plugin evaluates");
    let after_enable: serde_json::Value = decode_response_json(after_enable_response).await;
    let after_enable_event = &after_enable["event"];
    assert_eq!(after_enable_event["decision"]["effective"], "block");
    let detections = after_enable_event["detections"].as_array().unwrap();
    assert!(detections.iter().any(|detection| {
        detection["source"] == "plugin"
            && detection["plugin_id"] == "dummy_pre_eicar"
            && detection["detection_level"] == "critical"
            && detection["plugin_mode"] == "block"
    }));
}

#[tokio::test]
async fn credential_broker_detail_route_exposes_inventory_and_grant_surface() {
    let state = make_test_state();

    let Json(detail) =
        handle_profile_credential_broker_credentials_info(State(Arc::clone(&state)), Path("code".to_string()))
            .await
            .expect("credential broker detail");

    assert_eq!(detail.scope.profile_id, "code");
    assert_eq!(detail.plugin_id, "credential_broker");
    assert!(detail.store.ready);
    assert_eq!(detail.store.status, "ready");
    assert_eq!(
        detail.store.backend,
        capsem_core::credential_broker::credential_store_status().backend
    );
    assert!(detail.inventory.is_empty());
    assert!(detail.grants.profile_enabled);
    assert_eq!(
        detail.grants.fork_default,
        CredentialBrokerForkGrantDefault::InheritProfile
    );
    assert!(
        detail.grants.vm_grants.is_empty(),
        "VM-specific credential grants are explicit overrides, not hidden defaults"
    );
    assert!(
        detail.corp_constraints.is_empty(),
        "test profile has no corp broker OAuth/provider constraints"
    );
}

#[tokio::test]
async fn service_status_reports_ready_empty_credential_store_without_inventory_counters() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let _store_guard = EnvVarGuard::set("CAPSEM_CREDENTIAL_STORE_PATH", dir.path().join("credential-store.json"));
    capsem_core::credential_broker::hydrate_credential_runtime_cache_from_durable_store().unwrap();

    let state = make_test_state();
    let app = build_service_router(state);
    let (status, body) = route_request(app, axum::http::Method::GET, "/status", None).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ready"], true);
    assert_eq!(body["components"]["credential_store"]["ready"], true);
    assert_eq!(body["components"]["credential_store"]["status"], "ready");
    assert_eq!(
        body["components"]["credential_store"]["last_error"],
        serde_json::Value::Null
    );
    assert!(
        body["components"]["credential_store"]["cached_count"].is_null(),
        "credential inventory counters belong to the credential broker object, not /status"
    );
}

#[tokio::test]
async fn credential_broker_reload_route_rehydrates_store_and_returns_same_contract() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let test_store = dir.path().join("credential-store.json");
    let _store_guard = EnvVarGuard::set("CAPSEM_CREDENTIAL_STORE_PATH", test_store.clone());
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = dir.path().join("sessions").join("broker-reload-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "broker-reload-vm", std::process::id(), session_dir.clone());

    let credential_ref = capsem_logger::credential_reference("google", "ya29.reload-route");
    let store_json = serde_json::json!({
        capsem_core::credential_broker::credential_store_account(
            capsem_core::credential_broker::CredentialProvider::Google,
            &credential_ref,
        ): "ya29.reload-route"
    });
    std::fs::write(&test_store, serde_json::to_string_pretty(&store_json).unwrap()).unwrap();

    let event_json = format!(
        r#"{{
            "event_type": "http.request",
            "credential_observations": [
                {{
                    "provider": "google",
                    "source": "http.body.response.$.access_token",
                    "event_type": "http.request",
                    "trace_id": null,
                    "context_json": {{"domain":"oauth2.googleapis.com"}},
                    "credential_ref": "{credential_ref}"
                }}
            ],
            "credential_injections": []
        }}"#
    );
    let session_db = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&session_db, 16).unwrap();
    let stale_reader = state
        .register_session_db_handle("broker-reload-vm", &session_dir)
        .expect("pre-register session DB reader");
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "abcd1234ef56",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                event_json,
            ),
        ))
        .await;
    writer.shutdown_blocking();
    let direct_rows = capsem_logger::DbReader::open(&session_db)
        .unwrap()
        .recent_security_rule_events(10)
        .unwrap();
    assert_eq!(direct_rows.len(), 1);
    assert!(direct_rows[0].event_json.contains(&credential_ref));
    let (status, before) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/plugins/credential_broker/credentials/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(before["plugin_id"], "credential_broker");
    assert_eq!(before["store"]["backend"], "disk_override");
    assert_eq!(before["inventory"][0]["credential_ref"], credential_ref);
    assert_eq!(before["inventory"][0]["replay_available"], false);

    let (status, after) = route_request(
        app,
        axum::http::Method::POST,
        "/profiles/code/plugins/credential_broker/credentials/reload",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["plugin_id"], "credential_broker");
    assert_eq!(after["store"]["ready"], true);
    assert_eq!(after["store"]["status"], "ready");
    assert_eq!(after["store"]["backend"], "disk_override");
    assert_eq!(after["store"]["last_hydrated_count"], 1);
    assert!(after["store"]["last_hydrated_unix_ms"].as_u64().is_some());
    assert_eq!(after["inventory"][0]["credential_ref"], credential_ref);
    assert_eq!(after["inventory"][0]["replay_available"], true);
    let refreshed_reader = state
        .session_db_handle("broker-reload-vm")
        .expect("reload response re-registers session DB reader");
    assert!(
        !Arc::ptr_eq(&stale_reader, &refreshed_reader),
        "credential broker reload must rebuild profile session DB readers before reporting inventory"
    );
}

#[tokio::test]
async fn credential_broker_plugin_runtime_reports_security_ledger_activity() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("broker-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "broker-vm", std::process::id(), session_dir.clone());

    let event_json = r#"{
        "event_type": "http.request",
        "credential_observations": [
            {
                "provider": "google",
                "source": "http.body.response.$.access_token",
                "event_type": "http.request",
                "trace_id": null,
                "context_json": {"domain":"oauth2.googleapis.com"},
                "credential_ref": "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
            }
        ],
        "credential_injections": [
            {
                "provider": "google",
                "source": "http.request.header.authorization",
                "event_type": "http.request",
                "trace_id": null,
                "context_json": {"domain":"generativelanguage.googleapis.com"},
                "credential_ref": "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
            }
        ]
    }"#;
    let session_db = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&session_db, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "abc123def456",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                event_json,
            ),
        ))
        .await;
    writer.shutdown_blocking();
    let direct_rows = capsem_logger::DbReader::open(&session_db)
        .unwrap()
        .recent_security_rule_events(10)
        .unwrap();
    assert_eq!(direct_rows.len(), 1);
    assert!(direct_rows[0].event_json.contains("credential_observations"));
    let (status, list) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/plugins/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let broker = list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["id"] == "credential_broker")
        .expect("credential broker plugin is listed");
    assert_eq!(
        broker["runtime"]["event_count"], 0,
        "plugin list is a hot config route and must not hydrate runtime ledgers"
    );

    let (status, broker) = route_request(
        app,
        axum::http::Method::GET,
        "/profiles/code/plugins/credential_broker/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{broker}");
    assert_eq!(broker["runtime"]["event_count"], 2);
    assert_eq!(broker["runtime"]["rewrite_count"], 1);
    assert_eq!(
        broker["runtime"]["brokered_credentials"][0]["credential_ref"],
        "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
    );
    assert_eq!(broker["runtime"]["brokered_credentials"][0]["provider"], "google");
    assert_eq!(broker["runtime"]["brokered_credentials"][0]["observed_count"], 1);
    assert_eq!(broker["runtime"]["brokered_credentials"][0]["injected_count"], 1);
    assert_eq!(
        broker["runtime"]["brokered_credentials"][0]["replay_available"], false,
        "security event evidence alone must not imply the broker can replay the credential"
    );
}

#[tokio::test]
async fn plugin_runtime_reports_execution_latency_from_security_ledger_payloads() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let profile_dir = tempfile::tempdir().unwrap();
    let (config_root, profile) = install_file_asset_profile_fixture(&profile_dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let state = make_asset_state(profile_dir.path().join("assets"));
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("plugin-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir_and_pins(
        &state,
        "plugin-vm",
        std::process::id(),
        session_dir.clone(),
        profile.revision.clone(),
        profile_payload_hash(&profile).unwrap(),
        profile_asset_pins(&profile).unwrap(),
    );

    let event_json = r#"{
        "event_type": "http.request",
        "plugin_executions": [
            {
                "plugin_id": "credential_broker",
                "stage": "preprocess",
                "applied": false,
                "duration_us": 13
            },
            {
                "plugin_id": "log_sanitizer",
                "stage": "logging",
                "applied": true,
                "duration_us": 77
            },
            {
                "plugin_id": "dummy_post_allow",
                "stage": "postprocess",
                "applied": true,
                "duration_us": 31
            }
        ],
        "detections": [
            {
                "source": "plugin",
                "detection_level": "informational",
                "rule_id": null,
                "plugin_id": "log_sanitizer",
                "action": null,
                "plugin_mode": "rewrite",
                "reason": null
            },
            {
                "source": "plugin",
                "detection_level": "low",
                "rule_id": null,
                "plugin_id": "dummy_post_allow",
                "action": null,
                "plugin_mode": "allow",
                "reason": null
            }
        ]
    }"#;
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    for rule_id in ["profiles.rules.default_http", "profiles.rules.ai_google"] {
        writer
            .write(capsem_logger::WriteOp::SecurityRuleEvent(
                capsem_logger::SecurityRuleEvent::new(
                    1_789_000_123_456,
                    "abc123def456",
                    "http.request",
                    rule_id,
                    r#"{"name":"default_http"}"#,
                    event_json,
                ),
            ))
            .await;
    }
    writer.shutdown_blocking();
    let (status, list) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/plugins/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");

    let sanitizer = list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["id"] == "log_sanitizer")
        .expect("log sanitizer plugin is listed");
    assert_eq!(
        sanitizer["runtime"]["execution_count"], 0,
        "plugin list is a hot config route and must not hydrate runtime DB scans"
    );

    let (status, sanitizer_detail) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/plugins/log_sanitizer/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sanitizer_detail}");
    assert_eq!(
        sanitizer_detail["runtime"]["execution_count"], 1,
        "multiple rule rows for one security event must not double-count one plugin execution"
    );
    assert_eq!(sanitizer_detail["runtime"]["applied_count"], 1);
    assert_eq!(sanitizer_detail["runtime"]["skipped_count"], 0);
    assert_eq!(sanitizer_detail["runtime"]["detection_count"], 1);
    assert_eq!(sanitizer_detail["runtime"]["total_duration_us"], 77);
    assert_eq!(sanitizer_detail["runtime"]["max_duration_us"], 77);

    let (status, dummy_post) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/plugins/dummy_post_allow/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dummy_post}");
    assert_eq!(
        dummy_post["runtime"]["execution_count"], 1,
        "postprocess plugin executions must hydrate from the same security ledger payloads"
    );
    assert_eq!(dummy_post["runtime"]["applied_count"], 1);
    assert_eq!(dummy_post["runtime"]["skipped_count"], 0);
    assert_eq!(dummy_post["runtime"]["detection_count"], 1);
    assert_eq!(dummy_post["runtime"]["total_duration_us"], 31);
    assert_eq!(dummy_post["runtime"]["max_duration_us"], 31);

    let (status, broker) = route_request(
        app,
        axum::http::Method::GET,
        "/profiles/code/plugins/credential_broker/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{broker}");
    assert_eq!(broker["runtime"]["execution_count"], 1);
    assert_eq!(broker["runtime"]["applied_count"], 0);
    assert_eq!(broker["runtime"]["skipped_count"], 1);
    assert_eq!(broker["runtime"]["total_duration_us"], 13);
}

#[tokio::test]
async fn enforcement_rule_endpoints_add_delete_reload_and_reject_invalid_rules_atomically() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let state = make_asset_state(dir.path().join("assets"));
    let rule = capsem_core::net::policy_config::SecurityRule {
        name: "file_import_eicar_block".to_string(),
        action: capsem_core::net::policy_config::SecurityRuleAction::Block,
        condition: r#"file.import.content.contains("EICAR")"#.to_string(),
        enabled: true,
        detection_level: Some(capsem_core::net::policy_config::DetectionLevel::High),
        priority: Some(capsem_core::net::policy_config::SecurityRulePriority::Explicit(10)),
        corp_locked: false,
        reason: Some("debug EICAR fixture must block".to_string()),
        managed: None,
        plugin_config: BTreeMap::new(),
    };

    let Json(saved) = handle_enforcement_rule_upsert(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "eicar_block".to_string())),
        Json(rule.clone()),
    )
    .await
    .expect("valid profile enforcement rule should save");
    assert_eq!(saved.rule_id, "eicar_block");
    assert_eq!(saved.compiled_rule_id, "profiles.rules.eicar_block");
    let list_after_save: api::EnforcementRuleListResponse = decode_response_json(
        handle_enforcement_rules_list(State(Arc::clone(&state)), Path("code".to_string()))
            .await
            .expect("rules list cache should refresh after upsert"),
    )
    .await;
    assert!(
        list_after_save
            .rules
            .iter()
            .any(|rule| rule.rule_id == "profiles.rules.eicar_block"
                && rule.action == capsem_core::net::policy_config::SecurityRuleAction::Block),
        "upserted rule must be visible through cached rules/list route"
    );

    let enforcement_path = config_root.join("profiles/code/enforcement.toml");
    let loaded = SecurityRuleProfile::parse_toml(&std::fs::read_to_string(&enforcement_path).unwrap()).unwrap();
    assert_eq!(
        loaded.profiles.rules["eicar_block"].action,
        capsem_core::net::policy_config::SecurityRuleAction::Block
    );
    let profile_after_save: ProfileConfigFile =
        toml::from_str(&std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap()).unwrap();
    assert_eq!(
        profile_after_save.files.enforcement.unwrap().hash,
        Some(format!(
            "blake3:{}",
            capsem_assets::asset_manager::hash_file(&enforcement_path).unwrap()
        ))
    );

    let Json(reload) = handle_enforcement_reload(State(Arc::clone(&state)), Path("code".to_string()))
        .await
        .expect("reload alias should broadcast to zero instances");
    assert_eq!(reload["success"], serde_json::json!(true));
    assert_eq!(reload["reloaded"], serde_json::json!(0));

    let mut bad_priority = rule.clone();
    bad_priority.priority = Some(capsem_core::net::policy_config::SecurityRulePriority::Explicit(-100));
    let err = handle_enforcement_rule_upsert(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "bad_negative_priority".to_string())),
        Json(bad_priority),
    )
    .await
    .expect_err("user rule endpoint must reject negative user priority");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1.contains("cannot use negative priority"),
        "error should explain priority failure, got: {}",
        err.1
    );

    let mut corp_locked = rule.clone();
    corp_locked.corp_locked = true;
    let err = handle_enforcement_rule_upsert(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "corp_locked".to_string())),
        Json(corp_locked),
    )
    .await
    .expect_err("user rule endpoint must not create corp-locked rules");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);

    let loaded = SecurityRuleProfile::parse_toml(&std::fs::read_to_string(&enforcement_path).unwrap()).unwrap();
    assert!(
        !loaded.profiles.rules.contains_key("bad_negative_priority"),
        "rejected rule must not be persisted"
    );
    assert!(
        !loaded.profiles.rules.contains_key("corp_locked"),
        "rejected corp-locked rule must not be persisted"
    );
    assert!(
        loaded.profiles.rules.contains_key("eicar_block"),
        "valid existing rule must remain after rejected writes"
    );

    let Json(deleted) = handle_enforcement_rule_delete(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "eicar_block".to_string())),
    )
    .await
    .expect("delete should remove existing rule");
    assert!(deleted.deleted);
    assert_eq!(deleted.rule_id, "eicar_block");
    let list_after_delete: api::EnforcementRuleListResponse = decode_response_json(
        handle_enforcement_rules_list(State(Arc::clone(&state)), Path("code".to_string()))
            .await
            .expect("rules list cache should refresh after delete"),
    )
    .await;
    assert!(
        list_after_delete
            .rules
            .iter()
            .all(|rule| rule.rule_id != "profiles.rules.eicar_block"),
        "deleted rule must disappear from cached rules/list route"
    );
    let loaded = SecurityRuleProfile::parse_toml(&std::fs::read_to_string(&enforcement_path).unwrap()).unwrap();
    assert!(!loaded.profiles.rules.contains_key("eicar_block"));

    let err = handle_enforcement_rule_delete(State(state), Path(("code".to_string(), "eicar_block".to_string())))
        .await
        .expect_err("deleting a missing rule should return not found");
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn route_authored_detection_rule_triggers_runtime_ledger_and_latest_routes() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let state = make_asset_state(dir.path().join("assets"));
    let app = build_service_router(Arc::clone(&state));
    let session_dir = dir.path().join("sessions").join("route-ledger-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "route-ledger-vm", std::process::id(), session_dir.clone());

    let rule = capsem_core::net::policy_config::SecurityRule {
        name: "openai_http_observed".to_string(),
        action: capsem_core::net::policy_config::SecurityRuleAction::Allow,
        condition: r#"http.host.contains("openai.com")"#.to_string(),
        enabled: true,
        detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Informational),
        priority: Some(capsem_core::net::policy_config::SecurityRulePriority::Explicit(10)),
        corp_locked: false,
        reason: Some("route-authored detection proof".to_string()),
        managed: None,
        plugin_config: BTreeMap::new(),
    };

    let save_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::PUT)
                .uri("/profiles/code/detection/rules/openai_http_observed/edit")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&rule).unwrap()))
                .unwrap(),
        )
        .await
        .expect("detection route should respond");
    assert_eq!(save_response.status(), StatusCode::OK);
    let save_body = to_bytes(save_response.into_body(), usize::MAX).await.unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&save_body).unwrap();
    assert_eq!(saved["compiled_rule_id"], "profiles.rules.openai_http_observed");

    let profile = capsem_core::net::policy_config::Profile::load_from_dir(config_root.join("profiles/code")).unwrap();
    let compiled = profile
        .config()
        .security_rule_profile_from_files(profile.config_root())
        .unwrap()
        .compile(SecurityRuleSource::User)
        .expect("route-authored rules compile for runtime");
    let rule_set = SecurityRuleSet::new(compiled);
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    let event_id =
        capsem_core::security_engine::SecurityEventId::parse("abcdef123456").expect("fixed event id is 12 hex");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_route_authored_detection")
        .with_http(capsem_core::security_engine::HttpSecurityEvent {
            host: Some("api.openai.com".to_string()),
            method: Some("POST".to_string()),
            path: Some("/v1/responses".to_string()),
            query: None,
            status: Some("200".to_string()),
            body: None,
        });

    let emitted = capsem_core::security_engine::emit_matching_security_rules(
        &writer,
        event_id,
        RuntimeSecurityEventType::HttpRequest,
        &rule_set,
        &event,
        1_789_000_123_456,
    )
    .await
    .expect("matching rule emits ledger rows");
    writer.shutdown_blocking();
    assert!(
        emitted >= 1,
        "route-authored detection and profile default rules may both emit"
    );
    let latest_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/route-ledger-vm/security/latest?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("security latest route should respond");
    assert_eq!(latest_response.status(), StatusCode::OK);
    let latest_body = to_bytes(latest_response.into_body(), usize::MAX).await.unwrap();
    let events: Vec<capsem_logger::SecurityRuleEvent> = serde_json::from_slice(&latest_body).unwrap();
    let event = events
        .iter()
        .find(|event| event.rule_id == "profiles.rules.openai_http_observed")
        .expect("route-authored detection row should be in security latest");
    assert_eq!(event.event_id, "abcdef123456");
    assert_eq!(event.event_type, "http.request");
    assert_eq!(event.rule_action, capsem_logger::SecurityRuleAction::Allow);
    assert_eq!(
        event.detection_level,
        capsem_logger::SecurityDetectionLevel::Informational
    );
    assert!(event.rule_json.contains("openai_http_observed"));
    assert!(event.event_json.contains(r#""api.openai.com""#));
    assert_eq!(event.trace_id.as_deref(), Some("trace_route_authored_detection"));

    let detection_response = app
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/route-ledger-vm/detection/latest?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detection latest route should respond");
    assert_eq!(detection_response.status(), StatusCode::OK);
    let detection_body = to_bytes(detection_response.into_body(), usize::MAX).await.unwrap();
    let detection_events: Vec<capsem_logger::SecurityRuleEvent> = serde_json::from_slice(&detection_body).unwrap();
    assert!(detection_events
        .iter()
        .any(|detection| detection.rule_id == event.rule_id));
}

#[tokio::test]
async fn route_enforcement_evaluate_is_dry_run_and_does_not_write_ledger_rows() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, _, _) = install_empty_settings_env(&dir);
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = dir.path().join("sessions").join("dry-run-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "dry-run-vm", std::process::id(), session_dir.clone());
    capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16)
        .unwrap()
        .shutdown_blocking();

    let eval_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/profiles/code/enforcement/evaluate")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "rules_toml": r#"
[profiles.rules.eicar]
name = "eicar"
action = "block"
detection_level = "high"
match = 'file.import.content.contains("EICAR")'
"#,
                        "event": {
                            "event_type": "file.import",
                            "file_import_content": capsem_core::security_engine::DUMMY_EICAR_TEST_STRING,
                        }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .expect("evaluate route should respond");
    assert_eq!(eval_response.status(), StatusCode::OK);
    let eval_body = to_bytes(eval_response.into_body(), usize::MAX).await.unwrap();
    let evaluated: serde_json::Value = serde_json::from_slice(&eval_body).unwrap();
    assert_eq!(evaluated["event"]["decision"]["effective"], "block");

    let latest_response = app
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/dry-run-vm/security/latest?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("latest route should respond");
    assert_eq!(latest_response.status(), StatusCode::OK);
    let latest_body = to_bytes(latest_response.into_body(), usize::MAX).await.unwrap();
    let events: Vec<capsem_logger::SecurityRuleEvent> = serde_json::from_slice(&latest_body).unwrap();
    assert!(
        events.is_empty(),
        "evaluate routes are dry-run only; runtime boundaries must own ledger writes"
    )
}

#[tokio::test]
async fn handle_enforcement_evaluate_reuses_cached_raw_body_response() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let _home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let state = make_test_state();
    let request = EnforcementEvaluateRequest::eicar_fixture();
    let body = enforcement_evaluate_body(&request);

    let first_response = handle_enforcement_evaluate(State(Arc::clone(&state)), Path("code".to_string()), body.clone())
        .await
        .expect("first evaluate");
    let first: serde_json::Value = decode_response_json(first_response).await;
    assert_eq!(first["event"]["event_type"], "file.import");

    {
        assert_eq!(state.evaluate_response_cache.lock().unwrap().len(), 1);
        let mut last = state.evaluate_last_response_cache.lock().unwrap();
        let cached = last.as_mut().expect("last cached evaluate body");
        cached.response_body = Bytes::from_static(br#"{"event":{"event_type":"cached-sentinel"}}"#);
        drop(last);
    }

    let second_response = handle_enforcement_evaluate(State(state), Path("code".to_string()), body)
        .await
        .expect("second evaluate");
    let second: serde_json::Value = decode_response_json(second_response).await;
    assert_eq!(second["event"]["event_type"], "cached-sentinel");
}

#[tokio::test]
async fn mounted_service_ledger_routes_read_real_session_db_rows() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("service-ledger-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "service-ledger-vm", std::process::id(), session_dir.clone());

    let rule_set = SecurityRuleSet::new(
        SecurityRuleProfile {
            profiles: SecurityRuleGroup {
                rules: BTreeMap::from([(
                    "service_http_detect".to_string(),
                    capsem_core::net::policy_config::SecurityRule {
                        name: "service_http_detect".to_string(),
                        action: capsem_core::net::policy_config::SecurityRuleAction::Allow,
                        condition: r#"http.host.contains("example.com")"#.to_string(),
                        enabled: true,
                        detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Informational),
                        priority: Some(capsem_core::net::policy_config::SecurityRulePriority::Explicit(10)),
                        corp_locked: false,
                        reason: Some("service ledger route proof".to_string()),
                        managed: None,
                        plugin_config: BTreeMap::new(),
                    },
                )]),
            },
            ..SecurityRuleProfile::default()
        }
        .compile(SecurityRuleSource::User)
        .unwrap(),
    );
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    let event_id = capsem_core::security_engine::SecurityEventId::parse("123abc456def").unwrap();
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(
        capsem_core::security_engine::HttpSecurityEvent {
            host: Some("api.example.com".to_string()),
            method: Some("GET".to_string()),
            path: Some("/health".to_string()),
            query: None,
            status: Some("200".to_string()),
            body: None,
        },
    );
    let emitted = capsem_core::security_engine::emit_matching_security_rules(
        &writer,
        event_id,
        RuntimeSecurityEventType::HttpRequest,
        &rule_set,
        &event,
        1_789_000_223_456,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();
    assert_eq!(emitted, 1);
    for uri in [
        "/security/latest?limit=10",
        "/enforcement/latest?limit=10",
        "/detection/latest?limit=10",
    ] {
        let (status, rows) = route_request(app.clone(), axum::http::Method::GET, uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {rows}");
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 1, "{uri}: {rows:?}");
        assert_eq!(rows[0]["vm_id"], "service-ledger-vm");
        assert_eq!(rows[0]["event"]["event_id"], "123abc456def");
        assert_eq!(rows[0]["event"]["rule_id"], "profiles.rules.service_http_detect");
        assert_eq!(rows[0]["event"]["detection_level"], "informational");
    }

    for uri in ["/security/status", "/enforcement/status", "/detection/status"] {
        let (status, body) = route_request(app.clone(), axum::http::Method::GET, uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(body["total"], 1, "{uri}: {body}");
        assert_eq!(body["sessions"][0]["vm_id"], "service-ledger-vm");
    }
}

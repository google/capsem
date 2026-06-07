use super::*;
use std::sync::atomic::AtomicU64;

static SETTINGS_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn process_env_allowlist_forwards_mcp_timeout_knobs() {
    assert!(
        PROCESS_ENV_ALLOWLIST.contains(&"CAPSEM_HOME"),
        "CAPSEM_HOME must reach capsem-process so tests and custom installs use the same config root as capsem-service"
    );

    for key in [
        "CAPSEM_MCP_DEFAULT_TIMEOUT_SECS",
        "CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS",
        "CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS",
        "CAPSEM_EXPERIMENTAL_EROFS_DAX",
    ] {
        assert!(
            PROCESS_ENV_ALLOWLIST.contains(&key),
            "{key} must reach capsem-process because child-only boot/runtime config is read there"
        );
    }
}

#[test]
fn find_orphan_capsem_pids_matches_capsem_process_under_run_dir() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-abc");
    let ps = "\
  1502 /path/to/target/debug/capsem-process --env CAPSEM_VM_ID=orphan --id orphan --session-dir /var/folders/XY/T/capsem-test-abc/sessions/orphan --uds-path /tmp/capsem/abc.sock
  1742 /path/to/target/debug/capsem-process --id victim --session-dir /var/folders/XY/T/capsem-test-abc/persistent/victim --uds-path /tmp/capsem/def.sock
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert_eq!(pids, vec![1502, 1742]);
}

#[test]
fn find_orphan_capsem_pids_skips_processes_for_other_run_dirs() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-mine");
    let ps = "\
  1502 /path/to/target/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-other/sessions/foo
  1742 /path/to/target/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-mine/sessions/bar
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert_eq!(
        pids,
        vec![1742],
        "must not match neighbouring test run dirs"
    );
}

#[test]
fn find_orphan_capsem_pids_skips_non_capsem_process_binaries() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-abc");
    // A stray cargo invocation that happens to mention the run_dir path.
    let ps = "\
  99 /bin/cargo build --manifest-path /var/folders/XY/T/capsem-test-abc/Cargo.toml
  1502 /path/to/target/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-abc/sessions/orphan
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert_eq!(
        pids,
        vec![1502],
        "match must require 'capsem-process' in the line"
    );
}

#[test]
fn find_orphan_capsem_pids_returns_empty_on_no_match() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-empty");
    let ps = "\
  1 /sbin/launchd
  42 /usr/bin/bash
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert!(pids.is_empty());
}

fn test_magika() -> Mutex<magika::Session> {
    Mutex::new(
        magika::Session::builder()
            .with_inter_threads(1)
            .with_intra_threads(1)
            .build()
            .expect("magika init"),
    )
}

fn make_test_state() -> Arc<ServiceState> {
    let run_dir = PathBuf::from("/tmp/capsem-test-svc");
    let registry_path = run_dir.join("persistent_registry.json");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: PathBuf::from("/nonexistent/assets"),
        run_dir,
        job_counter: AtomicU64::new(1),
        manifest: None,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    })
}

fn make_asset_state(assets_dir: PathBuf) -> Arc<ServiceState> {
    let run_dir = assets_dir.join("run");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(
            assets_dir.join("persistent_registry.json"),
        )),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir,
        run_dir,
        job_counter: AtomicU64::new(1),
        manifest: None,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    })
}

fn insert_fake_instance(state: &ServiceState, id: &str, pid: u32) {
    insert_fake_instance_with_session_dir(
        state,
        id,
        pid,
        PathBuf::from(format!("/tmp/sessions/{}", id)),
    );
}

fn insert_fake_instance_with_session_dir(
    state: &ServiceState,
    id: &str,
    pid: u32,
    session_dir: PathBuf,
) {
    state.instances.lock().unwrap().insert(
        id.to_string(),
        InstanceInfo {
            id: id.to_string(),
            pid,
            uds_path: PathBuf::from(format!("/tmp/{}.sock", id)),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );
}

#[tokio::test]
async fn security_latest_returns_full_session_db_rule_ledger_rows() {
    let state = make_test_state();
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("vm-ledger");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "vm-ledger",
        std::process::id(),
        session_dir.clone(),
    );

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "abcdef123456",
                "model.call",
                "profiles.rules.ai_ollama_model_api",
                r#"{"name":"ollama_model_api_observed","match":"model.provider == \"ollama\""}"#,
                r#"{"model":{"provider":"ollama","name":"llama3.2"}}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow)
            .with_detection_level(capsem_logger::SecurityDetectionLevel::Informational)
            .with_trace_id("trace_ollama"),
        ))
        .await;
    drop(writer);

    let Json(events) = handle_security_latest(
        State(state),
        Path("vm-ledger".to_string()),
        Query(SecurityLedgerQuery { limit: Some(10) }),
    )
    .await
    .expect("security latest reads session.db");

    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.event_id, "abcdef123456");
    assert_eq!(event.event_type, "model.call");
    assert_eq!(event.rule_id, "profiles.rules.ai_ollama_model_api");
    assert_eq!(event.rule_action, capsem_logger::SecurityRuleAction::Allow);
    assert_eq!(
        event.detection_level,
        capsem_logger::SecurityDetectionLevel::Informational
    );
    assert!(event.rule_json.contains("ollama_model_api_observed"));
    assert!(event.event_json.contains(r#""provider":"ollama""#));
    assert_eq!(event.trace_id.as_deref(), Some("trace_ollama"));
}

#[test]
fn default_profile_summary_reflects_effective_contract() {
    let summary =
        build_default_profile_summary(&SettingsFile::default(), &SettingsFile::default(), 3);

    assert_eq!(summary.id, "default");
    assert_eq!(summary.name, "Default");
    assert_eq!(summary.description, "Built-in Capsem developer profile.");
    assert_eq!(summary.source, "effective");
    assert_eq!(summary.plugin_count, 3);
    assert!(
        summary.default_rule_count > 0,
        "default profile inventory must include built-in default security rules"
    );
    assert!(
        summary.rule_count >= summary.default_rule_count,
        "total rules cannot be lower than default rules"
    );
}

#[tokio::test]
async fn handle_profiles_list_returns_default_profile_inventory() {
    let state = make_test_state();

    let Json(response) = handle_profiles_list(State(state)).await.unwrap();

    assert_eq!(response.profiles.len(), 1);
    assert_eq!(response.profiles[0].id, "default");
    assert!(
        response.profiles[0].plugin_count > 0,
        "profile inventory should reflect editable plugin policy"
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
async fn handle_profile_validate_accepts_builtin_default_contract() {
    let response = handle_profile_validate(
        Path("default".to_string()),
        Json(api::ProfileValidateRequest {
            toml: None,
            profile: None,
        }),
    )
    .await
    .expect("builtin default profile should validate")
    .0;

    assert!(response.valid);
    assert_eq!(response.profile_id, "default");
}

#[tokio::test]
async fn handle_profile_validate_rejects_payload_route_mismatch() {
    let mut profile = ProfileConfigFile::builtin_default();
    profile.id = "strict".to_string();

    let err = handle_profile_validate(
        Path("default".to_string()),
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
async fn profile_mutation_routes_fail_explicitly_until_profile_files_exist() {
    let create = handle_profile_create().await.unwrap_err();
    assert_eq!(create.0, StatusCode::NOT_IMPLEMENTED);
    assert!(create.1.contains("profile file persistence"));

    let edit = handle_profile_edit(Path("default".to_string()))
        .await
        .unwrap_err();
    assert_eq!(edit.0, StatusCode::NOT_IMPLEMENTED);

    let delete = handle_profile_delete(Path("default".to_string()))
        .await
        .unwrap_err();
    assert_eq!(delete.0, StatusCode::NOT_IMPLEMENTED);

    let clone = handle_profile_clone(Path("default".to_string()))
        .await
        .unwrap_err();
    assert_eq!(clone.0, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn profile_skills_routes_reflect_manifest_and_gate_mutations() {
    let Json(info) = handle_profile_skills_info(Path("default".to_string()))
        .await
        .expect("skills info should reflect profile manifest");
    assert_eq!(info["profile_id"], "default");
    assert_eq!(info["skill_count"], 0);

    let Json(list) = handle_profile_skills_list(Path("default".to_string()))
        .await
        .expect("skills list should reflect profile manifest");
    assert_eq!(list["profile_id"], "default");
    assert!(list["skills"].as_array().unwrap().is_empty());

    let add = handle_profile_skill_add(Path("default".to_string()))
        .await
        .unwrap_err();
    assert_eq!(add.0, StatusCode::NOT_IMPLEMENTED);

    let edit = handle_profile_skill_edit(Path(("default".to_string(), "build".to_string())))
        .await
        .unwrap_err();
    assert_eq!(edit.0, StatusCode::NOT_IMPLEMENTED);

    let delete = handle_profile_skill_delete(Path(("default".to_string(), "build".to_string())))
        .await
        .unwrap_err();
    assert_eq!(delete.0, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn profile_credentials_routes_reflect_manifest_and_gate_inventory_mutations() {
    let Json(info) = handle_profile_credentials_info(Path("default".to_string()))
        .await
        .expect("credentials info should reflect profile manifest");
    assert_eq!(info["profile_id"], "default");
    assert_eq!(info["broker_enabled"], true);

    let Json(status) = handle_profile_credentials_status(Path("default".to_string()))
        .await
        .expect("credentials status should reflect profile manifest");
    assert_eq!(status["profile_id"], "default");
    assert_eq!(status["credential_count"], 0);

    let Json(list) = handle_profile_credentials_list(Path("default".to_string()))
        .await
        .expect("credentials list should be explicit");
    assert_eq!(list["profile_id"], "default");
    assert!(list["credentials"].as_array().unwrap().is_empty());

    let info = handle_profile_credential_info(Path(("default".to_string(), "openai".to_string())))
        .await
        .unwrap_err();
    assert_eq!(info.0, StatusCode::NOT_IMPLEMENTED);

    let delete =
        handle_profile_credential_delete(Path(("default".to_string(), "openai".to_string())))
            .await
            .unwrap_err();
    assert_eq!(delete.0, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn profile_assets_info_reflects_manifest_and_edit_is_gated() {
    let Json(info) = handle_profile_assets_info(Path("default".to_string()))
        .await
        .expect("assets info should reflect profile manifest");
    assert_eq!(info["profile_id"], "default");
    assert_eq!(info["rootfs"], "rootfs.erofs");

    let edit = handle_profile_assets_edit(Path("default".to_string()))
        .await
        .unwrap_err();
    assert_eq!(edit.0, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn profile_plugins_info_summarizes_effective_plugin_policy() {
    let state = make_test_state();

    let Json(info) = handle_profile_plugins_info(State(state), Path("default".to_string()))
        .await
        .expect("plugins info should summarize effective profile plugin policy");

    assert_eq!(info["scope"]["profile_id"], "default");
    assert!(info["plugin_count"].as_u64().unwrap() > 0);
    assert!(info["enabled_count"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn profile_mcp_info_summarizes_profile_mcp_config() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);
    let settings = capsem_core::net::policy_config::SettingsFile {
        mcp: Some(capsem_core::mcp::policy::McpUserConfig {
            servers: vec![capsem_core::mcp::policy::McpManualServer {
                name: "local".to_string(),
                url: "https://mcp.local".to_string(),
                headers: Default::default(),
                bearer_token: None,
                enabled: true,
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    capsem_core::net::policy_config::write_settings_file(&user_path, &settings).unwrap();

    let Json(info) = handle_profile_mcp_info(Path("default".to_string()))
        .await
        .expect("mcp info should summarize profile mcp config");

    assert_eq!(info["profile_id"], "default");
    assert_eq!(info["server_count"], 1);
    assert_eq!(info["user_server_count"], 1);
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
    let unknown_profile =
        handle_profile_plugins_info(State(make_test_state()), Path("strict".to_string()))
            .await
            .unwrap_err();
    assert_eq!(unknown_profile.0, StatusCode::NOT_FOUND);

    let unknown_vm = handle_vm_edit(
        State(make_test_state()),
        Path("missing-vm".to_string()),
        Json(api::VmEditRequest {
            ram_mb: Some(2048),
            ..Default::default()
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(unknown_vm.0, StatusCode::NOT_FOUND);

    let bad_rule = capsem_core::net::policy_config::SecurityRule {
        name: "bad_rule".to_string(),
        action: capsem_core::net::policy_config::SecurityRuleAction::Allow,
        condition: "file.read.path.contains(\"tmp\")".to_string(),
        detection_level: None,
        priority: None,
        corp_locked: false,
        reason: None,
        plugin: None,
        plugin_config: BTreeMap::new(),
    };
    let malformed_rule_id = handle_enforcement_rule_upsert(
        Path(("default".to_string(), "Bad Rule".to_string())),
        Json(bad_rule),
    )
    .await
    .unwrap_err();
    assert_eq!(malformed_rule_id.0, StatusCode::BAD_REQUEST);

    let invalid_enum = serde_json::from_value::<PluginUpdate>(json!({
        "mode": "teleport",
    }));
    assert!(invalid_enum.is_err());

    let immutable_profile = handle_vm_edit(
        State(make_test_state()),
        Path("missing-vm".to_string()),
        Json(api::VmEditRequest {
            profile_id: Some("strict".to_string()),
            ..Default::default()
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(immutable_profile.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn handle_enforcement_rules_list_returns_compiled_profile_rules() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);
    let mut settings = capsem_core::net::policy_config::SettingsFile::default();
    settings.profiles.rules.insert(
        "skill_loaded".to_string(),
        capsem_core::net::policy_config::SecurityRule {
            name: "skill_loaded".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Allow,
            condition: r#"file.read.path.contains("skills/")"#.to_string(),
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Informational),
            priority: None,
            corp_locked: false,
            reason: Some("record skill file reads".to_string()),
            plugin: None,
            plugin_config: BTreeMap::new(),
        },
    );
    capsem_core::net::policy_config::write_settings_file(&user_path, &settings).unwrap();

    let Json(response) = handle_enforcement_rules_list(Path("default".to_string()))
        .await
        .expect("rules list should compile effective profile");

    assert_eq!(response.profile_id, "default");
    assert!(
        response.rules.iter().any(
            |rule| rule.rule_id == "profiles.rules.default_http_requests"
                && rule.source == api::EnforcementRuleSource::BuiltinDefault
                && rule.default_rule
        ),
        "list must expose built-in default rules as first-class rows"
    );
    let custom = response
        .rules
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.skill_loaded")
        .expect("custom profile rule should be listed");
    assert_eq!(custom.source, api::EnforcementRuleSource::Profile);
    assert!(!custom.default_rule);
    assert_eq!(custom.priority, 10);
    assert_eq!(
        custom.detection_level,
        Some(capsem_core::net::policy_config::DetectionLevel::Informational)
    );
}

#[tokio::test]
async fn handle_enforcement_rules_list_rejects_unknown_profiles() {
    let err = handle_enforcement_rules_list(Path("strict".to_string()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("profile not found: strict"));
}

#[tokio::test]
async fn handle_enforcement_info_summarizes_compiled_rules() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);
    let mut settings = capsem_core::net::policy_config::SettingsFile::default();
    settings.profiles.rules.insert(
        "skill_loaded".to_string(),
        capsem_core::net::policy_config::SecurityRule {
            name: "skill_loaded".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Allow,
            condition: r#"file.read.path.contains("skills/")"#.to_string(),
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Informational),
            priority: None,
            corp_locked: false,
            reason: Some("record skill file reads".to_string()),
            plugin: None,
            plugin_config: BTreeMap::new(),
        },
    );
    capsem_core::net::policy_config::write_settings_file(&user_path, &settings).unwrap();

    let Json(info) = handle_enforcement_info(Path("default".to_string()))
        .await
        .expect("info should summarize effective rules");

    assert_eq!(info.profile_id, "default");
    assert!(info.rule_count > 0);
    assert!(info.default_rule_count > 0);
    assert!(info.custom_rule_count >= 1);
    assert!(info.detection_rule_count >= 1);
    assert_eq!(info.source_counts["profile"], 1);
    assert!(info.source_counts["builtin_default"] > 0);
    assert!(info.action_counts.contains_key("allow"));
}

#[tokio::test]
async fn handle_enforcement_info_rejects_unknown_profiles() {
    let err = handle_enforcement_info(Path("strict".to_string()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("profile not found: strict"));
}

#[tokio::test]
async fn handle_detection_rules_list_returns_detection_rules_only() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);
    let mut settings = capsem_core::net::policy_config::SettingsFile::default();
    settings.profiles.rules.insert(
        "skill_loaded".to_string(),
        capsem_core::net::policy_config::SecurityRule {
            name: "skill_loaded".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Allow,
            condition: r#"file.read.path.contains("skills/")"#.to_string(),
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Informational),
            priority: None,
            corp_locked: false,
            reason: Some("record skill file reads".to_string()),
            plugin: None,
            plugin_config: BTreeMap::new(),
        },
    );
    settings.profiles.rules.insert(
        "pure_block".to_string(),
        capsem_core::net::policy_config::SecurityRule {
            name: "pure_block".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Block,
            condition: r#"file.read.path.contains("tmp")"#.to_string(),
            detection_level: None,
            priority: None,
            corp_locked: false,
            reason: Some("block example without reporting".to_string()),
            plugin: None,
            plugin_config: BTreeMap::new(),
        },
    );
    capsem_core::net::policy_config::write_settings_file(&user_path, &settings).unwrap();

    let Json(response) = handle_detection_rules_list(Path("default".to_string()))
        .await
        .expect("detection rules list should compile effective profile");

    assert_eq!(response.profile_id, "default");
    assert!(
        response
            .rules
            .iter()
            .all(|rule| rule.detection_level.is_some()),
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
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);
    let mut settings = capsem_core::net::policy_config::SettingsFile::default();
    settings.profiles.rules.insert(
        "skill_loaded".to_string(),
        capsem_core::net::policy_config::SecurityRule {
            name: "skill_loaded".to_string(),
            action: capsem_core::net::policy_config::SecurityRuleAction::Allow,
            condition: r#"file.read.path.contains("skills/")"#.to_string(),
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Informational),
            priority: None,
            corp_locked: false,
            reason: Some("record skill file reads".to_string()),
            plugin: None,
            plugin_config: BTreeMap::new(),
        },
    );
    capsem_core::net::policy_config::write_settings_file(&user_path, &settings).unwrap();

    let Json(info) = handle_detection_info(Path("default".to_string()))
        .await
        .expect("detection info should summarize effective detection rules");

    assert_eq!(info.profile_id, "default");
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
        detection_level: None,
        priority: None,
        corp_locked: false,
        reason: Some("block without reporting".to_string()),
        plugin: None,
        plugin_config: BTreeMap::new(),
    };

    let err = handle_detection_rule_upsert(
        Path(("default".to_string(), "pure_block".to_string())),
        Json(rule),
    )
    .await
    .unwrap_err();

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("requires detection_level"));
}

#[tokio::test]
async fn handle_detection_rules_list_rejects_unknown_profiles() {
    let err = handle_detection_rules_list(Path("strict".to_string()))
        .await
        .unwrap_err();

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert!(err.1.contains("profile not found: strict"));
}

#[tokio::test]
async fn profile_plugin_endpoint_matrix_dynamically_controls_enforcement_evaluation() {
    let state = make_test_state();

    let Json(list) = handle_profile_plugins(State(Arc::clone(&state)), Path("default".to_string()))
        .await
        .expect("list plugins");
    assert_eq!(list.scope.profile_id, "default");
    assert!(
        list.plugins
            .iter()
            .any(|plugin| plugin.id == "dummy_pre_eicar"),
        "built-in plugin list must include dummy_pre_eicar"
    );

    let Json(info) = handle_profile_plugin_info(
        State(Arc::clone(&state)),
        Path(("default".to_string(), "dummy_pre_eicar".to_string())),
    )
    .await
    .expect("plugin info");
    assert_eq!(info.id, "dummy_pre_eicar");
    assert_eq!(info.scope.profile_id, "default");
    assert_eq!(
        info.config.mode,
        capsem_core::net::policy_config::SecurityPluginMode::Rewrite
    );
    assert_eq!(
        info.config.detection_level,
        capsem_core::net::policy_config::DetectionLevel::Informational
    );

    let request = EnforcementEvaluateRequest::eicar_fixture();
    let Json(enabled) = handle_enforcement_evaluate(
        State(Arc::clone(&state)),
        Path("default".to_string()),
        Json(request.clone()),
    )
    .await
    .expect("enabled plugin evaluates");
    let enabled_event = serde_json::to_value(&enabled.event).unwrap();
    assert_eq!(enabled_event["decision"]["effective"], "block");
    assert_eq!(enabled_event["detections"].as_array().unwrap().len(), 2);
    assert!(
        enabled_event.get("http").is_some(),
        "wire DTO must expose every first-party root, even when null"
    );

    let Json(disabled) = handle_profile_plugin_update(
        State(Arc::clone(&state)),
        Path(("default".to_string(), "dummy_pre_eicar".to_string())),
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

    let Json(after_disable) = handle_enforcement_evaluate(
        State(Arc::clone(&state)),
        Path("default".to_string()),
        Json(request.clone()),
    )
    .await
    .expect("disabled plugin evaluates");
    let after_disable_event = serde_json::to_value(&after_disable.event).unwrap();
    assert_eq!(after_disable_event["decision"]["effective"], "allow");
    assert_eq!(
        after_disable_event["detections"].as_array().unwrap().len(),
        1,
        "rule detection remains, disabled plugin detection disappears"
    );

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
        Path(("default".to_string(), "dummy_pre_eicar".to_string())),
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

    let Json(after_enable) =
        handle_enforcement_evaluate(State(state), Path("default".to_string()), Json(request))
            .await
            .expect("reenabled plugin evaluates");
    let after_enable_event = serde_json::to_value(&after_enable.event).unwrap();
    assert_eq!(after_enable_event["decision"]["effective"], "block");
    let detections = after_enable_event["detections"].as_array().unwrap();
    assert_eq!(detections.len(), 2);
    assert!(detections.iter().any(|detection| {
        detection["source"] == "plugin"
            && detection["plugin_id"] == "dummy_pre_eicar"
            && detection["detection_level"] == "critical"
            && detection["plugin_mode"] == "block"
    }));
}

#[tokio::test]
async fn enforcement_rule_endpoints_add_delete_reload_and_reject_invalid_rules_atomically() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);
    let rule = capsem_core::net::policy_config::SecurityRule {
        name: "file_import_eicar_block".to_string(),
        action: capsem_core::net::policy_config::SecurityRuleAction::Block,
        condition: r#"file.import.content.contains("EICAR")"#.to_string(),
        detection_level: Some(capsem_core::net::policy_config::DetectionLevel::High),
        priority: Some(capsem_core::net::policy_config::SecurityRulePriority::Explicit(10)),
        corp_locked: false,
        reason: Some("debug EICAR fixture must block".to_string()),
        plugin: None,
        plugin_config: BTreeMap::new(),
    };

    let Json(saved) = handle_enforcement_rule_upsert(
        Path(("default".to_string(), "eicar_block".to_string())),
        Json(rule.clone()),
    )
    .await
    .expect("valid profile enforcement rule should save");
    assert_eq!(saved.rule_id, "eicar_block");
    assert_eq!(saved.compiled_rule_id, "profiles.rules.eicar_block");

    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
    assert_eq!(
        loaded.profiles.rules["eicar_block"].action,
        capsem_core::net::policy_config::SecurityRuleAction::Block
    );

    let Json(reload) =
        handle_enforcement_reload(State(make_test_state()), Path("default".to_string()))
            .await
            .expect("reload alias should broadcast to zero instances");
    assert_eq!(reload["success"], serde_json::json!(true));
    assert_eq!(reload["reloaded"], serde_json::json!(0));

    let mut bad_priority = rule.clone();
    bad_priority.priority =
        Some(capsem_core::net::policy_config::SecurityRulePriority::Explicit(-100));
    let err = handle_enforcement_rule_upsert(
        Path(("default".to_string(), "bad_negative_priority".to_string())),
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
        Path(("default".to_string(), "corp_locked".to_string())),
        Json(corp_locked),
    )
    .await
    .expect_err("user rule endpoint must not create corp-locked rules");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);

    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
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

    let Json(deleted) =
        handle_enforcement_rule_delete(Path(("default".to_string(), "eicar_block".to_string())))
            .await
            .expect("delete should remove existing rule");
    assert!(deleted.deleted);
    assert_eq!(deleted.rule_id, "eicar_block");
    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
    assert!(!loaded.profiles.rules.contains_key("eicar_block"));

    let err =
        handle_enforcement_rule_delete(Path(("default".to_string(), "eicar_block".to_string())))
            .await
            .expect_err("deleting a missing rule should return not found");
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[test]
fn resolve_asset_paths_prefers_erofs_when_present() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vmlinuz"), b"kernel").unwrap();
    std::fs::write(dir.path().join("initrd.img"), b"initrd").unwrap();
    std::fs::write(dir.path().join("rootfs.squashfs"), b"squashfs").unwrap();
    std::fs::write(dir.path().join("rootfs.erofs"), b"erofs").unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let resolved = state.resolve_asset_paths().unwrap();
    assert_eq!(resolved.rootfs, dir.path().join("rootfs.erofs"));
}

#[test]
fn resolve_asset_paths_falls_back_to_squashfs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vmlinuz"), b"kernel").unwrap();
    std::fs::write(dir.path().join("initrd.img"), b"initrd").unwrap();
    std::fs::write(dir.path().join("rootfs.squashfs"), b"squashfs").unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let resolved = state.resolve_asset_paths().unwrap();
    assert_eq!(resolved.rootfs, dir.path().join("rootfs.squashfs"));
}

#[test]
fn asset_status_reports_reconcile_progress_fields() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vmlinuz"), b"kernel").unwrap();
    std::fs::write(dir.path().join("initrd.img"), b"initrd").unwrap();
    std::fs::write(dir.path().join("rootfs.erofs"), b"erofs").unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    {
        let mut reconcile = state.asset_reconcile.lock().unwrap();
        *reconcile = AssetReconcileState {
            in_progress: true,
            current_asset: Some("rootfs.erofs".to_string()),
            bytes_done: 128,
            bytes_total: Some(256),
            last_error: None,
            last_downloaded: None,
        };
    }

    let status = asset_status_value(&state);
    assert_eq!(status["ready"], true);
    assert_eq!(status["downloading"], true);
    assert_eq!(status["current_asset"], "rootfs.erofs");
    assert_eq!(status["bytes_done"], 128);
    assert_eq!(status["bytes_total"], 256);
}

#[test]
fn vm_asset_block_reason_reports_missing_assets() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let reason = vm_asset_block_reason(&state).expect("missing assets must block VM start");

    assert!(reason.contains("VM assets are not ready"));
    assert!(reason.contains("vmlinuz"));
    assert!(reason.contains("initrd.img"));
}

#[test]
fn vm_asset_block_reason_reports_downloading_assets() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    state.asset_reconcile.lock().unwrap().in_progress = true;

    let reason = vm_asset_block_reason(&state).expect("missing assets must block VM start");

    assert!(reason.contains("VM assets are still downloading"));
}

#[test]
fn vm_asset_block_reason_allows_ready_assets() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vmlinuz"), b"kernel").unwrap();
    std::fs::write(dir.path().join("initrd.img"), b"initrd").unwrap();
    std::fs::write(dir.path().join("rootfs.erofs"), b"erofs").unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    assert!(vm_asset_block_reason(&state).is_none());
}

#[test]
fn load_asset_reconcile_state_resets_stale_in_progress() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset-status.json");
    std::fs::write(
        &path,
        r#"{
          "in_progress": true,
          "current_asset": "rootfs.erofs",
          "bytes_done": 512,
          "bytes_total": 1024,
          "last_error": "prior failure",
          "last_downloaded": 2
        }"#,
    )
    .unwrap();

    let loaded = load_asset_reconcile_state(&path);

    assert!(
        !loaded.in_progress,
        "startup must not preserve stale active download state"
    );
    assert!(loaded.current_asset.is_none());
    assert_eq!(loaded.bytes_done, 0);
    assert!(loaded.bytes_total.is_none());
    assert_eq!(loaded.last_error.as_deref(), Some("prior failure"));
    assert_eq!(loaded.last_downloaded, Some(2));
}

#[test]
fn persist_asset_reconcile_state_roundtrips_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("asset-status.json");
    let status = AssetReconcileState {
        in_progress: false,
        current_asset: None,
        bytes_done: 0,
        bytes_total: None,
        last_error: Some("GET failed".to_string()),
        last_downloaded: Some(0),
    };

    persist_asset_reconcile_state(&path, &status).unwrap();
    let loaded = load_asset_reconcile_state(&path);

    assert_eq!(loaded.last_error.as_deref(), Some("GET failed"));
    assert_eq!(loaded.last_downloaded, Some(0));
    assert!(!loaded.in_progress);
}

#[tokio::test]
async fn ensure_assets_without_manifest_is_noop_success() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let downloaded = ensure_assets_for_state(Arc::clone(&state)).await.unwrap();

    assert_eq!(downloaded, 0);
    let reconcile = state.asset_reconcile.lock().unwrap();
    assert!(!reconcile.in_progress);
    assert_eq!(reconcile.last_downloaded, Some(0));
    assert!(reconcile.last_error.is_none());
    drop(reconcile);

    let persisted = load_asset_reconcile_state(&state.asset_status_path);
    assert!(!persisted.in_progress);
    assert_eq!(persisted.last_downloaded, Some(0));
    assert!(persisted.last_error.is_none());
}

#[tokio::test]
async fn ensure_assets_rejects_concurrent_reconcile() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    state
        .asset_reconcile_inflight
        .store(true, Ordering::Release);

    let err = ensure_assets_for_state(Arc::clone(&state))
        .await
        .expect_err("second reconcile must be rejected");

    assert!(
        err.contains("already in progress"),
        "unexpected error: {err}"
    );
    assert!(state.asset_reconcile_inflight.load(Ordering::Acquire));
    state
        .asset_reconcile_inflight
        .store(false, Ordering::Release);
}

// -----------------------------------------------------------------------
// next_job_id
// -----------------------------------------------------------------------

#[test]
fn next_job_id_starts_at_1() {
    let state = make_test_state();
    assert_eq!(state.next_job_id(), 1);
}

#[test]
fn next_job_id_increments() {
    let state = make_test_state();
    let a = state.next_job_id();
    let b = state.next_job_id();
    let c = state.next_job_id();
    assert_eq!(b, a + 1);
    assert_eq!(c, a + 2);
}

#[test]
fn next_job_id_unique_across_many() {
    let state = make_test_state();
    let ids: Vec<u64> = (0..1000).map(|_| state.next_job_id()).collect();
    let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), 1000);
}

// -----------------------------------------------------------------------
// Instance map CRUD
// -----------------------------------------------------------------------

#[test]
fn instance_insert_and_lookup() {
    let state = make_test_state();
    insert_fake_instance(&state, "test-vm", std::process::id());
    let instances = state.instances.lock().unwrap();
    assert!(instances.contains_key("test-vm"));
    assert_eq!(instances["test-vm"].ram_mb, 2048);
}

#[test]
fn instance_remove() {
    let state = make_test_state();
    insert_fake_instance(&state, "test-vm", std::process::id());
    state.instances.lock().unwrap().remove("test-vm");
    assert!(!state.instances.lock().unwrap().contains_key("test-vm"));
}

#[test]
fn instance_lookup_missing() {
    let state = make_test_state();
    assert!(!state.instances.lock().unwrap().contains_key("no-such-vm"));
}

#[test]
fn instance_count() {
    let state = make_test_state();
    insert_fake_instance(&state, "vm-1", std::process::id());
    insert_fake_instance(&state, "vm-2", std::process::id());
    insert_fake_instance(&state, "vm-3", std::process::id());
    assert_eq!(state.instances.lock().unwrap().len(), 3);
}

// -----------------------------------------------------------------------
// cleanup_stale_instances
// -----------------------------------------------------------------------

#[test]
fn cleanup_removes_dead_pid() {
    let state = make_test_state();
    // PID 99999999 should not exist
    insert_fake_instance(&state, "dead-vm", 99999999);
    assert_eq!(state.instances.lock().unwrap().len(), 1);
    state.cleanup_stale_instances();
    assert_eq!(state.instances.lock().unwrap().len(), 0);
}

#[test]
fn cleanup_keeps_live_pid() {
    let state = make_test_state();
    // Current process PID should be alive
    insert_fake_instance(&state, "live-vm", std::process::id());
    state.cleanup_stale_instances();
    assert_eq!(state.instances.lock().unwrap().len(), 1);
}

#[test]
fn cleanup_mixed_live_and_dead() {
    let state = make_test_state();
    insert_fake_instance(&state, "live", std::process::id());
    insert_fake_instance(&state, "dead", 99999999);
    state.cleanup_stale_instances();
    let instances = state.instances.lock().unwrap();
    assert_eq!(instances.len(), 1);
    assert!(instances.contains_key("live"));
}

// -----------------------------------------------------------------------
// drain_dead_instances: probe-and-evict contract, filesystem work is the
// caller's responsibility. Exists so `cleanup_stale_instances` can release
// the instances mutex BEFORE performing remove_dir_all -- otherwise every
// handler that touches instances.lock() blocks on slow fs I/O.
// -----------------------------------------------------------------------

#[test]
fn drain_dead_instances_returns_only_dead_entries() {
    let state = make_test_state();
    insert_fake_instance(&state, "live", std::process::id());
    insert_fake_instance(&state, "dead", 99999999);

    let evicted = state.drain_dead_instances();

    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].0, "dead");
    let map = state.instances.lock().unwrap();
    assert!(map.contains_key("live"));
    assert!(!map.contains_key("dead"));
}

#[test]
fn drain_dead_instances_empty_when_all_alive() {
    let state = make_test_state();
    insert_fake_instance(&state, "live-1", std::process::id());
    insert_fake_instance(&state, "live-2", std::process::id());

    let evicted = state.drain_dead_instances();

    assert!(evicted.is_empty());
    assert_eq!(state.instances.lock().unwrap().len(), 2);
}

#[test]
fn drain_dead_instances_releases_mutex_before_returning() {
    // Regression guard: the whole point of splitting drain from the
    // filesystem scrub is that the mutex must be FREE by the time
    // drain returns. If this test ever fails, the locking protocol
    // has regressed and concurrent handlers will block on cleanup I/O.
    let state = make_test_state();
    insert_fake_instance(&state, "dead", 99999999);

    let _evicted = state.drain_dead_instances();

    assert!(
        state.instances.try_lock().is_ok(),
        "mutex still held after drain_dead_instances returned"
    );
}

// -----------------------------------------------------------------------
// preserve_failed_session_dir + cull_failed_sessions
//
// The post-mortem pipeline: when any of the three loss paths
// (wait_for_vm_ready timeout, dead-process cleanup, unexpected
// child exit) would have silently `remove_dir_all`'d a session dir,
// it's renamed to a `-failed-*` sibling instead so process.log,
// mcp-aggregator.stderr.log, serial.log, and session.db survive.
// Cap: MAX_FAILED_SESSIONS (5).
// -----------------------------------------------------------------------

fn make_state_in(run_dir: PathBuf) -> Arc<ServiceState> {
    let registry_path = run_dir.join("persistent_registry.json");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    std::fs::create_dir_all(run_dir.join("sessions")).unwrap();
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: PathBuf::from("/nonexistent/assets"),
        run_dir,
        job_counter: AtomicU64::new(1),
        manifest: None,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    })
}

#[test]
fn preserve_renames_session_dir_and_keeps_logs() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("vm-abc");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), b"boot failed: ...").unwrap();
    std::fs::write(session_dir.join("serial.log"), b"kernel panic").unwrap();

    state.preserve_failed_session_dir(&session_dir, "vm-abc");

    assert!(
        !session_dir.exists(),
        "original dir should have been renamed"
    );
    let entries: Vec<_> = std::fs::read_dir(state.run_dir.join("sessions"))
        .unwrap()
        .flatten()
        .collect();
    let failed = entries
        .iter()
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("vm-abc-failed-")
        })
        .expect("a vm-abc-failed-* dir must exist");
    let preserved = failed.path().join("process.log");
    assert_eq!(std::fs::read(&preserved).unwrap(), b"boot failed: ...");
    let preserved_serial = failed.path().join("serial.log");
    assert_eq!(std::fs::read(&preserved_serial).unwrap(), b"kernel panic");
}

#[test]
fn cull_keeps_newest_and_prunes_oldest() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    // Create MAX_FAILED_SESSIONS + 2 failed dirs with staggered mtimes.
    // Using filetime to set mtime lets us assert deterministically
    // which ones get pruned (oldest) vs kept (newest).
    let total = MAX_FAILED_SESSIONS + 2;
    for i in 0..total {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        let p = sessions.join(&name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("process.log"), format!("run {i}")).unwrap();
        // Older i -> older mtime.
        let when = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(1_700_000_000 + i as u64 * 10);
        filetime::set_file_mtime(&p, filetime::FileTime::from_system_time(when)).unwrap();
    }

    state.cull_failed_sessions().unwrap();

    let remaining: std::collections::HashSet<String> = std::fs::read_dir(&sessions)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        remaining.len(),
        MAX_FAILED_SESSIONS,
        "should keep exactly MAX_FAILED_SESSIONS, got {remaining:?}"
    );
    // Oldest two (i=0, i=1) must be pruned; newest MAX_FAILED_SESSIONS kept.
    for i in 0..2 {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        assert!(
            !remaining.contains(&name),
            "oldest dir {name} should have been culled"
        );
    }
    for i in 2..total {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        assert!(
            remaining.contains(&name),
            "newer dir {name} should have been kept"
        );
    }
}

#[test]
fn cull_is_noop_when_under_cap() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    for i in 0..3 {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        std::fs::create_dir_all(sessions.join(&name)).unwrap();
    }

    state.cull_failed_sessions().unwrap();

    assert_eq!(std::fs::read_dir(&sessions).unwrap().count(), 3);
}

#[test]
fn cull_ignores_non_failed_dirs() {
    // Running sessions (no `-failed-` in the name) must never be
    // culled. This is the safety property: a misnamed cull is a
    // production outage.
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    std::fs::create_dir_all(sessions.join("vm-alive")).unwrap();
    for i in 0..(MAX_FAILED_SESSIONS + 3) {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        std::fs::create_dir_all(sessions.join(&name)).unwrap();
    }

    state.cull_failed_sessions().unwrap();

    assert!(
        sessions.join("vm-alive").exists(),
        "active VM dir must not be culled"
    );
}

// -----------------------------------------------------------------------
// Auto-ID generation format
// -----------------------------------------------------------------------

#[test]
fn auto_id_format() {
    // Verify the auto-ID pattern used in handle_provision
    let id = format!(
        "vm-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    assert!(id.starts_with("vm-"));
    // Should be "vm-" followed by digits
    let suffix = &id[3..];
    assert!(suffix.chars().all(|c| c.is_ascii_digit()));
}

// -----------------------------------------------------------------------
// Input validation edge cases (DTO level)
// -----------------------------------------------------------------------

#[test]
fn provision_request_no_name() {
    let json = serde_json::json!({"ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert!(req.name.is_none());
}

#[test]
fn provision_request_empty_name() {
    let json = serde_json::json!({"name": "", "ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.name.unwrap(), "");
}

#[test]
fn provision_request_name_with_path_separator() {
    // This is a security edge case -- names with / could create path traversal
    let json = serde_json::json!({"name": "../escape", "ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.name.unwrap(), "../escape");
    // Note: the service SHOULD reject this, but currently doesn't validate
}

#[test]
fn exec_request_empty_command() {
    let json = serde_json::json!({"command": ""});
    let req: ExecRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.command, "");
}

#[test]
fn exec_request_shell_metacharacters() {
    let json = serde_json::json!({"command": "echo $(whoami) && rm -rf /"});
    let req: ExecRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.command, "echo $(whoami) && rm -rf /");
}

#[test]
fn write_file_request_path_traversal() {
    let json = serde_json::json!({"path": "../../etc/passwd", "content": "evil"});
    let req: WriteFileRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.path, "../../etc/passwd");
    // Note: no validation at DTO level -- relies on guest-side enforcement
}

#[test]
fn inspect_request_sql_injection() {
    let json = serde_json::json!({"sql": "SELECT * FROM net_events; DROP TABLE net_events; --"});
    let req: InspectRequest = serde_json::from_value(json).unwrap();
    assert!(req.sql.contains("DROP TABLE"));
    // Note: backend should use read-only DB connection to prevent writes
}

// -----------------------------------------------------------------------
// Asset path resolution
// -----------------------------------------------------------------------

#[test]
fn asset_version_path_construction() {
    let base = PathBuf::from("/home/user/.capsem/assets");
    let version = "0.16.1";
    let v_path = base.join(format!("v{}", version));
    assert_eq!(v_path, PathBuf::from("/home/user/.capsem/assets/v0.16.1"));
}

#[test]
fn arch_detection_aarch64() {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    assert!(arch == "arm64" || arch == "x86_64");
}

// -----------------------------------------------------------------------
// UDS path length validation (macOS 104, Linux 108 including null)
// -----------------------------------------------------------------------

#[test]
fn long_vm_name_falls_back_to_tmp_socket() {
    let state = make_test_state();
    // A 100-char name exceeds SUN_PATH_MAX via run_dir/instances/ path,
    // but instance_socket_path should fall back to /tmp/capsem/.
    let long_name = "a".repeat(100);
    let path = state.instance_socket_path(&long_name);
    assert!(
        path.starts_with("/tmp/capsem/"),
        "expected /tmp/capsem/ fallback, got: {}",
        path.display()
    );
    assert!(
        path.as_os_str().len() < 104,
        "fallback path still too long: {}",
        path.as_os_str().len()
    );
}

#[test]
fn short_vm_name_uses_run_dir() {
    let state = make_test_state();
    let path = state.instance_socket_path("test-vm");
    assert_eq!(path, state.run_dir.join("instances/test-vm.sock"));
}

#[test]
fn provision_accepts_name_just_under_uds_limit() {
    let state = make_test_state();
    let prefix = state.run_dir.join("instances").join("").as_os_str().len();
    let suffix_len = ".sock".len();
    let sun_path_max: usize = if cfg!(target_os = "macos") { 104 } else { 108 };
    // One byte shorter than the limit -- should pass path validation
    let name_len = sun_path_max - prefix - suffix_len - 1;
    let ok_name = "x".repeat(name_len);
    let result = state.provision_sandbox(ProvisionOptions {
        id: &ok_name,
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: false,
        env: None,
        from: None,
        description: None,
    });
    // Will fail later (missing rootfs), but NOT for path length
    if let Err(e) = &result {
        let msg = e.to_string();
        assert!(
            !msg.contains("socket path"),
            "short name should not hit path limit: {msg}"
        );
    }
}

#[test]
fn provision_short_name_passes_path_check() {
    let state = make_test_state();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "my-vm",
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: false,
        env: None,
        from: None,
        description: None,
    });
    // Fails for missing assets, not path length
    if let Err(e) = &result {
        let msg = e.to_string();
        assert!(
            !msg.contains("socket path"),
            "normal name should not hit path limit: {msg}"
        );
    }
}

// -----------------------------------------------------------------------
// Provision rejects duplicate persistent VM
// -----------------------------------------------------------------------

#[test]
fn provision_persistent_rejects_duplicate_name() {
    let state = make_test_state();
    // Pre-register a persistent VM directly in the registry data
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "taken".into(),
            PersistentVmEntry {
                name: "taken".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: PathBuf::from("/tmp/taken"),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }
    let result = state.provision_sandbox(ProvisionOptions {
        id: "taken",
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: true,
        env: None,
        from: None,
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("already exists"),
        "expected duplicate error, got: {err}"
    );
    assert!(err.contains("resume"), "should suggest resume, got: {err}");
}

#[test]
fn provision_persistent_validates_name() {
    let state = make_test_state();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "../evil",
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: true,
        env: None,
        from: None,
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("must start with") || err.contains("must contain only"),
        "expected name validation error, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Image handler tests (service-level unit tests)
// -----------------------------------------------------------------------

fn make_test_state_with_tempdir() -> (Arc<ServiceState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let registry_path = dir.path().join("persistent_registry.json");
    let run_dir = dir.path().to_path_buf();
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: dir.path().join("assets"),
        run_dir,
        job_counter: AtomicU64::new(1),
        manifest: None,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    });
    (state, dir)
}

#[tokio::test]
async fn handle_fork_creates_persistent_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    // Create a real session dir for the fake instance
    let session_dir = state.run_dir.join("sessions/fork-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.instances.lock().unwrap().insert(
        "fork-src".into(),
        InstanceInfo {
            id: "fork-src".into(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/fork-src.sock"),
            session_dir: session_dir.clone(),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );
    let result = handle_fork(
        State(state.clone()),
        Path("fork-src".into()),
        Json(ForkRequest {
            name: "my-fork".into(),
            description: Some("test".into()),
        }),
    )
    .await
    .unwrap();
    assert_eq!(result.0.name, "my-fork");
    assert!(result.0.size_bytes > 0);
    // Verify fork created a persistent sandbox entry in the registry
    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("my-fork").unwrap();
    assert_eq!(entry.forked_from, Some("fork-src".into()));
    assert_eq!(entry.description, Some("test".into()));
    assert_eq!(entry.base_version, "0.0.0");
}

#[tokio::test]
async fn handle_fork_not_found() {
    let (state, _dir) = make_test_state_with_tempdir();
    // state is already Arc<ServiceState> from make_test_state*
    let err = handle_fork(
        State(state),
        Path("ghost".into()),
        Json(ForkRequest {
            name: "img".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handle_fork_duplicate_returns_conflict() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/dup-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.instances.lock().unwrap().insert(
        "dup-src".into(),
        InstanceInfo {
            id: "dup-src".into(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/dup-src.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );
    // state is already Arc<ServiceState> from make_test_state*
    // First fork succeeds
    let _ = handle_fork(
        State(state.clone()),
        Path("dup-src".into()),
        Json(ForkRequest {
            name: "same-name".into(),
            description: None,
        }),
    )
    .await
    .unwrap();
    // Second fork with same name returns CONFLICT
    let err = handle_fork(
        State(state),
        Path("dup-src".into()),
        Json(ForkRequest {
            name: "same-name".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn handle_fork_from_persistent_registry() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/pers-vm");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "pers-vm".into(),
            PersistentVmEntry {
                name: "pers-vm".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                session_dir: session_dir.clone(),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }
    // state is already Arc<ServiceState> from make_test_state*
    let result = handle_fork(
        State(state),
        Path("pers-vm".into()),
        Json(ForkRequest {
            name: "from-pers".into(),
            description: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(result.0.name, "from-pers");
}

#[test]
fn provision_rejects_nonexistent_source_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "vm1",
        ram_mb: 2048,
        cpus: 2,
        version_override: None,
        persistent: false,
        env: None,
        from: Some("ghost-sandbox".into()),
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found"),
        "expected sandbox not found, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Suspend/resume registry fixes (issues #4-8)
// -----------------------------------------------------------------------

#[tokio::test]
async fn handle_list_shows_suspended_status() {
    let (state, _dir) = make_test_state_with_tempdir();

    // Register a suspended persistent VM
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "susp-vm".into(),
            PersistentVmEntry {
                name: "susp-vm".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/susp-vm"),
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    // Register a stopped (not suspended) persistent VM
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "stop-vm".into(),
            PersistentVmEntry {
                name: "stop-vm".into(),
                ram_mb: 1024,
                cpus: 1,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/stop-vm"),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let Json(list) = handle_list(State(state)).await;

    let susp = list.sandboxes.iter().find(|s| s.id == "susp-vm").unwrap();
    assert_eq!(
        susp.status, "Suspended",
        "suspended VM should show Suspended status"
    );

    let stop = list.sandboxes.iter().find(|s| s.id == "stop-vm").unwrap();
    assert_eq!(
        stop.status, "Stopped",
        "non-suspended VM should show Stopped status"
    );
}

#[tokio::test]
async fn handle_info_shows_suspended_status() {
    let (state, _dir) = make_test_state_with_tempdir();

    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "info-susp".into(),
            PersistentVmEntry {
                name: "info-susp".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/info-susp"),
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    let result = handle_info(State(state), Path("info-susp".into())).await;
    let Json(info) = result.unwrap();
    assert_eq!(info.status, "Suspended");
}

#[tokio::test]
async fn handle_vm_edit_rejects_profile_id_mutation() {
    let state = make_test_state();
    insert_fake_instance(&state, "edit-vm", 4242);
    let request: api::VmEditRequest = serde_json::from_value(serde_json::json!({
        "profile_id": "other-profile"
    }))
    .unwrap();

    let err = handle_vm_edit(State(state), Path("edit-vm".into()), Json(request))
        .await
        .unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("profile_id is immutable"));
}

#[tokio::test]
async fn handle_vm_edit_rejects_unknown_fields() {
    let state = make_test_state();
    insert_fake_instance(&state, "edit-vm", 4242);
    let request: api::VmEditRequest = serde_json::from_value(serde_json::json!({
        "surprise": true
    }))
    .unwrap();

    let err = handle_vm_edit(State(state), Path("edit-vm".into()), Json(request))
        .await
        .unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("unknown VM edit fields"));
}

#[tokio::test]
async fn handle_vm_edit_resource_changes_fail_explicitly() {
    let state = make_test_state();
    insert_fake_instance(&state, "edit-vm", 4242);
    let request: api::VmEditRequest = serde_json::from_value(serde_json::json!({
        "ram_mb": 8192
    }))
    .unwrap();

    let err = handle_vm_edit(State(state), Path("edit-vm".into()), Json(request))
        .await
        .unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_IMPLEMENTED);
    assert!(err.1.contains("not supported yet"));
}

#[tokio::test]
async fn handle_vm_operation_status_reports_idle_for_existing_vm() {
    let state = make_test_state();
    insert_fake_instance(&state, "ops-vm", 5150);

    let Json(save) = handle_vm_save_status(State(Arc::clone(&state)), Path("ops-vm".into()))
        .await
        .unwrap();
    assert_eq!(save.vm_id, "ops-vm");
    assert_eq!(save.operation, "save");
    assert_eq!(save.status, "idle");
    assert!(!save.in_progress);

    let Json(fork) = handle_vm_fork_status(State(state), Path("ops-vm".into()))
        .await
        .unwrap();
    assert_eq!(fork.operation, "fork");
    assert_eq!(fork.status, "idle");
    assert!(!fork.in_progress);
}

#[tokio::test]
async fn handle_vm_operation_status_rejects_unknown_vm() {
    let state = make_test_state();

    let err = handle_vm_save_status(State(state), Path("missing-vm".into()))
        .await
        .unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handle_unsupported_vm_operations_fail_explicitly() {
    let state = make_test_state();
    insert_fake_instance(&state, "ops-vm", 5150);

    let restart = handle_vm_restart(State(Arc::clone(&state)), Path("ops-vm".into()))
        .await
        .unwrap_err();
    assert_eq!(restart.0, StatusCode::NOT_IMPLEMENTED);
    assert!(restart.1.contains("restart is not supported yet"));

    let reload = handle_vm_reload_profile(State(state), Path("ops-vm".into()))
        .await
        .unwrap_err();
    assert_eq!(reload.0, StatusCode::NOT_IMPLEMENTED);
    assert!(reload.1.contains("reload-profile is not supported yet"));
}

#[tokio::test]
async fn handle_suspend_rejects_ephemeral_vm() {
    let (state, _dir) = make_test_state_with_tempdir();

    // Insert an ephemeral VM in instances
    {
        let mut instances = state.instances.lock().unwrap();
        instances.insert(
            "eph-vm".into(),
            InstanceInfo {
                id: "eph-vm".into(),
                pid: 0,
                uds_path: state.run_dir.join("instances/eph-vm.sock"),
                session_dir: state.run_dir.join("sessions/eph-vm"),
                ram_mb: 2048,
                cpus: 2,
                start_time: std::time::Instant::now(),
                base_version: "0.0.0".into(),
                persistent: false,
                env: None,
                forked_from: None,
            },
        );
    }

    let result = handle_suspend(State(state), Path("eph-vm".into())).await;
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("ephemeral"));
}

#[tokio::test]
async fn handle_suspend_returns_not_found_for_missing_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = handle_suspend(State(state), Path("nonexistent".into())).await;
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[test]
fn archive_failed_restore_checkpoint_moves_checkpoint_aside() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/resume-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let checkpoint = session_dir.join("checkpoint.vzsave");
    std::fs::write(&checkpoint, b"bad checkpoint").unwrap();

    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "resume-vm".into(),
            PersistentVmEntry {
                name: "resume-vm".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: session_dir.clone(),
                forked_from: None,
                description: None,
                suspended: true,
                defunct: false,
                last_error: None,
                checkpoint_path: Some("checkpoint.vzsave".into()),
                env: None,
            },
        );
    }

    let archived = state
        .archive_failed_restore_checkpoint("resume-vm")
        .expect("checkpoint should be archived");

    assert!(!checkpoint.exists(), "original checkpoint must be moved");
    assert!(
        archived.exists(),
        "archived checkpoint should exist: {}",
        archived.display()
    );
    assert!(archived
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("checkpoint.vzsave.failed-restore-"));
}

// -----------------------------------------------------------------------
// main_db_path
// -----------------------------------------------------------------------

#[test]
fn main_db_path_resolves_to_sessions_dir() {
    let state = make_test_state();
    // run_dir = /tmp/capsem-test-svc => parent = /tmp => main.db = /tmp/sessions/main.db
    let path = state.main_db_path();
    assert!(
        path.ends_with("sessions/main.db"),
        "got: {}",
        path.display()
    );
}

// -----------------------------------------------------------------------
// SandboxInfo::new
// -----------------------------------------------------------------------

#[test]
fn sandbox_info_new_defaults_telemetry_to_none() {
    let info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
    assert_eq!(info.id, "test");
    assert_eq!(info.pid, 1);
    assert!(!info.persistent);
    assert!(info.total_input_tokens.is_none());
    assert!(info.total_estimated_cost.is_none());
    assert!(info.model_call_count.is_none());
    assert!(info.created_at.is_none());
    assert!(info.uptime_secs.is_none());
}

#[test]
fn sandbox_info_telemetry_fields_serialize_when_present() {
    let mut info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
    info.total_input_tokens = Some(1000);
    info.total_estimated_cost = Some(0.42);
    info.model_call_count = Some(5);
    let json = serde_json::to_string(&info).unwrap();
    assert!(json.contains("\"total_input_tokens\":1000"));
    assert!(json.contains("\"total_estimated_cost\":0.42"));
    assert!(json.contains("\"model_call_count\":5"));
}

#[test]
fn sandbox_info_telemetry_fields_omitted_when_none() {
    let info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
    let json = serde_json::to_string(&info).unwrap();
    assert!(!json.contains("total_input_tokens"));
    assert!(!json.contains("total_estimated_cost"));
    assert!(!json.contains("model_call_count"));
    assert!(!json.contains("uptime_secs"));
}

#[test]
fn sandbox_info_backwards_compatible_deserialization() {
    // Old JSON without telemetry fields should still deserialize
    let json = r#"{"id":"x","pid":1,"status":"Running","persistent":false}"#;
    let info: SandboxInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.id, "x");
    assert!(info.total_input_tokens.is_none());
}

// -----------------------------------------------------------------------
// StatsResponse
// -----------------------------------------------------------------------

#[test]
fn stats_response_serializes() {
    let resp = StatsResponse {
        global: capsem_core::session::GlobalStats {
            total_sessions: 10,
            total_input_tokens: 5000,
            total_output_tokens: 2000,
            total_estimated_cost: 1.50,
            total_tool_calls: 100,
            total_mcp_calls: 20,
            total_file_events: 300,
            total_requests: 400,
            total_allowed: 380,
            total_denied: 20,
        },
        sessions: vec![],
        top_providers: vec![],
        top_tools: vec![],
        top_mcp_tools: vec![],
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"total_sessions\":10"));
    assert!(json.contains("\"total_estimated_cost\":1.5"));
    assert!(json.contains("\"top_providers\":[]"));
}

// -----------------------------------------------------------------------
// handle_list includes uptime_secs for running VMs
// -----------------------------------------------------------------------

#[tokio::test]
async fn handle_list_includes_uptime_for_running_vms() {
    let state = make_test_state();
    insert_fake_instance(&state, "vm-1", 100);
    let resp = handle_list(State(state)).await;
    let list = resp.0;
    assert_eq!(list.sandboxes.len(), 1);
    assert!(list.sandboxes[0].uptime_secs.is_some());
}

// -----------------------------------------------------------------------
// handle_stats with tempdir
// -----------------------------------------------------------------------

#[tokio::test]
async fn handle_stats_returns_global_data() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();

    // Create main.db with a test session
    let idx = capsem_core::session::SessionIndex::open(&sessions_dir.join("main.db")).unwrap();
    let record = capsem_core::session::SessionRecord {
        id: "20260412-120000-abcd".into(),
        mode: "virtiofs".into(),
        command: Some("echo hello".into()),
        status: "stopped".into(),
        created_at: "2026-04-12T12:00:00Z".into(),
        stopped_at: Some("2026-04-12T12:05:00Z".into()),
        scratch_disk_size_gb: 16,
        ram_bytes: 4294967296,
        total_requests: 50,
        allowed_requests: 45,
        denied_requests: 5,
        total_input_tokens: 10000,
        total_output_tokens: 3000,
        total_estimated_cost: 0.42,
        total_tool_calls: 25,
        total_mcp_calls: 5,
        total_file_events: 100,
        compressed_size_bytes: None,
        vacuumed_at: None,
        storage_mode: "virtiofs".into(),
        rootfs_hash: None,
        rootfs_version: None,
        forked_from: None,
        persistent: false,
        exec_count: 0,
        audit_event_count: 0,
    };
    idx.create_session(&record).unwrap();
    drop(idx);

    let (state, _dir) = make_test_state_with_tempdir_at(dir);
    let result = handle_stats(State(state)).await;
    assert!(result.is_ok());
    let resp = result.unwrap().0;
    assert_eq!(resp.global.total_sessions, 1);
    assert_eq!(resp.global.total_input_tokens, 10000);
    assert_eq!(resp.global.total_estimated_cost, 0.42);
    assert_eq!(resp.sessions.len(), 1);
    assert_eq!(resp.sessions[0].id, "20260412-120000-abcd");
}

// -----------------------------------------------------------------------
// Settings handler tests
// -----------------------------------------------------------------------

struct SettingsEnvGuard {
    previous_user: Option<std::ffi::OsString>,
    previous_corp: Option<std::ffi::OsString>,
}

impl Drop for SettingsEnvGuard {
    fn drop(&mut self) {
        if let Some(previous_user) = self.previous_user.take() {
            std::env::set_var("CAPSEM_USER_CONFIG", previous_user);
        } else {
            std::env::remove_var("CAPSEM_USER_CONFIG");
        }

        if let Some(previous_corp) = self.previous_corp.take() {
            std::env::set_var("CAPSEM_CORP_CONFIG", previous_corp);
        } else {
            std::env::remove_var("CAPSEM_CORP_CONFIG");
        }
    }
}

fn install_empty_settings_env(dir: &tempfile::TempDir) -> (SettingsEnvGuard, PathBuf, PathBuf) {
    let user_path = dir.path().join("user.toml");
    let corp_path = dir.path().join("corp.toml");
    capsem_core::net::policy_config::write_settings_file(
        &user_path,
        &capsem_core::net::policy_config::SettingsFile::default(),
    )
    .unwrap();
    capsem_core::net::policy_config::write_settings_file(
        &corp_path,
        &capsem_core::net::policy_config::SettingsFile::default(),
    )
    .unwrap();

    let guard = SettingsEnvGuard {
        previous_user: std::env::var_os("CAPSEM_USER_CONFIG"),
        previous_corp: std::env::var_os("CAPSEM_CORP_CONFIG"),
    };
    std::env::set_var("CAPSEM_USER_CONFIG", &user_path);
    std::env::set_var("CAPSEM_CORP_CONFIG", &corp_path);
    (guard, user_path, corp_path)
}

#[tokio::test]
async fn handle_get_settings_returns_tree() {
    let Json(val) = handle_get_settings().await;
    assert!(val.get("tree").is_some(), "response must have 'tree'");
    assert!(val.get("issues").is_some(), "response must have 'issues'");
    assert!(
        val.get("presets").is_none(),
        "settings must not expose presets"
    );
    assert!(
        val.get("policy").is_none(),
        "retired policy compatibility payload must not be emitted"
    );
    assert!(
        val.get("providers").is_some(),
        "response must have provider status"
    );
    assert!(val["tree"].is_array());
    assert!(val["issues"].is_array());
    assert!(val["providers"].is_array());
}

#[tokio::test]
async fn handle_save_settings_rejects_unknown_key() {
    let mut changes = HashMap::new();
    changes.insert("nonexistent.setting.xyz".into(), serde_json::json!("value"));
    let result = handle_save_settings(Json(changes)).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn handle_save_settings_rejects_retired_policy_rule_keys_atomically() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);

    let mut changes = HashMap::new();
    changes.insert(
        "policy.http.block_openai_github".into(),
        serde_json::json!({
            "on": "http.request",
            "if": "http.host == 'github.com'",
            "decision": "block",
            "priority": 10
        }),
    );

    let err = handle_save_settings(Json(changes))
        .await
        .expect_err("retired policy rule key should be rejected by settings handler");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1
            .contains("unknown setting: policy.http.block_openai_github"),
        "error should point to the retired policy key, got: {}",
        err.1
    );
    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
    assert!(
        loaded.settings.is_empty(),
        "rejected retired policy update must not mutate user config"
    );
}

fn make_test_state_with_tempdir_at(
    dir: tempfile::TempDir,
) -> (Arc<ServiceState>, tempfile::TempDir) {
    let run_dir = dir.path().join("run");
    let registry_path = run_dir.join("persistent_registry.json");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: run_dir.join("assets"),
        run_dir,
        job_counter: AtomicU64::new(1),
        manifest: None,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    });
    (state, dir)
}

// -----------------------------------------------------------------------
// resolve_workspace_path
// -----------------------------------------------------------------------

#[test]
fn resolve_rejects_unknown_vm() {
    let state = make_test_state();
    let r = resolve_workspace_path(&state, "nonexistent", "src/main.rs");
    assert!(r.is_err());
}

#[test]
fn resolve_rejects_symlink_escape() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    // Create a symlink that points outside workspace
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(&outside, workspace.join("escape")).unwrap();

    let (state, _dir2) = make_test_state_with_tempdir();
    state.instances.lock().unwrap().insert(
        "test-vm".into(),
        InstanceInfo {
            id: "test-vm".into(),
            pid: 1,
            uds_path: PathBuf::from("/tmp/test.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );

    let r = resolve_workspace_path(&state, "test-vm", "escape/secret.txt");
    assert!(r.is_err());
}

#[test]
fn resolve_valid_path_inside_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("hello.txt"), "world").unwrap();

    let (state, _dir2) = make_test_state_with_tempdir();
    state.instances.lock().unwrap().insert(
        "test-vm".into(),
        InstanceInfo {
            id: "test-vm".into(),
            pid: 1,
            uds_path: PathBuf::from("/tmp/test.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );

    let r = resolve_workspace_path(&state, "test-vm", "hello.txt");
    assert!(r.is_ok());
    let (ws_root, resolved) = r.unwrap();
    assert!(resolved.starts_with(ws_root.canonicalize().unwrap()));
}

// -----------------------------------------------------------------------
// list_dir_recursive
// -----------------------------------------------------------------------

#[test]
fn list_dir_returns_correct_structure() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::write(ws.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(ws.join("README.md"), "# Hello").unwrap();

    let magika = test_magika();
    let entries = list_dir_recursive(ws, "", 1, 2, &magika);

    // Should have src/ dir and README.md file
    assert!(entries.len() >= 2);
    let dir_entry = entries.iter().find(|e| e.name == "src").unwrap();
    assert_eq!(dir_entry.entry_type, "directory");
    assert!(dir_entry.children.is_some());
    let children = dir_entry.children.as_ref().unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "main.rs");
    assert_eq!(children[0].entry_type, "file");

    let file_entry = entries.iter().find(|e| e.name == "README.md").unwrap();
    assert_eq!(file_entry.entry_type, "file");
    assert!(file_entry.size > 0);
}

#[test]
fn list_dir_respects_depth_limit() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join("a/b/c")).unwrap();
    std::fs::write(ws.join("a/b/c/deep.txt"), "deep").unwrap();

    let magika = test_magika();
    // depth 1: should list "a" but not recurse into "a/b"
    let entries = list_dir_recursive(ws, "", 1, 1, &magika);
    let a = entries.iter().find(|e| e.name == "a").unwrap();
    assert!(a.children.is_none());
}

#[test]
fn list_dir_skips_system_but_shows_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join(".hidden")).unwrap();
    std::fs::create_dir_all(ws.join("system")).unwrap();
    std::fs::write(ws.join("visible.txt"), "yes").unwrap();

    let magika = test_magika();
    let entries = list_dir_recursive(ws, "", 1, 1, &magika);
    // .hidden + visible.txt shown; system/ filtered out
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|e| e.name == ".hidden"));
    assert!(entries.iter().any(|e| e.name == "visible.txt"));
    assert!(!entries.iter().any(|e| e.name == "system"));
}

#[test]
fn list_dir_sorts_dirs_first_then_alphabetical() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("zebra.txt"), "z").unwrap();
    std::fs::create_dir_all(ws.join("alpha")).unwrap();
    std::fs::write(ws.join("apple.txt"), "a").unwrap();
    std::fs::create_dir_all(ws.join("beta")).unwrap();

    let magika = test_magika();
    let entries = list_dir_recursive(ws, "", 1, 1, &magika);
    // Dirs first (alpha, beta), then files (apple.txt, zebra.txt)
    assert_eq!(entries[0].name, "alpha");
    assert_eq!(entries[1].name, "beta");
    assert_eq!(entries[2].name, "apple.txt");
    assert_eq!(entries[3].name, "zebra.txt");
}

// -----------------------------------------------------------------------
// Download / Upload via resolve_workspace_path
// -----------------------------------------------------------------------

fn setup_vm_with_workspace(state: &ServiceState, dir: &std::path::Path, vm_id: &str) {
    setup_vm_with_workspace_and_uds(state, dir, vm_id, PathBuf::from("/tmp/test.sock"));
}

fn setup_vm_with_workspace_and_uds(
    state: &ServiceState,
    dir: &std::path::Path,
    vm_id: &str,
    uds_path: PathBuf,
) {
    let session_dir = dir.join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    state.instances.lock().unwrap().insert(
        vm_id.into(),
        InstanceInfo {
            id: vm_id.into(),
            pid: 1,
            uds_path,
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );
}

async fn spawn_file_boundary_ipc(
    expected_messages: usize,
) -> (
    tempfile::TempDir,
    PathBuf,
    tokio::task::JoinHandle<Vec<ServiceToProcess>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let uds_path = dir.path().join("process.sock");
    let listener = tokio::net::UnixListener::bind(&uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    let handle = tokio::spawn(async move {
        let mut messages = Vec::new();
        for _ in 0..expected_messages {
            let (stream, _) = listener.accept().await.unwrap();
            let std_stream = stream.into_std().unwrap();
            let std_stream = tokio::task::spawn_blocking(move || {
                let mut std_stream = std_stream;
                capsem_core::ipc_handshake::negotiate_responder(
                    &mut std_stream,
                    "capsem-process-test",
                    "",
                )?;
                Ok::<_, capsem_proto::handshake::HandshakeError>(std_stream)
            })
            .await
            .unwrap()
            .unwrap();
            let (tx, rx): (
                tokio_unix_ipc::Sender<ProcessToService>,
                tokio_unix_ipc::Receiver<ServiceToProcess>,
            ) = tokio_unix_ipc::channel_from_std(std_stream).unwrap();
            let msg = rx.recv().await.unwrap();
            match &msg {
                ServiceToProcess::LogFileBoundary { id, .. } => {
                    tx.send(ProcessToService::LogFileBoundaryResult {
                        id: *id,
                        success: true,
                        error: None,
                    })
                    .await
                    .unwrap();
                }
                ServiceToProcess::WriteFile { id, .. } => {
                    tx.send(ProcessToService::WriteFileResult {
                        id: *id,
                        success: true,
                        error: None,
                    })
                    .await
                    .unwrap();
                }
                ServiceToProcess::ReadFile { id, .. } => {
                    tx.send(ProcessToService::ReadFileResult {
                        id: *id,
                        data: Some(b"guest export".to_vec()),
                        error: None,
                    })
                    .await
                    .unwrap();
                }
                other => panic!("unexpected IPC message in file boundary test: {other:?}"),
            }
            messages.push(msg);
        }
        messages
    });
    (dir, uds_path, handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_logs_file_import_before_writing_workspace_file() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "up-ledger-vm", uds_path);

    let result = handle_upload_file(
        State(state),
        Path("up-ledger-vm".to_string()),
        Query(FileContentQuery {
            path: "new.txt".to_string(),
        }),
        axum::body::Bytes::from_static(b"uploaded through ledger"),
    )
    .await
    .expect("upload should succeed after boundary log");

    assert_eq!(result.size, b"uploaded through ledger".len() as u64);
    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Import);
            assert_eq!(path, "new.txt");
            assert_eq!(data, b"uploaded through ledger");
            assert_eq!(*size, b"uploaded through ledger".len() as u64);
        }
        other => panic!("upload must log file import before write, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("session/guest/workspace/new.txt")).unwrap(),
        "uploaded through ledger"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_logs_file_export_before_returning_response() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "dl-ledger-vm", uds_path);
    let workspace_file = dir.path().join("session/guest/workspace/report.txt");
    std::fs::write(&workspace_file, b"export through ledger").unwrap();

    let response = handle_download_file(
        State(state),
        Path("dl-ledger-vm".to_string()),
        Query(FileContentQuery {
            path: "report.txt".to_string(),
        }),
    )
    .await
    .expect("download should succeed after boundary log");

    assert_eq!(response.status(), StatusCode::OK);
    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Export);
            assert_eq!(path, "report.txt");
            assert_eq!(data, b"export through ledger");
            assert_eq!(*size, b"export through ledger".len() as u64);
        }
        other => panic!("download must log file export before response, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_does_not_write_workspace_file_when_import_ledger_fails() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let ipc_dir = tempfile::tempdir().unwrap();
    let uds_path = ipc_dir.path().join("process.sock");
    let listener = tokio::net::UnixListener::bind(&uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    let ipc = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let std_stream = stream.into_std().unwrap();
        let std_stream = tokio::task::spawn_blocking(move || {
            let mut std_stream = std_stream;
            capsem_core::ipc_handshake::negotiate_responder(
                &mut std_stream,
                "capsem-process-test",
                "",
            )?;
            Ok::<_, capsem_proto::handshake::HandshakeError>(std_stream)
        })
        .await
        .unwrap()
        .unwrap();
        let (tx, rx): (
            tokio_unix_ipc::Sender<ProcessToService>,
            tokio_unix_ipc::Receiver<ServiceToProcess>,
        ) = tokio_unix_ipc::channel_from_std(std_stream).unwrap();
        let msg = rx.recv().await.unwrap();
        match &msg {
            ServiceToProcess::LogFileBoundary { id, .. } => {
                tx.send(ProcessToService::LogFileBoundaryResult {
                    id: *id,
                    success: false,
                    error: Some("security ledger rejected import".to_string()),
                })
                .await
                .unwrap();
            }
            other => panic!("unexpected IPC message in import denial test: {other:?}"),
        }
        msg
    });
    setup_vm_with_workspace_and_uds(&state, dir.path(), "deny-ledger-vm", uds_path);

    let err = handle_upload_file(
        State(state),
        Path("deny-ledger-vm".to_string()),
        Query(FileContentQuery {
            path: "blocked.txt".to_string(),
        }),
        axum::body::Bytes::from_static(b"must not land"),
    )
    .await
    .expect_err("failed import ledger write must fail closed");

    assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(err.1.contains("security ledger rejected import"));
    let msg = ipc.await.unwrap();
    assert!(matches!(msg, ServiceToProcess::LogFileBoundary { .. }));
    assert!(
        !dir.path()
            .join("session/guest/workspace/blocked.txt")
            .exists(),
        "upload must not write bytes when import ledger fails"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_file_logs_import_before_guest_write() {
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(2).await;
    state.instances.lock().unwrap().insert(
        "write-ledger-vm".into(),
        InstanceInfo {
            id: "write-ledger-vm".into(),
            pid: 1,
            uds_path,
            session_dir: state.run_dir.join("sessions/write-ledger-vm"),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );

    let _ = handle_write_file(
        State(state),
        Path("write-ledger-vm".to_string()),
        Json(WriteFileRequest {
            path: "/workspace/from-api.txt".to_string(),
            content: "guest write".to_string(),
        }),
    )
    .await
    .expect("write_file should succeed after import ledger");

    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 2);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Import);
            assert_eq!(path, "/workspace/from-api.txt");
            assert_eq!(data, b"guest write");
            assert_eq!(*size, b"guest write".len() as u64);
        }
        other => panic!("write_file first IPC must be import ledger, got {other:?}"),
    }
    assert!(matches!(
        messages[1],
        ServiceToProcess::WriteFile { ref path, .. } if path == "/workspace/from-api.txt"
    ));
}

#[test]
fn download_reads_correct_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "dl-vm");

    let ws = dir.path().join("session/guest/workspace");
    let content = b"hello world\nline 2\n";
    std::fs::write(ws.join("test.txt"), content).unwrap();

    let (_, resolved) = resolve_workspace_path(&state, "dl-vm", "test.txt").unwrap();
    let data = std::fs::read(&resolved).unwrap();
    assert_eq!(data, content);
}

#[test]
fn download_binary_preserves_content() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "bin-vm");

    let ws = dir.path().join("session/guest/workspace");
    let binary: Vec<u8> = (0..256).map(|i| i as u8).collect();
    std::fs::write(ws.join("data.bin"), &binary).unwrap();

    let (_, resolved) = resolve_workspace_path(&state, "bin-vm", "data.bin").unwrap();
    let data = std::fs::read(&resolved).unwrap();
    assert_eq!(data, binary);
}

#[test]
fn upload_creates_file_with_content() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "up-vm");

    let ws = dir.path().join("session/guest/workspace");
    let (_, target) = resolve_workspace_path(&state, "up-vm", "new.txt").unwrap();
    std::fs::write(&target, b"uploaded").unwrap();

    assert_eq!(
        std::fs::read_to_string(ws.join("new.txt")).unwrap(),
        "uploaded"
    );
}

#[test]
fn upload_creates_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "mkdir-vm");

    let ws = dir.path().join("session/guest/workspace");
    // resolve_workspace_path should succeed even for non-existing nested paths
    let (_, target) = resolve_workspace_path(&state, "mkdir-vm", "deep/nested/file.txt").unwrap();
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"deep content").unwrap();

    assert_eq!(
        std::fs::read_to_string(ws.join("deep/nested/file.txt")).unwrap(),
        "deep content"
    );
}

#[test]
fn upload_path_traversal_blocked() {
    let r = sanitize_file_path("../../etc/passwd");
    assert!(r.is_err());
}

#[test]
fn download_nonexistent_file_resolve_ok_but_not_exists() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "404-vm");

    // Resolving a non-existent file path still works (for upload target)
    let result = resolve_workspace_path(&state, "404-vm", "nonexistent.txt");
    assert!(result.is_ok());
    let (_, resolved) = result.unwrap();
    assert!(!resolved.exists());
}

// is_launchd_cleanup_transient identifies the misleading "missing
// entitlement" NSError that VZ emits when launchd's PETRIFIED-cleanup
// queue is saturated under rapid VM churn. The error string is
// stable across VZ releases (Apple's localizedDescription); pattern-
// match conservatively so a real codesign regression doesn't get
// silently retried.
#[test]
fn launchd_transient_matches_actual_vz_entitlement_error() {
    let tail = "Error: failed to boot VM\n\nCaused by:\n    \
        VM config validation failed: NSError { code: 2, \
        localizedDescription: \"Invalid virtual machine configuration. \
        The process doesn't have the \u{201c}com.apple.security.\
        virtualization\u{201d} entitlement.\", domain: \"VZErrorDomain\", \
        userInfo: {} }";
    assert!(is_launchd_cleanup_transient(tail));
}

#[test]
fn launchd_transient_matches_straight_quote_variant() {
    // Same content with ASCII quotes around the entitlement key.
    let tail = "VM config validation failed: NSError { code: 2, \
        localizedDescription: \"...The process doesn't have the \
        \\\"com.apple.security.virtualization\\\" entitlement.\" }";
    assert!(is_launchd_cleanup_transient(tail));
}

#[test]
fn launchd_transient_rejects_other_failures() {
    let unrelated = "Error: failed to build VmConfig\n\nCaused by:\n    \
        hash mismatch for ...img: expected abc, got def";
    assert!(!is_launchd_cleanup_transient(unrelated));

    let no_log = "(no preserved log found)";
    assert!(!is_launchd_cleanup_transient(no_log));

    let empty = "";
    assert!(!is_launchd_cleanup_transient(empty));
}

#[test]
fn launchd_transient_rejects_partial_match() {
    // The word "entitlement" alone in some unrelated error must not match;
    // the matcher requires the full VZ-specific phrase.
    let mention_only = "warn: this command may need an entitlement";
    assert!(!is_launchd_cleanup_transient(mention_only));
}

// classify_attempt_decision is the pure routing function the
// poll_until-based retry loop in handle_provision delegates to.
// Testing it directly lets us prove the retry path engages on the
// LaunchdTransient outcome (the actual fix for Bug A) without
// spawning a real VM. If a future refactor breaks the routing
// (e.g., maps LaunchdTransient to BailWithError), these fail.

#[test]
fn classify_ready_outcome_succeeds() {
    let uds = PathBuf::from("/tmp/x.sock");
    match classify_attempt_decision(
        ProvisionAttemptOutcome::Ready {
            uds_path: uds.clone(),
        },
        "vm-1",
    ) {
        AttemptDecision::Succeed(p) => assert_eq!(p, uds),
        other => panic!("expected Succeed, got {other:?}"),
    }
}

#[test]
fn classify_still_booting_timeout_succeeds_with_uds() {
    let uds = PathBuf::from("/tmp/y.sock");
    match classify_attempt_decision(
        ProvisionAttemptOutcome::StillBootingTimedOut {
            uds_path: uds.clone(),
        },
        "vm-2",
    ) {
        AttemptDecision::Succeed(p) => assert_eq!(p, uds),
        other => panic!("expected Succeed for still-booting envelope, got {other:?}"),
    }
}

#[test]
fn classify_launchd_transient_routes_to_retry() {
    // The core of the Bug A fix: LaunchdTransient must trigger a retry,
    // not bail with the misleading entitlement error.
    match classify_attempt_decision(ProvisionAttemptOutcome::LaunchdTransient, "vm-3") {
        AttemptDecision::RetryAfterCleanup => {}
        other => panic!("expected RetryAfterCleanup for LaunchdTransient, got {other:?}"),
    }
}

#[test]
fn classify_boot_crash_bails_with_500_and_tail() {
    let tail = "Error: failed to boot VM\n\nCaused by:\n    bogus".to_string();
    match classify_attempt_decision(
        ProvisionAttemptOutcome::BootCrash { tail: tail.clone() },
        "vm-4",
    ) {
        AttemptDecision::BailWithError(AppError(status, msg)) => {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert!(msg.contains("vm-4"), "msg should embed the id: {msg}");
            assert!(msg.contains(&tail), "msg should embed the log tail: {msg}");
            assert!(
                msg.contains("capsem logs vm-4"),
                "msg should hint at follow-up cmd"
            );
        }
        other => panic!("expected BailWithError(500), got {other:?}"),
    }
}

#[test]
fn classify_provision_error_already_exists_returns_409() {
    let err = anyhow::anyhow!("persistent VM \"vm-5\" already exists. Use `capsem resume vm-5`.");
    match classify_attempt_decision(ProvisionAttemptOutcome::ProvisionError(err), "vm-5") {
        AttemptDecision::BailWithError(AppError(status, _)) => {
            assert_eq!(status, StatusCode::CONFLICT,
                "duplicate-name errors must return 409 so clients can distinguish from server failures");
        }
        other => panic!("expected BailWithError(409) for already-exists, got {other:?}"),
    }
}

#[test]
fn classify_provision_error_other_returns_500() {
    let err = anyhow::anyhow!("rootfs not found at /missing/path");
    match classify_attempt_decision(ProvisionAttemptOutcome::ProvisionError(err), "vm-6") {
        AttemptDecision::BailWithError(AppError(status, msg)) => {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert!(
                msg.contains("rootfs not found"),
                "underlying error preserved: {msg}"
            );
        }
        other => panic!("expected BailWithError(500), got {other:?}"),
    }
}

// wait_for_vm_ready polls a cheap local sentinel file. Typical VM boot
// ready-time is sub-second, so the backoff must not overshoot readiness
// by hundreds of ms -- that shows up directly in provision->exec latency.
#[tokio::test]
async fn wait_for_vm_ready_detects_ready_within_tight_overshoot() {
    let dir = tempfile::tempdir().unwrap();
    let uds_path = dir.path().join("vm.sock");
    let ready_path = uds_path.with_extension("ready");

    // Simulate a VM that becomes ready ~200ms after provision. Real VM
    // boots land in the 400-700ms range, so 200ms is a conservative stand-in.
    let ready_clone = ready_path.clone();
    let creator = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        std::fs::write(&ready_clone, b"").unwrap();
    });

    let start = std::time::Instant::now();
    wait_for_vm_ready(&uds_path, 30, None, None)
        .await
        .expect("ready should be detected");
    let elapsed_ms = start.elapsed().as_millis();
    creator.await.unwrap();

    // Overshoot budget: a tight poll curve should catch the sentinel
    // within ~100ms of it appearing. A 500ms max_delay would miss the
    // 200ms creation and catch it at ~350ms instead.
    assert!(
        elapsed_ms < 300,
        "wait_for_vm_ready overshot: {elapsed_ms}ms (ready created at ~200ms, budget 300ms)"
    );
}

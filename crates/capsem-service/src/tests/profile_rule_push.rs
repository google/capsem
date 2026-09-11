//! Profile rule edits reach running VMs before the mutation route returns.
//!
//! Plugin edits already re-materialize each session's active profile and send
//! `ReloadConfig`; a rule edit that only rewrote the profile file left every
//! running VM enforcing the old rules until someone called the reload route.
use super::*;

fn rule(name: &str, condition: &str) -> capsem_core::net::policy_config::SecurityRule {
    capsem_core::net::policy_config::SecurityRule {
        name: name.to_string(),
        action: capsem_core::net::policy_config::SecurityRuleAction::Block,
        condition: condition.to_string(),
        enabled: true,
        detection_level: Some(capsem_core::net::policy_config::DetectionLevel::High),
        priority: Some(capsem_core::net::policy_config::SecurityRulePriority::Explicit(10)),
        corp_locked: false,
        reason: Some("rule push proof".to_string()),
        managed: None,
        plugin_config: BTreeMap::new(),
    }
}

#[tokio::test]
async fn rule_mutation_routes_push_reload_to_running_profile_instances() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let state = make_asset_state(dir.path().join("assets"));
    let session_dir = dir.path().join("sessions").join("rule-push-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "rule-push-vm", std::process::id(), session_dir.clone());
    let uds_path = state.instances.lock().unwrap()["rule-push-vm"].uds_path.clone();
    // One IPC connection per mutation route below.
    let process = spawn_fake_process_reload_ack(&uds_path, 4);
    let active_profile = session_dir.join("vm/active_profile.toml");

    let Json(saved) = handle_enforcement_rule_upsert(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "rule_push_block".to_string())),
        Json(rule("rule_push_block", r#"file.import.content.contains("EICAR")"#)),
    )
    .await
    .expect("enforcement upsert");
    assert_eq!(saved.compiled_rule_id, "profiles.rules.rule_push_block");
    assert!(
        std::fs::read_to_string(&active_profile)
            .unwrap()
            .contains("rule_push_block"),
        "upsert must materialize the rule into the running session's active profile"
    );
    let Json(saved) = handle_detection_rule_upsert(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "rule_push_detect".to_string())),
        Json(rule("rule_push_detect", r#"http.host.contains("example.invalid")"#)),
    )
    .await
    .expect("detection upsert");
    assert_eq!(saved.compiled_rule_id, "profiles.rules.rule_push_detect");
    let Json(deleted) = handle_enforcement_rule_delete(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "rule_push_block".to_string())),
    )
    .await
    .expect("enforcement delete");
    assert!(deleted.deleted);
    assert!(
        !std::fs::read_to_string(&active_profile)
            .unwrap()
            .contains("rule_push_block"),
        "delete must remove the rule from the running session's active profile"
    );
    let Json(deleted) = handle_detection_rule_delete(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "rule_push_detect".to_string())),
    )
    .await
    .expect("detection delete");
    assert!(deleted.deleted);

    let received = tokio::time::timeout(std::time::Duration::from_secs(10), process)
        .await
        .expect("every mutation route must contact the running instance")
        .unwrap();
    assert_eq!(received.len(), 4);
    assert!(
        received
            .iter()
            .all(|message| matches!(message, ServiceToProcess::ReloadConfig)),
        "each rule mutation sends exactly one ReloadConfig"
    );
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

    let uds_path = state.instances.lock().unwrap()["runtime-refresh"].uds_path.clone();
    let process = profile_rule_push::spawn_fake_process_reload_ack(&uds_path, 1);
    let Json(plugin_info) = handle_profile_plugin_update(
        State(Arc::clone(&state)),
        Path(("code".to_string(), "dummy_pre_eicar".to_string())),
        Json(PluginUpdate {
            mode: Some(capsem_core::net::policy_config::SecurityPluginMode::Block),
            detection_level: Some(capsem_core::net::policy_config::DetectionLevel::Critical),
        }),
    )
    .await
    .expect("plugin edit should update profile override");
    assert!(
        matches!(process.await.unwrap().as_slice(), [ServiceToProcess::ReloadConfig]),
        "the plugin edit must reach the running VM before the route returns"
    );
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

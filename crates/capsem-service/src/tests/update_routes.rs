use super::*;

#[tokio::test]
async fn update_route_check_dry_run_plans_cli_check() {
    let app = build_service_router(make_test_state());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/update/check",
        Some(json!({ "dry_run": true })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "planned");
    assert_eq!(body["command"]["args"], json!(["update", "--check"]));
}

#[tokio::test]
async fn update_route_check_rejects_ambiguous_request_body() {
    let app = build_service_router(make_test_state());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/update/check",
        Some(json!({
            "dry_run": true,
            "action": "assets",
        })),
    )
    .await;

    assert!(
        status.is_client_error(),
        "ambiguous update check body must be rejected, got {status}"
    );
    assert_ne!(status, StatusCode::OK);
    assert!(
        body.to_string().contains("unknown field") || body.to_string().contains("unknown variant"),
        "unexpected rejection body: {body}"
    );
}

#[tokio::test]
async fn update_route_check_live_executes_non_mutating_cli_check() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("capsem");
    let log = dir.path().join("args.log");
    std::fs::write(
        &cli,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\n", log.display()),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&cli).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&cli, permissions).unwrap();
    let previous = std::env::var_os("CAPSEM_CLI");
    std::env::set_var("CAPSEM_CLI", &cli);

    let assets_dir = dir.path().join("assets");
    write_update_runtime_manifest(&assets_dir, "0.0.0", "profiles-1");
    let app = build_service_router(make_asset_state(assets_dir));
    let (status, body) = route_request(app, axum::http::Method::POST, "/update/check", Some(json!({}))).await;
    match previous {
        Some(value) => std::env::set_var("CAPSEM_CLI", value),
        None => std::env::remove_var("CAPSEM_CLI"),
    }

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "succeeded");
    assert_eq!(body["command"]["args"], json!(["update", "--check"]));
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&log, usize::MAX).unwrap(),
        "update --check\n"
    );
}

#[tokio::test]
async fn update_route_apply_dry_run_plans_one_atomic_update() {
    let app = build_service_router(make_test_state());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/update/apply",
        Some(json!({ "dry_run": true })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "planned");
    assert_eq!(body["command"]["args"], json!(["update", "--yes"]));
}

#[tokio::test]
async fn update_route_apply_requires_confirmation_for_live_command() {
    let app = build_service_router(make_test_state());
    let (status, body) = route_request(app, axum::http::Method::POST, "/update/apply", Some(json!({}))).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "update apply requires confirmed=true or dry_run=true");
}

#[tokio::test]
async fn update_route_apply_rejects_obsolete_split_action_body() {
    let app = build_service_router(make_test_state());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/update/apply",
        Some(json!({
            "confirmed": true,
            "action": "assets",
        })),
    )
    .await;

    assert!(
        status.is_client_error(),
        "ambiguous update action body must be rejected, got {status}"
    );
    assert_ne!(status, StatusCode::OK);
    assert!(
        body.to_string().contains("unknown field") || body.to_string().contains("unknown variant"),
        "unexpected rejection body: {body}"
    );
}

#[tokio::test]
async fn update_route_apply_confirmed_dispatches_one_atomic_update() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("capsem");
    let log = dir.path().join("args.log");
    std::fs::write(
        &cli,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", log.display()),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&cli).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&cli, permissions).unwrap();
    let previous = std::env::var_os("CAPSEM_CLI");
    std::env::set_var("CAPSEM_CLI", &cli);

    let assets_dir = dir.path().join("assets");
    write_update_runtime_manifest(&assets_dir, "0.0.0", "profiles-1");
    let app = build_service_router(make_asset_state(assets_dir));
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/update/apply",
        Some(json!({ "confirmed": true })),
    )
    .await;
    match previous {
        Some(value) => std::env::set_var("CAPSEM_CLI", value),
        None => std::env::remove_var("CAPSEM_CLI"),
    }

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "succeeded");
    assert_eq!(body["command"]["args"], json!(["update", "--yes"]));
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&log, usize::MAX).unwrap(),
        "update --yes\n"
    );
}

#[tokio::test]
async fn update_route_live_commands_share_one_serial_lock() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("capsem");
    let log = dir.path().join("serial.log");
    let active = dir.path().join("active");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\n\
             if ! mkdir '{}'; then printf '%s\\n' overlap >> '{}'; exit 9; fi\n\
             printf '%s\\n' start >> '{}'\n\
             sleep 0.1\n\
             printf '%s\\n' end >> '{}'\n\
             rmdir '{}'\n",
            active.display(),
            log.display(),
            log.display(),
            log.display(),
            active.display(),
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&cli).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&cli, permissions).unwrap();
    let previous = std::env::var_os("CAPSEM_CLI");
    std::env::set_var("CAPSEM_CLI", &cli);

    let assets_dir = dir.path().join("assets");
    write_update_runtime_manifest(&assets_dir, "0.0.0", "profiles-1");
    let app = build_service_router(make_asset_state(assets_dir));
    let first = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/update/apply",
        Some(json!({ "confirmed": true })),
    );
    let second = route_request(app, axum::http::Method::POST, "/update/check", Some(json!({})));
    let (first, second) = tokio::join!(first, second);
    match previous {
        Some(value) => std::env::set_var("CAPSEM_CLI", value),
        None => std::env::remove_var("CAPSEM_CLI"),
    }

    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(first.1["status"], "succeeded");
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(second.1["status"], "succeeded");
    let execution = capsem_foundation::telemetry::read_log_tail(&log, usize::MAX).unwrap();
    assert!(!execution.contains("overlap"), "{execution}");
    assert_eq!(execution, "start\nend\nstart\nend\n");
}

fn write_update_runtime_manifest(assets_dir: &StdPath, binary: &str, assets: &str) {
    std::fs::create_dir_all(assets_dir).unwrap();
    let manifest = capsem_assets::asset_manager::ManifestV2 {
        format: 2,
        refresh_policy: "24h".to_string(),
        asset_base: None,
        assets: capsem_assets::asset_manager::AssetsSection {
            current: assets.to_string(),
            releases: HashMap::new(),
        },
        binaries: capsem_assets::asset_manager::BinariesSection {
            current: binary.to_string(),
            releases: HashMap::new(),
        },
    };
    std::fs::write(
        assets_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn update_runtime_reloads_profile_only_activation_without_restart() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    let state = make_asset_state(assets_dir.clone());
    write_update_runtime_manifest(&assets_dir, "0.0.0", "profiles-2");

    let disposition = reload_activated_update_runtime(&state).unwrap();

    assert_eq!(disposition, UpdateRuntimeDisposition::Reloaded);
    assert_eq!(
        state.manifest.read().unwrap().as_ref().unwrap().assets.current,
        "profiles-2"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), state.update_restart.notified())
            .await
            .is_err(),
        "profile-only activation must not restart a current binary"
    );
}

#[tokio::test]
async fn update_runtime_requests_restart_after_binary_activation() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    let state = make_asset_state(assets_dir.clone());
    write_update_runtime_manifest(&assets_dir, "9.9.9", "profiles-2");

    let disposition = reload_activated_update_runtime(&state).unwrap();

    assert_eq!(disposition, UpdateRuntimeDisposition::RestartRequested);
    // Already pending, so this asks whether it arrives, not how fast. The 10ms
    // sibling above stays: there the timeout firing *is* the assertion.
    tokio::time::timeout(std::time::Duration::from_secs(10), state.update_restart.notified())
        .await
        .expect("binary activation must request a managed service restart");
    assert_eq!(
        state.manifest.read().unwrap().as_ref().unwrap().binaries.current,
        "9.9.9"
    );
}

#[test]
fn update_runtime_rejects_invalid_manifest_without_replacing_cached_graph() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    write_update_runtime_manifest(&assets_dir, "0.0.0", "profiles-1");
    let state = make_asset_state(assets_dir.clone());
    std::fs::write(assets_dir.join("manifest.json"), b"{not-json").unwrap();

    let error = reload_activated_update_runtime(&state).unwrap_err();

    assert!(error.1.contains("validate activated update manifest"));
    assert_eq!(
        state.manifest.read().unwrap().as_ref().unwrap().assets.current,
        "profiles-1"
    );
}

fn resolved_automatic_update(
    value: capsem_core::net::policy_config::SettingValue,
) -> capsem_core::net::policy_config::ResolvedSetting {
    capsem_core::net::policy_config::ResolvedSetting {
        id: "app.auto_update".to_string(),
        category: "App".to_string(),
        name: "Automatic updates".to_string(),
        description: String::new(),
        setting_type: capsem_core::net::policy_config::SettingType::Bool,
        default_value: capsem_core::net::policy_config::SettingValue::Bool(true),
        effective_value: value,
        source: capsem_core::net::policy_config::PolicySource::User,
        modified: Some("test".to_string()),
        corp_locked: false,
        enabled_by: None,
        enabled: true,
        metadata: capsem_core::net::policy_config::SettingMetadata::default(),
        collapsed: false,
        history: Vec::new(),
    }
}

#[test]
fn automatic_update_setting_defaults_on_and_honors_false() {
    use capsem_core::net::policy_config::SettingValue;

    assert!(automatic_updates_enabled_from_resolved(&[]));
    assert!(automatic_updates_enabled_from_resolved(&[resolved_automatic_update(
        SettingValue::Bool(true)
    )]));
    assert!(!automatic_updates_enabled_from_resolved(&[resolved_automatic_update(
        SettingValue::Bool(false)
    )]));
    assert!(automatic_updates_enabled_from_resolved(&[resolved_automatic_update(
        SettingValue::Text("false".to_string())
    )]));
}

#[test]
fn automatic_update_failure_backoff_is_bounded() {
    assert_eq!(
        automatic_update_failure_backoff(1),
        std::time::Duration::from_secs(AUTOMATIC_UPDATE_POLL_SECS)
    );
    assert_eq!(
        automatic_update_failure_backoff(2),
        std::time::Duration::from_secs(AUTOMATIC_UPDATE_POLL_SECS * 2)
    );
    assert_eq!(
        automatic_update_failure_backoff(100),
        std::time::Duration::from_secs(AUTOMATIC_UPDATE_MAX_BACKOFF_SECS)
    );
}

#[test]
fn automatic_update_poll_delay_override_is_positive_and_fail_closed() {
    use std::ffi::OsStr;

    let fallback = std::time::Duration::from_secs(AUTOMATIC_UPDATE_POLL_SECS);
    assert_eq!(
        automatic_update_delay_from_value(None, AUTOMATIC_UPDATE_POLL_SECS),
        fallback
    );
    assert_eq!(
        automatic_update_delay_from_value(Some(OsStr::new("")), AUTOMATIC_UPDATE_POLL_SECS),
        fallback
    );
    assert_eq!(
        automatic_update_delay_from_value(Some(OsStr::new("0")), AUTOMATIC_UPDATE_POLL_SECS),
        fallback
    );
    assert_eq!(
        automatic_update_delay_from_value(Some(OsStr::new("invalid")), AUTOMATIC_UPDATE_POLL_SECS),
        fallback
    );
    assert_eq!(
        automatic_update_delay_from_value(Some(OsStr::new("2")), AUTOMATIC_UPDATE_POLL_SECS),
        std::time::Duration::from_secs(2)
    );
}

#[test]
fn automatic_update_loop_runs_only_for_the_unbounded_installed_service() {
    assert!(should_start_automatic_update_loop(None));
    assert!(!should_start_automatic_update_loop(Some(1234)));
}

#[tokio::test]
async fn automatic_update_disabled_setting_skips_command_execution() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path().join("home");
    std::fs::create_dir_all(&capsem_home).unwrap();
    std::fs::write(
        capsem_home.join("settings.toml"),
        "[settings.\"app.auto_update\"]\nvalue = false\nmodified = \"test\"\n",
    )
    .unwrap();
    let _capsem_paths = capsem_foundation::paths::CapsemPathsGuard::redirect(&capsem_home);
    let state = make_test_state();

    let outcome = run_automatic_update_once(&state).await;

    assert_eq!(outcome, AutomaticUpdateOutcome::Disabled);
}

#[tokio::test]
async fn automatic_update_skips_without_queueing_when_explicit_update_is_active() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let _capsem_paths = capsem_foundation::paths::CapsemPathsGuard::redirect(dir.path());
    let state = make_test_state();
    let explicit_guard = state.update_lock.lock().await;

    let outcome = run_automatic_update_once(&state).await;

    drop(explicit_guard);
    assert_eq!(outcome, AutomaticUpdateOutcome::Busy);
}

#[tokio::test]
async fn automatic_update_runs_one_complete_apply_and_reloads_runtime() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path().join("home");
    let assets_dir = capsem_home.join("assets");
    let cli = dir.path().join("capsem");
    let log = dir.path().join("automatic.log");
    write_update_runtime_manifest(&assets_dir, "0.0.0", "profiles-2");
    std::fs::write(
        &cli,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\n", log.display()),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&cli).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&cli, permissions).unwrap();
    let previous_cli = std::env::var_os("CAPSEM_CLI");
    let _capsem_paths = capsem_foundation::paths::CapsemPathsGuard::redirect(&capsem_home);
    std::env::set_var("CAPSEM_CLI", &cli);
    let state = make_asset_state(assets_dir);

    let outcome = run_automatic_update_once(&state).await;

    match previous_cli {
        Some(value) => std::env::set_var("CAPSEM_CLI", value),
        None => std::env::remove_var("CAPSEM_CLI"),
    }
    assert_eq!(
        outcome,
        AutomaticUpdateOutcome::Succeeded(UpdateRuntimeDisposition::Reloaded)
    );
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&log, usize::MAX).unwrap(),
        "update --yes\n"
    );
}

#[tokio::test]
async fn automatic_update_reports_command_failure_for_backoff() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("capsem");
    std::fs::write(&cli, "#!/bin/sh\nprintf 'network unavailable\\n' >&2\nexit 7\n").unwrap();
    let mut permissions = std::fs::metadata(&cli).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&cli, permissions).unwrap();
    let previous_cli = std::env::var_os("CAPSEM_CLI");
    let _capsem_paths = capsem_foundation::paths::CapsemPathsGuard::redirect(dir.path());
    std::env::set_var("CAPSEM_CLI", &cli);
    let state = make_test_state();

    let outcome = run_automatic_update_once(&state).await;

    match previous_cli {
        Some(value) => std::env::set_var("CAPSEM_CLI", value),
        None => std::env::remove_var("CAPSEM_CLI"),
    }
    let AutomaticUpdateOutcome::Failed(error) = outcome else {
        panic!("expected automatic update failure");
    };
    assert!(error.contains("exit Some(7)"), "{error}");
    assert!(error.contains("network unavailable"), "{error}");
}

pub(super) async fn decode_response_json<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

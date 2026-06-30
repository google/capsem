use super::*;
use axum::body::{to_bytes, Body};
use capsem_core::net::policy_config::{ProfileObomConfig, ProfileObomDescriptor};
use std::sync::atomic::AtomicU64;
use tower::ServiceExt;

static SETTINGS_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn update_status_reports_binary_and_asset_tracks_from_cache_and_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest.json"),
        serde_json::json!({
            "format": 2,
            "refresh_policy": "24h",
            "assets": {
                "current": "2026.0627.1",
                "releases": {}
            },
            "binaries": {
                "current": "1.3.1782582155",
                "releases": {}
            }
        })
        .to_string(),
    )
    .unwrap();
    let manifest_hash = capsem_core::asset_manager::hash_file(&assets_dir.join("manifest.json"))
        .expect("manifest hash should be computable");
    std::fs::write(
        assets_dir.join("manifest-origin.json"),
        serde_json::json!({
            "schema": "capsem.manifest_origin.v1",
            "origin": "update",
            "source": "https://release.capsem.org/assets/stable/manifest.json"
        })
        .to_string(),
    )
    .unwrap();
    let cache_path = dir.path().join("update-check.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "checked_at": 1000,
            "latest_version": "1.3.1782600000",
            "update_available": true,
            "latest_assets": "2026.0628.1",
            "assets_update_available": true,
            "source": "https://release.capsem.org/health.json",
            "channel_hash": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "validation_status": "valid"
        })
        .to_string(),
    )
    .unwrap();

    let status =
        update_status_response_from_paths("1.3.1782582155", &assets_dir, &cache_path, 1200);

    assert_eq!(status.checked_at, Some(1000));
    assert!(!status.stale);
    assert_eq!(
        status.channel_url.as_deref(),
        Some("https://release.capsem.org/health.json")
    );
    assert_eq!(
        status.channel_hash.as_deref(),
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
    );
    assert_eq!(
        status.supply_chain.manifest.origin.as_deref(),
        Some("update")
    );
    assert_eq!(
        status.supply_chain.manifest.source.as_deref(),
        Some("https://release.capsem.org/assets/stable/manifest.json")
    );
    assert_eq!(
        status.supply_chain.manifest.path,
        assets_dir.join("manifest.json").display().to_string()
    );
    assert_eq!(
        status.supply_chain.manifest.blake3.as_deref(),
        Some(manifest_hash.as_str())
    );
    assert_eq!(
        status.supply_chain.channel_index.url.as_deref(),
        Some("https://release.capsem.org/health.json")
    );
    assert_eq!(
        status.supply_chain.channel_index.sha256.as_deref(),
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
    );
    assert_eq!(status.supply_chain.host_sbom.name, "host_sbom");
    assert_eq!(
        status.supply_chain.host_sbom.release_artifact.as_deref(),
        Some("capsem-sbom.spdx.json")
    );
    assert_eq!(
        status.supply_chain.vm_obom.route.as_deref(),
        Some("/profiles/{profile_id}/obom")
    );
    assert!(
        status
            .supply_chain
            .attestations
            .iter()
            .any(|reference| reference.name == "github_attestations_vm_assets"),
        "asset rail attestation reference should be explicit"
    );
    assert_eq!(status.validation_status.as_deref(), Some("valid"));
    assert_eq!(status.validation_error, None);
    assert_eq!(status.last_error, None);
    assert_eq!(status.binary.current.as_deref(), Some("1.3.1782582155"));
    assert_eq!(status.binary.latest.as_deref(), Some("1.3.1782600000"));
    assert_eq!(status.binary.state, api::UpdateTrackState::UpdateAvailable);
    assert_eq!(
        status.binary.compatibility,
        api::UpdateCompatibilityState::Compatible
    );
    assert_eq!(status.assets.current.as_deref(), Some("2026.0627.1"));
    assert_eq!(status.assets.latest.as_deref(), Some("2026.0628.1"));
    assert_eq!(status.assets.state, api::UpdateTrackState::UpdateAvailable);
    assert_eq!(status.profiles.state, api::UpdateTrackState::NotPublished);
    assert_eq!(status.images.state, api::UpdateTrackState::NotPublished);
}

#[test]
fn update_status_reports_profile_and_image_tracks_from_release_cache() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let cache_path = dir.path().join("update-check.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "checked_at": 1000,
            "latest_version": "1.3.1782582155",
            "update_available": false,
            "latest_profiles": "profiles-2030.0101.1",
            "current_profiles": "profiles-2030.0101.0",
            "profiles_update_available": true,
            "profiles_state": "update_available",
            "latest_images": "images-2030.0101.1",
            "images_update_available": false,
            "images_state": "published",
            "source": "https://release.capsem.org/health.json"
        })
        .to_string(),
    )
    .unwrap();

    let status =
        update_status_response_from_paths("1.3.1782582155", &assets_dir, &cache_path, 1200);

    assert_eq!(
        status.profiles.current.as_deref(),
        Some("profiles-2030.0101.0")
    );
    assert_eq!(
        status.profiles.latest.as_deref(),
        Some("profiles-2030.0101.1")
    );
    assert!(status.profiles.update_available);
    assert_eq!(
        status.profiles.state,
        api::UpdateTrackState::UpdateAvailable
    );
    assert_eq!(
        status.profiles.compatibility,
        api::UpdateCompatibilityState::Compatible
    );
    assert_eq!(status.profiles.blocked_reason, None);
    assert_eq!(status.images.latest.as_deref(), Some("images-2030.0101.1"));
    assert!(!status.images.update_available);
    assert_eq!(status.images.state, api::UpdateTrackState::Current);
}

#[test]
fn update_status_reports_blocked_profile_track_from_release_cache() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let cache_path = dir.path().join("update-check.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "checked_at": 1000,
            "latest_version": "1.3.1782582155",
            "update_available": false,
            "latest_profiles": "profiles-2030.0101.1",
            "current_profiles": "profiles-2030.0101.0",
            "profiles_update_available": false,
            "profiles_state": "published",
            "profiles_blocked_reason": "requires binary 1.4.0 or newer",
            "source": "https://release.capsem.org/health.json"
        })
        .to_string(),
    )
    .unwrap();

    let status =
        update_status_response_from_paths("1.3.1782582155", &assets_dir, &cache_path, 1200);

    assert_eq!(
        status.profiles.current.as_deref(),
        Some("profiles-2030.0101.0")
    );
    assert_eq!(
        status.profiles.latest.as_deref(),
        Some("profiles-2030.0101.1")
    );
    assert!(!status.profiles.update_available);
    assert_eq!(status.profiles.state, api::UpdateTrackState::Unknown);
    assert_eq!(
        status.profiles.compatibility,
        api::UpdateCompatibilityState::Unknown
    );
    assert_eq!(
        status.profiles.blocked_reason.as_deref(),
        Some("requires binary 1.4.0 or newer")
    );
}

#[test]
fn update_status_reports_blocked_asset_track_from_release_cache() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest.json"),
        serde_json::json!({
            "format": 2,
            "refresh_policy": "24h",
            "assets": {
                "current": "2026.0627.1",
                "releases": {}
            },
            "binaries": {
                "current": "1.3.1782582155",
                "releases": {}
            }
        })
        .to_string(),
    )
    .unwrap();
    let cache_path = dir.path().join("update-check.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "checked_at": 1000,
            "latest_version": "1.3.1782582155",
            "update_available": false,
            "latest_assets": "2030.0101.1",
            "assets_update_available": false,
            "assets_state": "published",
            "assets_blocked_reason": "requires binary 99.99.99 or newer",
            "source": "https://release.capsem.org/health.json"
        })
        .to_string(),
    )
    .unwrap();

    let status =
        update_status_response_from_paths("1.3.1782582155", &assets_dir, &cache_path, 1200);

    assert_eq!(status.assets.current.as_deref(), Some("2026.0627.1"));
    assert_eq!(status.assets.latest.as_deref(), Some("2030.0101.1"));
    assert!(!status.assets.update_available);
    assert_eq!(status.assets.state, api::UpdateTrackState::Unknown);
    assert_eq!(
        status.assets.compatibility,
        api::UpdateCompatibilityState::Unknown
    );
    assert_eq!(
        status.assets.blocked_reason.as_deref(),
        Some("requires binary 99.99.99 or newer")
    );
}

#[test]
fn update_status_reports_unknown_when_cache_is_missing_and_keeps_manifest_channel() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-origin.json"),
        serde_json::json!({
            "schema": "capsem.manifest_origin.v1",
            "origin": "package",
            "source": "https://corp.example/capsem/assets/internal/manifest.json"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths(
        "1.3.1782582155",
        &assets_dir,
        &dir.path().join("missing-update-check.json"),
        1200,
    );

    assert_eq!(status.checked_at, None);
    assert!(status.stale);
    assert_eq!(
        status.channel_url.as_deref(),
        Some("https://corp.example/capsem/health.json")
    );
    assert_eq!(
        status.supply_chain.channel_index.url.as_deref(),
        Some("https://corp.example/capsem/health.json")
    );
    assert_eq!(
        status.supply_chain.manifest.source.as_deref(),
        Some("https://corp.example/capsem/assets/internal/manifest.json")
    );
    assert_eq!(status.last_error, None);
    assert_eq!(status.binary.current.as_deref(), Some("1.3.1782582155"));
    assert_eq!(status.binary.latest, None);
    assert_eq!(status.binary.state, api::UpdateTrackState::Current);
    assert_eq!(status.assets.current, None);
    assert_eq!(status.assets.state, api::UpdateTrackState::Unknown);
}

#[test]
fn update_status_derives_health_url_from_manifest_origin_when_cache_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-origin.json"),
        serde_json::json!({
            "schema": "capsem.manifest_origin.v1",
            "origin": "package",
            "source": "https://updates.corp.example/releases/assets/stable/manifest.json"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths(
        "1.3.1782582155",
        &assets_dir,
        &dir.path().join("missing-update-check.json"),
        1200,
    );

    assert_eq!(
        status.channel_url.as_deref(),
        Some("https://updates.corp.example/releases/health.json")
    );
    assert_eq!(
        status.supply_chain.channel_index.url.as_deref(),
        Some("https://updates.corp.example/releases/health.json")
    );
    assert_eq!(
        status.supply_chain.manifest.source.as_deref(),
        Some("https://updates.corp.example/releases/assets/stable/manifest.json")
    );
}

#[test]
fn update_status_reports_cache_parse_errors_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("update-check.json");
    std::fs::write(&cache_path, "not json").unwrap();

    let status = update_status_response_from_paths("1.3.1782582155", dir.path(), &cache_path, 1200);

    assert!(status.stale);
    assert!(status
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("parse")));
}

#[test]
fn update_status_reports_cached_channel_validation_errors() {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("update-check.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "checked_at": 1000,
            "source": "https://release.capsem.org/health.json",
            "channel_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "validation_status": "fetch_error",
            "validation_error": "GET https://release.capsem.org/health.json timed out"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths("1.3.1782582155", dir.path(), &cache_path, 1200);

    assert_eq!(
        status.channel_hash.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(status.validation_status.as_deref(), Some("fetch_error"));
    assert_eq!(
        status.validation_error.as_deref(),
        Some("GET https://release.capsem.org/health.json timed out")
    );
    assert_eq!(
        status.last_error.as_deref(),
        Some("GET https://release.capsem.org/health.json timed out")
    );
}

#[test]
fn process_env_allowlist_forwards_mcp_timeout_knobs() {
    assert!(
        PROCESS_ENV_ALLOWLIST.contains(&"CAPSEM_HOME"),
        "CAPSEM_HOME must reach capsem-process so tests and custom installs use the same config root as capsem-service"
    );

    for key in [
        "CAPSEM_CORP_CONFIG",
        "CAPSEM_CREDENTIAL_STORE_PATH",
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
fn snapshot_status_from_session_dir_reads_snapshot_metadata_without_db() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path();
    std::fs::create_dir_all(session.join("workspace")).unwrap();
    std::fs::create_dir_all(session.join("system")).unwrap();
    std::fs::create_dir_all(session.join("auto_snapshots")).unwrap();
    std::fs::write(session.join("workspace/hello.txt"), "hello").unwrap();

    let mut scheduler = capsem_core::auto_snapshot::AutoSnapshotScheduler::new(
        session.to_path_buf(),
        10,
        12,
        std::time::Duration::from_secs(300),
    );
    scheduler.take_snapshot().unwrap();
    scheduler.take_named_snapshot("manual_check").unwrap();

    let status = snapshot_status_from_session_dir(session);
    assert_eq!(status.total, 2);
    assert_eq!(status.auto_count, 1);
    assert_eq!(status.manual_count, 1);
    assert_eq!(status.manual_available, 11);
    assert!(status
        .snapshots
        .iter()
        .any(|snapshot| snapshot.origin == "manual"
            && snapshot.name.as_deref() == Some("manual_check")));

    let db_path = session.join("session.db");
    assert!(
        !db_path.exists(),
        "snapshot route backing must not require session.db"
    );
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

fn test_profile_summary_cache() -> Vec<api::ProfileSummary> {
    build_profile_summary_cache().expect("test profile summary cache should build")
}

fn test_profile_cache() -> BTreeMap<String, Profile> {
    build_profile_cache().expect("test profile cache should build")
}

fn test_profile_rule_cache() -> Mutex<BTreeMap<String, Vec<api::EnforcementRuleInfo>>> {
    Mutex::new(build_profile_rule_cache(None).expect("test profile rule cache should build"))
}

fn test_profile_plugin_policy_cache(
) -> Mutex<BTreeMap<String, BTreeMap<String, SecurityPluginConfig>>> {
    Mutex::new(
        build_profile_plugin_policy_cache(None)
            .expect("test profile plugin policy cache should build"),
    )
}

fn test_profile_mutation_db(run_dir: &StdPath) -> Arc<capsem_logger::DbHandle> {
    ServiceState::open_profile_mutation_db_handle(run_dir).unwrap()
}

fn make_test_state() -> Arc<ServiceState> {
    let run_dir = PathBuf::from("/tmp/capsem-test-svc");
    let registry_path = run_dir.join("persistent_registry.json");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: PathBuf::from("/nonexistent/assets"),
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest: None,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        profile_summary_cache: Mutex::new(test_profile_summary_cache()),
        profile_cache: Mutex::new(test_profile_cache()),
        profile_status_cache: Mutex::new(None),
        profile_rule_cache: test_profile_rule_cache(),
        profile_plugin_policy_cache: test_profile_plugin_policy_cache(),
        mcp_tool_cache: Mutex::new(capsem_core::mcp::load_tool_cache()),
        profile_mutation_db: test_profile_mutation_db(&run_dir),
        last_defunct_reconcile_ms: AtomicU64::new(0),
        stats_response_cache: Mutex::new(None),
        stats_detail_response_cache: Mutex::new(HashMap::new()),
        storage_diagnostics_cache: Mutex::new(HashMap::new()),
        persistent_resume_state_cache: Mutex::new(HashMap::new()),
        evaluate_rule_cache: Mutex::new(HashMap::new()),
        profile_rule_response_cache: Mutex::new(HashMap::new()),
        profile_plugin_response_cache: Mutex::new(HashMap::new()),
        evaluate_response_cache: Mutex::new(HashMap::new()),
        evaluate_last_response_cache: Mutex::new(None),
        save_restore_lock: tokio::sync::RwLock::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    })
}

async fn route_request(
    app: axum::Router,
    method: axum::http::Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    let request_body = if let Some(body) = body {
        builder = builder.header(axum::http::header::CONTENT_TYPE, "application/json");
        Body::from(serde_json::to_vec(&body).unwrap())
    } else {
        Body::empty()
    };
    let response = app
        .oneshot(builder.body(request_body).unwrap())
        .await
        .expect("route should respond");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes).to_string() }))
    };
    (status, json)
}

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

    let app = build_service_router(make_test_state());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/update/check",
        Some(json!({})),
    )
    .await;
    match previous {
        Some(value) => std::env::set_var("CAPSEM_CLI", value),
        None => std::env::remove_var("CAPSEM_CLI"),
    }

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "succeeded");
    assert_eq!(body["command"]["args"], json!(["update", "--check"]));
    assert_eq!(std::fs::read_to_string(log).unwrap(), "update --check\n");
}

#[tokio::test]
async fn update_route_apply_dry_run_plans_binary_profiles_and_assets() {
    let app = build_service_router(make_test_state());
    let (binary_status, binary_body) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/update/apply",
        Some(json!({ "action": "binary_profiles", "dry_run": true })),
    )
    .await;
    let (assets_status, assets_body) = route_request(
        app,
        axum::http::Method::POST,
        "/update/apply",
        Some(json!({ "action": "assets", "dry_run": true })),
    )
    .await;

    assert_eq!(binary_status, StatusCode::OK);
    assert_eq!(binary_body["status"], "planned");
    assert_eq!(binary_body["command"]["args"], json!(["update", "--yes"]));
    assert_eq!(assets_status, StatusCode::OK);
    assert_eq!(assets_body["status"], "planned");
    assert_eq!(
        assets_body["command"]["args"],
        json!(["update", "--assets"])
    );
}

#[tokio::test]
async fn update_route_apply_requires_confirmation_for_live_commands() {
    let app = build_service_router(make_test_state());
    for action in ["binary_profiles", "assets"] {
        let (status, body) = route_request(
            app.clone(),
            axum::http::Method::POST,
            "/update/apply",
            Some(json!({ "action": action })),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{action}");
        assert_eq!(
            body["error"], "update apply requires confirmed=true or dry_run=true",
            "{action}"
        );
    }
}

#[tokio::test]
async fn update_route_apply_rejects_ambiguous_action_body() {
    let app = build_service_router(make_test_state());
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/update/apply",
        Some(json!({
            "action": "binary_profiles",
            "confirmed": true,
            "assets": true,
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
async fn update_route_apply_confirmed_dispatches_binary_profiles_and_assets() {
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

    let app = build_service_router(make_test_state());
    let (binary_status, binary_body) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/update/apply",
        Some(json!({ "action": "binary_profiles", "confirmed": true })),
    )
    .await;
    let (assets_status, assets_body) = route_request(
        app,
        axum::http::Method::POST,
        "/update/apply",
        Some(json!({ "action": "assets", "confirmed": true })),
    )
    .await;
    match previous {
        Some(value) => std::env::set_var("CAPSEM_CLI", value),
        None => std::env::remove_var("CAPSEM_CLI"),
    }

    assert_eq!(binary_status, StatusCode::OK);
    assert_eq!(binary_body["status"], "succeeded");
    assert_eq!(binary_body["command"]["args"], json!(["update", "--yes"]));
    assert_eq!(assets_status, StatusCode::OK);
    assert_eq!(assets_body["status"], "succeeded");
    assert_eq!(
        assets_body["command"]["args"],
        json!(["update", "--assets"])
    );
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "update --yes\nupdate --assets\n"
    );
}

async fn decode_response_json<T: serde::de::DeserializeOwned>(
    response: axum::response::Response,
) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn enforcement_evaluate_body(request: &EnforcementEvaluateRequest) -> Bytes {
    Bytes::from(serde_json::to_vec(request).unwrap())
}

fn make_asset_state(assets_dir: PathBuf) -> Arc<ServiceState> {
    let run_dir = assets_dir.join("run");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    let manifest = capsem_core::asset_manager::load_manifest_for_assets(&assets_dir).map(Arc::new);
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(
            assets_dir.join("persistent_registry.json"),
        )),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir,
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        profile_summary_cache: Mutex::new(test_profile_summary_cache()),
        profile_cache: Mutex::new(test_profile_cache()),
        profile_status_cache: Mutex::new(None),
        profile_rule_cache: test_profile_rule_cache(),
        profile_plugin_policy_cache: test_profile_plugin_policy_cache(),
        mcp_tool_cache: Mutex::new(capsem_core::mcp::load_tool_cache()),
        profile_mutation_db: test_profile_mutation_db(&run_dir),
        last_defunct_reconcile_ms: AtomicU64::new(0),
        stats_response_cache: Mutex::new(None),
        stats_detail_response_cache: Mutex::new(HashMap::new()),
        storage_diagnostics_cache: Mutex::new(HashMap::new()),
        persistent_resume_state_cache: Mutex::new(HashMap::new()),
        evaluate_rule_cache: Mutex::new(HashMap::new()),
        profile_rule_response_cache: Mutex::new(HashMap::new()),
        profile_plugin_response_cache: Mutex::new(HashMap::new()),
        evaluate_response_cache: Mutex::new(HashMap::new()),
        evaluate_last_response_cache: Mutex::new(None),
        save_restore_lock: tokio::sync::RwLock::new(()),
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
    insert_fake_instance_with_session_dir_and_pins(
        state,
        id,
        pid,
        session_dir,
        test_profile_revision(),
        test_profile_payload_hash(),
        test_asset_pins(),
    );
}

fn insert_fake_instance_with_session_dir_and_pins(
    state: &ServiceState,
    id: &str,
    pid: u32,
    session_dir: PathBuf,
    profile_revision: String,
    profile_payload_hash: String,
    asset_pins: BootAssetPins,
) {
    state.instances.lock().unwrap().insert(
        id.to_string(),
        InstanceInfo {
            id: id.to_string(),
            name: id.to_string(),
            profile_id: "code".into(),
            profile_revision,
            profile_payload_hash,
            asset_pins,
            pid,
            uds_path: PathBuf::from(format!("/tmp/{}.sock", id)),
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
}

fn test_profile_revision() -> String {
    ProfileConfigFile::builtin_primary().revision
}

fn materialized_test_profile() -> ProfileConfigFile {
    materialized_test_profile_for("code")
}

fn materialized_test_profile_for(profile_id: &str) -> ProfileConfigFile {
    let profile_path = checked_in_profile_dir(profile_id).join("profile.toml");
    let mut profile: ProfileConfigFile =
        toml::from_str(&std::fs::read_to_string(profile_path).unwrap()).unwrap();
    let hash = format!("blake3:{}", blake3::hash(b"test-asset").to_hex());
    let size = b"test-asset".len() as u64;
    for arch_assets in profile.assets.arch.values_mut() {
        for asset in [
            &mut arch_assets.kernel,
            &mut arch_assets.initrd,
            &mut arch_assets.rootfs,
        ] {
            asset.hash = Some(hash.clone());
            asset.size = Some(size);
        }
    }
    pin_checked_in_profile_files(&mut profile);
    profile
}

fn test_profile_payload_hash() -> String {
    profile_payload_hash(&materialized_test_profile()).unwrap()
}

fn test_asset_pins() -> BootAssetPins {
    profile_asset_pins(&materialized_test_profile()).unwrap()
}

fn install_test_profile_assets(state: &ServiceState) {
    let profile = materialized_test_profile();
    install_test_profile_catalog(state, &profile);

    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = state.assets_dir.join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let assets = profile.assets.current_arch_assets().unwrap();
    for asset in [&assets.kernel, &assets.initrd, &assets.rootfs] {
        std::fs::write(
            arch_dir.join(profile_asset_hash_name(asset).expect("profile asset hash name")),
            b"test-asset",
        )
        .unwrap();
    }
}

fn install_test_profile_catalog(state: &ServiceState, profile: &ProfileConfigFile) {
    let config_root = state.run_dir.join("config");
    let profile_dir = config_root.join("profiles").join(&profile.id);
    copy_dir_all(checked_in_profile_dir(&profile.id).as_path(), &profile_dir);
    std::fs::write(
        profile_dir.join("profile.toml"),
        toml::to_string_pretty(&profile).unwrap(),
    )
    .unwrap();
    super::set_test_profile_dir_override(Some(config_root.join("profiles")));
}

fn test_persistent_entry(name: &str, session_dir: PathBuf) -> PersistentVmEntry {
    PersistentVmEntry {
        id: new_persistent_vm_id(),
        name: name.into(),
        profile_id: "code".into(),
        profile_revision: test_profile_revision(),
        profile_payload_hash: test_profile_payload_hash(),
        asset_pins: test_asset_pins(),
        ram_mb: 2048,
        cpus: 2,
        base_version: "0.0.0".into(),
        created_at: "0".into(),
        session_dir,
        forked_from: None,
        description: None,
        suspended: false,
        defunct: false,
        last_error: None,
        checkpoint_path: None,
        env: None,
    }
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn checked_in_profile_dir(profile_id: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../config/profiles")
        .join(profile_id)
}

fn install_code_profile_fixture(dir: &tempfile::TempDir) -> PathBuf {
    let config_root = dir.path().join("config");
    let profile_dir = config_root.join("profiles/code");
    copy_dir_all(checked_in_profile_dir("code").as_path(), &profile_dir);
    config_root
}

fn profile_file_descriptor(
    config_root: &std::path::Path,
    path: &std::path::Path,
) -> capsem_core::net::policy_config::ProfileFileDescriptor {
    let bytes = std::fs::metadata(path).unwrap().len();
    let hash = capsem_core::asset_manager::hash_file(path).unwrap();
    let relative = path
        .strip_prefix(config_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    capsem_core::net::policy_config::ProfileFileDescriptor {
        path: relative,
        hash: Some(format!("blake3:{hash}")),
        size: Some(bytes),
    }
}

fn assign_file_descriptor_profile(
    profile: &mut ProfileConfigFile,
    descriptor: capsem_core::net::policy_config::ProfileFileDescriptor,
) {
    match std::path::Path::new(&descriptor.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap()
    {
        "enforcement.toml" => {
            profile.files.enforcement = Some(descriptor);
        }
        "detection.yaml" => {
            profile.files.detection = Some(descriptor);
        }
        "mcp.json" => {
            profile.files.mcp = Some(descriptor);
        }
        "apt-packages.txt" => {
            profile.files.apt_packages = Some(descriptor);
        }
        "python-requirements.txt" => {
            profile.files.python_requirements = Some(descriptor);
        }
        "npm-packages.txt" => {
            profile.files.npm_packages = Some(descriptor);
        }
        "build.sh" => {
            profile.files.build = Some(descriptor);
        }
        "tips.txt" => {
            profile.files.tips = Some(descriptor);
        }
        "root.manifest.json" => {
            profile.files.root_manifest = Some(descriptor);
        }
        other => panic!("unsupported profile fixture descriptor {other}"),
    }
}

fn write_file_descriptor_profile(
    profile: &mut ProfileConfigFile,
    config_root: &std::path::Path,
    path: &std::path::Path,
) {
    assign_file_descriptor_profile(profile, profile_file_descriptor(config_root, path));
}

fn pin_checked_in_profile_files(profile: &mut ProfileConfigFile) {
    let repo_config_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    let profile_dir = repo_config_root.join("profiles").join(&profile.id);
    for filename in [
        "enforcement.toml",
        "detection.yaml",
        "mcp.json",
        "apt-packages.txt",
        "python-requirements.txt",
        "npm-packages.txt",
        "build.sh",
        "tips.txt",
        "root.manifest.json",
    ] {
        write_file_descriptor_profile(profile, &repo_config_root, &profile_dir.join(filename));
    }
}

fn install_file_asset_profile_fixture(dir: &tempfile::TempDir) -> (PathBuf, ProfileConfigFile) {
    let config_root = install_code_profile_fixture(dir);
    let profile_dir = config_root.join("profiles/code");
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let source_dir = dir.path().join("asset-source").join(arch);
    std::fs::create_dir_all(&source_dir).unwrap();

    let mut profile = ProfileConfigFile::builtin_primary();
    for (name, body) in [
        ("vmlinuz", b"fixture-kernel".as_slice()),
        ("initrd.img", b"fixture-initrd".as_slice()),
        ("rootfs.erofs", b"fixture-rootfs".as_slice()),
    ] {
        std::fs::write(source_dir.join(name), body).unwrap();
    }
    let arch_assets = profile.assets.arch.get_mut(arch).unwrap();
    for asset in [
        &mut arch_assets.kernel,
        &mut arch_assets.initrd,
        &mut arch_assets.rootfs,
    ] {
        let source = source_dir.join(&asset.name);
        let hash = capsem_core::asset_manager::hash_file(&source).unwrap();
        asset.url = format!("file://{}", source.display());
        asset.hash = Some(format!("blake3:{hash}"));
        asset.size = Some(std::fs::metadata(&source).unwrap().len());
    }
    for filename in [
        "enforcement.toml",
        "detection.yaml",
        "mcp.json",
        "apt-packages.txt",
        "python-requirements.txt",
        "npm-packages.txt",
        "build.sh",
        "tips.txt",
        "root.manifest.json",
    ] {
        write_file_descriptor_profile(&mut profile, &config_root, &profile_dir.join(filename));
    }
    std::fs::write(
        profile_dir.join("profile.toml"),
        toml::to_string_pretty(&profile).unwrap(),
    )
    .unwrap();
    (config_root, profile)
}

fn add_profile_enforcement_rule(
    config_root: &std::path::Path,
    rule_id: &str,
    rule: capsem_core::net::policy_config::SecurityRule,
) {
    let profile_dir = config_root.join("profiles/code");
    let enforcement_path = profile_dir.join("enforcement.toml");
    let content = std::fs::read_to_string(&enforcement_path).unwrap();
    let mut rule_profile = SecurityRuleProfile::parse_toml(&content).unwrap();
    rule_profile
        .profiles
        .rules
        .insert(rule_id.to_string(), rule);
    std::fs::write(
        &enforcement_path,
        toml::to_string_pretty(&rule_profile).unwrap(),
    )
    .unwrap();
    let mut profile: ProfileConfigFile =
        toml::from_str(&std::fs::read_to_string(profile_dir.join("profile.toml")).unwrap())
            .unwrap();
    write_file_descriptor_profile(&mut profile, config_root, &enforcement_path);
    std::fs::write(
        profile_dir.join("profile.toml"),
        toml::to_string_pretty(&profile).unwrap(),
    )
    .unwrap();
}

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

    let (status, body) =
        route_request(app, axum::http::Method::GET, "/profiles/status", None).await;
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
    let rootfs_target = assets_dir
        .join(arch)
        .join(capsem_core::asset_manager::hash_filename(
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
    assert_eq!(ensured["ready"], true);
    assert_eq!(ensured["downloaded"], 3);
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
    assert!(cached_after_tamper["invalid_assets"]
        .as_array()
        .unwrap()
        .is_empty());

    let (status, repaired) = route_request(
        app,
        axum::http::Method::POST,
        "/profiles/code/assets/ensure",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{repaired}");
    assert_eq!(repaired["ready"], true);
    assert_eq!(repaired["downloaded"], 1);
}

#[cfg(unix)]
#[tokio::test]
async fn profile_asset_status_does_not_read_asset_contents_on_hot_path() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, profile) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let assets_dir = dir.path().join("assets");
    let state = make_asset_state(assets_dir.clone());
    let app = build_service_router(state);

    let (status, ensured) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/assets/ensure",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ensured}");
    assert_eq!(ensured["ready"], true);

    let arch = capsem_core::net::policy_config::current_profile_arch();
    let rootfs = &profile.assets.current_arch_assets().unwrap().rootfs;
    let rootfs_path = assets_dir
        .join(arch)
        .join(capsem_core::asset_manager::hash_filename(
            &rootfs.name,
            rootfs
                .hash
                .as_deref()
                .expect("rootfs hash")
                .strip_prefix("blake3:")
                .unwrap(),
        ));

    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&rootfs_path, std::fs::Permissions::from_mode(0o000)).unwrap();

    let (status, hot_status) =
        route_request(app, axum::http::Method::GET, "/profiles/status", None).await;
    assert_eq!(status, StatusCode::OK, "{hot_status}");
    assert_eq!(
        hot_status["profiles"][0]["ready"], true,
        "profile status is a hot readiness route and must not hash/read asset contents"
    );

    std::fs::set_permissions(&rootfs_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let loaded =
        capsem_core::net::policy_config::Profile::load_from_dir(config_root.join("profiles/code"))
            .unwrap();
    std::fs::set_permissions(&rootfs_path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let error = loaded
        .check(&assets_dir, arch)
        .expect_err("explicit profile verification still reads and rejects unreadable assets");
    assert!(error.contains("rootfs"), "{error}");
    std::fs::set_permissions(&rootfs_path, std::fs::Permissions::from_mode(0o644)).unwrap();
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

    let enforcement = std::fs::read_to_string(config_root.join("profiles/code/enforcement.toml"))
        .expect("mutated enforcement file");
    let rule_profile = SecurityRuleProfile::parse_toml(&enforcement).unwrap();
    let rule = rule_profile
        .profiles
        .rules
        .get("mcp_local_fetch_http_permission")
        .expect("profile-managed MCP permission rule");
    assert_eq!(
        rule.action,
        capsem_core::net::policy_config::SecurityRuleAction::Ask
    );
    assert_eq!(
        rule.condition,
        r#"mcp.server.name == "local" && mcp.tool_call.name == "fetch_http""#
    );

    let profile: ProfileConfigFile = toml::from_str(
        &std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap(),
    )
    .unwrap();
    let descriptor = profile.files.enforcement.expect("updated enforcement pin");
    assert_eq!(descriptor.path, "profiles/code/enforcement.toml");
    assert_eq!(
        descriptor.hash,
        Some(format!(
            "blake3:{}",
            capsem_core::asset_manager::hash_file(
                &config_root.join("profiles/code/enforcement.toml")
            )
            .unwrap()
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
        json!([
            "code",
            "mcp",
            "mcp_tool",
            "local/fetch_http",
            "permission",
            "applied"
        ])
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

    let enforcement = std::fs::read_to_string(config_root.join("profiles/code/enforcement.toml"))
        .expect("mutated enforcement file");
    let rule_profile = SecurityRuleProfile::parse_toml(&enforcement).unwrap();
    let default = rule_profile.default.get("mcp").expect("default mcp rule");
    assert_eq!(
        default.action,
        capsem_core::net::policy_config::SecurityRuleAction::Ask
    );

    let profile: ProfileConfigFile = toml::from_str(
        &std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap(),
    )
    .unwrap();
    let descriptor = profile.files.enforcement.expect("updated enforcement pin");
    assert_eq!(descriptor.path, "profiles/code/enforcement.toml");
    assert_eq!(
        descriptor.hash,
        Some(format!(
            "blake3:{}",
            capsem_core::asset_manager::hash_file(
                &config_root.join("profiles/code/enforcement.toml")
            )
            .unwrap()
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
        json!([
            "code",
            "mcp",
            "mcp_default",
            "default.mcp",
            "permission",
            "applied"
        ])
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

    let (status, default_info) = route_request(
        app,
        axum::http::Method::GET,
        "/profiles/code/mcp/default/info",
        None,
    )
    .await;
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
    assert_eq!(
        edited["mutation"]["affected_path"],
        "profiles/code/profile.toml"
    );
    assert_eq!(edited["mutation"]["target_kind"], "mcp_server");
    assert_eq!(edited["mutation"]["target_key"], "github");
    assert_eq!(edited["mutation"]["operation"], "upsert");
    assert_eq!(edited["mutation"]["status"], "applied");

    let profile: ProfileConfigFile = toml::from_str(
        &std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap(),
    )
    .unwrap();
    assert!(profile
        .mcp
        .as_ref()
        .unwrap()
        .servers
        .iter()
        .any(|server| server.name == "github"
            && server.url == "https://mcp.invalid/github"
            && server.enabled));

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

    let profile: ProfileConfigFile = toml::from_str(
        &std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap(),
    )
    .unwrap();
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

#[tokio::test]
async fn security_routes_read_security_ledger_from_session_db() {
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
    let db_path_for_writer = db_path.clone();
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&db_path_for_writer, 16).unwrap();
        writer.write_blocking(capsem_logger::WriteOp::SecurityRuleEvent(
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
        ));
        writer.shutdown_blocking();
    })
    .await
    .unwrap();
    let response = handle_security_latest(
        State(Arc::clone(&state)),
        Path("vm-ledger".to_string()),
        Query(SecurityLedgerQuery { limit: Some(10) }),
    )
    .await
    .expect("security latest reads session ledger");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let events: Vec<capsem_logger::SecurityRuleEvent> = serde_json::from_slice(&bytes).unwrap();

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

    let response = handle_security_info(State(state), Path("vm-ledger".to_string()))
        .await
        .expect("security status reads session ledger");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let stats: capsem_logger::SecurityRuleStats = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stats.total, 1);
    assert_eq!(stats.by_action[0].rule_action, "allow");
    assert_eq!(stats.by_action[0].count, 1);
    assert_eq!(stats.by_event_type[0].event_type, "model.call");
    assert_eq!(stats.by_event_type[0].count, 1);
    assert_eq!(stats.by_level[0].detection_level, "informational");
    assert_eq!(stats.by_level[0].count, 1);
    assert_eq!(
        stats.by_rule[0].rule_id,
        "profiles.rules.ai_ollama_model_api"
    );
    assert_eq!(stats.by_rule[0].rule_action, "allow");
    assert_eq!(stats.by_rule[0].detection_level, "informational");
    assert_eq!(stats.by_rule[0].count, 1);
    assert_eq!(stats.by_rule[0].latest_event_id, "abcdef123456");
}

#[tokio::test]
async fn history_routes_read_history_ledger_from_session_db() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("history-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "history-vm",
        std::process::id(),
        session_dir.clone(),
    );

    let db_path = session_dir.join("session.db");
    let marker = "history-ledger-marker";
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::ExecEvent(
            capsem_logger::ExecEvent {
                event_id: None,
                timestamp: std::time::SystemTime::now(),
                exec_id: 41,
                command: format!("echo {marker}"),
                source: "api".to_string(),
                trace_id: Some("trace-history".to_string()),
                process_name: Some("bash".to_string()),
                credential_ref: None,
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEventComplete(
            capsem_logger::ExecEventComplete {
                exec_id: 41,
                exit_code: 0,
                duration_ms: 17,
                stdout_preview: Some(format!("{marker}\n")),
                stderr_preview: None,
                stdout_bytes: (marker.len() + 1) as u64,
                stderr_bytes: 0,
                pid: Some(123),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::AuditEvent(
            capsem_logger::AuditEvent {
                event_id: None,
                timestamp: std::time::SystemTime::now(),
                pid: 123,
                ppid: 1,
                uid: 0,
                exe: "/usr/bin/bash".to_string(),
                comm: Some("bash".to_string()),
                argv: format!("bash -lc 'echo {marker}'"),
                cwd: Some("/root".to_string()),
                tty: None,
                session_id: Some(1),
                audit_id: Some("audit-history".to_string()),
                exec_event_id: Some(41),
                parent_exe: Some("/usr/bin/sh".to_string()),
                trace_id: Some("trace-history".to_string()),
                credential_ref: None,
            },
        ))
        .await;
    writer.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let direct_counts = reader.history_counts().unwrap();
    assert_eq!(direct_counts.exec_count, 1);
    assert_eq!(direct_counts.audit_count, 1);
    let (status, counts) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/history-vm/history/counts",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{counts}");
    assert_eq!(counts["exec_count"], 1);
    assert_eq!(counts["audit_count"], 1);

    let (status, history) = route_request(
        app.clone(),
        axum::http::Method::GET,
        &format!("/vms/history-vm/history?search={marker}&limit=10"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["total"], 2);
    let commands = history["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 2, "{history}");
    assert!(commands.iter().any(|entry| entry["layer"] == "exec"
        && entry["command"].as_str().unwrap().contains(marker)
        && entry["stdout_preview"].as_str().unwrap().contains(marker)));
    assert!(commands.iter().any(|entry| entry["layer"] == "audit"
        && entry["command"].as_str().unwrap().contains(marker)
        && entry["details"]["exe"] == "/usr/bin/bash"));

    let (status, processes) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/history-vm/history/processes",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{processes}");
    assert_eq!(processes["processes"][0]["exe"], "/usr/bin/bash");
    assert_eq!(processes["processes"][0]["command_count"], 1);
}

#[tokio::test]
async fn detection_latest_route_filters_non_detection_rule_rows() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("detect-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "detect-vm",
        std::process::id(),
        session_dir.clone(),
    );

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "aaaaaa000000",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                r#"{"event_type":"http.request"}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow),
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_457,
                "bbbbbb000000",
                "model.call",
                "profiles.rules.ai_unknown_provider",
                r#"{"name":"ai_unknown_provider"}"#,
                r#"{"event_type":"model.call","model":{"provider":"unknown"}}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow)
            .with_detection_level(capsem_logger::SecurityDetectionLevel::High),
        ))
        .await;
    writer.shutdown_blocking();
    let direct_rows = capsem_logger::DbReader::open(&db_path)
        .unwrap()
        .recent_security_rule_events(10)
        .unwrap();
    assert_eq!(direct_rows.len(), 2);
    assert!(direct_rows.iter().any(|row| row.event_id == "bbbbbb000000"
        && row.detection_level == capsem_logger::SecurityDetectionLevel::High));
    let (status, detection) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/detect-vm/detection/latest?limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detection}");
    let detection_rows = detection.as_array().unwrap();
    assert_eq!(detection_rows.len(), 1, "{detection}");
    assert_eq!(detection_rows[0]["event_id"], "bbbbbb000000");
    assert_eq!(detection_rows[0]["detection_level"], "high");

    let (status, enforcement) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/detect-vm/enforcement/latest?limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{enforcement}");
    let enforcement_rows = enforcement.as_array().unwrap();
    assert_eq!(enforcement_rows.len(), 2, "{enforcement}");
    assert!(enforcement_rows
        .iter()
        .any(|row| row["event_id"] == "aaaaaa000000"));
    assert!(enforcement_rows
        .iter()
        .any(|row| row["event_id"] == "bbbbbb000000"));
}

#[tokio::test]
async fn timeline_route_reads_timeline_ledger_from_session_db() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("timeline-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "timeline-vm",
        std::process::id(),
        session_dir.clone(),
    );

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::ExecEvent(
            capsem_logger::ExecEvent {
                event_id: Some("ccc111000000".to_string()),
                timestamp: std::time::SystemTime::now(),
                exec_id: 77,
                command: "echo timeline-marker".to_string(),
                source: "api".to_string(),
                trace_id: Some("trace-timeline".to_string()),
                process_name: Some("bash".to_string()),
                credential_ref: None,
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEventComplete(
            capsem_logger::ExecEventComplete {
                exec_id: 77,
                exit_code: 0,
                duration_ms: 11,
                stdout_preview: Some("timeline-marker\n".to_string()),
                stderr_preview: None,
                stdout_bytes: 16,
                stderr_bytes: 0,
                pid: Some(456),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
            event_id: Some("ddd222000000".to_string()),
            timestamp: std::time::SystemTime::now(),
            domain: "127.0.0.1".to_string(),
            port: 3713,
            decision: capsem_logger::Decision::Allowed,
            process_name: Some("curl".to_string()),
            pid: Some(456),
            method: Some("POST".to_string()),
            path: Some("/echo".to_string()),
            query: None,
            status_code: Some(200),
            bytes_sent: 2,
            bytes_received: 17,
            duration_ms: 9,
            matched_rule: Some("profiles.rules.default_http".to_string()),
            request_headers: None,
            response_headers: None,
            request_body_preview: Some("{}".to_string()),
            response_body_preview: Some(r#"{"ok":true}"#.to_string()),
            request_body_full: Some("{}".to_string()),
            response_body_full: Some(r#"{"ok":true}"#.to_string()),
            conn_type: Some("http".to_string()),
            policy_mode: None,
            policy_action: Some("allow".to_string()),
            policy_rule: Some("profiles.rules.default_http".to_string()),
            policy_reason: None,
            trace_id: Some("trace-timeline".to_string()),
            credential_ref: None,
        }))
        .await;
    writer.shutdown_blocking();

    let (status, timeline) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/timeline-vm/timeline?trace_id=trace-timeline&layers=exec,net&limit=20",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    assert_eq!(
        timeline["columns"],
        json!([
            "timestamp",
            "layer",
            "ref",
            "summary",
            "status",
            "duration_ms",
            "trace_id"
        ])
    );
    let rows = timeline["rows"].as_array().unwrap();
    assert!(rows.iter().any(|row| row[1] == "exec"
        && row[2] == 77
        && row[3] == "echo timeline-marker"
        && row[4] == 0
        && row[5] == 11
        && row[6] == "trace-timeline"));
    assert!(rows.iter().any(|row| row[1] == "net"
        && row[2] == 1
        && row[3] == "POST 127.0.0.1/echo"
        && row[4] == 200
        && row[5] == 9
        && row[6] == "trace-timeline"));
}

#[tokio::test]
async fn triage_route_reads_triage_ledger_from_session_db() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("triage-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "triage-vm",
        std::process::id(),
        session_dir.clone(),
    );

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
            event_id: Some("aaa111000000".to_string()),
            timestamp: std::time::SystemTime::now(),
            domain: "evil.test".to_string(),
            port: 443,
            decision: capsem_logger::Decision::Denied,
            process_name: Some("curl".to_string()),
            pid: Some(789),
            method: Some("GET".to_string()),
            path: Some("/blocked".to_string()),
            query: None,
            status_code: Some(403),
            bytes_sent: 3,
            bytes_received: 0,
            duration_ms: 13,
            matched_rule: Some("corp.rules.block_evil".to_string()),
            request_headers: None,
            response_headers: None,
            request_body_preview: None,
            response_body_preview: None,
            request_body_full: None,
            response_body_full: None,
            conn_type: Some("http".to_string()),
            policy_mode: None,
            policy_action: Some("block".to_string()),
            policy_rule: Some("corp.rules.block_evil".to_string()),
            policy_reason: Some("test denied net".to_string()),
            trace_id: Some("trace-triage".to_string()),
            credential_ref: None,
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::McpCall(capsem_logger::McpCall {
            event_id: Some("bbb111000000".to_string()),
            timestamp: std::time::SystemTime::now(),
            server_name: "local".to_string(),
            method: "tools/call".to_string(),
            tool_name: Some("fetch_http".to_string()),
            transport: "vsock_frame".to_string(),
            request_id: Some("mcp-request-1".to_string()),
            request_preview: Some(r#"{"url":"https://evil.test"}"#.to_string()),
            response_preview: None,
            decision: "error".to_string(),
            duration_ms: 17,
            error_message: Some("boom".to_string()),
            process_name: Some("agent".to_string()),
            bytes_sent: 33,
            bytes_received: 0,
            policy_mode: Some("enforce".to_string()),
            policy_action: Some("block".to_string()),
            policy_rule: Some("profiles.rules.mcp_local_fetch_http".to_string()),
            policy_reason: Some("test mcp error".to_string()),
            trace_id: Some("trace-triage".to_string()),
            credential_ref: None,
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEvent(
            capsem_logger::ExecEvent {
                event_id: Some("ccc111000000".to_string()),
                timestamp: std::time::SystemTime::now(),
                exec_id: 91,
                command: "false".to_string(),
                source: "api".to_string(),
                trace_id: Some("trace-triage".to_string()),
                process_name: Some("bash".to_string()),
                credential_ref: None,
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEventComplete(
            capsem_logger::ExecEventComplete {
                exec_id: 91,
                exit_code: 2,
                duration_ms: 19,
                stdout_preview: None,
                stderr_preview: Some("failed\n".to_string()),
                stdout_bytes: 0,
                stderr_bytes: 7,
                pid: Some(789),
            },
        ))
        .await;
    writer.shutdown_blocking();

    let db = state
        .register_session_db_handle("triage-vm", &session_dir)
        .expect("test installs external DB reader after the process writer created session.db");
    let direct_triage = session_db_triage("triage-vm", &db, &db_path, 5)
        .await
        .unwrap();
    assert_eq!(
        direct_triage["denied_net"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{direct_triage}"
    );
    assert_eq!(
        direct_triage["tool_errors"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{direct_triage}"
    );
    assert_eq!(
        direct_triage["exec_failures"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{direct_triage}"
    );

    let (status, triage) = route_request(
        app,
        axum::http::Method::GET,
        "/triage?id=triage-vm&limit=5",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{triage}");
    assert_eq!(triage["session_id"], "triage-vm");
    assert_eq!(
        triage["session"]["denied_net"]["columns"],
        json!([
            "timestamp",
            "domain",
            "decision",
            "status_code",
            "duration_ms"
        ])
    );
    let denied_net = triage["session"]["denied_net"]["rows"].as_array().unwrap();
    assert_eq!(denied_net.len(), 1, "{triage}");
    assert_eq!(denied_net[0][1], "evil.test");
    assert_eq!(denied_net[0][2], "denied");
    assert_eq!(denied_net[0][3], 403);
    assert_eq!(denied_net[0][4], 13);

    assert_eq!(
        triage["session"]["tool_errors"]["columns"],
        json!([
            "timestamp",
            "server_name",
            "method",
            "decision",
            "policy_mode",
            "policy_action",
            "policy_rule",
            "policy_reason",
            "error_message",
            "duration_ms"
        ])
    );
    let tool_errors = triage["session"]["tool_errors"]["rows"].as_array().unwrap();
    assert_eq!(tool_errors.len(), 1, "{triage}");
    assert_eq!(tool_errors[0][1], "local");
    assert_eq!(tool_errors[0][2], "tools/call");
    assert_eq!(tool_errors[0][3], "error");
    assert_eq!(tool_errors[0][5], "block");
    assert_eq!(tool_errors[0][8], "boom");
    assert_eq!(tool_errors[0][9], 17);

    assert_eq!(
        triage["session"]["exec_failures"]["columns"],
        json!([
            "timestamp",
            "exec_id",
            "command",
            "exit_code",
            "duration_ms"
        ])
    );
    let exec_failures = triage["session"]["exec_failures"]["rows"]
        .as_array()
        .unwrap();
    assert_eq!(exec_failures.len(), 1, "{triage}");
    assert_eq!(exec_failures[0][1], 91);
    assert_eq!(exec_failures[0][2], "false");
    assert_eq!(exec_failures[0][3], 2);
    assert_eq!(exec_failures[0][4], 19);
}

#[tokio::test]
async fn winterfell_routes_read_session_ledgers_after_startup_cache_hydration() {
    let dir = tempfile::tempdir().unwrap();
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let idx = capsem_core::session::SessionIndex::open(&sessions_dir.join("main.db")).unwrap();
    idx.create_session(&capsem_core::session::SessionRecord {
        id: "winterfell-vm".to_string(),
        mode: "virtiofs".to_string(),
        command: Some("winterfell".to_string()),
        status: "running".to_string(),
        created_at: "2026-06-24T00:00:00Z".to_string(),
        stopped_at: None,
        scratch_disk_size_gb: 16,
        ram_bytes: 4_294_967_296,
        total_requests: 0,
        allowed_requests: 0,
        denied_requests: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_estimated_cost: 0.0,
        total_tool_calls: 0,
        total_file_events: 0,
        compressed_size_bytes: None,
        vacuumed_at: None,
        storage_mode: "virtiofs".to_string(),
        rootfs_hash: None,
        rootfs_version: None,
        forked_from: None,
        persistent: false,
        exec_count: 0,
        audit_event_count: 0,
    })
    .unwrap();
    drop(idx);

    let (state, _dir) = make_test_state_with_tempdir_at(dir);
    let app = build_service_router(Arc::clone(&state));
    let session_dir = sessions_dir.join("winterfell-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "winterfell-vm",
        std::process::id(),
        session_dir.clone(),
    );

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::SecurityRuleEvent(
        capsem_logger::SecurityRuleEvent::new(
            1_789_000_223_456,
            "abcdef123450",
            "model.call",
            "profiles.rules.ai_unknown_provider",
            r#"{"name":"ai_unknown_provider","match":"model.provider == \"unknown\""}"#,
            r#"{"event_type":"model.call","model":{"provider":"unknown"}}"#,
        )
        .with_rule_action(capsem_logger::SecurityRuleAction::Allow)
        .with_detection_level(capsem_logger::SecurityDetectionLevel::High)
        .with_trace_id("trace-winterfell")
        .with_turn_id("trace-winterfell")
        .with_credential_ref(
            "credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
    ));
    writer.write_blocking(capsem_logger::WriteOp::ExecEvent(
        capsem_logger::ExecEvent {
            event_id: Some("abcdef123451".to_string()),
            timestamp: std::time::SystemTime::now(),
            exec_id: 501,
            command: "echo winterfell".to_string(),
            source: "api".to_string(),
            trace_id: Some("trace-winterfell".to_string()),
            process_name: Some("bash".to_string()),
            credential_ref: None,
        },
    ));
    writer.write_blocking(capsem_logger::WriteOp::ExecEventComplete(
        capsem_logger::ExecEventComplete {
            exec_id: 501,
            exit_code: 0,
            duration_ms: 12,
            stdout_preview: Some("winterfell\n".to_string()),
            stderr_preview: None,
            stdout_bytes: 11,
            stderr_bytes: 0,
            pid: Some(777),
        },
    ));
    writer.write_blocking(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
        event_id: Some("abcdef123452".to_string()),
        timestamp: std::time::SystemTime::now(),
        domain: "mock.capsem.test".to_string(),
        port: 443,
        decision: capsem_logger::Decision::Allowed,
        process_name: Some("codex".to_string()),
        pid: Some(777),
        method: Some("POST".to_string()),
        path: Some("/v1/responses".to_string()),
        query: None,
        status_code: Some(200),
        bytes_sent: 44,
        bytes_received: 55,
        duration_ms: 8,
        matched_rule: Some("profiles.rules.default_http".to_string()),
        request_headers: Some("content-type: application/json".to_string()),
        response_headers: Some("content-type: application/json".to_string()),
        request_body_preview: Some(r#"{"input":"winterfell"}"#.to_string()),
        response_body_preview: Some(r#"{"output_text":"the wall holds"}"#.to_string()),
        request_body_full: Some(r#"{"input":"winterfell"}"#.to_string()),
        response_body_full: Some(r#"{"output_text":"the wall holds"}"#.to_string()),
        conn_type: Some("https".to_string()),
        policy_mode: None,
        policy_action: Some("allow".to_string()),
        policy_rule: Some("profiles.rules.default_http".to_string()),
        policy_reason: None,
        trace_id: Some("trace-winterfell".to_string()),
        credential_ref: Some(
            "credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        ),
    }));
    writer.write_blocking(capsem_logger::WriteOp::ModelCall(
        capsem_logger::ModelCall {
            event_id: Some("abcdef123453".to_string()),
            timestamp: std::time::SystemTime::now(),
            provider: "openai".to_string(),
            protocol: Some("openai".to_string()),
            model: Some("gpt-5-nano".to_string()),
            process_name: Some("codex".to_string()),
            pid: Some(777),
            method: "POST".to_string(),
            path: "/v1/responses".to_string(),
            stream: false,
            system_prompt_preview: None,
            messages_count: 1,
            tools_count: 1,
            request_bytes: 64,
            request_body_preview: Some(r#"{"input":"write winterfell"}"#.to_string()),
            request_body_full: Some(r#"{"input":"write winterfell"}"#.to_string()),
            message_id: Some("msg-winterfell".to_string()),
            status_code: Some(200),
            text_content: Some("the wall holds".to_string()),
            thinking_content: Some("prepare ledger proof".to_string()),
            response_body_full: Some(r#"{"output_text":"the wall holds"}"#.to_string()),
            stop_reason: Some("end_turn".to_string()),
            input_tokens: Some(9),
            output_tokens: Some(4),
            usage_details: BTreeMap::new(),
            duration_ms: 31,
            response_bytes: 33,
            estimated_cost_usd: 0.00001,
            trace_id: Some("trace-winterfell".to_string()),
            credential_ref: Some("credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            tool_calls: vec![capsem_logger::ToolCallEntry {
                call_index: 0,
                call_id: "tool-winterfell".to_string(),
                tool_name: "Write".to_string(),
                arguments: Some(r#"{"path":"/root/winterfell.md"}"#.to_string()),
                origin: "model".to_string(),
                trace_id: Some("trace-winterfell".to_string()),
            }],
            tool_responses: vec![capsem_logger::ToolResponseEntry {
                call_id: "tool-winterfell".to_string(),
                content_preview: Some("Wrote winterfell.md".to_string()),
                is_error: false,
                trace_id: Some("trace-winterfell".to_string()),
                credential_ref: Some("credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            }],
        },
    ));
    writer.flush().await;
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();
    let direct_rows = capsem_logger::DbReader::open(&db_path)
        .unwrap()
        .query_raw(
            "SELECT \
             (SELECT COUNT(*) FROM model_calls), \
             (SELECT COUNT(*) FROM net_events), \
             (SELECT COUNT(*) FROM exec_events), \
             (SELECT COUNT(*) FROM security_rule_events)",
        )
        .unwrap();
    assert_eq!(
        direct_rows,
        r#"{"columns":["(SELECT COUNT(*) FROM model_calls)","(SELECT COUNT(*) FROM net_events)","(SELECT COUNT(*) FROM exec_events)","(SELECT COUNT(*) FROM security_rule_events)"],"rows":[[1,1,1,1]]}"#
    );

    hydrate_startup_route_caches(&state).expect("startup hydrates profile route caches");

    let (status, stats) = route_request(app.clone(), axum::http::Method::GET, "/stats", None).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(stats["global"]["total_sessions"], 1);
    assert_eq!(stats["sessions"][0]["id"], "winterfell-vm");

    let (status, detail) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/winterfell-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        detail["model_events"][0]["event_id"], "abcdef123453",
        "{detail}"
    );
    assert_eq!(detail["model_events"][0]["provider"], "openai", "{detail}");
    assert_eq!(detail["model_events"][0]["input_tokens"], 9, "{detail}");
    assert_eq!(
        detail["tool_events"][0]["call_id"], "tool-winterfell",
        "{detail}"
    );
    assert_eq!(detail["tool_events"][0]["tool_name"], "Write", "{detail}");
    assert_eq!(
        detail["body_blobs"]["abcdef123453"][0]["body"],
        r#"{"input":"write winterfell"}"#
    );

    let (status, security) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/winterfell-vm/security/latest?limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{security}");
    assert_eq!(security[0]["event_id"], "abcdef123450");
    assert_eq!(security[0]["turn_id"], "trace-winterfell");
    assert_eq!(
        security[0]["credential_ref"],
        "credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );

    let (status, history) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/winterfell-vm/history/counts",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["exec_count"], 1);

    let (status, timeline) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/winterfell-vm/timeline?trace_id=trace-winterfell&layers=exec,net,model,tool&limit=20",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let rows = timeline["rows"].as_array().unwrap();
    assert!(
        rows.iter()
            .any(|row| row[1] == "exec" && row[3] == "echo winterfell"),
        "{timeline}"
    );
    assert!(
        rows.iter()
            .any(|row| row[1] == "net" && row[3] == "POST mock.capsem.test/v1/responses"),
        "{timeline}"
    );
    assert!(
        rows.iter()
            .any(|row| row[1] == "model" && row[3] == "openai/gpt-5-nano"),
        "{timeline}"
    );
    assert!(
        rows.iter().any(|row| row[1] == "tool"
            && row[3].as_str().is_some_and(
                |summary| summary.contains("Write") && summary.contains("tool-winterfell")
            )),
        "{timeline}"
    );
}

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
    assert_eq!(
        summary.description,
        "Optimized for coding and long-running agents."
    );
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
    assert_eq!(
        refreshed_status["asset_manifest"]["validation_status"],
        "valid"
    );
    assert_eq!(refreshed_status["asset_manifest"]["format"], 2);
    assert_eq!(
        refreshed_status["asset_manifest"]["assets_current"],
        "2099.0101.1"
    );
}

#[test]
fn profile_catalog_status_reports_directory_catalog_readiness() {
    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let state = make_asset_state(dir.path().join("assets"));
    let profile =
        capsem_core::net::policy_config::Profile::load_from_dir(config_root.join("profiles/code"))
            .unwrap();
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
    assert_eq!(
        status["profiles"][0]["missing_assets"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn vm_list_omits_legacy_global_asset_health_when_profiles_are_authoritative() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (config_root, _) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let state = make_asset_state(dir.path().join("assets"));
    let profile =
        capsem_core::net::policy_config::Profile::load_from_dir(config_root.join("profiles/code"))
            .unwrap();
    profile
        .download_assets(
            &state.assets_dir,
            capsem_core::net::policy_config::current_profile_arch(),
        )
        .unwrap();
    let app = build_service_router(state);

    let (status, profiles) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/status",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{profiles}");
    assert_eq!(profiles["ready_count"], 1, "{profiles}");
    assert_eq!(
        profiles["profiles"][0]["missing_assets"],
        json!([]),
        "{profiles}"
    );

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
    let state = make_asset_state(repo_root.join("target/test-empty-assets"));

    let status = profile_catalog_status_value(&state, &catalog);
    let profile_ids = status["profiles"]
        .as_array()
        .expect("profiles array")
        .iter()
        .map(|profile| profile["id"].as_str().expect("profile id").to_string())
        .collect::<Vec<_>>();

    assert_eq!(status["profile_count"], 2);
    assert!(profile_ids.contains(&"code".to_string()), "{profile_ids:?}");
    assert!(
        profile_ids.contains(&"co-work".to_string()),
        "{profile_ids:?}"
    );
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
    insert_fake_instance_with_session_dir(
        &state,
        "runtime-refresh",
        std::process::id(),
        session_dir.clone(),
    );

    state
        .refresh_active_profiles(Some("code"))
        .expect("initial runtime profile materialization");
    let active_profile = session_dir.join("vm/active_profile.toml");
    assert!(
        active_profile.exists(),
        "session must carry one active profile file"
    );
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
        (
            axum::http::Method::GET,
            "/profiles/{profile}/enforcement/info",
        ),
        (
            axum::http::Method::GET,
            "/profiles/{profile}/enforcement/rules/list",
        ),
        (
            axum::http::Method::GET,
            "/profiles/{profile}/detection/info",
        ),
        (
            axum::http::Method::GET,
            "/profiles/{profile}/detection/rules/list",
        ),
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
        (
            axum::http::Method::GET,
            "/profiles/{profile}/mcp/default/info",
        ),
        (
            axum::http::Method::GET,
            "/profiles/{profile}/mcp/servers/list",
        ),
        (axum::http::Method::GET, "/profiles/{profile}/skills/info"),
        (axum::http::Method::GET, "/profiles/{profile}/skills/list"),
    ];

    for profile in ["code", "co-work"] {
        for (method, route) in routes.iter() {
            let path = route.replace("{profile}", profile);
            let (status, body) = route_request(
                build_service_router(Arc::clone(&state)),
                method.clone(),
                &path,
                None,
            )
            .await;
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

    let (status, info) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/skills/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["profile_id"], "code");
    assert_eq!(info["skill_count"], 0);

    let (status, list) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/skills/list",
        None,
    )
    .await;
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

    let (status, list) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/skills/list",
        None,
    )
    .await;
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

    let profile: ProfileConfigFile = toml::from_str(
        &std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap(),
    )
    .unwrap();
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
        json!([
            "code",
            "skills",
            "profile.toml",
            "skill",
            "security",
            "add",
            "applied"
        ]),
        json!([
            "code",
            "skills",
            "profile.toml",
            "skill",
            "review",
            "edit",
            "applied"
        ]),
        json!([
            "code",
            "skills",
            "profile.toml",
            "skill",
            "review",
            "delete",
            "applied"
        ]),
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
    // This settings-owned MCP server must not contribute to
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
    let err = handle_profile_mcp_server_tools(
        State(state),
        Path(("code".to_string(), "settings-only".to_string())),
    )
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
    let unknown_profile =
        handle_profile_plugins_info(State(make_test_state()), Path("strict".to_string()))
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

    let (status, profiles) =
        route_request(app.clone(), axum::http::Method::GET, "/profiles/list", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(profiles["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|profile| profile["id"] == "code" && profile["name"].is_string()));

    let (status, profile) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["profile"]["id"], "code");
    assert!(profile["profile"]["description"].is_string());

    let (status, status_body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/status",
        None,
    )
    .await;
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

    let (status, assets_info) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/assets/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assets_info["profile_id"], "code");
    assert_eq!(assets_info["format"], "profile-assets.v1");
    assert_eq!(
        assets_info["current_assets"]["rootfs"]["name"],
        "rootfs.erofs"
    );
    assert!(
        assets_info.get("filesystem").is_none() && assets_info.get("compression").is_none(),
        "assets route must not expose build-only filesystem/compression metadata: {assets_info}"
    );

    let (status, mcp_info) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/mcp/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mcp_info["profile_id"], "code");
    assert_eq!(mcp_info["manual_server_count"], 0);
    assert_eq!(mcp_info["builtin_local_enabled"], true);

    let (status, settings) =
        route_request(app.clone(), axum::http::Method::GET, "/settings/info", None).await;
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
    std::fs::write(
        profile_dir.join("profile.toml"),
        toml::to_string(&profile).unwrap(),
    )
    .unwrap();
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", &profiles_dir);

    let state = make_test_state();
    let app = build_service_router(state);

    let (status, info) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["obom"]["hash"], format!("blake3:{obom_hash}"));
    assert_eq!(info["obom"]["scope"], "base_image");
    assert_eq!(
        info["obom"]["rootfs_hash"],
        serde_json::json!(profile.assets.current_arch_assets().unwrap().rootfs.hash)
    );
    assert_eq!(info["obom"]["route"], "/profiles/code/obom");

    let (status, obom) =
        route_request(app, axum::http::Method::GET, "/profiles/code/obom", None).await;
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

    let (status, info) =
        route_request(app.clone(), axum::http::Method::GET, "/corp/info", None).await;
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
        .any(|detection| detection["plugin_id"] == "dummy_pre_eicar"
            && detection["plugin_mode"] == "rewrite"));

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
        .any(|detection| detection["plugin_id"] == "dummy_pre_eicar"
            && detection["detection_level"] == "critical"));
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

    let (status, mcp_info) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/mcp/info",
        None,
    )
    .await;
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
        list.plugins
            .iter()
            .any(|plugin| plugin.id == "dummy_pre_eicar"),
        "built-in plugin list must include dummy_pre_eicar"
    );
    assert!(
        list.plugins
            .iter()
            .any(|plugin| plugin.id == "log_sanitizer"),
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
        list.plugins
            .iter()
            .any(|plugin| plugin.stage == PluginStage::Logging),
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
    assert_eq!(
        broker.capabilities.event_families,
        vec!["http", "file", "mcp"]
    );
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
    assert_eq!(
        broker.detail_routes[0].kind,
        PluginDetailRouteKind::CredentialBroker
    );
    assert_eq!(
        broker.detail_routes[0].path,
        "/profiles/code/plugins/credential_broker/credentials/info"
    );
    assert_eq!(
        broker.detail_routes[1].id,
        "credential_broker_credentials_reload"
    );
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
    assert!(default_disabled_detections.iter().any(|detection| {
        detection["source"] == "rule" && detection["rule_id"] == "profiles.rules.eicar"
    }));
    assert!(!default_disabled_detections.iter().any(|detection| {
        detection["source"] == "plugin" && detection["plugin_id"] == "dummy_pre_eicar"
    }));
    assert!(!default_disabled_detections.iter().any(|detection| {
        detection["source"] == "plugin" && detection["plugin_id"] == "dummy_post_allow"
    }));
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
    assert_eq!(
        enabled_event["file"]["import_content"],
        "[capsem-rewritten-eicar]"
    );
    let enabled_detections = enabled_event["detections"].as_array().unwrap();
    assert!(enabled_detections.iter().any(|detection| {
        detection["source"] == "plugin"
            && detection["plugin_id"] == "dummy_pre_eicar"
            && detection["plugin_mode"] == "rewrite"
    }));
    assert!(enabled_detections.iter().any(|detection| {
        detection["source"] == "plugin" && detection["plugin_id"] == "dummy_post_allow"
    }));

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
    assert!(after_disable_detections.iter().any(|detection| {
        detection["source"] == "rule" && detection["rule_id"] == "profiles.rules.eicar"
    }));
    assert!(!after_disable_detections.iter().any(|detection| {
        detection["source"] == "plugin" && detection["plugin_id"] == "dummy_pre_eicar"
    }));

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

    let Json(detail) = handle_profile_credential_broker_credentials_info(
        State(Arc::clone(&state)),
        Path("code".to_string()),
    )
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
    let _store_guard = EnvVarGuard::set(
        "CAPSEM_CREDENTIAL_STORE_PATH",
        dir.path().join("credential-store.json"),
    );
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
    insert_fake_instance_with_session_dir(
        &state,
        "broker-reload-vm",
        std::process::id(),
        session_dir.clone(),
    );

    let credential_ref = capsem_logger::credential_reference("google", "ya29.reload-route");
    let store_json = serde_json::json!({
        capsem_core::credential_broker::credential_store_account(
            capsem_core::credential_broker::CredentialProvider::Google,
            &credential_ref,
        ): "ya29.reload-route"
    });
    std::fs::write(
        &test_store,
        serde_json::to_string_pretty(&store_json).unwrap(),
    )
    .unwrap();

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
}

#[tokio::test]
async fn credential_broker_plugin_runtime_reports_security_ledger_activity() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("broker-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "broker-vm",
        std::process::id(),
        session_dir.clone(),
    );

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
    assert!(direct_rows[0]
        .event_json
        .contains("credential_observations"));
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
    assert_eq!(
        broker["runtime"]["brokered_credentials"][0]["provider"],
        "google"
    );
    assert_eq!(
        broker["runtime"]["brokered_credentials"][0]["observed_count"],
        1
    );
    assert_eq!(
        broker["runtime"]["brokered_credentials"][0]["injected_count"],
        1
    );
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
    let loaded =
        SecurityRuleProfile::parse_toml(&std::fs::read_to_string(&enforcement_path).unwrap())
            .unwrap();
    assert_eq!(
        loaded.profiles.rules["eicar_block"].action,
        capsem_core::net::policy_config::SecurityRuleAction::Block
    );
    let profile_after_save: ProfileConfigFile = toml::from_str(
        &std::fs::read_to_string(config_root.join("profiles/code/profile.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        profile_after_save.files.enforcement.unwrap().hash,
        Some(format!(
            "blake3:{}",
            capsem_core::asset_manager::hash_file(&enforcement_path).unwrap()
        ))
    );

    let Json(reload) =
        handle_enforcement_reload(State(Arc::clone(&state)), Path("code".to_string()))
            .await
            .expect("reload alias should broadcast to zero instances");
    assert_eq!(reload["success"], serde_json::json!(true));
    assert_eq!(reload["reloaded"], serde_json::json!(0));

    let mut bad_priority = rule.clone();
    bad_priority.priority =
        Some(capsem_core::net::policy_config::SecurityRulePriority::Explicit(-100));
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

    let loaded =
        SecurityRuleProfile::parse_toml(&std::fs::read_to_string(&enforcement_path).unwrap())
            .unwrap();
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
    let loaded =
        SecurityRuleProfile::parse_toml(&std::fs::read_to_string(&enforcement_path).unwrap())
            .unwrap();
    assert!(!loaded.profiles.rules.contains_key("eicar_block"));

    let err = handle_enforcement_rule_delete(
        State(state),
        Path(("code".to_string(), "eicar_block".to_string())),
    )
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
    insert_fake_instance_with_session_dir(
        &state,
        "route-ledger-vm",
        std::process::id(),
        session_dir.clone(),
    );

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
    let save_body = to_bytes(save_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&save_body).unwrap();
    assert_eq!(
        saved["compiled_rule_id"],
        "profiles.rules.openai_http_observed"
    );

    let profile =
        capsem_core::net::policy_config::Profile::load_from_dir(config_root.join("profiles/code"))
            .unwrap();
    let compiled = profile
        .config()
        .security_rule_profile_from_files(profile.config_root())
        .unwrap()
        .compile(SecurityRuleSource::User)
        .expect("route-authored rules compile for runtime");
    let rule_set = SecurityRuleSet::new(compiled);
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    let event_id = capsem_core::security_engine::SecurityEventId::parse("abcdef123456")
        .expect("fixed event id is 12 hex");
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
    let latest_body = to_bytes(latest_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let events: Vec<capsem_logger::SecurityRuleEvent> =
        serde_json::from_slice(&latest_body).unwrap();
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
    assert_eq!(
        event.trace_id.as_deref(),
        Some("trace_route_authored_detection")
    );

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
    let detection_body = to_bytes(detection_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let detection_events: Vec<capsem_logger::SecurityRuleEvent> =
        serde_json::from_slice(&detection_body).unwrap();
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
    insert_fake_instance_with_session_dir(
        &state,
        "dry-run-vm",
        std::process::id(),
        session_dir.clone(),
    );
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
    let eval_body = to_bytes(eval_response.into_body(), usize::MAX)
        .await
        .unwrap();
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
    let latest_body = to_bytes(latest_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let events: Vec<capsem_logger::SecurityRuleEvent> =
        serde_json::from_slice(&latest_body).unwrap();
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

    let first_response = handle_enforcement_evaluate(
        State(Arc::clone(&state)),
        Path("code".to_string()),
        body.clone(),
    )
    .await
    .expect("first evaluate");
    let first: serde_json::Value = decode_response_json(first_response).await;
    assert_eq!(first["event"]["event_type"], "file.import");

    {
        assert_eq!(state.evaluate_response_cache.lock().unwrap().len(), 1);
        let mut last = state.evaluate_last_response_cache.lock().unwrap();
        let cached = last.as_mut().expect("last cached evaluate body");
        cached.response_body = Bytes::from_static(br#"{"event":{"event_type":"cached-sentinel"}}"#);
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
    insert_fake_instance_with_session_dir(
        &state,
        "service-ledger-vm",
        std::process::id(),
        session_dir.clone(),
    );

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
                        detection_level: Some(
                            capsem_core::net::policy_config::DetectionLevel::Informational,
                        ),
                        priority: Some(
                            capsem_core::net::policy_config::SecurityRulePriority::Explicit(10),
                        ),
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
        assert_eq!(
            rows[0]["event"]["rule_id"],
            "profiles.rules.service_http_detect"
        );
        assert_eq!(rows[0]["event"]["detection_level"], "informational");
    }

    for uri in [
        "/security/status",
        "/enforcement/status",
        "/detection/status",
    ] {
        let (status, body) = route_request(app.clone(), axum::http::Method::GET, uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(body["total"], 1, "{uri}: {body}");
        assert_eq!(body["sessions"][0]["vm_id"], "service-ledger-vm");
    }
}

#[test]
fn resolve_asset_paths_prefers_erofs_when_present() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vmlinuz"), b"kernel").unwrap();
    std::fs::write(dir.path().join("initrd.img"), b"initrd").unwrap();
    std::fs::write(dir.path().join("rootfs.erofs"), b"erofs").unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let resolved = state.resolve_asset_paths().unwrap();
    assert_eq!(resolved.rootfs, dir.path().join("rootfs.erofs"));
}

#[test]
fn resolve_asset_paths_does_not_accept_squashfs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vmlinuz"), b"kernel").unwrap();
    std::fs::write(dir.path().join("initrd.img"), b"initrd").unwrap();
    std::fs::write(dir.path().join("rootfs.squashfs"), b"squashfs").unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let resolved = state.resolve_asset_paths().unwrap();
    assert_eq!(resolved.rootfs, dir.path().join("rootfs.erofs"));
    assert!(!resolved.rootfs.exists());
}

#[test]
fn asset_status_reports_reconcile_progress_fields() {
    let dir = tempfile::tempdir().unwrap();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = dir.path().join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let profile = materialized_test_profile();
    let arch_assets = profile.assets.current_arch_assets().unwrap();
    for asset in [
        &arch_assets.kernel,
        &arch_assets.initrd,
        &arch_assets.rootfs,
    ] {
        std::fs::write(
            arch_dir.join(profile_asset_hash_name(asset).expect("profile asset hash name")),
            b"asset",
        )
        .unwrap();
    }
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

    let status = profile_asset_status_value(&state, &profile);
    assert_eq!(status["profile_id"], "code");
    assert_eq!(status["manifest"]["origin"], "missing");
    assert_eq!(status["ready"], true);
    assert_eq!(status["downloading"], true);
    assert_eq!(status["current_asset"], "rootfs.erofs");
    assert_eq!(status["bytes_done"], 128);
    assert_eq!(status["bytes_total"], 256);
}

#[test]
fn profile_asset_status_uses_profile_current_arch_contract() {
    let dir = tempfile::tempdir().unwrap();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = dir.path().join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let profile = materialized_test_profile();
    let arch_assets = profile.assets.current_arch_assets().unwrap();
    for asset in [&arch_assets.kernel, &arch_assets.rootfs] {
        let hash = asset
            .hash
            .as_deref()
            .expect("profile asset hash")
            .strip_prefix("blake3:")
            .unwrap();
        let name = capsem_core::asset_manager::hash_filename(&asset.name, hash);
        std::fs::write(arch_dir.join(name), b"asset").unwrap();
    }

    let status = profile_asset_status_value(&state, &profile);

    assert_eq!(status["profile_id"], "code");
    assert_eq!(status["revision"], profile.revision);
    assert_eq!(status["profile_payload_hash"], test_profile_payload_hash());
    assert_eq!(status["current_arch"], arch);
    assert_eq!(status["manifest"]["origin"], "missing");
    assert_eq!(status["ready"], false, "initrd is intentionally missing");
    assert!(
        status.get("filesystem").is_none(),
        "asset status must not expose build filesystem metadata"
    );
    assert!(
        status.get("compression").is_none(),
        "asset status must not expose build compression metadata"
    );
    let assets = status["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 3);
    assert!(assets.iter().any(|asset| {
        asset["kind"] == "kernel"
            && asset["name"] == "vmlinuz"
            && asset["resolved_name"]
                .as_str()
                .is_some_and(|name| name.starts_with("vmlinuz-"))
            && asset["status"] == "present"
            && asset["hash"]
                .as_str()
                .is_some_and(|hash| hash.starts_with("blake3:"))
    }));
    assert!(assets.iter().any(|asset| {
        asset["kind"] == "initrd" && asset["name"] == "initrd.img" && asset["status"] == "missing"
    }));
    assert!(assets.iter().any(|asset| {
        asset["kind"] == "rootfs"
            && asset["name"] == "rootfs.erofs"
            && asset["resolved_name"]
                .as_str()
                .is_some_and(|name| name.starts_with("rootfs-"))
            && asset["status"] == "present"
            && asset.get("compression").is_none()
            && asset.get("compression_level").is_none()
    }));
}

#[test]
fn profile_asset_status_rejects_unmaterialized_asset_descriptors() {
    let dir = tempfile::tempdir().unwrap();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = dir.path().join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let mut profile = ProfileConfigFile::builtin_primary();
    let arch_assets = profile.assets.arch.get_mut(arch).unwrap();

    for asset in [
        &mut arch_assets.kernel,
        &mut arch_assets.initrd,
        &mut arch_assets.rootfs,
    ] {
        std::fs::write(arch_dir.join(&asset.name), b"stale logical asset").unwrap();
        asset.hash = None;
        asset.size = None;
    }

    let status = profile_asset_status_value(&state, &profile);

    assert_eq!(status["ready"], false);
    let assets = status["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 3);
    assert!(assets.iter().all(|asset| asset["status"] == "error"));
    assert!(assets.iter().all(|asset| asset["error"]
        .as_str()
        .is_some_and(|error| error.contains("missing a materialized hash"))));
}

#[test]
fn profile_asset_status_reports_installed_manifest_origin_and_hash() {
    let dir = tempfile::tempdir().unwrap();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    std::fs::create_dir_all(dir.path().join(arch)).unwrap();
    let manifest_json = serde_json::json!({
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": "2026.0609.11",
            "releases": {
                "2026.0609.11": {
                    "date": "2026-06-09",
                    "deprecated": false,
                    "min_binary": "1.0.0",
                    "arches": {}
                }
            }
        },
        "binaries": {
            "current": "1.3.1781035201",
            "releases": {
                "1.3.1781035201": {
                    "date": "2026-06-09",
                    "deprecated": false,
                    "min_assets": "2026.0609.11"
                }
            }
        }
    })
    .to_string();
    let manifest_path = dir.path().join("manifest.json");
    std::fs::write(&manifest_path, manifest_json).unwrap();
    let origin_path = dir.path().join("manifest-origin.json");
    std::fs::write(
        &origin_path,
        serde_json::json!({
            "schema": "capsem.manifest_origin.v1",
            "origin": "package",
            "source": "/tmp/corp/manifest.json",
            "packaged_at": "2026-06-09T12:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
    let expected_hash = capsem_core::asset_manager::hash_file(&manifest_path).unwrap();

    let state = make_asset_state(dir.path().to_path_buf());
    let profile = ProfileConfigFile::builtin_primary();
    let status = profile_asset_status_value(&state, &profile);

    assert_eq!(status["manifest"]["origin"], "package");
    assert_eq!(
        status["manifest"]["path"],
        manifest_path.display().to_string()
    );
    assert_eq!(
        status["manifest"]["origin_path"],
        origin_path.display().to_string()
    );
    assert_eq!(
        status["manifest"]["origin_source"],
        "/tmp/corp/manifest.json"
    );
    assert_eq!(status["manifest"]["packaged_at"], "2026-06-09T12:00:00Z");
    assert_eq!(status["manifest"]["blake3"], expected_hash);
    assert_eq!(status["manifest"]["validation_status"], "valid");
    assert!(status["manifest"]["refreshed_at"].as_str().is_some());
    assert_eq!(status["manifest"]["format"], 2);
    assert_eq!(status["manifest"]["assets_current"], "2026.0609.11");
    assert_eq!(status["manifest"]["binaries_current"], "1.3.1781035201");
}

#[test]
fn profile_asset_status_reports_invalid_manifest_without_stale_truth() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::json!({
            "format": 2,
            "refresh_policy": "24h",
            "assets": {
                "current": "2026.0609.stale",
                "releases": {
                    "2026.0609.stale": {
                        "date": "2026-06-09",
                        "deprecated": false,
                        "min_binary": "1.0.0",
                        "arches": {
                            "arm64": {
                                "vmlinuz": {
                                    "hash": "1111111111111111111111111111111111111111111111111111111111111111",
                                    "size": 1
                                }
                            }
                        }
                    }
                }
            },
            "binaries": {
                "current": "1.3.stale",
                "releases": {
                    "1.3.stale": {
                        "date": "2026-06-09",
                        "deprecated": false,
                        "min_assets": "2026.0609.stale"
                    }
                }
            }
        })
        .to_string(),
    )
    .unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    std::fs::write(&manifest_path, r#"{"format":2}"#).unwrap();

    let profile = ProfileConfigFile::builtin_primary();
    let status = profile_asset_status_value(&state, &profile);

    assert_eq!(status["manifest"]["origin"], "installed");
    assert_eq!(status["manifest"]["validation_status"], "invalid");
    assert!(!status["manifest"]["validation_error"]
        .as_str()
        .unwrap()
        .is_empty());
    assert_eq!(
        status["manifest"]["path"],
        manifest_path.display().to_string()
    );
    assert!(status["manifest"].get("assets_current").is_none());
    assert!(status["manifest"].get("binaries_current").is_none());
}

#[test]
fn asset_cleanup_preserves_profile_catalog_and_persistent_vm_pins() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let profile_dir = tempfile::tempdir().unwrap();
    let (config_root, profile) = install_file_asset_profile_fixture(&profile_dir);
    let catalog = ProfileCatalog::load_from_dir(&config_root.join("profiles")).unwrap();
    let catalog_rootfs = profile_asset_hash_name(
        &profile
            .assets
            .current_arch_assets()
            .expect("built-in profile has current arch assets")
            .rootfs,
    )
    .expect("catalog rootfs hash name");
    let pinned_rootfs = "rootfs-dddddddddddddddd.erofs";
    let disposable_rootfs = "rootfs-1111111111111111.erofs";
    for filename in [catalog_rootfs.as_str(), pinned_rootfs, disposable_rootfs] {
        std::fs::write(base.join(filename), filename.as_bytes()).unwrap();
    }

    let mut pins = test_asset_pins();
    pins.rootfs.hash =
        "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into();
    let registry_path = base.join("persistent_registry.json");
    let mut registry = PersistentRegistry::load(registry_path);
    registry.data.vms.insert(
        "saved-vm".into(),
        PersistentVmEntry {
            id: new_persistent_vm_id(),
            name: "saved-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: pins,
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.0.0".into(),
            created_at: "0".into(),
            session_dir: base.join("persistent/saved-vm"),
            forked_from: None,
            description: None,
            suspended: false,
            defunct: false,
            last_error: None,
            checkpoint_path: None,
            env: None,
        },
    );

    let manifest = capsem_core::asset_manager::ManifestV2 {
        format: 2,
        refresh_policy: "24h".into(),
        asset_base: None,
        assets: capsem_core::asset_manager::AssetsSection {
            current: "empty".into(),
            releases: HashMap::new(),
        },
        binaries: capsem_core::asset_manager::BinariesSection {
            current: "1.0.0".into(),
            releases: HashMap::new(),
        },
    };
    let mut preserve = profile_catalog_asset_filenames(&catalog);
    preserve.extend(persistent_registry_asset_filenames(&registry));

    let removed =
        capsem_core::asset_manager::cleanup_unused_assets_preserving(base, &manifest, preserve)
            .unwrap();

    assert_eq!(removed, vec![base.join(disposable_rootfs)]);
    assert!(base.join(catalog_rootfs).exists());
    assert!(base.join(pinned_rootfs).exists());
    assert!(!base.join(disposable_rootfs).exists());
}

#[test]
fn deprecated_asset_cleanup_preserves_persistent_vm_pins() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let pinned_rootfs = "rootfs-dddddddddddddddd.erofs";
    let deprecated_unpinned_rootfs = "rootfs-eeeeeeeeeeeeeeee.erofs";
    for filename in [pinned_rootfs, deprecated_unpinned_rootfs] {
        std::fs::write(base.join(filename), filename.as_bytes()).unwrap();
    }

    let mut pins = test_asset_pins();
    pins.rootfs.hash =
        "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into();
    let registry_path = base.join("persistent_registry.json");
    let mut registry = PersistentRegistry::load(registry_path);
    registry.data.vms.insert(
        "saved-vm".into(),
        PersistentVmEntry {
            id: new_persistent_vm_id(),
            name: "saved-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: pins,
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.0.0".into(),
            created_at: "0".into(),
            session_dir: base.join("persistent/saved-vm"),
            forked_from: None,
            description: None,
            suspended: false,
            defunct: false,
            last_error: None,
            checkpoint_path: None,
            env: None,
        },
    );

    let manifest = capsem_core::asset_manager::ManifestV2 {
        format: 2,
        refresh_policy: "24h".into(),
        asset_base: None,
        assets: capsem_core::asset_manager::AssetsSection {
            current: "2030.0101.1".into(),
            releases: [(
                "2030.0101.1".into(),
                capsem_core::asset_manager::AssetRelease {
                    date: "2030-01-01".into(),
                    deprecated: true,
                    deprecated_date: Some("2030-01-02".into()),
                    min_binary: "1.0.0".into(),
                    arches: [(
                        "arm64".into(),
                        [
                            (
                                "rootfs.erofs".into(),
                                capsem_core::asset_manager::AssetEntry {
                                    hash: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
                                    size: 1,
                                },
                            ),
                            (
                                "rootfs-pinned.erofs".into(),
                                capsem_core::asset_manager::AssetEntry {
                                    hash: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into(),
                                    size: 1,
                                },
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    )]
                    .into_iter()
                    .collect(),
                },
            )]
            .into_iter()
            .collect(),
        },
        binaries: capsem_core::asset_manager::BinariesSection {
            current: "1.0.0".into(),
            releases: HashMap::new(),
        },
    };
    let preserve = persistent_registry_asset_filenames(&registry);

    let removed =
        capsem_core::asset_manager::cleanup_unused_assets_preserving(base, &manifest, preserve)
            .unwrap();

    assert_eq!(removed, vec![base.join(deprecated_unpinned_rootfs)]);
    assert!(base.join(pinned_rootfs).exists());
    assert!(!base.join(deprecated_unpinned_rootfs).exists());
}

#[test]
fn resolve_profile_asset_paths_uses_profile_hash_prefixed_assets() {
    let dir = tempfile::tempdir().unwrap();
    let profile = materialized_test_profile();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = dir.path().join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let arch_assets = profile.assets.current_arch_assets().unwrap();
    for asset in [
        &arch_assets.kernel,
        &arch_assets.initrd,
        &arch_assets.rootfs,
    ] {
        let hash = asset
            .hash
            .as_deref()
            .expect("profile asset hash")
            .strip_prefix("blake3:")
            .unwrap();
        let name = capsem_core::asset_manager::hash_filename(&asset.name, hash);
        std::fs::write(arch_dir.join(name), b"asset").unwrap();
    }
    let state = make_asset_state(dir.path().to_path_buf());

    let resolved = state.resolve_profile_asset_paths(&profile).unwrap();

    assert!(resolved.kernel.exists());
    assert!(resolved.initrd.exists());
    assert!(resolved.rootfs.exists());
    assert!(resolved.asset_version.starts_with("profile:code@"));
    assert_ne!(resolved.rootfs.file_name().unwrap(), "rootfs.erofs");
}

#[test]
fn vm_asset_block_reason_reports_unmaterialized_profile_asset_pins() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let mut profile = ProfileConfigFile::builtin_primary();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    profile.assets.arch.get_mut(arch).unwrap().rootfs.hash = None;

    let reason = state
        .validate_profile_asset_files(&profile, &test_asset_pins())
        .expect_err("unmaterialized profile asset pins must block VM start");

    assert!(reason.to_string().contains("missing a materialized hash"));
}

#[tokio::test]
async fn ensure_profile_assets_downloads_profile_descriptors() {
    let dir = tempfile::tempdir().unwrap();
    let source_dir = dir.path().join("sources");
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&source_dir).unwrap();

    let mut profile = ProfileConfigFile::builtin_primary();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let replacements = [
        ("kernel", "kernel-bytes".as_bytes()),
        ("initrd", "initrd-bytes".as_bytes()),
        ("rootfs", "rootfs-bytes".as_bytes()),
    ];
    {
        let arch_assets = profile.assets.arch.get_mut(arch).unwrap();
        for (kind, bytes) in replacements {
            let descriptor = match kind {
                "kernel" => &mut arch_assets.kernel,
                "initrd" => &mut arch_assets.initrd,
                "rootfs" => &mut arch_assets.rootfs,
                _ => unreachable!(),
            };
            let source = source_dir.join(&descriptor.name);
            std::fs::write(&source, bytes).unwrap();
            descriptor.url = format!("file://{}", source.display());
            descriptor.hash = Some(format!(
                "blake3:{}",
                capsem_core::asset_manager::hash_file(&source).unwrap()
            ));
            descriptor.size = Some(bytes.len() as u64);
        }
    }
    let state = make_asset_state(assets_dir.clone());

    let downloaded = ensure_profile_assets_for_state(Arc::clone(&state), &profile)
        .await
        .expect("profile ensure should download file fixtures");

    assert_eq!(downloaded, 3);
    let resolved = state.resolve_profile_asset_paths(&profile).unwrap();
    assert!(resolved.kernel.exists());
    assert!(resolved.initrd.exists());
    assert!(resolved.rootfs.exists());
    assert!(
        resolved
            .rootfs
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("rootfs-"),
        "profile ensure stores hash-prefixed assets"
    );
    let reconcile = state.asset_reconcile.lock().unwrap().clone();
    assert_eq!(reconcile.last_downloaded, Some(3));
    assert!(reconcile.last_error.is_none());

    let status = profile_asset_status_value(&state, &profile);
    assert_eq!(status["ready"], true);
    assert_eq!(
        status["profile_payload_hash"],
        profile_payload_hash(&profile).unwrap()
    );
    let assets = status["assets"].as_array().unwrap();
    assert!(assets.iter().all(|asset| asset["status"] == "present"));
    assert!(assets.iter().any(|asset| {
        asset["kind"] == "rootfs"
            && asset["resolved_name"]
                .as_str()
                .is_some_and(|name| name.starts_with("rootfs-"))
    }));

    let downloaded = ensure_profile_assets_for_state(state, &profile)
        .await
        .expect("already verified profile assets should skip download");
    assert_eq!(downloaded, 0);
}

#[tokio::test]
async fn ensure_profile_assets_rejects_unmaterialized_profile_descriptors() {
    let dir = tempfile::tempdir().unwrap();
    let source_dir = dir.path().join("sources");
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&source_dir).unwrap();
    let mut profile = ProfileConfigFile::builtin_primary();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let kernel = &mut profile.assets.arch.get_mut(arch).unwrap().kernel;
    let source = source_dir.join(&kernel.name);
    std::fs::write(&source, b"rootfs").unwrap();
    kernel.url = format!("file://{}", source.display());
    kernel.hash = None;
    kernel.size = None;
    let state = make_asset_state(assets_dir);

    let error = ensure_profile_assets_for_state(Arc::clone(&state), &profile)
        .await
        .expect_err("unmaterialized profile descriptors must not be downloaded");

    assert!(error.contains("missing a materialized hash"));
    let reconcile = state.asset_reconcile.lock().unwrap().clone();
    assert_eq!(reconcile.last_downloaded, Some(0));
    assert!(reconcile
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("missing a materialized hash")));
}

#[test]
fn vm_asset_block_reason_reports_missing_assets() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let profile = materialized_test_profile();
    install_test_profile_catalog(&state, &profile);

    let reason = vm_asset_block_reason(&state, "code").expect("missing assets must block VM start");

    assert!(reason.contains("VM assets are not ready"));
    assert!(reason.contains("vmlinuz"));
    assert!(reason.contains("initrd.img"));
}

#[test]
fn vm_asset_block_reason_reports_downloading_assets() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let profile = materialized_test_profile();
    install_test_profile_catalog(&state, &profile);
    state.asset_reconcile.lock().unwrap().in_progress = true;

    let reason = vm_asset_block_reason(&state, "code").expect("missing assets must block VM start");

    assert!(reason.contains("VM assets are still downloading"));
}

#[test]
fn vm_asset_block_reason_allows_ready_assets() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    install_test_profile_assets(&state);

    assert!(vm_asset_block_reason(&state, "code").is_none());
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
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: PathBuf::from("/nonexistent/assets"),
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest: None,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        profile_summary_cache: Mutex::new(test_profile_summary_cache()),
        profile_cache: Mutex::new(test_profile_cache()),
        profile_status_cache: Mutex::new(None),
        profile_rule_cache: test_profile_rule_cache(),
        profile_plugin_policy_cache: test_profile_plugin_policy_cache(),
        mcp_tool_cache: Mutex::new(capsem_core::mcp::load_tool_cache()),
        profile_mutation_db: test_profile_mutation_db(&run_dir),
        last_defunct_reconcile_ms: AtomicU64::new(0),
        stats_response_cache: Mutex::new(None),
        stats_detail_response_cache: Mutex::new(HashMap::new()),
        storage_diagnostics_cache: Mutex::new(HashMap::new()),
        persistent_resume_state_cache: Mutex::new(HashMap::new()),
        evaluate_rule_cache: Mutex::new(HashMap::new()),
        profile_rule_response_cache: Mutex::new(HashMap::new()),
        profile_plugin_response_cache: Mutex::new(HashMap::new()),
        evaluate_response_cache: Mutex::new(HashMap::new()),
        evaluate_last_response_cache: Mutex::new(None),
        save_restore_lock: tokio::sync::RwLock::new(()),
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
    let json = serde_json::json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert!(req.name.is_none());
}

#[test]
fn provision_request_rejects_missing_profile_id() {
    let json = serde_json::json!({"ram_mb": 2048, "cpus": 2});
    let err = serde_json::from_value::<ProvisionRequest>(json).unwrap_err();
    assert!(err.to_string().contains("profile_id"));
}

#[test]
fn provision_request_empty_name() {
    let json = serde_json::json!({"name": "", "profile_id": "code", "ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.name.unwrap(), "");
}

#[test]
fn provision_request_name_with_path_separator() {
    // This is a security edge case -- names with / could create path traversal
    let json =
        serde_json::json!({"name": "../escape", "profile_id": "code", "ram_mb": 2048, "cpus": 2});
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
        name: &ok_name,
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
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
        name: "my-vm",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
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

#[test]
fn provision_rejects_unknown_profile_before_boot() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "my-vm",
        name: "my-vm",
        profile_id: "missing-profile".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: false,
        env: None,
        from: None,
        description: None,
    });
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("profile not found: missing-profile"),
        "unknown profile must fail before boot, got: {err}"
    );
    assert!(
        !state.run_dir.join("sessions/my-vm").exists(),
        "unknown profile must not create session state"
    );
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
                id: new_persistent_vm_id(),
                name: "taken".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
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
        name: "taken",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
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

#[tokio::test]
async fn purge_default_removes_defunct_persistent_and_keeps_healthy_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().join("assets"));
    let defunct_dir = state.run_dir.join("persistent/defunct-vm");
    let healthy_dir = state.run_dir.join("persistent/healthy-vm");
    std::fs::create_dir_all(&defunct_dir).unwrap();
    std::fs::create_dir_all(&healthy_dir).unwrap();
    std::fs::write(defunct_dir.join("process.log"), "boot failed").unwrap();
    std::fs::write(healthy_dir.join("process.log"), "stopped cleanly").unwrap();

    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "defunct-vm".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "defunct-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: defunct_dir.clone(),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: true,
                last_error: Some("boot failed".into()),
                checkpoint_path: None,
                env: None,
            },
        );
        reg.data.vms.insert(
            "healthy-vm".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "healthy-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: healthy_dir.clone(),
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

    let app = build_service_router(Arc::clone(&state));
    let (status, body) = route_request(
        app,
        axum::http::Method::POST,
        "/purge",
        Some(json!({ "all": false })),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["purged"], 1);
    assert_eq!(body["persistent_purged"], 1);
    assert_eq!(body["ephemeral_purged"], 0);

    let registry = state.persistent_registry.lock().unwrap();
    assert!(registry.get("defunct-vm").is_none());
    assert!(registry.get("healthy-vm").is_some());
    assert!(!defunct_dir.exists());
    assert!(healthy_dir.exists());
}

#[test]
fn provision_persistent_validates_name() {
    let state = make_test_state();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "../evil",
        name: "../evil",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
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
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: dir.path().join("assets"),
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest: None,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        profile_summary_cache: Mutex::new(test_profile_summary_cache()),
        profile_cache: Mutex::new(test_profile_cache()),
        profile_status_cache: Mutex::new(None),
        profile_rule_cache: test_profile_rule_cache(),
        profile_plugin_policy_cache: test_profile_plugin_policy_cache(),
        mcp_tool_cache: Mutex::new(capsem_core::mcp::load_tool_cache()),
        profile_mutation_db: test_profile_mutation_db(&run_dir),
        last_defunct_reconcile_ms: AtomicU64::new(0),
        stats_response_cache: Mutex::new(None),
        stats_detail_response_cache: Mutex::new(HashMap::new()),
        storage_diagnostics_cache: Mutex::new(HashMap::new()),
        persistent_resume_state_cache: Mutex::new(HashMap::new()),
        evaluate_rule_cache: Mutex::new(HashMap::new()),
        profile_rule_response_cache: Mutex::new(HashMap::new()),
        profile_plugin_response_cache: Mutex::new(HashMap::new()),
        evaluate_response_cache: Mutex::new(HashMap::new()),
        evaluate_last_response_cache: Mutex::new(None),
        save_restore_lock: tokio::sync::RwLock::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    });
    (state, dir)
}

#[tokio::test]
async fn handle_fork_creates_persistent_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    // Create a real session dir for the fake instance
    let session_dir = state.run_dir.join("sessions/fork-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.instances.lock().unwrap().insert(
        "fork-src".into(),
        InstanceInfo {
            id: "fork-src".into(),
            name: "fork-src".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
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
    assert_ne!(result.0.id, "my-fork");
    uuid::Uuid::parse_str(&result.0.id).expect("fork response id should be a UUID");
    assert_eq!(result.0.name, "my-fork");
    assert!(result.0.size_bytes > 0);
    // Verify fork created a persistent sandbox entry in the registry
    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("my-fork").unwrap();
    assert_eq!(entry.profile_id, "code");
    assert_eq!(entry.profile_revision, test_profile_revision());
    assert_eq!(entry.profile_payload_hash, test_profile_payload_hash());
    assert_eq!(entry.asset_pins, test_asset_pins());
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
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("sessions/dup-src");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    state.instances.lock().unwrap().insert(
        "dup-src".into(),
        InstanceInfo {
            id: "dup-src".into(),
            name: "dup-src".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
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
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/pers-vm");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "pers-vm".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "pers-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
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
        State(state.clone()),
        Path(vm_id),
        Json(ForkRequest {
            name: "from-pers".into(),
            description: None,
        }),
    )
    .await
    .unwrap();
    assert_ne!(result.0.id, "from-pers");
    uuid::Uuid::parse_str(&result.0.id).expect("fork response id should be a UUID");
    assert_eq!(result.0.name, "from-pers");
    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("from-pers").unwrap();
    assert_eq!(entry.profile_id, "code");
    assert_eq!(entry.profile_revision, test_profile_revision());
    assert_eq!(entry.profile_payload_hash, test_profile_payload_hash());
    assert_eq!(entry.asset_pins, test_asset_pins());
}

#[tokio::test]
async fn handle_persist_preserves_profile_identity() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("sessions/persist-src");
    std::fs::create_dir_all(&session_dir).unwrap();
    state.instances.lock().unwrap().insert(
        "persist-src".into(),
        InstanceInfo {
            id: "persist-src".into(),
            name: "persist-src".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: std::process::id(),
            uds_path: PathBuf::from("/tmp/persist-src.sock"),
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

    let _ = handle_persist(
        State(state.clone()),
        Path("persist-src".into()),
        Json(PersistRequest {
            name: "persisted".into(),
        }),
    )
    .await
    .unwrap();

    let registry = state.persistent_registry.lock().unwrap();
    let entry = registry.get("persisted").unwrap();
    assert_eq!(entry.id, "persist-src");
    assert_eq!(entry.name, "persisted");
    assert_eq!(entry.profile_id, "code");
    assert_eq!(entry.profile_revision, test_profile_revision());
    assert_eq!(entry.profile_payload_hash, test_profile_payload_hash());
    assert_eq!(entry.asset_pins, test_asset_pins());
    drop(registry);

    let instances = state.instances.lock().unwrap();
    let info = instances.get("persist-src").unwrap();
    assert_eq!(info.id, "persist-src");
    assert_eq!(info.profile_id, "code");
    assert_eq!(info.profile_revision, test_profile_revision());
    assert_eq!(info.profile_payload_hash, test_profile_payload_hash());
    assert_eq!(info.asset_pins, test_asset_pins());
    assert!(info.persistent);
}

#[test]
fn resume_rejects_profile_revision_drift() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/revision-drift");
    std::fs::create_dir_all(&session_dir).unwrap();
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "revision-drift".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "revision-drift".into(),
                profile_id: "code".into(),
                profile_revision: "old-revision".into(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
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

    let err = state.resume_sandbox(&vm_id, None, None).unwrap_err();
    assert!(
        err.to_string().contains("revision mismatch"),
        "resume must fail closed on profile revision drift, got: {err}"
    );
}

#[test]
fn resume_rejects_profile_payload_hash_drift() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/payload-hash-drift");
    std::fs::create_dir_all(&session_dir).unwrap();
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "payload-hash-drift".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "payload-hash-drift".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash:
                    "blake3:0000000000000000000000000000000000000000000000000000000000000000"
                        .into(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
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

    let err = state.resume_sandbox(&vm_id, None, None).unwrap_err();
    assert!(
        err.to_string().contains("payload hash mismatch"),
        "resume must fail closed on profile payload hash drift, got: {err}"
    );
}

#[tokio::test]
async fn handle_fork_rejects_asset_pin_drift() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/pin-drift");
    std::fs::create_dir_all(session_dir.join("system")).unwrap();
    std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
    std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
    let mut pins = test_asset_pins();
    pins.rootfs.hash =
        "blake3:0000000000000000000000000000000000000000000000000000000000000000".into();
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "pin-drift".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "pin-drift".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: pins,
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
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

    let err = handle_fork(
        State(state),
        Path(vm_id),
        Json(ForkRequest {
            name: "blocked-fork".into(),
            description: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.0, StatusCode::PRECONDITION_FAILED);
    assert!(
        err.1.contains("asset pins changed"),
        "fork must fail closed on asset pin drift, got: {}",
        err.1
    );
}

#[test]
fn provision_rejects_nonexistent_source_sandbox() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "vm1",
        name: "vm1",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
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

#[test]
fn provision_rejects_source_with_different_profile() {
    let (state, _dir) = make_test_state_with_tempdir();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "other-profile-source".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "other-profile-source".into(),
                profile_id: "other-profile".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: PathBuf::from("/tmp/other-profile-source"),
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
        id: "vm1",
        name: "vm1",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: false,
        env: None,
        from: Some("other-profile-source".into()),
        description: None,
    });
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("uses profile 'other-profile', not 'code'"),
        "source profile mismatch must fail, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Suspend/resume registry fixes (issues #4-8)
// -----------------------------------------------------------------------

#[tokio::test]
async fn handle_list_shows_suspended_status() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let suspended_dir = state.run_dir.join("persistent/susp-vm");
    let stopped_dir = state.run_dir.join("persistent/stop-vm");
    capsem_core::create_virtiofs_session(&suspended_dir, 64).unwrap();
    capsem_core::create_virtiofs_session(&stopped_dir, 64).unwrap();

    // Register a suspended persistent VM
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "susp-vm".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "susp-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: suspended_dir,
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
                id: new_persistent_vm_id(),
                name: "stop-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 1024,
                cpus: 1,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: stopped_dir,
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

    let susp = list
        .sandboxes
        .iter()
        .find(|s| s.name.as_deref() == Some("susp-vm"))
        .unwrap();
    assert_ne!(susp.id, "susp-vm");
    assert_eq!(
        susp.status,
        VmLifecycleState::Suspended,
        "suspended VM should show Suspended status"
    );

    let stop = list
        .sandboxes
        .iter()
        .find(|s| s.name.as_deref() == Some("stop-vm"))
        .unwrap();
    assert_ne!(stop.id, "stop-vm");
    assert_eq!(
        stop.status,
        VmLifecycleState::Stopped,
        "non-suspended VM should show Stopped status"
    );
}

#[tokio::test]
async fn handle_info_shows_suspended_status() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/info-susp");
    capsem_core::create_virtiofs_session(&session_dir, 64).unwrap();

    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "info-susp".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "info-susp".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
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

    let result = handle_info(State(state), Path(vm_id)).await;
    let Json(info) = result.unwrap();
    assert_eq!(info.status, VmLifecycleState::Suspended);
}

#[tokio::test]
async fn handle_info_reports_storage_diagnostics_for_persistent_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/storage-info");
    std::fs::create_dir_all(session_dir.join("guest/system")).unwrap();
    let rootfs = session_dir.join("guest/system/rootfs.img");
    let file = std::fs::File::create(&rootfs).unwrap();
    file.set_len(8 * 1024 * 1024 * 1024).unwrap();

    let entry = test_persistent_entry("storage-info", session_dir.clone());
    let vm_id = entry.id.clone();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert("storage-info".into(), entry);
    }

    let Json(info) = handle_info(State(state), Path(vm_id)).await.unwrap();
    let storage = info.storage.expect("info must include storage diagnostics");
    assert_eq!(
        storage.rootfs_image_path,
        rootfs.to_string_lossy().to_string()
    );
    assert_eq!(storage.rootfs_image_logical_bytes, 8 * 1024 * 1024 * 1024);
    assert!(
        storage.rootfs_image_physical_bytes < storage.rootfs_image_logical_bytes,
        "sparse rootfs image should report allocated blocks separately from logical size"
    );
    assert!(storage.host_available_bytes > 0);
    assert_eq!(storage.guest_overlay_device, "/dev/vdb");
    assert_eq!(storage.guest_overlay_mount, "/");
}

#[tokio::test]
async fn handle_vm_status_reports_storage_diagnostics_for_persistent_vm() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/storage-status");
    capsem_core::create_virtiofs_session(&session_dir, 4).unwrap();
    let rootfs = session_dir.join("guest/system/rootfs.img");

    let entry = test_persistent_entry("storage-status", session_dir);
    let vm_id = entry.id.clone();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert("storage-status".into(), entry);
    }

    let Json(status) = handle_vm_status(State(state), Path(vm_id)).await.unwrap();
    let storage = status
        .storage
        .expect("status must include storage diagnostics");
    assert_eq!(
        storage.rootfs_image_path,
        rootfs.to_string_lossy().to_string()
    );
    assert_eq!(storage.rootfs_image_logical_bytes, 4 * 1024 * 1024 * 1024);
    assert!(storage.host_free_bytes > 0);
    assert_eq!(storage.guest_overlay_device, "/dev/vdb");
    assert_eq!(storage.guest_overlay_mount, "/");
}

#[tokio::test]
async fn handle_list_marks_profile_payload_drift_incompatible() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "payload-drift".into(),
            PersistentVmEntry {
            id: new_persistent_vm_id(),
                name: "payload-drift".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash:
                    "blake3:0000000000000000000000000000000000000000000000000000000000000000"
                        .into(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/payload-drift"),
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
    let vm = list
        .sandboxes
        .iter()
        .find(|s| s.name.as_deref() == Some("payload-drift"))
        .unwrap();
    assert_ne!(vm.id, "payload-drift");
    assert_eq!(vm.status, VmLifecycleState::Incompatible);
    assert!(!vm.can_resume);
    assert!(vm
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("payload hash mismatch"));
}

#[tokio::test]
async fn handle_info_marks_profile_payload_drift_incompatible() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "payload-drift-info".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "payload-drift-info".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash:
                    "blake3:0000000000000000000000000000000000000000000000000000000000000000"
                        .into(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/payload-drift-info"),
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

    let Json(info) = handle_info(State(state), Path(vm_id)).await.unwrap();
    assert_eq!(info.status, VmLifecycleState::Incompatible);
    assert!(!info.can_resume);
    assert!(info
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("payload hash mismatch"));
}

#[tokio::test]
async fn handle_list_marks_profile_rootfs_size_drift_incompatible() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_profile_assets(&state);
    let session_dir = state.run_dir.join("persistent/rootfs-size-drift");
    capsem_core::create_virtiofs_session(&session_dir, 2).unwrap();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "rootfs-size-drift".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "rootfs-size-drift".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
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

    let Json(list) = handle_list(State(state.clone())).await;
    let vm = list
        .sandboxes
        .iter()
        .find(|s| s.name.as_deref() == Some("rootfs-size-drift"))
        .unwrap();
    assert_ne!(vm.id, "rootfs-size-drift");
    assert_eq!(vm.status, VmLifecycleState::Incompatible);
    assert!(!vm.can_resume);
    let reason = vm.resume_blocked_reason.as_deref().unwrap_or_default();
    assert!(
        reason.contains("rootfs.img logical size mismatch"),
        "{reason}"
    );
    assert!(reason.contains("2 GiB"), "{reason}");
    assert!(reason.contains("64 GiB"), "{reason}");
    assert_eq!(
        vm.available_actions,
        VmLifecycleState::Incompatible.available_actions(false)
    );

    let Json(info) = handle_info(State(state.clone()), Path(vm.id.clone()))
        .await
        .unwrap();
    assert_eq!(info.status, VmLifecycleState::Incompatible);
    assert!(!info.can_resume);
    assert!(info
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("rootfs.img logical size mismatch"));

    let Json(status) = handle_vm_status(State(state), Path(vm.id.clone()))
        .await
        .unwrap();
    assert_eq!(status.status, VmLifecycleState::Incompatible);
    assert!(!status.can_resume);
    assert!(status
        .resume_blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("rootfs.img logical size mismatch"));
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
async fn handle_suspend_rejects_ephemeral_vm() {
    let (state, _dir) = make_test_state_with_tempdir();

    // Insert an ephemeral VM in instances
    {
        let mut instances = state.instances.lock().unwrap();
        instances.insert(
            "eph-vm".into(),
            InstanceInfo {
                id: "eph-vm".into(),
                name: "eph-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
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
    let complete = session_dir.join("checkpoint.vzsave.complete");
    std::fs::write(&checkpoint, b"bad checkpoint").unwrap();
    std::fs::write(&complete, b"ok\n").unwrap();

    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "resume-vm".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "resume-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
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
        .archive_failed_restore_checkpoint(&vm_id)
        .expect("checkpoint should be archived");

    assert!(!checkpoint.exists(), "original checkpoint must be moved");
    assert!(!complete.exists(), "completion marker must be moved");
    assert!(
        archived.exists(),
        "archived checkpoint should exist: {}",
        archived.display()
    );
    let archived_complete = session_dir.join(format!(
        "{}.complete",
        archived.file_name().unwrap().to_string_lossy()
    ));
    assert!(
        archived_complete.exists(),
        "archived completion marker should exist: {}",
        archived_complete.display()
    );
    assert!(archived
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("checkpoint.vzsave.failed-restore-"));
}

#[test]
fn existing_resume_checkpoint_requires_completion_marker() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/resume-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let checkpoint = session_dir.join("checkpoint.vzsave");
    let complete = session_dir.join("checkpoint.vzsave.complete");
    std::fs::write(&checkpoint, b"partial checkpoint").unwrap();

    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "resume-vm".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "resume-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
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

    assert!(
        !state.has_existing_resume_checkpoint(&vm_id),
        "bare checkpoint without completion marker must not be resumable"
    );

    std::fs::write(&complete, b"ok\n").unwrap();
    assert!(
        state.has_existing_resume_checkpoint(&vm_id),
        "checkpoint with completion marker should be resumable"
    );
}

#[test]
fn clear_resume_checkpoint_removes_completion_marker() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("persistent/resume-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let complete = session_dir.join("checkpoint.vzsave.complete");
    std::fs::write(session_dir.join("checkpoint.vzsave"), b"checkpoint").unwrap();
    std::fs::write(&complete, b"ok\n").unwrap();

    let vm_id = new_persistent_vm_id();
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "resume-vm".into(),
            PersistentVmEntry {
                id: vm_id.clone(),
                name: "resume-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir,
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

    state.clear_resume_checkpoint(&vm_id);
    assert!(
        !complete.exists(),
        "completion marker must be removed once checkpoint state is cleared"
    );
    let reg = state.persistent_registry.lock().unwrap();
    let entry = reg.get("resume-vm").unwrap();
    assert!(!entry.suspended);
    assert!(entry.checkpoint_path.is_none());
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

#[test]
fn profile_mutation_db_startup_initializes_session_index_schema() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();

    let handle = ServiceState::open_profile_mutation_db_handle(&run_dir).unwrap();
    drop(handle);

    let db_path = main_db_path_for_run_dir(&run_dir);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(session_count, 0);

    let mutation_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM profile_mutation_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(mutation_count, 0);
}

#[test]
fn session_index_start_records_uuid_id_not_display_name() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir) = make_test_state_with_tempdir_at(dir);
    let id = new_persistent_vm_id();
    uuid::Uuid::parse_str(&id).expect("VM route id should be a UUID");
    let display_name = "code-1";

    state
        .record_session_index_start(
            &id,
            false,
            16,
            2048,
            Some("blake3:abc"),
            Some("1.3.1782496403"),
            None,
        )
        .unwrap();

    let conn = rusqlite::Connection::open(state.main_db_path()).unwrap();
    let by_id: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    let by_name: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            [display_name],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(by_id, 1);
    assert_eq!(by_name, 0);
}

// -----------------------------------------------------------------------
// SandboxInfo::new
// -----------------------------------------------------------------------

#[test]
fn sandbox_info_new_defaults_telemetry_to_none() {
    let info = SandboxInfo::new(
        "test".into(),
        "code".into(),
        1,
        VmLifecycleState::Running,
        false,
    );
    assert_eq!(info.id, "test");
    assert_eq!(info.pid, 1);
    assert!(!info.persistent);
    assert!(info.total_input_tokens.is_none());
    assert!(info.total_estimated_cost.is_none());
    assert!(info.model_call_count.is_none());
    assert!(info.created_at.is_none());
    assert!(info.uptime_secs.is_none());
}

#[tokio::test]
async fn vm_list_and_info_are_in_memory_only() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/list-hot-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let file_event = capsem_logger::FileEvent {
        event_id: Some("abcdef123456".into()),
        timestamp: std::time::SystemTime::now(),
        action: capsem_logger::FileAction::Created,
        path: "/root/list-hot-proof.txt".into(),
        size: Some(12),
        trace_id: Some("tracelisthot".into()),
        credential_ref: None,
    };
    let db_path = session_dir.join("session.db");
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
        writer.write_blocking(capsem_logger::WriteOp::FileEvent(file_event));
        writer.shutdown_blocking();
    })
    .await
    .unwrap();
    insert_fake_instance_with_session_dir(&state, "list-hot-vm", 4242, session_dir);

    let Json(list) = handle_list(State(Arc::clone(&state))).await;
    let listed = list
        .sandboxes
        .iter()
        .find(|vm| vm.id == "list-hot-vm")
        .expect("running VM listed");
    assert!(
        listed.total_input_tokens.is_none(),
        "/vms/list is a hot route and must not read session.db telemetry"
    );
    assert!(listed.model_call_count.is_none());

    let Json(info) = handle_info(State(state), Path("list-hot-vm".into()))
        .await
        .expect("detail route stays lifecycle/storage only");
    assert!(
        info.total_file_events.is_none(),
        "/vms/{{id}}/info must not inline raw telemetry SQL; use ledger DB APIs"
    );
    assert!(info.model_call_count.is_none());
}

#[test]
fn vm_lifecycle_available_actions_are_contractual() {
    use api::VmAction;

    assert_eq!(
        VmLifecycleState::Running.available_actions(false),
        vec![
            VmAction::Pause,
            VmAction::Stop,
            VmAction::Fork,
            VmAction::Delete
        ]
    );
    assert_eq!(
        VmLifecycleState::Stopped.available_actions(true),
        vec![VmAction::Start, VmAction::Fork, VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Stopped.available_actions(false),
        vec![VmAction::Fork, VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Suspended.available_actions(true),
        vec![VmAction::Resume, VmAction::Fork, VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Suspended.available_actions(false),
        vec![VmAction::Fork, VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Defunct.available_actions(false),
        vec![VmAction::Delete]
    );
    assert_eq!(
        VmLifecycleState::Incompatible.available_actions(false),
        vec![VmAction::Delete]
    );
}

#[test]
fn sandbox_info_telemetry_fields_serialize_when_present() {
    let mut info = SandboxInfo::new(
        "test".into(),
        "code".into(),
        1,
        VmLifecycleState::Running,
        false,
    );
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
    let info = SandboxInfo::new(
        "test".into(),
        "code".into(),
        1,
        VmLifecycleState::Running,
        false,
    );
    let json = serde_json::to_string(&info).unwrap();
    assert!(!json.contains("total_input_tokens"));
    assert!(!json.contains("total_estimated_cost"));
    assert!(!json.contains("model_call_count"));
    assert!(!json.contains("uptime_secs"));
}

#[test]
fn sandbox_info_rejects_missing_profile_id() {
    let json = r#"{"id":"x","pid":1,"status":"Running","persistent":false}"#;
    let err = serde_json::from_str::<SandboxInfo>(json).unwrap_err();
    assert!(err.to_string().contains("profile_id"));
}

#[test]
fn profile_vm_resources_drive_new_session_defaults() {
    let profile = ProfileConfigFile::builtin_primary();

    let default_resources = resolve_profile_vm_resources(&profile, None, None);
    assert_eq!(default_resources.cpus, profile.vm.cpu_count);
    assert_eq!(default_resources.ram_mb, profile.vm.ram_gb as u64 * 1024);
    assert_eq!(
        default_resources.scratch_disk_size_gb,
        profile.vm.scratch_disk_size_gb
    );

    let customized_resources = resolve_profile_vm_resources(&profile, Some(3072), Some(2));
    assert_eq!(customized_resources.cpus, 2);
    assert_eq!(customized_resources.ram_mb, 3072);
    assert_eq!(
        customized_resources.scratch_disk_size_gb, profile.vm.scratch_disk_size_gb,
        "scratch image size is profile-owned and must not fall back to hidden service defaults"
    );
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
async fn db_boundary_route_contract_handle_stats_returns_global_data() {
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
    if let Err(error) = &result {
        panic!("stats route must read seeded main.db rows through the logger DB handle: {error:?}");
    }
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(resp["global"]["total_sessions"], 1);
    assert_eq!(resp["global"]["total_input_tokens"], 10000);
    assert_eq!(resp["global"]["total_estimated_cost"], 0.42);
    assert_eq!(resp["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(resp["sessions"][0]["id"], "20260412-120000-abcd");
}

#[tokio::test]
async fn stats_detail_route_reads_session_db_ledger() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("stats-detail-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "stats-detail-vm",
        std::process::id(),
        session_dir.clone(),
    );

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::ModelCall(
        capsem_logger::ModelCall {
            event_id: Some("abc123abc123".to_string()),
            timestamp: std::time::SystemTime::now(),
            provider: "google".to_string(),
            protocol: Some("google".to_string()),
            model: Some("gemini-3.5-flash".to_string()),
            process_name: Some("agy".to_string()),
            pid: Some(42),
            method: "POST".to_string(),
            path: "/v1internal:streamGenerateContent".to_string(),
            stream: true,
            system_prompt_preview: None,
            messages_count: 1,
            tools_count: 1,
            request_bytes: 32,
            request_body_preview: Some(r#"{"contents":[{"text":"write"}]}"#.to_string()),
            request_body_full: Some(
                r#"{"contents":[{"text":"write full bounded body"}]}"#.to_string(),
            ),
            message_id: Some("msg-1".to_string()),
            status_code: Some(200),
            text_content: Some("created poem.md".to_string()),
            thinking_content: Some("plan file write".to_string()),
            response_body_full: Some(
                r#"{"candidates":[{"content":{"parts":[{"text":"created poem.md"}]}}]}"#
                    .to_string(),
            ),
            stop_reason: Some("end_turn".to_string()),
            input_tokens: Some(12),
            output_tokens: Some(7),
            usage_details: BTreeMap::new(),
            duration_ms: 25,
            response_bytes: 64,
            estimated_cost_usd: 0.001,
            trace_id: Some("trace-stats-detail".to_string()),
            credential_ref: None,
            tool_calls: vec![capsem_logger::ToolCallEntry {
                call_index: 0,
                call_id: "tool-1".to_string(),
                tool_name: "Create".to_string(),
                arguments: Some(r#"{"path":"/root/poem.md"}"#.to_string()),
                origin: "native".to_string(),
                trace_id: Some("trace-stats-detail".to_string()),
            }],
            tool_responses: vec![capsem_logger::ToolResponseEntry {
                call_id: "tool-1".to_string(),
                content_preview: Some("Wrote 4 lines to poem.md".to_string()),
                is_error: false,
                trace_id: Some("trace-stats-detail".to_string()),
                credential_ref: None,
            }],
        },
    ));
    writer
        .write(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
            event_id: Some("def456def456".to_string()),
            timestamp: std::time::SystemTime::now(),
            domain: "generativelanguage.googleapis.com".to_string(),
            port: 443,
            decision: capsem_logger::Decision::Allowed,
            process_name: Some("agy".to_string()),
            pid: Some(42),
            method: Some("POST".to_string()),
            path: Some("/v1internal:streamGenerateContent".to_string()),
            query: None,
            status_code: Some(200),
            bytes_sent: 32,
            bytes_received: 64,
            duration_ms: 21,
            matched_rule: Some("profiles.rules.ai_google_http_googleapis".to_string()),
            request_headers: Some("content-type: application/json".to_string()),
            response_headers: Some("content-type: application/json".to_string()),
            request_body_preview: Some(r#"{"model":"gemini-3.5-flash"}"#.to_string()),
            response_body_preview: Some(r#"{"ok":true}"#.to_string()),
            request_body_full: Some(
                r#"{"model":"gemini-3.5-flash","contents":[{"text":"write full body"}]}"#
                    .to_string(),
            ),
            response_body_full: Some(
                r#"{"ok":true,"body":"full response body from gateway"}"#.to_string(),
            ),
            conn_type: Some("https".to_string()),
            policy_mode: None,
            policy_action: Some("allow".to_string()),
            policy_rule: Some("profiles.rules.ai_google_http_googleapis".to_string()),
            policy_reason: None,
            trace_id: Some("trace-stats-detail".to_string()),
            credential_ref: None,
        }))
        .await;
    writer.shutdown_blocking();

    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/stats-detail-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["model_stats"][0]["provider"], "google");
    assert_eq!(body["model_stats"][0]["model"], "gemini-3.5-flash");
    assert_eq!(body["model_stats"][0]["call_count"], 1);
    assert_eq!(body["model_stats"][0]["input_tokens"], 12);
    assert_eq!(body["model_stats"][0]["output_tokens"], 7);
    assert_eq!(body["model_events"][0]["event_id"], "abc123abc123");
    assert_eq!(body["model_events"][0]["input_tokens"], 12);
    assert_eq!(
        body["model_events"].as_array().unwrap().len(),
        body["model_stats"][0]["call_count"].as_u64().unwrap() as usize,
        "model_stats.call_count must agree with model_events"
    );
    assert!(body["model_events"][0]
        .get("request_body_preview")
        .is_none());
    assert!(body["model_events"][0]
        .get("response_body_preview")
        .is_none());
    assert_eq!(body["tool_events"][0]["tool_name"], "Create");
    assert_eq!(body["tool_events"][0]["call_id"], "tool-1");
    assert_eq!(body["tool_events"][0]["source"], "native");
    assert_eq!(body["tool_events"][0]["model_parent_missing"], 0);
    assert!(body["tool_events"][0]["model_call_id"].as_i64().is_some());
    assert_eq!(
        body["tool_events"][0]["arguments"],
        r#"{"path":"/root/poem.md"}"#
    );
    assert_eq!(
        body["tool_events"][0]["response_preview"],
        "Wrote 4 lines to poem.md"
    );
    assert_eq!(body["http_events"][0]["event_id"], "def456def456");
    assert_eq!(
        body["http_events"][0]["domain"],
        "generativelanguage.googleapis.com"
    );
    assert!(body["http_events"][0].get("request_body_preview").is_none());
    assert!(body["http_events"][0]
        .get("response_body_preview")
        .is_none());
    assert_eq!(
        body["body_blobs"]["abc123abc123"][0]["direction"],
        "request"
    );
    assert_eq!(
        body["body_blobs"]["abc123abc123"][0]["body"],
        r#"{"contents":[{"text":"write full bounded body"}]}"#
    );
    assert_eq!(
        body["body_blobs"]["abc123abc123"][1]["direction"],
        "response"
    );
    assert_eq!(
        body["body_blobs"]["abc123abc123"][1]["body"],
        r#"{"candidates":[{"content":{"parts":[{"text":"created poem.md"}]}}]}"#
    );
    assert_eq!(
        body["body_blobs"]["def456def456"][0]["direction"],
        "request"
    );
    assert_eq!(
        body["body_blobs"]["def456def456"][0]["body"],
        r#"{"model":"gemini-3.5-flash","contents":[{"text":"write full body"}]}"#
    );
    assert_eq!(
        body["body_blobs"]["def456def456"][0]["stored_bytes"],
        r#"{"model":"gemini-3.5-flash","contents":[{"text":"write full body"}]}"#.len()
    );
    assert_eq!(
        body["body_blobs"]["def456def456"][1]["direction"],
        "response"
    );
    assert_eq!(
        body["body_blobs"]["def456def456"][1]["body"],
        r#"{"ok":true,"body":"full response body from gateway"}"#
    );

    let (status, info) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/stats-detail-vm/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(
        info.get("model_call_count"),
        None,
        "/vms/{{id}}/info stays lifecycle/storage only; stats/detail is the ledger surface"
    );
    assert_eq!(info.get("total_input_tokens"), None);
    assert_eq!(info.get("total_output_tokens"), None);
    assert_eq!(info.get("total_tool_calls"), None);
}

async fn write_test_model_call(
    db_path: &std::path::Path,
    provider: &str,
    model: &str,
    event_id: &str,
) {
    let writer = capsem_logger::DbWriter::open(db_path, 16).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::ModelCall(
        capsem_logger::ModelCall {
            event_id: Some(event_id.to_string()),
            timestamp: std::time::SystemTime::now(),
            provider: provider.to_string(),
            protocol: Some(provider.to_string()),
            model: Some(model.to_string()),
            process_name: Some("agy".to_string()),
            pid: Some(42),
            method: "POST".to_string(),
            path: "/v1internal:streamGenerateContent".to_string(),
            stream: true,
            system_prompt_preview: None,
            messages_count: 1,
            tools_count: 0,
            request_bytes: 32,
            request_body_preview: None,
            request_body_full: None,
            message_id: Some(format!("{event_id}-message")),
            status_code: Some(200),
            text_content: Some("ok".to_string()),
            thinking_content: None,
            response_body_full: None,
            stop_reason: Some("end_turn".to_string()),
            input_tokens: Some(12),
            output_tokens: Some(7),
            usage_details: BTreeMap::new(),
            duration_ms: 25,
            response_bytes: 64,
            estimated_cost_usd: 0.001,
            trace_id: Some(format!("trace-{event_id}")),
            credential_ref: None,
            tool_calls: vec![],
            tool_responses: vec![],
        },
    ));
    writer.shutdown_blocking();
}

#[tokio::test]
async fn stats_detail_route_reopens_session_db_handle_when_vm_id_rebinds_to_new_path() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let old_session_dir = dir.path().join("sessions").join("co-work1-old");
    let selected_session_dir = dir.path().join("sessions").join("co-work1-selected");
    std::fs::create_dir_all(&old_session_dir).unwrap();
    std::fs::create_dir_all(&selected_session_dir).unwrap();

    write_test_model_call(
        &old_session_dir.join("session.db"),
        "ollama",
        "llama3.2",
        "badbadbadbad",
    )
    .await;
    write_test_model_call(
        &selected_session_dir.join("session.db"),
        "google",
        "gemini-3.5-flash",
        "abcabcabcabc",
    )
    .await;
    let conn = rusqlite::Connection::open(selected_session_dir.join("session.db")).unwrap();
    let direct_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM model_calls", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        direct_count, 1,
        "selected DB fixture must contain one model call"
    );

    state
        .register_session_db_handle("co-work1", &old_session_dir)
        .expect("test installs stale cached DB handle");
    insert_fake_instance_with_session_dir(
        &state,
        "co-work1",
        std::process::id(),
        selected_session_dir.clone(),
    );

    let (status, body) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/co-work1/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["model_stats"][0]["provider"], "google",
        "stats/detail must use the DB resolved for the selected session id, not a stale cached handle: {body}"
    );
    assert_eq!(body["model_stats"][0]["model"], "gemini-3.5-flash");
    assert_eq!(body["model_events"][0]["event_id"], "abcabcabcabc");
    assert_eq!(
        state.session_db_handle("co-work1").unwrap().path(),
        selected_session_dir.join("session.db").as_path(),
        "the stale cached handle must be replaced with the selected session DB"
    );
}

#[tokio::test]
async fn persistent_session_routes_keep_uuid_id_separate_from_display_name() {
    let (state, _dir) = make_test_state_with_tempdir();
    let vm_id = "11111111-1111-4111-8111-111111111111";
    let session_dir = state.run_dir.join("persistent").join(vm_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let mut entry = test_persistent_entry("co-work1", session_dir.clone());
    entry.id = vm_id.to_string();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("co-work1".to_string(), entry);

    let listing = handle_list(State(Arc::clone(&state))).await.0;
    let row = listing
        .sandboxes
        .iter()
        .find(|row| row.name.as_deref() == Some("co-work1"))
        .expect("persistent session appears by display name");
    assert_eq!(row.id, vm_id);
    assert_eq!(row.name.as_deref(), Some("co-work1"));

    let info = handle_info(State(Arc::clone(&state)), Path(vm_id.to_string()))
        .await
        .unwrap()
        .0;
    assert_eq!(info.id, vm_id);
    assert_eq!(info.name.as_deref(), Some("co-work1"));

    let status = handle_vm_status(State(state), Path(vm_id.to_string()))
        .await
        .unwrap()
        .0;
    assert_eq!(status.id, vm_id);
}

#[test]
fn resume_sandbox_requires_uuid_route_id_not_display_name() {
    let (state, _dir) = make_test_state_with_tempdir();
    let vm_id = "22222222-2222-4222-8222-222222222222";
    let session_dir = state.run_dir.join("persistent").join(vm_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let mut entry = test_persistent_entry("co-work1", session_dir);
    entry.id = vm_id.to_string();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("co-work1".to_string(), entry);

    let err = state.resume_sandbox("co-work1", None, None).unwrap_err();
    assert!(
        err.to_string().contains("no persistent VM with id"),
        "display names must be translated before service routes call resume: {err}"
    );
}

#[test]
fn persistent_route_identity_source_guard() {
    let source = include_str!("main.rs");
    for forbidden in [
        "registry.get(&id)",
        "registry.get(id)",
        "registry.get_mut(&id)",
        "registry.get_mut(id)",
        "registry.unregister(&id)",
        "instances.contains_key(&entry.name)",
        "SandboxInfo::new(\n            entry.name.clone()",
    ] {
        assert!(
            !source.contains(forbidden),
            "{forbidden} reintroduced the VM identity footgun: route `id` is the opaque session id; registry `name` is display/resume identity only"
        );
    }
}

#[tokio::test]
async fn db_boundary_route_contract_db_handle_route_rewire() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("db-handle-route-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "db-handle-route-vm",
        std::process::id(),
        session_dir.clone(),
    );

    assert!(
        state.session_db_handle("db-handle-route-vm").is_none(),
        "session handles are registered lazily after capsem-process creates session.db"
    );
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_111_000_000,
                "abcdef123456",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                r#"{"event_type":"http.request"}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow),
        ))
        .await;
    writer.shutdown_blocking();

    let (status, stats_detail) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/db-handle-route-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stats_detail}");
    assert_eq!(stats_detail["model_stats"], json!([]));
    assert_eq!(stats_detail["body_blobs"], json!({}));

    let (status, security_status) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/db-handle-route-vm/security/status",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{security_status}");
    assert_eq!(security_status["total"], 1);
    assert_eq!(security_status["by_action"][0]["rule_action"], "allow");
    assert!(
        state.session_db_handle("db-handle-route-vm").is_some(),
        "first ledger route registers the external DB reader once session.db exists"
    );
}

#[tokio::test]
async fn db_boundary_route_contract_stats_routes_do_not_return_empty_on_broken_schema() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("broken-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "broken-db-vm",
        std::process::id(),
        session_dir.clone(),
    );
    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("DROP TABLE net_events", []).unwrap();
    conn.execute("CREATE TABLE net_events (id INTEGER PRIMARY KEY)", [])
        .unwrap();
    drop(conn);

    let (status, body) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/broken-db-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let body_text = body.to_string();
    assert!(
        body_text.contains("stats_detail ledger")
            && (body_text.contains("not ready")
                || body_text.contains("no such column")
                || body_text.contains("missing required column")),
        "broken schemas must fail loudly, not return empty fake data: {body}"
    );
}

#[test]
fn logged_data_routes_do_not_bypass_logger_db_boundary() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("service source must be readable");
    let forbidden = [
        "ready_blocking(",
        "query_raw_blocking(",
        "with_reader_blocking(",
        "DbReader::open(",
        "SessionIndex::open(",
        "SessionDb::new(",
        "read_stats_response_from_main_db(&state.main_db_path())",
        "_projection",
    ];
    for needle in forbidden {
        assert!(
            !source.contains(needle),
            "{needle} reintroduced a logged-data route bypass. See AGENTS.md, \
             skills/dev-testing/SKILL.md Logged-data DB ownership, and \
             skills/dev-rust-patterns/SKILL.md Logger DB boundary: routes own query intent, \
             capsem-logger owns DB execution/storage, and missing schemas fail loudly."
        );
    }
}

#[tokio::test]
async fn session_db_handle_state_contract() {
    let state = make_test_state();
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("db-state-old");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(
        &state,
        "db-state-old",
        std::process::id(),
        session_dir.clone(),
    );
    state
        .register_session_db_handle("db-state-old", &session_dir)
        .expect("test installs external reader after session.db exists");

    let original = state
        .session_db_handle("db-state-old")
        .expect("session registration must install a DB handle");
    original.ready().await.unwrap();

    state.rename_session_db_handle("db-state-old", "db-state-new");
    assert!(
        state.session_db_handle("db-state-old").is_none(),
        "renaming a session must not leave a stale DB handle under the old id"
    );
    let renamed = state
        .session_db_handle("db-state-new")
        .expect("renaming a session must move its DB handle");
    assert!(
        Arc::ptr_eq(&original, &renamed),
        "renaming must move the existing DB handle instead of opening a second rail"
    );

    state.unregister_session_db_handle("db-state-new");
    assert!(
        state.session_db_handle("db-state-new").is_none(),
        "unregistering a session must remove the DB handle"
    );
}

#[tokio::test]
async fn service_rehydrates_session_db_handles() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions").join("startup-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    state.persistent_registry.lock().unwrap().data.vms.insert(
        "startup-db-vm".to_string(),
        test_persistent_entry("startup-db-vm", session_dir),
    );

    assert!(
        state.session_db_handle("startup-db-vm").is_none(),
        "test must prove startup hydration installs the handle"
    );
    state.hydrate_session_db_handles();

    let handle = state
        .session_db_handle("startup-db-vm")
        .expect("startup hydration must install a persistent-session DB handle");
    handle
        .ready()
        .await
        .expect("hydrated handle must prove schema readiness");
}

#[tokio::test]
async fn status_reports_db_readiness() {
    let (state, _dir) = make_test_state_with_tempdir();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = state.run_dir.join("sessions").join("status-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(&state, "status-db-vm", std::process::id(), session_dir);

    let (status, body) =
        route_request(app, axum::http::Method::GET, "/vms/status-db-vm/info", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["session_db"]["ready"], true,
        "session status must expose DB readiness from the service-owned DbHandle: {body}"
    );
    assert!(
        body["session_db"].get("error").is_none(),
        "ready session DB status must not invent an error: {body}"
    );
}

#[tokio::test]
async fn broken_session_db_schema_is_explicit_error_for_session_status() {
    let (state, _dir) = make_test_state_with_tempdir();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = state.run_dir.join("sessions").join("status-broken-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(session_dir.join("session.db")).unwrap();
    conn.execute("DROP TABLE net_events", []).unwrap();
    conn.execute("CREATE TABLE net_events (id INTEGER PRIMARY KEY)", [])
        .unwrap();
    drop(conn);
    let entry = test_persistent_entry("status-broken-db-vm", session_dir);
    let vm_id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("status-broken-db-vm".to_string(), entry);
    state.hydrate_session_db_handles();
    assert!(
        state.session_db_handle(&vm_id).is_none(),
        "startup hydration must not install a ready handle for malformed session schema"
    );

    let (status, body) = route_request(
        app,
        axum::http::Method::GET,
        &format!("/vms/{vm_id}/info"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["session_db"]["ready"], false,
        "broken session schemas must be visible in status instead of being treated as ready: {body}"
    );
    let error = body["session_db"]["error"]
        .as_str()
        .expect("broken DB status must carry the explicit DB readiness error");
    assert!(
        error.contains("not ready")
            || error.contains("missing required column")
            || error.contains("no such column"),
        "broken DB status must expose the schema failure, got: {error}"
    );
}

#[test]
fn service_db_handle_open_is_owned_by_explicit_service_state_owners() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("service source must be readable");
    let opens = source.matches("DbHandle::open(").count();
    let external_reader_opens = source.matches("DbHandle::open_external_reader(").count();
    assert_eq!(
        opens, 1,
        "DbHandle::open must live only in the service main-ledger owner. Per-session \
         route ledgers are external readers because capsem-process owns writes. Routes and helpers \
         resolve registered handles and call ready/query/write; they do not create a second \
         DB lifecycle."
    );
    assert_eq!(
        external_reader_opens, 1,
        "DbHandle::open_external_reader must live only in register_session_db_handle for \
         per-session ledgers written by capsem-process."
    );
    assert!(
        source.contains("fn register_session_db_handle(")
            && source.contains("DbHandle::open_external_reader("),
        "the session-state registration method must own the external DB reader lifecycle"
    );
    assert!(
        source.contains("fn open_profile_mutation_db_handle("),
        "one DbHandle::open owner must be the profile mutation main-ledger method"
    );
    assert!(
        !source.contains("Arc<capsem_logger::DbWriter>"),
        "service state must not own DbWriter directly. See AGENTS.md, skills/dev-testing/SKILL.md \
         Logged-data DB ownership, and skills/dev-rust-patterns/SKILL.md Logger DB boundary: \
         service owns DbHandle references; capsem-logger owns writer channels and storage mechanics."
    );
    assert!(
        !source.contains("DbWriter::open("),
        "service production code must not open DbWriter side paths. Create a DbHandle owner and \
         call db.write(event).await so structured DB logging, future mem/disk ownership, and \
         explicit schema failure semantics stay centralized."
    );
}

#[tokio::test]
async fn stats_detail_ledger_exposes_orphan_tool_parent_inconsistency() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("ledger-inconsistent-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(
        &state,
        "ledger-inconsistent-vm",
        std::process::id(),
        session_dir.clone(),
    );

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "INSERT INTO tool_calls (
            event_id, timestamp, model_call_id, provider, status, call_index,
            call_id, tool_name, arguments, origin, server_name, method,
            decision, duration_ms, trace_id, turn_id, credential_ref
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17
         )",
        rusqlite::params![
            "badbad000001",
            "2026-06-24T01:02:03Z",
            99_999_i64,
            "google",
            "observed",
            0_i64,
            "orphan-tool",
            "Write",
            r#"{"path":"/root/orphan.md","content":"ledger proof"}"#,
            "model",
            "model",
            "tool.call",
            "allowed",
            13_i64,
            "trace-orphan-tool",
            "trace-orphan-tool",
            "credential:blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tool_responses (
            model_call_id, call_id, content_preview, is_error, trace_id, turn_id,
            credential_ref
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            99_999_i64,
            "orphan-tool",
            "Wrote orphan.md",
            0_i64,
            "trace-orphan-tool",
            "trace-orphan-tool",
            "credential:blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ],
    )
    .unwrap();
    drop(conn);

    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/ledger-inconsistent-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model_events"].as_array().unwrap().len(), 0);
    assert_eq!(body["model_stats"].as_array().unwrap().len(), 0);
    assert_eq!(body["tool_events"].as_array().unwrap().len(), 1);
    let tool = &body["tool_events"][0];
    assert_eq!(tool["event_id"], "badbad000001");
    assert_eq!(tool["call_id"], "orphan-tool");
    assert_eq!(tool["model_call_id"], 99_999);
    assert_eq!(tool["model_parent_missing"], 1);
    assert_eq!(tool["source"], "model");
    assert_eq!(tool["server_name"], "model");
    assert_eq!(tool["tool_name"], "Write");
    assert_eq!(
        tool["arguments"],
        r#"{"path":"/root/orphan.md","content":"ledger proof"}"#
    );
    assert_eq!(tool["response_preview"], "Wrote orphan.md");
    assert_eq!(
        tool["credential_ref"],
        "credential:blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );

    let (status, info) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/ledger-inconsistent-vm/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(
        info.get("total_tool_calls"),
        None,
        "/vms/{{id}}/info stays lifecycle/storage only; stats/detail is the ledger surface"
    );
    assert!(
        info.get("model_call_count").is_none()
            || info["model_call_count"] == serde_json::Value::Null,
        "orphan tool diagnostics must not invent a model count"
    );

    let (status, timeline) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/ledger-inconsistent-vm/timeline?trace_id=trace-orphan-tool&layers=tool&limit=20",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let rows = timeline["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{timeline}");
    assert_eq!(rows[0][1], "tool");
    assert_eq!(rows[0][3], "model/Write (call_id=orphan-tool)");
    assert_eq!(rows[0][4], "allowed");
    assert_eq!(rows[0][6], "trace-orphan-tool");
}

// -----------------------------------------------------------------------
// Settings handler tests
// -----------------------------------------------------------------------

struct SettingsEnvGuard {
    previous_home_override: Option<std::ffi::OsString>,
    previous_corp: Option<std::ffi::OsString>,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
    previous_test_profile_dir_override: Option<Option<PathBuf>>,
}

struct TestBuiltinMcpBinaryGuard {
    path: PathBuf,
    remove_on_drop: bool,
}

fn ensure_test_builtin_mcp_binary() -> TestBuiltinMcpBinaryGuard {
    let path = std::env::current_exe()
        .expect("test binary path")
        .parent()
        .expect("test binary parent")
        .join("capsem-mcp-builtin");
    let remove_on_drop = !path.exists();
    if remove_on_drop {
        std::fs::write(&path, "#!/bin/sh\n").expect("write test builtin MCP binary placeholder");
    }
    TestBuiltinMcpBinaryGuard {
        path,
        remove_on_drop,
    }
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        let previous_test_profile_dir_override = if key == "CAPSEM_PROFILES_DIR" {
            Some(super::set_test_profile_dir_override(Some(PathBuf::from(
                value.as_ref(),
            ))))
        } else {
            None
        };
        std::env::set_var(key, value);
        Self {
            key,
            previous,
            previous_test_profile_dir_override,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
        if let Some(previous) = self.previous_test_profile_dir_override.take() {
            super::set_test_profile_dir_override(previous);
        }
    }
}

impl Drop for TestBuiltinMcpBinaryGuard {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Drop for SettingsEnvGuard {
    fn drop(&mut self) {
        if let Some(previous_home_override) = self.previous_home_override.take() {
            std::env::set_var("CAPSEM_HOME", previous_home_override);
        } else {
            std::env::remove_var("CAPSEM_HOME");
        }

        if let Some(previous_corp) = self.previous_corp.take() {
            std::env::set_var("CAPSEM_CORP_CONFIG", previous_corp);
        } else {
            std::env::remove_var("CAPSEM_CORP_CONFIG");
        }
    }
}

fn install_empty_settings_env(dir: &tempfile::TempDir) -> (SettingsEnvGuard, PathBuf, PathBuf) {
    let settings_path = dir.path().join("settings.toml");
    let corp_path = dir.path().join("corp.toml");
    capsem_core::net::policy_config::write_settings_file(
        &settings_path,
        &capsem_core::net::policy_config::SettingsFile::default(),
    )
    .unwrap();
    capsem_core::net::policy_config::write_settings_file(
        &corp_path,
        &capsem_core::net::policy_config::SettingsFile::default(),
    )
    .unwrap();

    let guard = SettingsEnvGuard {
        previous_home_override: std::env::var_os("CAPSEM_HOME"),
        previous_corp: std::env::var_os("CAPSEM_CORP_CONFIG"),
    };
    std::env::set_var("CAPSEM_HOME", dir.path());
    std::env::set_var("CAPSEM_CORP_CONFIG", &corp_path);
    (guard, settings_path, corp_path)
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
        val.get("providers").is_none(),
        "settings response must not expose provider status"
    );
    assert!(val["tree"].is_array());
    assert!(val["issues"].is_array());
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
    let retired_key = "policy".to_string() + ".http.block_openai_github";
    changes.insert(
        retired_key.clone(),
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
        err.1.contains(&format!("unknown setting: {retired_key}")),
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
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: run_dir.join("assets"),
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest: None,
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        profile_summary_cache: Mutex::new(test_profile_summary_cache()),
        profile_cache: Mutex::new(test_profile_cache()),
        profile_status_cache: Mutex::new(None),
        profile_rule_cache: test_profile_rule_cache(),
        profile_plugin_policy_cache: test_profile_plugin_policy_cache(),
        mcp_tool_cache: Mutex::new(capsem_core::mcp::load_tool_cache()),
        profile_mutation_db: test_profile_mutation_db(&run_dir),
        last_defunct_reconcile_ms: AtomicU64::new(0),
        stats_response_cache: Mutex::new(None),
        stats_detail_response_cache: Mutex::new(HashMap::new()),
        storage_diagnostics_cache: Mutex::new(HashMap::new()),
        persistent_resume_state_cache: Mutex::new(HashMap::new()),
        evaluate_rule_cache: Mutex::new(HashMap::new()),
        profile_rule_response_cache: Mutex::new(HashMap::new()),
        profile_plugin_response_cache: Mutex::new(HashMap::new()),
        evaluate_response_cache: Mutex::new(HashMap::new()),
        evaluate_last_response_cache: Mutex::new(None),
        save_restore_lock: tokio::sync::RwLock::new(()),
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
            name: "test-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
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
            name: "test-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
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
            name: vm_id.into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
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
                        data: None,
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
async fn download_file_content_does_not_wait_on_stats_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "fast-file-vm", uds_path);
    std::fs::write(
        dir.path().join("session/guest/workspace/latency.txt"),
        b"file content must not wait on ledger rebuild",
    )
    .unwrap();

    let response = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        handle_download_file(
            State(state),
            Path("fast-file-vm".to_string()),
            Query(FileContentQuery {
                path: "latency.txt".to_string(),
            }),
        ),
    )
    .await
    .expect("file content route waited for stats rebuild")
    .expect("download should succeed after boundary log");

    assert_eq!(response.status(), StatusCode::OK);
    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        &messages[0],
        ServiceToProcess::LogFileBoundary {
            action: FileBoundaryAction::Export,
            path,
            ..
        } if path == "latency.txt"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mounted_file_import_export_routes_log_boundary_events() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(2).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "file-route-vm", uds_path);
    let app = build_service_router(state);

    let upload_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/vms/file-route-vm/files/content?path=new.txt")
                .body(Body::from("uploaded over mounted route"))
                .unwrap(),
        )
        .await
        .expect("upload route should respond");
    assert_eq!(upload_response.status(), StatusCode::OK);
    let upload_body = to_bytes(upload_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let upload_json: serde_json::Value = serde_json::from_slice(&upload_body).unwrap();
    assert_eq!(upload_json["success"], true);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("session/guest/workspace/new.txt")).unwrap(),
        "uploaded over mounted route"
    );

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/file-route-vm/files/content?path=new.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("download route should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let downloaded = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&downloaded[..], b"uploaded over mounted route");

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
            assert_eq!(path, "new.txt");
            assert_eq!(data, b"uploaded over mounted route");
            assert_eq!(*size, b"uploaded over mounted route".len() as u64);
        }
        other => panic!("upload route must log import first, got {other:?}"),
    }
    match &messages[1] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Export);
            assert_eq!(path, "new.txt");
            assert_eq!(data, b"uploaded over mounted route");
            assert_eq!(*size, b"uploaded over mounted route".len() as u64);
        }
        other => panic!("download route must log export first, got {other:?}"),
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
                    data: None,
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
            name: "write-ledger-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
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

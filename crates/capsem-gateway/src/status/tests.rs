//! Tests for `status` (extracted from inline `mod tests`).

use super::*;

#[test]
fn status_response_serializes() {
    let resp = StatusResponse {
        service: "running".into(),
        gateway_version: "0.1.0".into(),
        vm_count: 1,
        vms: vec![test_vm(
            "abc123",
            Some("dev"),
            VmLifecycleState::Running,
            true,
        )],
        resource_summary: Some(ResourceSummary {
            total_ram_mb: 2048,
            total_cpus: 2,
            running_count: 1,
            stopped_count: 0,
            suspended_count: 0,
        }),
        profiles: None,
    };

    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["service"], "running");
    assert_eq!(json["vm_count"], 1);
    assert_eq!(json["vms"][0]["id"], "abc123");
    assert_eq!(json["resource_summary"]["total_ram_mb"], 2048);
    assert!(json.get("assets").is_none());
}

#[test]
fn unavailable_response_shape() {
    let resp = StatusResponse {
        service: "unavailable".into(),
        gateway_version: "0.1.0".into(),
        vm_count: 0,
        vms: vec![],
        resource_summary: None,
        profiles: None,
    };

    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["service"], "unavailable");
    assert_eq!(json["vm_count"], 0);
    assert!(json["resource_summary"].is_null());
}

#[test]
fn status_response_multiple_vms_resource_aggregation() {
    let resp = StatusResponse {
        service: "running".into(),
        gateway_version: "0.1.0".into(),
        vm_count: 3,
        vms: vec![
            test_vm("a", Some("dev"), VmLifecycleState::Running, true),
            test_vm("b", None, VmLifecycleState::Running, false),
            test_vm("c", Some("ci"), VmLifecycleState::Stopped, true),
        ],
        resource_summary: Some(ResourceSummary {
            total_ram_mb: 6144,
            total_cpus: 6,
            running_count: 2,
            stopped_count: 1,
            suspended_count: 0,
        }),
        profiles: None,
    };

    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["vm_count"], 3);
    assert_eq!(json["resource_summary"]["running_count"], 2);
    assert_eq!(json["resource_summary"]["stopped_count"], 1);
    assert_eq!(json["resource_summary"]["total_ram_mb"], 6144);
    // VM with no name should serialize name as null
    assert!(json["vms"][1]["name"].is_null());
}

#[test]
fn vm_summary_name_null_when_absent() {
    let vm = test_vm("x", None, VmLifecycleState::Running, false);
    let json = serde_json::to_value(&vm).unwrap();
    assert!(json["name"].is_null());
    assert!(!json["persistent"].as_bool().unwrap());
}

#[test]
fn list_response_deserializes() {
    let json = r#"{"sandboxes":[{"id":"abc","profile_id":"code","pid":123,"status":"Running","persistent":true,"ram_mb":2048,"cpus":2,"available_actions":["pause","stop","fork","delete"]}]}"#;
    let list: ListResponse = serde_json::from_str(json).unwrap();
    assert_eq!(list.sessions.len(), 1);
    assert_eq!(list.sessions[0].id, "abc");
    assert_eq!(list.sessions[0].profile_id, "code");
    assert!(list.sessions[0].persistent);
    assert_eq!(list.sessions[0].ram_mb, Some(2048));
}

#[test]
fn list_response_handles_missing_optional_fields() {
    let json = r#"{"sandboxes":[{"id":"abc","profile_id":"code","pid":123,"status":"Stopped","available_actions":["fork","delete"]}]}"#;
    let list: ListResponse = serde_json::from_str(json).unwrap();
    assert_eq!(list.sessions[0].profile_id, "code");
    assert_eq!(list.sessions[0].ram_mb, None);
    assert_eq!(list.sessions[0].cpus, None);
    assert!(!list.sessions[0].persistent);
}

#[tokio::test]
async fn fetch_status_preserves_session_available_actions() {
    let mock = axum::Router::new().route(
        "/vms/list",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({
                "sandboxes": [
                    {
                        "id": "bad-vm",
                        "profile_id": "code",
                        "pid": 0,
                        "status": "Incompatible",
                        "persistent": true,
                        "available_actions": ["delete"]
                    }
                ]
            }))
        }),
    );
    let (path, handle, _dir) = mock_uds(mock).await;
    let state = test_app_state(&path);

    let status = fetch_status(&state).await;
    assert_eq!(status.vms[0].available_actions, vec![VmAction::Delete]);

    handle.abort();
}

#[test]
fn list_response_rejects_missing_lifecycle_state() {
    let json = r#"{"sandboxes":[{"id":"abc","profile_id":"code","pid":123}]}"#;
    let err = serde_json::from_str::<ListResponse>(json).err().unwrap();
    assert!(err.to_string().contains("status"));
}

#[tokio::test]
async fn cache_returns_fresh_data() {
    let cache = StatusCache::new();
    let resp = StatusResponse {
        service: "running".into(),
        gateway_version: "test".into(),
        vm_count: 1,
        vms: vec![],
        resource_summary: None,
        profiles: None,
    };

    // Populate cache
    {
        let mut guard = cache.inner.write().await;
        *guard = Some((Instant::now(), resp.clone()));
    }

    // Read back within TTL
    {
        let guard = cache.inner.read().await;
        let (ts, ref cached) = guard.as_ref().unwrap();
        assert!(ts.elapsed() < CACHE_TTL);
        assert_eq!(cached.service, "running");
        assert_eq!(cached.vm_count, 1);
    }
}

#[tokio::test]
async fn cache_expires_after_ttl() {
    let cache = StatusCache::new();
    let resp = StatusResponse {
        service: "running".into(),
        gateway_version: "test".into(),
        vm_count: 0,
        vms: vec![],
        resource_summary: None,
        profiles: None,
    };

    // Populate cache with a timestamp beyond the 1s TTL
    {
        let mut guard = cache.inner.write().await;
        *guard = Some((Instant::now() - Duration::from_secs(2), resp));
    }

    // Cache should be stale
    {
        let guard = cache.inner.read().await;
        let (ts, _) = guard.as_ref().unwrap();
        assert!(ts.elapsed() >= CACHE_TTL);
    }
}

#[tokio::test]
async fn cache_starts_empty() {
    let cache = StatusCache::new();
    let guard = cache.inner.read().await;
    assert!(guard.is_none());
}

// --- fetch_status with mock UDS ---

use crate::AppState;

fn test_vm(id: &str, name: Option<&str>, status: VmLifecycleState, persistent: bool) -> VmSummary {
    VmSummary {
        id: id.into(),
        name: name.map(|s| s.into()),
        status,
        persistent,
        profile_id: "code".into(),
        uptime_secs: None,
        total_input_tokens: None,
        total_output_tokens: None,
        total_estimated_cost: None,
        total_tool_calls: None,
        total_mcp_calls: None,
        total_requests: None,
        allowed_requests: None,
        denied_requests: None,
        total_file_events: None,
        model_call_count: None,
        can_resume: false,
        resume_blocked_reason: None,
        available_actions: match status {
            VmLifecycleState::Running => vec![
                VmAction::Pause,
                VmAction::Stop,
                VmAction::Fork,
                VmAction::Delete,
            ],
            VmLifecycleState::Stopped | VmLifecycleState::Suspended => {
                vec![VmAction::Fork, VmAction::Delete]
            }
            VmLifecycleState::Defunct | VmLifecycleState::Incompatible => vec![VmAction::Delete],
        },
    }
}

fn test_app_state(uds_path: &str) -> AppState {
    AppState {
        token: "test".into(),
        uds_path: uds_path.into(),
        status_cache: StatusCache::new(),
        auth_failures: crate::auth::AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    }
}

async fn mock_uds(app: axum::Router) -> (String, tokio::task::JoinHandle<()>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("mock.sock");
    let path_str = sock_path.to_str().unwrap().to_string();
    let uds = tokio::net::UnixListener::bind(&sock_path).unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(uds, app).await.ok();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (path_str, handle, dir)
}

#[tokio::test]
async fn fetch_status_empty_vm_list() {
    let mock = axum::Router::new().route(
        "/vms/list",
        axum::routing::get(|| async { axum::Json(serde_json::json!({"sandboxes": []})) }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let state = test_app_state(&path);
    let resp = fetch_status(&state).await;
    assert_eq!(resp.service, "running");
    assert_eq!(resp.vm_count, 0);
    assert!(resp.vms.is_empty());
    let rs = resp.resource_summary.unwrap();
    assert_eq!(rs.total_ram_mb, 0);
    assert_eq!(rs.total_cpus, 0);
    assert_eq!(rs.running_count, 0);
    assert_eq!(rs.stopped_count, 0);
    h.abort();
}

#[tokio::test]
async fn fetch_status_preserves_profile_catalog_and_manifest_provenance() {
    let mock = axum::Router::new()
        .route(
            "/vms/list",
            axum::routing::get(|| async { axum::Json(serde_json::json!({"sandboxes": []})) }),
        )
        .route(
            "/profiles/status",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "source": "directory",
                    "profile_count": 2,
                    "ready_count": 1,
                    "asset_manifest": {
                        "origin": "package",
                        "path": "/Users/test/.capsem/assets/manifest.json",
                        "origin_path": "/Users/test/.capsem/assets/manifest-origin.json",
                        "origin_source": "file:///tmp/corp/manifest.json",
                        "packaged_at": "2026-06-13T00:00:00Z",
                        "blake3": "0123456789abcdef",
                        "validation_status": "valid",
                        "refresh_policy": "24h",
                        "assets_current": "2026.0613.1",
                        "binaries_current": "1.3.0"
                    },
                    "profiles": [
                        {
                            "id": "code",
                            "name": "Code",
                            "description": "Optimized for coding and long-running agents.",
                            "ready": true,
                            "current_arch": "arm64",
                            "missing_assets": [],
                            "invalid_assets": [],
                            "invalid_files": [],
                            "errors": [],
                            "asset_count": 3
                        },
                        {
                            "id": "co-work",
                            "name": "Co-work",
                            "description": "Shared profile for collaborative agent sessions.",
                            "ready": false,
                            "current_arch": "arm64",
                            "missing_assets": [{"kind": "rootfs", "path": "/missing/rootfs.erofs", "valid": false}],
                            "invalid_assets": [],
                            "invalid_files": [],
                            "errors": ["missing rootfs"],
                            "asset_count": 3
                        }
                    ]
                }))
            }),
        );
    let (path, h, _d) = mock_uds(mock).await;

    let state = test_app_state(&path);
    let resp = fetch_status(&state).await;

    assert_eq!(resp.service, "running");
    let profiles = resp
        .profiles
        .expect("gateway status must include profile status");
    assert_eq!(profiles["source"], "directory");
    assert_eq!(profiles["profile_count"], 2);
    assert_eq!(profiles["ready_count"], 1);
    assert_eq!(profiles["asset_manifest"]["origin"], "package");
    assert_eq!(
        profiles["asset_manifest"]["origin_source"],
        "file:///tmp/corp/manifest.json"
    );
    assert_eq!(profiles["asset_manifest"]["blake3"], "0123456789abcdef");
    assert_eq!(profiles["asset_manifest"]["validation_status"], "valid");
    assert_eq!(profiles["asset_manifest"]["refresh_policy"], "24h");
    assert_eq!(profiles["profiles"][0]["id"], "code");
    assert_eq!(profiles["profiles"][0]["ready"], true);
    assert_eq!(profiles["profiles"][1]["id"], "co-work");
    assert_eq!(
        profiles["profiles"][1]["missing_assets"][0]["kind"],
        "rootfs"
    );
    h.abort();
}

#[tokio::test]
async fn fetch_status_ignores_retired_global_asset_health() {
    let mock = axum::Router::new()
        .route(
            "/vms/list",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "sandboxes": [],
                    "asset_health": {
                        "ready": false,
                        "version": "2026.0618.18",
                        "missing": ["initrd.img"]
                    }
                }))
            }),
        )
        .route(
            "/profiles/status",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "source": "directory",
                    "profile_count": 1,
                    "ready_count": 1,
                    "profiles": [
                        {
                            "id": "code",
                            "name": "Code",
                            "ready": true,
                            "missing_assets": [],
                            "invalid_assets": [],
                            "invalid_files": [],
                            "errors": [],
                            "asset_count": 3
                        }
                    ]
                }))
            }),
        );
    let (path, h, _d) = mock_uds(mock).await;

    let state = test_app_state(&path);
    let resp = fetch_status(&state).await;
    let json = serde_json::to_value(&resp).unwrap();
    assert!(
        json.get("assets").is_none(),
        "gateway /status must not expose retired top-level asset health"
    );

    h.abort();
}

#[tokio::test]
async fn fetch_status_multiple_vms() {
    let mock = axum::Router::new()
        .route("/vms/list", axum::routing::get(|| async {
            axum::Json(serde_json::json!({
                "sandboxes": [
                    {"id": "vm1", "profile_id": "code", "name": "dev", "pid": 100, "status": "Running", "persistent": true, "ram_mb": 2048, "cpus": 2, "available_actions": ["pause", "stop", "fork", "delete"]},
                    {"id": "vm2", "profile_id": "code", "pid": 200, "status": "Running", "persistent": false, "ram_mb": 4096, "cpus": 4, "available_actions": ["pause", "stop", "fork", "delete"]},
                    {"id": "vm3", "profile_id": "code", "name": "ci", "pid": 300, "status": "Stopped", "persistent": true, "ram_mb": 1024, "cpus": 1, "available_actions": ["fork", "delete"]},
                ]
            }))
        }));
    let (path, h, _d) = mock_uds(mock).await;

    let state = test_app_state(&path);
    let resp = fetch_status(&state).await;
    assert_eq!(resp.service, "running");
    assert_eq!(resp.vm_count, 3);
    assert_eq!(resp.vms[0].name, Some("dev".into()));
    assert_eq!(resp.vms[1].name, None); // no name in /vms/list response
    assert_eq!(resp.vms[2].name, Some("ci".into()));
    let rs = resp.resource_summary.unwrap();
    assert_eq!(rs.total_ram_mb, 7168);
    assert_eq!(rs.total_cpus, 7);
    assert_eq!(rs.running_count, 2);
    assert_eq!(rs.stopped_count, 1);
    h.abort();
}

#[tokio::test]
async fn fetch_status_service_unavailable() {
    let state = test_app_state("/tmp/capsem-gw-test-no-such-socket.sock");
    let resp = fetch_status(&state).await;
    assert_eq!(resp.service, "unavailable");
    assert_eq!(resp.vm_count, 0);
    assert!(resp.resource_summary.is_none());
}

#[tokio::test]
async fn fetch_status_malformed_list_json() {
    let mock = axum::Router::new().route(
        "/vms/list",
        axum::routing::get(|| async { "not json at all" }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let state = test_app_state(&path);
    let resp = fetch_status(&state).await;
    assert_eq!(resp.service, "unavailable");
    h.abort();
}

#[tokio::test]
async fn cache_prevents_duplicate_fetches() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    let mock = axum::Router::new().route(
        "/vms/list",
        axum::routing::get(move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                axum::Json(serde_json::json!({"sandboxes": []}))
            }
        }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let state = Arc::new(AppState {
        token: "test".into(),
        uds_path: path.into(),
        status_cache: StatusCache::new(),
        auth_failures: crate::auth::AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    });

    // First call -- cache miss, fetches from UDS
    handle_status(axum::extract::State(state.clone())).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Second call within TTL -- should use cache
    handle_status(axum::extract::State(state.clone())).await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "cache should prevent second fetch"
    );
    h.abort();
}

// --- Suspended count (issue #8) ---

#[tokio::test]
async fn fetch_status_counts_suspended_vms() {
    let mock = axum::Router::new()
        .route("/vms/list", axum::routing::get(|| async {
            axum::Json(serde_json::json!({
                "sandboxes": [
                    {"id": "vm1", "profile_id": "code", "pid": 100, "status": "Running", "persistent": true, "ram_mb": 2048, "cpus": 2, "available_actions": ["pause", "stop", "fork", "delete"]},
                    {"id": "vm2", "profile_id": "code", "pid": 0, "status": "Suspended", "persistent": true, "ram_mb": 2048, "cpus": 2, "available_actions": ["resume", "fork", "delete"]},
                    {"id": "vm3", "profile_id": "code", "pid": 0, "status": "Stopped", "persistent": true, "ram_mb": 1024, "cpus": 1, "available_actions": ["fork", "delete"]},
                ]
            }))
        }));
    let (path, h, _d) = mock_uds(mock).await;

    let state = test_app_state(&path);
    let resp = fetch_status(&state).await;
    assert_eq!(resp.vm_count, 3);
    let rs = resp.resource_summary.unwrap();
    assert_eq!(rs.running_count, 1);
    assert_eq!(rs.suspended_count, 1);
    assert_eq!(rs.stopped_count, 1);
    h.abort();
}

#[test]
fn suspended_count_serializes_in_json() {
    let rs = ResourceSummary {
        total_ram_mb: 4096,
        total_cpus: 4,
        running_count: 1,
        stopped_count: 1,
        suspended_count: 2,
    };
    let json = serde_json::to_value(&rs).unwrap();
    assert_eq!(json["suspended_count"], 2);
}

// --- Telemetry pass-through ---

#[test]
fn vm_summary_includes_telemetry_when_present() {
    let mut vm = test_vm("t1", None, VmLifecycleState::Running, false);
    vm.uptime_secs = Some(300);
    vm.total_input_tokens = Some(5000);
    vm.total_estimated_cost = Some(1.23);
    let json = serde_json::to_value(&vm).unwrap();
    assert_eq!(json["uptime_secs"], 300);
    assert_eq!(json["total_input_tokens"], 5000);
    assert_eq!(json["total_estimated_cost"], 1.23);
}

#[test]
fn vm_summary_omits_absent_telemetry() {
    let vm = test_vm("t2", None, VmLifecycleState::Stopped, true);
    let json = serde_json::to_value(&vm).unwrap();
    assert!(json.get("uptime_secs").is_none());
    assert!(json.get("total_input_tokens").is_none());
    assert!(json.get("total_estimated_cost").is_none());
}

#[test]
fn list_response_deserializes_telemetry() {
    let json = r#"{"sandboxes":[{"id":"vm1","profile_id":"code","pid":100,"status":"Running","persistent":false,"ram_mb":2048,"cpus":2,"uptime_secs":60,"total_input_tokens":1000,"total_output_tokens":500,"total_estimated_cost":0.42,"available_actions":["pause","stop","fork","delete"]}]}"#;
    let list: ListResponse = serde_json::from_str(json).unwrap();
    assert_eq!(list.sessions[0].profile_id, "code");
    assert_eq!(list.sessions[0].uptime_secs, Some(60));
    assert_eq!(list.sessions[0].total_input_tokens, Some(1000));
    assert_eq!(list.sessions[0].total_output_tokens, Some(500));
    assert_eq!(list.sessions[0].total_estimated_cost, Some(0.42));
}

#[tokio::test]
async fn fetch_status_passes_through_telemetry() {
    let mock = axum::Router::new().route(
        "/vms/list",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({
                "sandboxes": [{
                    "id": "vm1", "profile_id": "code", "pid": 100, "status": "Running", "persistent": false,
                    "ram_mb": 2048, "cpus": 2,
                    "uptime_secs": 120, "total_input_tokens": 3000,
                    "total_output_tokens": 1000, "total_estimated_cost": 0.99,
                    "total_tool_calls": 10, "model_call_count": 5,
                    "available_actions": ["pause", "stop", "fork", "delete"]
                }]
            }))
        }),
    );
    let (path, h, _d) = mock_uds(mock).await;

    let state = test_app_state(&path);
    let resp = fetch_status(&state).await;
    assert_eq!(resp.vms.len(), 1);
    let vm = &resp.vms[0];
    assert_eq!(vm.uptime_secs, Some(120));
    assert_eq!(vm.total_input_tokens, Some(3000));
    assert_eq!(vm.total_output_tokens, Some(1000));
    assert_eq!(vm.total_estimated_cost, Some(0.99));
    assert_eq!(vm.total_tool_calls, Some(10));
    assert_eq!(vm.model_call_count, Some(5));
    // Fields not in JSON should be None
    assert_eq!(vm.total_mcp_calls, None);
    assert_eq!(vm.total_file_events, None);
    h.abort();
}

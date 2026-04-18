use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::UnixStream;
use tokio::sync::RwLock;

use crate::AppState;

const CACHE_TTL: Duration = Duration::from_secs(1);

pub struct StatusCache {
    inner: RwLock<Option<(Instant, StatusResponse)>>,
    /// Serializes refresh calls so only one fetch runs at a time.
    refresh: tokio::sync::Mutex<()>,
}

impl StatusCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
            refresh: tokio::sync::Mutex::new(()),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct AssetHealth {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub missing: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct StatusResponse {
    pub service: String,
    pub gateway_version: String,
    pub vm_count: usize,
    pub vms: Vec<VmSummary>,
    pub resource_summary: Option<ResourceSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<AssetHealth>,
}

#[derive(Serialize, Clone)]
pub struct VmSummary {
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub persistent: bool,
    // Telemetry (present for running VMs, absent for stopped)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_estimated_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tool_calls: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_mcp_calls: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_file_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_call_count: Option<u64>,
}

#[derive(Serialize, Clone)]
pub struct ResourceSummary {
    pub total_ram_mb: u64,
    pub total_cpus: u32,
    pub running_count: usize,
    pub stopped_count: usize,
    pub suspended_count: usize,
}

/// GET /status -- aggregated system health for tray polling.
///
/// Uses a refresh mutex to prevent thundering herd: when the cache expires,
/// only one request fetches from the service while others wait and reuse
/// the refreshed cache.
pub async fn handle_status(State(state): State<Arc<AppState>>) -> Response {
    // Fast path: serve from cache if fresh
    {
        let cache = state.status_cache.inner.read().await;
        if let Some((ts, ref resp)) = *cache {
            if ts.elapsed() < CACHE_TTL {
                return (StatusCode::OK, axum::Json(resp.clone())).into_response();
            }
        }
    }

    // Slow path: acquire refresh lock so only one caller fetches
    let _refresh_guard = state.status_cache.refresh.lock().await;

    // Double-check: another caller may have refreshed while we waited
    {
        let cache = state.status_cache.inner.read().await;
        if let Some((ts, ref resp)) = *cache {
            if ts.elapsed() < CACHE_TTL {
                return (StatusCode::OK, axum::Json(resp.clone())).into_response();
            }
        }
    }

    let old_vms: Vec<(String, String)> = {
        let cache = state.status_cache.inner.read().await;
        cache.as_ref().map(|(_, r)| {
            r.vms.iter().map(|v| (v.id.clone(), v.status.clone())).collect()
        }).unwrap_or_default()
    };

    let resp = fetch_status(&state).await;

    // Detect VM state changes and broadcast events.
    for vm in &resp.vms {
        let old_status = old_vms.iter().find(|(id, _)| id == &vm.id).map(|(_, s)| s.as_str());
        let changed = match old_status {
            Some(prev) => prev != vm.status,
            None => true, // new VM appeared
        };
        if changed {
            let event = serde_json::json!({
                "type": "vm-state-changed",
                "payload": {
                    "id": vm.id,
                    "state": vm.status,
                    "trigger": "status_poll",
                }
            });
            let _ = state.events_tx.send(event.to_string());
        }
    }

    {
        let mut cache = state.status_cache.inner.write().await;
        *cache = Some((Instant::now(), resp.clone()));
    }

    (StatusCode::OK, axum::Json(resp)).into_response()
}

#[derive(Deserialize)]
struct ServiceAssetHealth {
    ready: bool,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    missing: Vec<String>,
}

#[derive(Deserialize)]
struct ListResponse {
    #[serde(rename = "sandboxes")]
    sessions: Vec<SessionInfo>,
    #[serde(default)]
    asset_health: Option<ServiceAssetHealth>,
}

#[derive(Deserialize)]
struct SessionInfo {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    persistent: bool,
    #[serde(default)]
    ram_mb: Option<u64>,
    #[serde(default)]
    cpus: Option<u32>,
    // Telemetry pass-through from service /list
    #[serde(default)]
    uptime_secs: Option<u64>,
    #[serde(default)]
    total_input_tokens: Option<u64>,
    #[serde(default)]
    total_output_tokens: Option<u64>,
    #[serde(default)]
    total_estimated_cost: Option<f64>,
    #[serde(default)]
    total_tool_calls: Option<u64>,
    #[serde(default)]
    total_mcp_calls: Option<u64>,
    #[serde(default)]
    total_requests: Option<u64>,
    #[serde(default)]
    allowed_requests: Option<u64>,
    #[serde(default)]
    denied_requests: Option<u64>,
    #[serde(default)]
    total_file_events: Option<u64>,
    #[serde(default)]
    model_call_count: Option<u64>,
}

async fn fetch_status(state: &AppState) -> StatusResponse {
    let unavailable = StatusResponse {
        service: "unavailable".into(),
        gateway_version: env!("CARGO_PKG_VERSION").into(),
        vm_count: 0,
        vms: vec![],
        resource_summary: None,
        assets: None,
    };

    let list = match uds_get(&state.uds_path, "/list").await {
        Ok(body) => match serde_json::from_slice::<ListResponse>(&body) {
            Ok(l) => l,
            Err(_) => return unavailable,
        },
        Err(_) => return unavailable,
    };

    let mut vms = Vec::with_capacity(list.sessions.len());
    let mut total_ram: u64 = 0;
    let mut total_cpus: u32 = 0;
    let mut running: usize = 0;
    let mut stopped: usize = 0;
    let mut suspended: usize = 0;

    for sess in &list.sessions {
        if let Some(ram) = sess.ram_mb {
            total_ram += ram;
        }
        if let Some(cpus) = sess.cpus {
            total_cpus += cpus;
        }

        let status_lower = sess.status.to_lowercase();
        if status_lower.contains("running") {
            running += 1;
        } else if status_lower.contains("suspended") {
            suspended += 1;
        } else {
            stopped += 1;
        }

        vms.push(VmSummary {
            id: sess.id.clone(),
            name: sess.name.clone(),
            status: sess.status.clone(),
            persistent: sess.persistent,
            uptime_secs: sess.uptime_secs,
            total_input_tokens: sess.total_input_tokens,
            total_output_tokens: sess.total_output_tokens,
            total_estimated_cost: sess.total_estimated_cost,
            total_tool_calls: sess.total_tool_calls,
            total_mcp_calls: sess.total_mcp_calls,
            total_requests: sess.total_requests,
            allowed_requests: sess.allowed_requests,
            denied_requests: sess.denied_requests,
            total_file_events: sess.total_file_events,
            model_call_count: sess.model_call_count,
        });
    }

    let assets = list.asset_health.map(|h| AssetHealth {
        ready: h.ready,
        version: h.version,
        missing: h.missing,
    });

    StatusResponse {
        service: "running".into(),
        gateway_version: env!("CARGO_PKG_VERSION").into(),
        vm_count: vms.len(),
        vms,
        resource_summary: Some(ResourceSummary {
            total_ram_mb: total_ram,
            total_cpus,
            running_count: running,
            stopped_count: stopped,
            suspended_count: suspended,
        }),
        assets,
    }
}

/// Simple GET request over UDS.
async fn uds_get(uds_path: &std::path::Path, path: &str) -> anyhow::Result<Bytes> {
    let stream = UnixStream::connect(uds_path).await?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::error!(error = %e, "UDS connection error in status fetch");
        }
    });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(format!("http://localhost{}", path))
        .body(Full::new(Bytes::new()))?;

    let res = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .map_err(|_| anyhow::anyhow!("status request timed out"))??;

    Ok(res.into_body().collect().await?.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_response_serializes() {
        let resp = StatusResponse {
            service: "running".into(),
            gateway_version: "0.1.0".into(),
            vm_count: 1,
            vms: vec![test_vm("abc123", Some("dev"), "running", true)],
            resource_summary: Some(ResourceSummary {
                total_ram_mb: 2048,
                total_cpus: 2,
                running_count: 1,
                stopped_count: 0,
                suspended_count: 0,
            }),
            assets: None,
        };

        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["service"], "running");
        assert_eq!(json["vm_count"], 1);
        assert_eq!(json["vms"][0]["id"], "abc123");
        assert_eq!(json["resource_summary"]["total_ram_mb"], 2048);
    }

    #[test]
    fn unavailable_response_shape() {
        let resp = StatusResponse {
            service: "unavailable".into(),
            gateway_version: "0.1.0".into(),
            vm_count: 0,
            vms: vec![],
            resource_summary: None,
            assets: None,
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
                test_vm("a", Some("dev"), "running", true),
                test_vm("b", None, "running", false),
                test_vm("c", Some("ci"), "stopped", true),
            ],
            resource_summary: Some(ResourceSummary {
                total_ram_mb: 6144,
                total_cpus: 6,
                running_count: 2,
                stopped_count: 1,
                suspended_count: 0,
            }),
            assets: None,
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
        let vm = test_vm("x", None, "running", false);
        let json = serde_json::to_value(&vm).unwrap();
        assert!(json["name"].is_null());
        assert!(!json["persistent"].as_bool().unwrap());
    }

    #[test]
    fn list_response_deserializes() {
        let json = r#"{"sandboxes":[{"id":"abc","pid":123,"status":"Running","persistent":true,"ram_mb":2048,"cpus":2}]}"#;
        let list: ListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(list.sessions.len(), 1);
        assert_eq!(list.sessions[0].id, "abc");
        assert!(list.sessions[0].persistent);
        assert_eq!(list.sessions[0].ram_mb, Some(2048));
    }

    #[test]
    fn list_response_handles_missing_optional_fields() {
        let json = r#"{"sandboxes":[{"id":"abc","pid":123}]}"#;
        let list: ListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(list.sessions[0].ram_mb, None);
        assert_eq!(list.sessions[0].cpus, None);
        assert!(!list.sessions[0].persistent);
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
            assets: None,
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
            assets: None,
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

    fn test_vm(id: &str, name: Option<&str>, status: &str, persistent: bool) -> VmSummary {
        VmSummary {
            id: id.into(),
            name: name.map(|s| s.into()),
            status: status.into(),
            persistent,
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
        let mock = axum::Router::new()
            .route("/list", axum::routing::get(|| async {
                axum::Json(serde_json::json!({"sandboxes": []}))
            }));
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
    async fn fetch_status_multiple_vms() {
        let mock = axum::Router::new()
            .route("/list", axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "sandboxes": [
                        {"id": "vm1", "name": "dev", "pid": 100, "status": "Running", "persistent": true, "ram_mb": 2048, "cpus": 2},
                        {"id": "vm2", "pid": 200, "status": "Running", "persistent": false, "ram_mb": 4096, "cpus": 4},
                        {"id": "vm3", "name": "ci", "pid": 300, "status": "Stopped", "persistent": true, "ram_mb": 1024, "cpus": 1},
                    ]
                }))
            }));
        let (path, h, _d) = mock_uds(mock).await;

        let state = test_app_state(&path);
        let resp = fetch_status(&state).await;
        assert_eq!(resp.service, "running");
        assert_eq!(resp.vm_count, 3);
        assert_eq!(resp.vms[0].name, Some("dev".into()));
        assert_eq!(resp.vms[1].name, None); // no name in /list response
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
        let mock = axum::Router::new()
            .route("/list", axum::routing::get(|| async { "not json at all" }));
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
        let mock = axum::Router::new()
            .route("/list", axum::routing::get(move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({"sandboxes": []}))
                }
            }));
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
        assert_eq!(counter.load(Ordering::SeqCst), 1, "cache should prevent second fetch");
        h.abort();
    }

    // --- Suspended count (issue #8) ---

    #[tokio::test]
    async fn fetch_status_counts_suspended_vms() {
        let mock = axum::Router::new()
            .route("/list", axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "sandboxes": [
                        {"id": "vm1", "pid": 100, "status": "Running", "persistent": true, "ram_mb": 2048, "cpus": 2},
                        {"id": "vm2", "pid": 0, "status": "Suspended", "persistent": true, "ram_mb": 2048, "cpus": 2},
                        {"id": "vm3", "pid": 0, "status": "Stopped", "persistent": true, "ram_mb": 1024, "cpus": 1},
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
        let mut vm = test_vm("t1", None, "running", false);
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
        let vm = test_vm("t2", None, "stopped", true);
        let json = serde_json::to_value(&vm).unwrap();
        assert!(json.get("uptime_secs").is_none());
        assert!(json.get("total_input_tokens").is_none());
        assert!(json.get("total_estimated_cost").is_none());
    }

    #[test]
    fn list_response_deserializes_telemetry() {
        let json = r#"{"sandboxes":[{"id":"vm1","pid":100,"status":"Running","persistent":false,"ram_mb":2048,"cpus":2,"uptime_secs":60,"total_input_tokens":1000,"total_output_tokens":500,"total_estimated_cost":0.42}]}"#;
        let list: ListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(list.sessions[0].uptime_secs, Some(60));
        assert_eq!(list.sessions[0].total_input_tokens, Some(1000));
        assert_eq!(list.sessions[0].total_output_tokens, Some(500));
        assert_eq!(list.sessions[0].total_estimated_cost, Some(0.42));
    }

    #[tokio::test]
    async fn fetch_status_passes_through_telemetry() {
        let mock = axum::Router::new()
            .route("/list", axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "sandboxes": [{
                        "id": "vm1", "pid": 100, "status": "Running", "persistent": false,
                        "ram_mb": 2048, "cpus": 2,
                        "uptime_secs": 120, "total_input_tokens": 3000,
                        "total_output_tokens": 1000, "total_estimated_cost": 0.99,
                        "total_tool_calls": 10, "model_call_count": 5
                    }]
                }))
            }));
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
}

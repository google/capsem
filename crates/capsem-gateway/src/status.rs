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
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    pub missing: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<AssetProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub retry_count: u32,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saved_vm_dependencies: Vec<SavedVmAssetDependency>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedVmAssetDependency {
    pub vm: String,
    pub asset_version: String,
    pub arch: String,
    pub missing: Vec<String>,
    pub recovery_hint: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AssetProgress {
    pub logical_name: String,
    pub bytes_done: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
    pub done: bool,
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
        cache
            .as_ref()
            .map(|(_, r)| {
                r.vms
                    .iter()
                    .map(|v| (v.id.clone(), v.status.clone()))
                    .collect()
            })
            .unwrap_or_default()
    };

    let resp = fetch_status(&state).await;

    // Detect VM state changes and broadcast events.
    for vm in &resp.vms {
        let old_status = old_vms
            .iter()
            .find(|(id, _)| id == &vm.id)
            .map(|(_, s)| s.as_str());
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
    #[serde(default = "default_asset_state")]
    state: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    arch: Option<String>,
    #[serde(default)]
    missing: Vec<String>,
    #[serde(default)]
    progress: Option<AssetProgress>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    retry_count: u32,
    #[serde(default)]
    retryable: bool,
    #[serde(default)]
    saved_vm_dependencies: Vec<SavedVmAssetDependency>,
}

fn default_asset_state() -> String {
    "unknown".to_string()
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
        state: h.state,
        version: h.version,
        arch: h.arch,
        missing: h.missing,
        progress: h.progress,
        error: h.error,
        retry_count: h.retry_count,
        retryable: h.retryable,
        saved_vm_dependencies: h.saved_vm_dependencies,
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
mod tests;

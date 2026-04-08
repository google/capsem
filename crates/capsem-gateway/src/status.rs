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

const CACHE_TTL: Duration = Duration::from_secs(2);

pub struct StatusCache {
    inner: RwLock<Option<(Instant, StatusResponse)>>,
}

impl StatusCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct StatusResponse {
    pub service: String,
    pub gateway_version: String,
    pub vm_count: usize,
    pub vms: Vec<VmSummary>,
    pub resource_summary: Option<ResourceSummary>,
}

#[derive(Serialize, Clone)]
pub struct VmSummary {
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub persistent: bool,
}

#[derive(Serialize, Clone)]
pub struct ResourceSummary {
    pub total_ram_mb: u64,
    pub total_cpus: u32,
    pub running_count: usize,
    pub stopped_count: usize,
}

/// GET /status -- aggregated system health for tray polling.
pub async fn handle_status(State(state): State<Arc<AppState>>) -> Response {
    // Check cache
    {
        let cache = state.status_cache.inner.read().await;
        if let Some((ts, ref resp)) = *cache {
            if ts.elapsed() < CACHE_TTL {
                return (StatusCode::OK, axum::Json(resp.clone())).into_response();
            }
        }
    }

    // Fetch fresh data
    let resp = fetch_status(&state).await;

    // Update cache
    {
        let mut cache = state.status_cache.inner.write().await;
        *cache = Some((Instant::now(), resp.clone()));
    }

    (StatusCode::OK, axum::Json(resp)).into_response()
}

#[derive(Deserialize)]
struct ListResponse {
    sandboxes: Vec<SandboxInfo>,
}

#[derive(Deserialize)]
struct SandboxInfo {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    persistent: bool,
    #[serde(default)]
    ram_mb: Option<u64>,
    #[serde(default)]
    cpus: Option<u32>,
}

async fn fetch_status(state: &AppState) -> StatusResponse {
    let unavailable = StatusResponse {
        service: "unavailable".into(),
        gateway_version: env!("CARGO_PKG_VERSION").into(),
        vm_count: 0,
        vms: vec![],
        resource_summary: None,
    };

    let list = match uds_get(&state.uds_path, "/list").await {
        Ok(body) => match serde_json::from_slice::<ListResponse>(&body) {
            Ok(l) => l,
            Err(_) => return unavailable,
        },
        Err(_) => return unavailable,
    };

    let mut vms = Vec::with_capacity(list.sandboxes.len());
    let mut total_ram: u64 = 0;
    let mut total_cpus: u32 = 0;
    let mut running: usize = 0;
    let mut stopped: usize = 0;

    for sb in &list.sandboxes {
        // Try to get name from /info endpoint
        let name = match uds_get(&state.uds_path, &format!("/info/{}", sb.id)).await {
            Ok(body) => serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("name")?.as_str().map(String::from)),
            Err(_) => None,
        };

        if let Some(ram) = sb.ram_mb {
            total_ram += ram;
        }
        if let Some(cpus) = sb.cpus {
            total_cpus += cpus;
        }

        if sb.status.to_lowercase().contains("running") {
            running += 1;
        } else {
            stopped += 1;
        }

        vms.push(VmSummary {
            id: sb.id.clone(),
            name,
            status: sb.status.clone(),
            persistent: sb.persistent,
        });
    }

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
        }),
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
            vms: vec![VmSummary {
                id: "abc123".into(),
                name: Some("dev".into()),
                status: "running".into(),
                persistent: true,
            }],
            resource_summary: Some(ResourceSummary {
                total_ram_mb: 2048,
                total_cpus: 2,
                running_count: 1,
                stopped_count: 0,
            }),
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
        };

        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["service"], "unavailable");
        assert_eq!(json["vm_count"], 0);
        assert!(json["resource_summary"].is_null());
    }
}

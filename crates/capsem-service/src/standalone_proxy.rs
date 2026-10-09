//! Lifecycle owner for VM-free model proxy sessions.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::Json;
use capsem_api::{
    CreateProxyRequest, CreateProxyResponse, ProxyHeartbeatResponse, ProxyLeaseRequest, StopProxyResponse,
};
use capsem_proto::proxy_control::ProxyCapability;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::instance::WorkerAuthority;
use crate::{proxy_worker, upstream_broker, AppError, ServiceState};

const LEASE_TTL: Duration = Duration::from_secs(15);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct StandaloneProxy {
    lease_token: String,
    expires_at: Instant,
    worker: proxy_worker::ProxyWorker,
    authority: WorkerAuthority,
    accept_task: JoinHandle<()>,
    broker_task: JoinHandle<()>,
    session_dir: PathBuf,
    provider_id: String,
    upstream_policy: upstream_broker::PolicyPublisher,
}

impl ServiceState {
    async fn create_standalone_proxy(
        self: &Arc<Self>,
        request: CreateProxyRequest,
    ) -> Result<CreateProxyResponse, AppError> {
        let bind_ip = request.bind.parse::<IpAddr>().map_err(|_| {
            AppError(
                axum::http::StatusCode::BAD_REQUEST,
                format!("proxy bind address {:?} is not an IP address", request.bind),
            )
        })?;
        if request.provider.is_empty() || request.provider.len() > 128 {
            return Err(AppError(
                axum::http::StatusCode::BAD_REQUEST,
                "proxy provider must contain 1 to 128 bytes".to_string(),
            ));
        }

        let session_id = format!("proxy-{}", uuid::Uuid::new_v4().simple());
        let session_dir = self.run_dir.join("sessions").join(&session_id);
        capsem_foundation::unix::fs::ensure_private_dir(&session_dir)
            .map_err(|error| internal(format!("create standalone proxy session: {error}")))?;
        let started = self
            .start_standalone_proxy(
                &session_id,
                &session_dir,
                request.provider.clone(),
                bind_ip,
                request.port,
            )
            .await;
        let (entry, local_addr, base_path) = match started {
            Ok(started) => started,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&session_dir).await;
                return Err(internal(format!("start standalone proxy: {error:#}")));
            }
        };
        let lease_token = entry.lease_token.clone();
        let expires_at = entry.expires_at;
        let generation = entry.worker.generation();
        let stopped = entry.worker.stop_receiver();
        self.standalone_proxies.lock().await.insert(session_id.clone(), entry);
        spawn_lease_monitor(Arc::clone(self), session_id.clone());
        spawn_worker_monitor(Arc::clone(self), session_id.clone(), generation, stopped);

        let advertised_ip = match local_addr.ip() {
            IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            ip => ip,
        };
        let advertised = SocketAddr::new(advertised_ip, local_addr.port());
        Ok(CreateProxyResponse {
            session_id,
            provider: request.provider,
            bind: bind_ip.to_string(),
            port: local_addr.port(),
            base_url: format!("http://{advertised}{base_path}"),
            lease_token,
            lease_expires_unix_ms: expiry_unix_ms(expires_at),
        })
    }

    async fn start_standalone_proxy(
        self: &Arc<Self>,
        session_id: &str,
        session_dir: &std::path::Path,
        provider_id: String,
        bind_ip: IpAddr,
        port: u16,
    ) -> Result<(StandaloneProxy, SocketAddr, String)> {
        let active_policy = self.materialize_active_policy(session_dir)?;
        let target = capsem_core::net::mitm_proxy::StandaloneModelTarget::from_registry(
            &active_policy.runtime.model_endpoints,
            &provider_id,
        )
        .map_err(anyhow::Error::msg)?;
        let (broker, upstream_policy) = upstream_broker::PendingBroker::pair(active_policy.broker_policy())?;
        let listener = TcpListener::bind(SocketAddr::new(bind_ip, port))
            .await
            .context("bind standalone proxy data listener")?;
        let local_addr = listener
            .local_addr()
            .context("read standalone proxy listener address")?;
        let log = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(session_dir.join("proxy.log"))
            .await
            .context("open standalone proxy log")?
            .into_std()
            .await;
        let worker = proxy_worker::ProxyWorker::spawn_standalone(
            &self.proxy_binary,
            active_policy.bytes,
            Stdio::from(log.try_clone()?),
            Stdio::from(log),
            provider_id.clone(),
        )
        .await?;
        if let Err(error) = self.grant_proxy_ledger(session_id, session_dir, &worker).await {
            let _ = worker.shutdown().await;
            return Err(error);
        }
        if let Err(error) = self.grant_proxy_credentials(&worker).await {
            let _ = worker.shutdown().await;
            let _ = self.ledger_workers.shutdown(session_id).await;
            return Err(error);
        }
        let proxy_upstream = broker.worker_stream()?;
        if let Err(error) = worker.grant(ProxyCapability::Upstream, proxy_upstream.into()).await {
            let _ = worker.shutdown().await;
            let _ = self.ledger_workers.shutdown(session_id).await;
            return Err(error.context("grant standalone proxy upstream broker"));
        }

        let authority = WorkerAuthority::default();
        if let Err(error) = crate::service_runtime::telemetry_export::grant_standalone_proxy_metric_broker(
            self,
            session_id,
            authority.grant(),
            &worker,
        )
        .await
        {
            authority.revoke();
            let _ = worker.shutdown().await;
            let _ = self.ledger_workers.shutdown(session_id).await;
            return Err(error);
        }
        let broker_task = broker.start(authority.grant());
        let accept_task = spawn_accept_loop(listener, worker.clone(), authority.grant());
        let lease_token = uuid::Uuid::new_v4().simple().to_string();
        Ok((
            StandaloneProxy {
                lease_token,
                expires_at: Instant::now() + LEASE_TTL,
                worker,
                authority,
                accept_task,
                broker_task,
                session_dir: session_dir.to_path_buf(),
                provider_id,
                upstream_policy,
            },
            local_addr,
            target.base_path().to_string(),
        ))
    }

    async fn renew_standalone_proxy(
        &self,
        session_id: &str,
        request: ProxyLeaseRequest,
    ) -> Result<ProxyHeartbeatResponse, AppError> {
        let mut proxies = self.standalone_proxies.lock().await;
        let entry = proxies
            .get_mut(session_id)
            .ok_or_else(|| AppError(axum::http::StatusCode::NOT_FOUND, "proxy session not found".to_string()))?;
        verify_lease(entry, &request)?;
        entry.expires_at = Instant::now() + LEASE_TTL;
        let expires_at = entry.expires_at;
        drop(proxies);
        Ok(ProxyHeartbeatResponse {
            session_id: session_id.to_string(),
            lease_expires_unix_ms: expiry_unix_ms(expires_at),
        })
    }

    async fn stop_standalone_proxy(
        &self,
        session_id: &str,
        request: ProxyLeaseRequest,
    ) -> Result<StopProxyResponse, AppError> {
        let entry = {
            let mut proxies = self.standalone_proxies.lock().await;
            let entry = proxies
                .get(session_id)
                .ok_or_else(|| AppError(axum::http::StatusCode::NOT_FOUND, "proxy session not found".to_string()))?;
            verify_lease(entry, &request)?;
            proxies.remove(session_id).expect("validated proxy still registered")
        };
        cleanup(self, session_id, entry)
            .await
            .map_err(|error| internal(format!("stop proxy: {error:#}")))?;
        Ok(StopProxyResponse {
            session_id: session_id.to_string(),
            stopped: true,
        })
    }

    pub(crate) async fn stop_all_standalone_proxies(&self) {
        let entries = std::mem::take(&mut *self.standalone_proxies.lock().await);
        for (session_id, entry) in entries {
            if let Err(error) = cleanup(self, &session_id, entry).await {
                tracing::warn!(%session_id, %error, "standalone proxy shutdown failed");
            }
        }
    }
}

pub(crate) async fn refresh_policies(state: &Arc<ServiceState>) -> Result<(usize, Vec<String>), AppError> {
    let targets = {
        let proxies = state.standalone_proxies.lock().await;
        proxies
            .iter()
            .map(|(id, entry)| {
                (
                    id.clone(),
                    entry.session_dir.clone(),
                    entry.provider_id.clone(),
                    entry.worker.clone(),
                    entry.worker.generation(),
                    entry.upstream_policy.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let total = targets.len();
    let materialized = state
        .off_worker({
            let targets = targets
                .iter()
                .map(|(id, dir, ..)| (id.clone(), dir.clone()))
                .collect::<Vec<_>>();
            move |state| {
                targets
                    .into_iter()
                    .map(|(id, dir)| {
                        let result = state
                            .materialize_active_policy(&dir)
                            .map_err(|error| format!("{error:#}"));
                        (id, result)
                    })
                    .collect::<Vec<_>>()
            }
        })
        .await?;

    let by_id = materialized.into_iter().collect::<std::collections::HashMap<_, _>>();
    let mut failures = Vec::new();
    for (id, _, provider_id, worker, generation, publisher) in targets {
        let still_current = state
            .standalone_proxies
            .lock()
            .await
            .get(&id)
            .is_some_and(|entry| entry.worker.generation() == generation);
        if !still_current {
            continue;
        }
        let Some(published) = by_id.get(&id) else {
            continue;
        };
        let published = match published {
            Ok(published) => published,
            Err(error) => {
                failures.push(format!("{id}: {error}"));
                continue;
            }
        };
        if let Err(error) = capsem_core::net::mitm_proxy::StandaloneModelTarget::from_registry(
            &published.runtime.model_endpoints,
            &provider_id,
        ) {
            failures.push(format!("{id}: {error}"));
            continue;
        }
        match worker.apply_policy(published.bytes.clone()).await {
            Ok(applied) if applied == published.digest => {
                if let Err(error) = publisher.publish(published.broker_policy()).await {
                    failures.push(format!("{id}: {error}"));
                }
            }
            Ok(applied) => failures.push(format!(
                "{id}: proxy applied active policy {applied}, expected {}",
                published.digest
            )),
            Err(error) => failures.push(format!("{id}: proxy reload refused: {error:#}")),
        }
    }
    Ok((total, failures))
}

pub(crate) async fn handle_create(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<CreateProxyRequest>,
) -> Result<Json<CreateProxyResponse>, AppError> {
    state.create_standalone_proxy(request).await.map(Json)
}

pub(crate) async fn handle_heartbeat(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(request): Json<ProxyLeaseRequest>,
) -> Result<Json<ProxyHeartbeatResponse>, AppError> {
    state.renew_standalone_proxy(&id, request).await.map(Json)
}

pub(crate) async fn handle_stop(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(request): Json<ProxyLeaseRequest>,
) -> Result<Json<StopProxyResponse>, AppError> {
    state.stop_standalone_proxy(&id, request).await.map(Json)
}

fn spawn_accept_loop(
    listener: TcpListener,
    worker: proxy_worker::ProxyWorker,
    authority: crate::instance::WorkerGrant,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                biased;
                () = authority.revoked() => return,
                accepted = listener.accept() => accepted,
            };
            let (stream, peer) = match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!(%error, "standalone proxy listener failed");
                    return;
                }
            };
            let descriptor = match stream.into_std() {
                Ok(stream) => stream.into(),
                Err(error) => {
                    tracing::warn!(%peer, %error, "standalone proxy could not adopt client");
                    continue;
                }
            };
            if let Err(error) = worker.grant(ProxyCapability::HttpTraffic, descriptor).await {
                tracing::debug!(%peer, %error, "standalone proxy worker stopped accepting traffic");
                return;
            }
        }
    })
}

fn spawn_lease_monitor(state: Arc<ServiceState>, session_id: String) {
    tokio::spawn(async move {
        loop {
            let sleep_for = {
                let proxies = state.standalone_proxies.lock().await;
                let Some(entry) = proxies.get(&session_id) else {
                    return;
                };
                let sleep_for = entry.expires_at.saturating_duration_since(Instant::now());
                drop(proxies);
                sleep_for
            };
            tokio::time::sleep(sleep_for).await;
            let expired = {
                let mut proxies = state.standalone_proxies.lock().await;
                if proxies
                    .get(&session_id)
                    .is_some_and(|entry| entry.expires_at <= Instant::now())
                {
                    proxies.remove(&session_id)
                } else {
                    None
                }
            };
            if let Some(entry) = expired {
                if let Err(error) = cleanup(&state, &session_id, entry).await {
                    tracing::warn!(%session_id, %error, "expired standalone proxy cleanup failed");
                }
                return;
            }
        }
    });
}

fn spawn_worker_monitor(
    state: Arc<ServiceState>,
    session_id: String,
    generation: capsem_proto::proxy_control::ProxyGeneration,
    mut stopped: tokio::sync::watch::Receiver<Option<String>>,
) {
    tokio::spawn(async move {
        if stopped.borrow().is_none() && stopped.changed().await.is_err() {
            return;
        }
        let removed = {
            let mut proxies = state.standalone_proxies.lock().await;
            if proxies
                .get(&session_id)
                .is_some_and(|entry| entry.worker.generation() == generation)
            {
                proxies.remove(&session_id)
            } else {
                None
            }
        };
        if let Some(entry) = removed {
            let reason = stopped
                .borrow()
                .clone()
                .unwrap_or_else(|| "proxy worker stopped".to_string());
            if let Err(error) = cleanup(&state, &session_id, entry).await {
                tracing::warn!(%session_id, %reason, %error, "stopped standalone proxy cleanup failed");
            }
        }
    });
}

async fn cleanup(state: &ServiceState, session_id: &str, entry: StandaloneProxy) -> Result<()> {
    entry.authority.revoke();
    entry.accept_task.abort();
    let _ = entry.accept_task.await;
    let worker_result = entry.worker.shutdown().await;
    let ledger_result = state.ledger_workers.shutdown(session_id).await;
    let mut broker_task = entry.broker_task;
    if tokio::time::timeout(CLEANUP_TIMEOUT, &mut broker_task).await.is_err() {
        broker_task.abort();
        let _ = broker_task.await;
    }
    let remove_result = tokio::fs::remove_dir_all(&entry.session_dir).await;
    worker_result.context("shutdown proxy worker")?;
    ledger_result.context("shutdown proxy ledger")?;
    remove_result.with_context(|| format!("remove {}", entry.session_dir.display()))?;
    Ok(())
}

fn verify_lease(entry: &StandaloneProxy, request: &ProxyLeaseRequest) -> Result<(), AppError> {
    if entry.lease_token != request.lease_token {
        return Err(AppError(
            axum::http::StatusCode::FORBIDDEN,
            "proxy lease token is invalid".to_string(),
        ));
    }
    Ok(())
}

fn expiry_unix_ms(expires_at: Instant) -> u64 {
    let remaining = expires_at.saturating_duration_since(Instant::now());
    SystemTime::now()
        .checked_add(remaining)
        .unwrap_or(SystemTime::now())
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn internal(message: String) -> AppError {
    AppError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, message)
}

#[cfg(test)]
mod tests;

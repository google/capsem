//! Metric export for the service: its own facade metrics, and every
//! session's totals observed from the ledgers' counter snapshots.
//!
//! Off unless the corp config's `open_telemetry` or `OTEL_EXPORTER_OTLP_*`
//! asks for it (see `capsem_telemetry::export`). The session instruments are
//! observable: OpenTelemetry calls them on each export, synchronously, so a
//! background task keeps a table of the latest snapshots and the callbacks
//! read that. Attributes are exactly `session.id` and
//! `persistent`.

use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use capsem_core::net::policy_config::SettingsFile;
use capsem_logger::counters::{usd_from_micro, LedgerCounters};
use capsem_telemetry::export::{Destination, Exporter, KeyValue, Meter};
use capsem_telemetry::session as names;

use super::*;

/// How often the session table is refreshed. Well inside the SDK's default
/// 60 s export interval, so each export sees totals at most this old.
const REFRESH_INTERVAL: Duration = Duration::from_secs(15);

/// Install export for the service when the corp config or environment asks
/// for it. `None` means export is off; a failure to install is logged and
/// leaves it off -- metrics are never a reason the service does not start.
pub(crate) fn install() -> Option<Exporter> {
    let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    let destination = Destination::resolve(corp.corp_rule_files.open_telemetry.as_deref(), |name| {
        std::env::var(name).ok()
    })?;
    match capsem_telemetry::export::install(&destination, "capsem-service", Vec::new()) {
        Ok(exporter) => {
            info!(?destination, "metric export installed");
            Some(exporter)
        }
        Err(error) => {
            warn!(%error, ?destination, "metric export not installed");
            None
        }
    }
}

/// Grant a VM process access to the local metric broker.
///
/// The presence of a valid corp endpoint decides whether export is on, but
/// neither that endpoint nor environment-carried collector credentials enter
/// the guest-facing process.
pub(crate) fn grant_metric_broker(command: &mut tokio::process::Command, corp: &SettingsFile) {
    if Destination::corp(corp.corp_rule_files.open_telemetry.as_deref()).is_some() {
        command.arg("--metric-broker");
    }
}

/// Grant the confined proxy a generation-bound metrics relay. The channel
/// carries only OTLP bodies; collector identity and credentials stay here.
pub(crate) async fn grant_proxy_metric_broker(
    state: &Arc<ServiceState>,
    id: &str,
    generation: uuid::Uuid,
    worker: &crate::proxy_worker::ProxyWorker,
) -> anyhow::Result<()> {
    let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    if Destination::corp(corp.corp_rule_files.open_telemetry.as_deref()).is_none() {
        return Ok(());
    }
    let owner = {
        let instances = state.instances.lock().unwrap();
        let instance = instances
            .get(id)
            .filter(|instance| instance.generation == generation)
            .ok_or_else(|| anyhow::anyhow!("VM owner changed before proxy metric grant"))?;
        let owner = owner_connection::OwnerConnection::capture(instance).map_err(anyhow::Error::msg)?;
        drop(instances);
        owner
    };
    let (service, proxy) = std::os::unix::net::UnixStream::pair().context("create proxy metric capability")?;
    let serving = tokio::spawn(serve_proxy_metric_channel(
        Arc::clone(state),
        id.to_string(),
        owner,
        service,
    ));
    if let Err(error) = worker
        .grant(capsem_proto::proxy_control::ProxyCapability::Telemetry, proxy)
        .await
    {
        serving.abort();
        let _ = serving.await;
        return Err(error.context("grant proxy metric capability"));
    }
    tokio::spawn(async move {
        match serving.await {
            Ok(Ok(())) => tracing::debug!("proxy metric capability disconnected"),
            Ok(Err(error)) => tracing::warn!(%error, "proxy metric capability failed"),
            Err(error) => tracing::warn!(%error, "proxy metric capability task failed"),
        }
    });
    Ok(())
}

const COLLECTOR_TIMEOUT: Duration = Duration::from_secs(10);
static RELAY_CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();

fn relay_client() -> Result<&'static reqwest::Client, AppError> {
    RELAY_CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(COLLECTOR_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|_| AppError(StatusCode::BAD_GATEWAY, "metric collector client unavailable".into()))
}

fn current_collector_url() -> Result<reqwest::Url, AppError> {
    let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    let Some(Destination::Corp(base)) = Destination::corp(corp.corp_rule_files.open_telemetry.as_deref()) else {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            "metric export is not configured".into(),
        ));
    };
    let url = reqwest::Url::parse(&format!("{base}/v1/metrics")).map_err(|_| {
        AppError(
            StatusCode::BAD_GATEWAY,
            "configured metric collector endpoint is invalid".into(),
        )
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError(
            StatusCode::BAD_GATEWAY,
            "configured metric collector endpoint must use HTTP or HTTPS".into(),
        ));
    }
    Ok(url)
}

/// Relay one worker's encoded OTLP metrics to the current fixed collector.
pub(crate) async fn handle_metric_relay(
    State(state): State<Arc<ServiceState>>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<ServicePeer>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, axum::http::HeaderMap, axum::body::Bytes), AppError> {
    let owner = owner_connection::OwnerConnection::current(&state, &id, peer.0)
        .map_err(|error| AppError(StatusCode::FORBIDDEN, error))?;
    if headers.get(axum::http::header::CONTENT_TYPE)
        != Some(&axum::http::HeaderValue::from_static("application/x-protobuf"))
    {
        return Err(AppError(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "metric broker accepts OTLP protobuf only".into(),
        ));
    }
    if body.len() > capsem_core::service_uds::MAX_BODY_BYTES {
        return Err(AppError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "metric request body is too large".into(),
        ));
    }
    let response = relay_metric_body(&state, &id, &owner, body).await?;
    Ok((response.status, response.headers, response.body))
}

struct MetricRelayResponse {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
}

async fn relay_metric_body(
    state: &Arc<ServiceState>,
    id: &str,
    owner: &owner_connection::OwnerConnection,
    body: axum::body::Bytes,
) -> Result<MetricRelayResponse, AppError> {
    if body.len() > capsem_proto::proxy_metrics::MAX_PROXY_METRIC_BODY_BYTES {
        return Err(AppError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "metric request body is too large".into(),
        ));
    }
    let collector = current_collector_url()?;
    owner
        .validate(state, false)
        .map_err(|error| AppError(StatusCode::FORBIDDEN, error))?;
    let request = relay_client()?
        .post(collector)
        .header(axum::http::header::CONTENT_TYPE, "application/x-protobuf")
        .body(body)
        .send();
    tokio::pin!(request);
    let response = match tokio::select! {
        biased;
        () = owner.revoked() => {
            return Err(AppError(StatusCode::FORBIDDEN, "VM owner authority was revoked".into()));
        }
        response = &mut request => response,
    } {
        Ok(response) => response,
        Err(error) => {
            warn!(vm = id, %error, "metric collector request failed");
            let status = if error.is_timeout() {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::BAD_GATEWAY
            };
            return Err(AppError(status, "metric collector request failed".into()));
        }
    };
    let status = response.status();
    let content_type = response.headers().get(axum::http::header::CONTENT_TYPE).cloned();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    use futures::StreamExt as _;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            warn!(vm = id, %error, "metric collector response failed");
            AppError(StatusCode::BAD_GATEWAY, "metric collector response failed".into())
        })?;
        if bytes
            .len()
            .checked_add(chunk.len())
            .is_none_or(|length| length > capsem_core::service_uds::MAX_BODY_BYTES)
        {
            return Err(AppError(
                StatusCode::BAD_GATEWAY,
                "metric collector response is too large".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    owner
        .validate(state, false)
        .map_err(|error| AppError(StatusCode::FORBIDDEN, error))?;
    let mut response_headers = axum::http::HeaderMap::new();
    if let Some(content_type) = content_type {
        response_headers.insert(axum::http::header::CONTENT_TYPE, content_type);
    }
    Ok(MetricRelayResponse {
        status,
        headers: response_headers,
        body: axum::body::Bytes::from(bytes),
    })
}

pub(crate) async fn serve_proxy_metric_channel(
    state: Arc<ServiceState>,
    id: String,
    owner: owner_connection::OwnerConnection,
    stream: std::os::unix::net::UnixStream,
) -> anyhow::Result<()> {
    use capsem_proto::proxy_metrics::{ProxyMetricRequest, ProxyMetricResponse};

    let (responses, requests) =
        capsem_foundation::ipc_channel::channel_from_std::<ProxyMetricResponse, ProxyMetricRequest>(stream)
            .context("open proxy metric channel")?;
    loop {
        let request = match requests.recv().await {
            Ok(request) => request,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error).context("receive proxy metric request"),
        };
        let response = match relay_metric_body(&state, &id, &owner, request.body.into()).await {
            Ok(relayed) => ProxyMetricResponse::Relayed {
                status: relayed.status.as_u16(),
                content_type: relayed
                    .headers
                    .get(axum::http::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
                body: relayed.body.to_vec(),
            },
            Err(AppError(_, message)) => ProxyMetricResponse::Rejected { message },
        };
        responses.send(response).await.context("send proxy metric response")?;
    }
}

/// One session's totals and the attributes its series carry.
#[derive(Clone, Debug)]
pub(crate) struct SessionTotals {
    pub(crate) attributes: Vec<KeyValue>,
    pub(crate) counters: Arc<LedgerCounters>,
}

/// The table the observable callbacks read.
pub(crate) type SessionTable = Arc<RwLock<Vec<SessionTotals>>>;

/// Register the session instruments on `meter`, reading `table`.
///
/// The instruments are returned so the caller keeps them; OpenTelemetry
/// observes them for as long as they, and the provider, live.
pub(crate) fn register(meter: &Meter, table: &SessionTable) -> Vec<Box<dyn std::any::Any + Send + Sync>> {
    fn describe(name: &'static str) -> &'static str {
        capsem_telemetry::all()
            .find(|spec| spec.name == name)
            .map(|spec| spec.description)
            .unwrap_or_default()
    }
    type Observe = fn(&LedgerCounters, &mut dyn FnMut(u64, &[KeyValue]));
    let counts: [(&'static str, Observe); 6] = [
        (names::SESSION_REQUESTS_TOTAL, |c, emit| {
            emit(c.net.allowed, &[KeyValue::new("decision", "allowed")]);
            emit(c.net.denied, &[KeyValue::new("decision", "denied")]);
            emit(c.net.error, &[KeyValue::new("decision", "error")]);
        }),
        (names::SESSION_TOKENS_TOTAL, |c, emit| {
            emit(c.model.total.input_tokens, &[KeyValue::new("direction", "input")]);
            emit(c.model.total.output_tokens, &[KeyValue::new("direction", "output")]);
        }),
        (names::SESSION_MODEL_CALLS_TOTAL, |c, emit| {
            emit(c.model.total.calls, &[])
        }),
        (names::SESSION_TOOL_CALLS_TOTAL, |c, emit| emit(c.tools.calls, &[])),
        (names::SESSION_FILE_EVENTS_TOTAL, |c, emit| {
            let overflow = c
                .files
                .by_action
                .get(capsem_logger::FileAction::Overflow.as_str())
                .copied()
                .unwrap_or_default();
            emit(c.files.events.saturating_sub(overflow), &[]);
        }),
        (names::SESSION_RULE_MATCHES_TOTAL, |c, emit| {
            emit(c.security.matches, &[])
        }),
    ];
    let mut instruments: Vec<Box<dyn std::any::Any + Send + Sync>> = Vec::new();
    for (name, observe) in counts {
        let table = Arc::clone(table);
        instruments.push(Box::new(
            meter
                .u64_observable_counter(name)
                .with_description(describe(name))
                .with_unit("{count}")
                .with_callback(move |observer| {
                    for session in table.read().unwrap_or_else(|poisoned| poisoned.into_inner()).iter() {
                        observe(&session.counters, &mut |value, extra| {
                            let mut attributes = session.attributes.clone();
                            attributes.extend_from_slice(extra);
                            observer.observe(value, &attributes);
                        });
                    }
                })
                .build(),
        ));
    }
    let cost_table = Arc::clone(table);
    instruments.push(Box::new(
        meter
            .f64_observable_counter(names::SESSION_COST_USD_TOTAL)
            .with_description(describe(names::SESSION_COST_USD_TOTAL))
            .with_unit("USD")
            .with_callback(move |observer| {
                for session in cost_table
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .iter()
                {
                    observer.observe(
                        usd_from_micro(session.counters.model.total.cost_micro_usd),
                        &session.attributes,
                    );
                }
            })
            .build(),
    ));
    instruments
}

/// Every listed VM's totals, from its ledger's counter snapshot. A VM whose
/// ledger is not ready has no series, not zeros.
pub(crate) async fn collect(state: &Arc<ServiceState>) -> Result<Vec<SessionTotals>, AppError> {
    let (listed, session_dirs) = state
        .off_worker(|state| sandbox_info::build_list_response(&state))
        .await?;
    let mut totals = Vec::with_capacity(listed.sandboxes.len());
    for (info, session_dir) in listed.sandboxes.iter().zip(&session_dirs) {
        let Some(counters) = ledger_routes::activity::counters_if_ready(state, &info.id, session_dir).await else {
            continue;
        };
        totals.push(SessionTotals {
            attributes: vec![
                KeyValue::new("session.id", info.id.clone()),
                KeyValue::new("persistent", info.persistent),
            ],
            counters,
        });
    }
    Ok(totals)
}

/// Keep the table current for as long as the service runs.
pub(crate) fn spawn_refresh(state: Arc<ServiceState>, table: SessionTable) {
    tokio::spawn(async move {
        loop {
            match collect(&state).await {
                Ok(totals) => *table.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = totals,
                Err(error) => warn!(error = %error.1, "session metric refresh failed"),
            }
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    });
}

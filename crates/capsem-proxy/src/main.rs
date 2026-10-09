use std::collections::{BTreeMap, HashSet};
use std::io;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use capsem_core::credential_broker::{BrokeredCredential, BrokeredUpstreamCredentials, CredentialObservation};
use capsem_core::net::ai_traffic::provider::ProviderKind;
use capsem_core::net::proxy_engine::{
    ProxyCapabilityFuture, ProxyCredentials, ProxyEngine, ProxyLedger, ProxyPolicyHandle, ProxyRuntimePolicy,
};
use capsem_foundation::ipc_channel;
use capsem_foundation::unix::worker_sandbox::{Policy, Role};
use capsem_foundation::unix::{fd, router_channel};
use capsem_logger::WriteOp;
use capsem_proto::proxy_control::{
    decode_proxy_control_request, encode_proxy_control_event, ProxyCapability, ProxyChannelCloseReason,
    ProxyControlEvent, ProxyControlRejection, ProxyControlRequest, ProxyGeneration, PROXY_CONTROL_FRAME_SIZE,
    PROXY_CONTROL_MAX_FDS,
};
use capsem_proto::proxy_policy::{ProxyPolicyRequest, ProxyPolicyResponse};
use clap::Parser;
use tokio::task::{JoinError, JoinSet};

mod credential_client;
mod mcp_client;
mod metric_client;
mod private_names_client;

const CONTROL_QUEUE_CAPACITY: usize = 16;
const GRANT_LIMIT: usize = 71;
const TRAFFIC_GRANT_LIMIT: usize = 64;
const CONTROL_SEND_TIMEOUT: Duration = Duration::from_secs(5);

type ControlSender = router_channel::DescriptorSender<PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS>;
type ControlReceiver = router_channel::DescriptorReceiver<PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS>;
type ControlFrame = router_channel::DescriptorFrame<PROXY_CONTROL_FRAME_SIZE>;
type CapabilityResult = (ProxyCapability, u64, ProxyChannelCloseReason);

#[derive(Parser)]
#[command(version, about = "Confined per-session Capsem proxy worker")]
struct Args {
    #[arg(long)]
    parent_pid: u32,
    /// Sixteen-byte worker generation as 32 lowercase hexadecimal digits.
    #[arg(long, value_parser = parse_generation)]
    generation: ProxyGeneration,
    /// Trusted configured provider selected by the service for standalone mode.
    #[arg(long)]
    standalone_provider: Option<String>,
}

fn main() -> Result<()> {
    // SAFETY: process entry, before telemetry, runtimes or capability
    // channels can observe environment state or own unrelated descriptors.
    unsafe {
        capsem_foundation::unix::process::clear_inherited_environment();
        fd::close_inherited_descriptors()?;
    }
    let _telemetry = capsem_foundation::telemetry::init(capsem_foundation::telemetry::TelemetryConfig {
        service: "capsem-proxy",
        sink: capsem_foundation::telemetry::LogSink::Stderr,
        default_filter: "capsem_proxy=info,capsem_core=info,capsem_foundation=warn",
    })?;
    let args = Args::parse();
    capsem_guard::watch_parent_or_exit(Some(args.parent_pid))?;
    let stdin = io::stdin();
    let control = UnixStream::from(fd::duplicate(stdin.as_fd())?);
    let denied_file = std::env::current_exe().context("resolve proxy executable before confinement")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    capsem_foundation::unix::worker_sandbox::confine(&Policy::new(Role::Proxy))
        .context("confine proxy worker before readiness")?;
    attest_confinement(&denied_file, args.parent_pid)?;
    runtime.block_on(run_control(control, args.generation, args.standalone_provider))
}

fn attest_confinement(denied_file: &std::path::Path, parent_pid: u32) -> Result<()> {
    if std::env::vars_os().next().is_some() {
        bail!("proxy worker retained inherited environment state");
    }
    require_denied(std::fs::read(denied_file), "read an ambient file")?;
    require_denied(UnixStream::pair(), "create a Unix socket")?;
    require_denied(
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)),
        "create a TCP listener",
    )?;
    require_denied(
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 9)),
        "dial a TCP address",
    )?;
    require_denied(Command::new(denied_file).arg("--version").status(), "execute a process")?;
    let parent = capsem_foundation::unix::process::ProcessId::try_from(parent_pid)?;
    if capsem_foundation::unix::process::probe_signal_authority(parent)?
        != capsem_foundation::unix::process::SignalProbe::Denied
    {
        bail!("proxy confinement allowed it to signal its parent");
    }
    Ok(())
}

fn require_denied<T>(result: io::Result<T>, operation: &str) -> Result<()> {
    if result.is_ok() {
        bail!("proxy confinement allowed it to {operation}");
    }
    Ok(())
}

async fn run_control(
    control: UnixStream,
    generation: ProxyGeneration,
    standalone_provider: Option<String>,
) -> Result<()> {
    let shutdown = control.try_clone()?;
    let sender = ControlSender::new(control.try_clone()?)?;
    let receiver = ControlReceiver::new(control)?;
    let (frames_sender, mut frames) = tokio::sync::mpsc::channel(CONTROL_QUEUE_CAPACITY);
    let reader = tokio::spawn(read_control(receiver, frames_sender));

    let outcome = async {
        send_event(&sender, ProxyControlEvent::Ready { generation }).await?;
        let state = Arc::new(Mutex::new(ProxyRuntimeState::new(standalone_provider)));
        let mut grants = BTreeMap::<ProxyCapability, Vec<u64>>::new();
        let mut grant_ids = HashSet::new();
        let mut tasks = JoinSet::<CapabilityResult>::new();

        loop {
            let input = if tasks.is_empty() {
                ControlInput::Frame(frames.recv().await.context("proxy control channel closed")?)
            } else {
                tokio::select! {
                    frame = frames.recv() => {
                        ControlInput::Frame(frame.context("proxy control channel closed")?)
                    }
                    result = tasks.join_next() => {
                        ControlInput::Capability(result.expect("nonempty proxy capability task set"))
                    }
                }
            };
            match input {
                ControlInput::Capability(result) => {
                    let (capability, grant_id, reason) = result.context("proxy capability task failed")?;
                    remove_grant(&mut grants, &mut grant_ids, capability, grant_id);
                    send_event(
                        &sender,
                        ProxyControlEvent::Closed {
                            generation,
                            grant_id,
                            reason,
                        },
                    )
                    .await?;
                }
                ControlInput::Frame(frame) => {
                    let request = decode_proxy_control_request(&frame.bytes)?;
                    if request.generation() != generation {
                        bail!("proxy control request names a stale generation");
                    }
                    match request {
                        ProxyControlRequest::Attach(grant) => {
                            if frame.fds.len() != grant.expected_descriptor_count() {
                                bail!("proxy capability attach descriptor count does not match its grant");
                            }
                            let capability = grant.capability();
                            let grant_id = grant.grant_id();
                            let current = grants.get(&capability).map_or(0, Vec::len);
                            let rejection = if grant_ids.contains(&grant_id) {
                                Some(ProxyControlRejection::DuplicateGrant)
                            } else if grant_ids.len() >= GRANT_LIMIT
                                || (is_traffic(capability) && current >= TRAFFIC_GRANT_LIMIT)
                            {
                                Some(ProxyControlRejection::Capacity)
                            } else if !is_traffic(capability) && current != 0 {
                                Some(ProxyControlRejection::DuplicateCapability)
                            } else if is_traffic(capability)
                                && !state
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .traffic_ready(capability)?
                            {
                                Some(ProxyControlRejection::NotReady)
                            } else {
                                None
                            };
                            if let Some(reason) = rejection {
                                send_event(
                                    &sender,
                                    ProxyControlEvent::Rejected {
                                        generation,
                                        grant_id,
                                        reason,
                                    },
                                )
                                .await?;
                                continue;
                            }
                            let mut descriptors = frame.fds.into_iter();
                            let descriptor = descriptors.next().expect("at least one checked descriptor");
                            grants.entry(capability).or_default().push(grant_id);
                            grant_ids.insert(grant_id);
                            if capability == ProxyCapability::Policy {
                                let stream = UnixStream::from(descriptor);
                                let state = Arc::clone(&state);
                                tasks.spawn(async move {
                                    let reason = serve_policy(stream, state).await;
                                    (capability, grant_id, reason)
                                });
                            } else if capability == ProxyCapability::Ledger {
                                let stream = UnixStream::from(descriptor);
                                let commitment = UnixStream::from(
                                    descriptors.next().expect("ledger commitment descriptor was checked"),
                                );
                                let ledger_grant = grant
                                    .ledger_grant()
                                    .expect("ledger capability was decoded with exact authority");
                                let writer = tokio::task::spawn_blocking(move || {
                                    capsem_logger::DbWriter::from_ledger_channel(
                                        stream,
                                        commitment,
                                        ledger_grant,
                                        Path::new("capability:session-ledger"),
                                        1024,
                                    )
                                })
                                .await
                                .map_err(|error| anyhow::anyhow!("join proxy ledger handshake: {error}"))?
                                .context("authenticate proxy ledger capability")?;
                                state
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .attach_ledger(Arc::new(writer));
                            } else if capability == ProxyCapability::Credential {
                                let stream = UnixStream::from(descriptor);
                                let (credentials, mut closed) = credential_client::CredentialClient::start(stream)
                                    .context("open proxy credential capability")?;
                                state
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .attach_credentials(Arc::new(credentials));
                                tasks.spawn(async move {
                                    while closed.borrow().is_none() && closed.changed().await.is_ok() {}
                                    let reason = closed.borrow().unwrap_or(ProxyChannelCloseReason::Disconnected);
                                    (capability, grant_id, reason)
                                });
                            } else if capability == ProxyCapability::Upstream {
                                let stream = UnixStream::from(descriptor);
                                let grants = Arc::new(
                                    capsem_core::net::upstream_grant::UpstreamGrantClient::start(stream)
                                        .context("open proxy upstream capability")?,
                                );
                                let mut stopped = grants.stop_receiver();
                                let mut runtime = state.lock().unwrap_or_else(|error| error.into_inner());
                                runtime.attach_upstream(grants);
                                tracing::debug!(
                                    upstream_capability = runtime.upstream_grants.is_some(),
                                    "proxy upstream authority attached"
                                );
                                drop(runtime);
                                tasks.spawn(async move {
                                    while stopped.borrow().is_none() && stopped.changed().await.is_ok() {}
                                    (capability, grant_id, ProxyChannelCloseReason::Disconnected)
                                });
                            } else if capability == ProxyCapability::PrivateNames {
                                let stream = UnixStream::from(descriptor);
                                let (private_names, mut closed) =
                                    private_names_client::PrivateNameClient::start(stream)
                                        .context("open proxy private-name capability")?;
                                let mut runtime = state.lock().unwrap_or_else(|error| error.into_inner());
                                runtime.attach_private_names(Arc::new(private_names));
                                tracing::debug!(
                                    private_names_capability = runtime.private_names.is_some(),
                                    "proxy private-name authority attached"
                                );
                                drop(runtime);
                                tasks.spawn(async move {
                                    while closed.borrow().is_none() && closed.changed().await.is_ok() {}
                                    let reason = closed.borrow().unwrap_or(ProxyChannelCloseReason::Disconnected);
                                    (capability, grant_id, reason)
                                });
                            } else if capability == ProxyCapability::Mcp {
                                let stream = UnixStream::from(descriptor);
                                let (client, hello, mut closed) =
                                    mcp_client::start(stream).await.context("open proxy MCP capability")?;
                                state
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .attach_mcp(client, hello);
                                tasks.spawn(async move {
                                    while !*closed.borrow() && closed.changed().await.is_ok() {}
                                    (capability, grant_id, ProxyChannelCloseReason::Disconnected)
                                });
                            } else if capability == ProxyCapability::Telemetry {
                                let stream = UnixStream::from(descriptor);
                                let (client, session_id) = metric_client::start(stream)
                                    .await
                                    .context("open proxy metric capability")?;
                                let exporter = capsem_telemetry::export::install_with_http_client(
                                    client,
                                    "capsem-proxy",
                                    vec![capsem_telemetry::export::KeyValue::new("session.id", session_id)],
                                )
                                .context("install proxy metric exporter")?;
                                state
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .attach_metric_exporter(exporter);
                            } else if capability == ProxyCapability::HttpTraffic {
                                let runtime = state
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .http_runtime()?
                                    .context("proxy HTTP runtime became unavailable after readiness check")?;
                                tasks.spawn(async move {
                                    if let Some(target) = runtime.target {
                                        capsem_core::net::mitm_proxy::handle_standalone_connection(
                                            descriptor,
                                            runtime.config,
                                            target,
                                        )
                                        .await;
                                    } else {
                                        capsem_core::net::mitm_proxy::handle_connection(descriptor, runtime.config)
                                            .await;
                                    }
                                    (capability, grant_id, ProxyChannelCloseReason::Disconnected)
                                });
                            } else if capability == ProxyCapability::DnsTraffic {
                                let runtime = state
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .dns_runtime()
                                    .context("proxy DNS runtime became unavailable after readiness check")?;
                                tasks.spawn(async move {
                                    capsem_core::net::dns::session::serve_dns_session(
                                        descriptor,
                                        Arc::clone(&runtime.handler),
                                        Arc::clone(&runtime.db),
                                        runtime.policy.clone(),
                                    )
                                    .await;
                                    (capability, grant_id, ProxyChannelCloseReason::Disconnected)
                                });
                            } else {
                                bail!("proxy capability {capability:?} has no consumer");
                            }
                            send_event(&sender, ProxyControlEvent::Adopted { generation, grant_id }).await?;
                        }
                        ProxyControlRequest::Shutdown { .. } => {
                            if !frame.fds.is_empty() {
                                bail!("proxy shutdown must not carry descriptors");
                            }
                            break;
                        }
                    }
                }
            }
        }

        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        drop(grants);
        send_event(&sender, ProxyControlEvent::Stopped { generation }).await
    }
    .await;

    let _ = shutdown.shutdown(std::net::Shutdown::Both);
    let reader_outcome = reader.await.context("proxy control reader task failed")?;
    outcome?;
    reader_outcome?;
    Ok(())
}

const fn is_traffic(capability: ProxyCapability) -> bool {
    matches!(capability, ProxyCapability::HttpTraffic | ProxyCapability::DnsTraffic)
}

enum ControlInput {
    Frame(ControlFrame),
    Capability(std::result::Result<CapabilityResult, JoinError>),
}

fn remove_grant(
    grants: &mut BTreeMap<ProxyCapability, Vec<u64>>,
    grant_ids: &mut HashSet<u64>,
    capability: ProxyCapability,
    grant_id: u64,
) {
    grant_ids.remove(&grant_id);
    if let Some(ids) = grants.get_mut(&capability) {
        ids.retain(|candidate| *candidate != grant_id);
        if ids.is_empty() {
            grants.remove(&capability);
        }
    }
}

struct ProxyRuntimeState {
    standalone_provider: Option<String>,
    engine: Option<Arc<ProxyEngine>>,
    ledger: Arc<dyn ProxyLedger>,
    db: Option<Arc<capsem_logger::DbWriter>>,
    credentials: Arc<dyn ProxyCredentials>,
    credentials_attached: bool,
    upstream_grants: Option<Arc<capsem_core::net::upstream_grant::UpstreamGrantClient>>,
    private_names: Option<Arc<dyn capsem_core::net::dns::private::PrivateNames>>,
    dns_upstreams: Vec<std::net::SocketAddr>,
    mcp: capsem_core::mcp::policy::McpConfig,
    mcp_client: Option<(
        capsem_proto::mcp_aggregator::AggregatorClient,
        capsem_proto::proxy_mcp::ProxyMcpHello,
    )>,
    http_config: Option<Arc<capsem_core::net::mitm_proxy::MitmProxyConfig>>,
    dns_runtime: Option<Arc<DnsRuntime>>,
    metric_exporter: Option<capsem_telemetry::export::Exporter>,
}

impl Default for ProxyRuntimeState {
    fn default() -> Self {
        Self::new(None)
    }
}

struct HttpRuntime {
    config: Arc<capsem_core::net::mitm_proxy::MitmProxyConfig>,
    target: Option<capsem_core::net::mitm_proxy::StandaloneModelTarget>,
}

impl ProxyRuntimeState {
    fn new(standalone_provider: Option<String>) -> Self {
        Self {
            standalone_provider,
            engine: None,
            ledger: Arc::new(UnavailableLedger),
            db: None,
            credentials: Arc::new(UnavailableCredentials),
            credentials_attached: false,
            upstream_grants: None,
            private_names: None,
            dns_upstreams: Vec::new(),
            mcp: capsem_core::mcp::policy::McpConfig::default(),
            mcp_client: None,
            http_config: None,
            dns_runtime: None,
            metric_exporter: None,
        }
    }

    #[cfg(test)]
    fn standalone(provider: String) -> Self {
        Self::new(Some(provider))
    }
    fn apply(&mut self, active_policy: &[u8]) -> Result<String> {
        let runtime = ProxyRuntimePolicy::compile(active_policy).map_err(anyhow::Error::msg)?;
        let digest = runtime.snapshot().digest().to_string();
        let (snapshot, dns_upstreams, mcp) = runtime.into_parts();
        if let Some(engine) = &self.engine {
            engine.policy().replace(snapshot);
        } else {
            self.engine = Some(Arc::new(ProxyEngine::new(
                ProxyPolicyHandle::new(snapshot),
                Arc::clone(&self.ledger),
                Arc::clone(&self.credentials),
            )));
        }
        self.dns_upstreams = dns_upstreams;
        self.mcp = mcp;
        self.dns_runtime = None;
        Ok(digest)
    }

    fn attach_ledger(&mut self, ledger: Arc<capsem_logger::DbWriter>) {
        self.ledger = ledger.clone();
        self.db = Some(ledger);
        if let Some(engine) = self.engine.take() {
            self.engine = Some(Arc::new(ProxyEngine::new(
                engine.policy().clone(),
                Arc::clone(&self.ledger),
                Arc::clone(&self.credentials),
            )));
        }
        self.http_config = None;
        self.dns_runtime = None;
    }

    fn attach_credentials(&mut self, credentials: Arc<dyn ProxyCredentials>) {
        self.credentials = credentials;
        self.credentials_attached = true;
        if let Some(engine) = self.engine.take() {
            self.engine = Some(Arc::new(ProxyEngine::new(
                engine.policy().clone(),
                Arc::clone(&self.ledger),
                Arc::clone(&self.credentials),
            )));
        }
        self.http_config = None;
    }

    fn attach_upstream(&mut self, upstream: Arc<capsem_core::net::upstream_grant::UpstreamGrantClient>) {
        self.upstream_grants = Some(upstream);
        self.http_config = None;
        self.dns_runtime = None;
    }

    fn attach_private_names(&mut self, private_names: Arc<dyn capsem_core::net::dns::private::PrivateNames>) {
        self.private_names = Some(private_names);
        self.dns_runtime = None;
    }

    fn attach_mcp(
        &mut self,
        client: capsem_proto::mcp_aggregator::AggregatorClient,
        hello: capsem_proto::proxy_mcp::ProxyMcpHello,
    ) {
        self.mcp_client = Some((client, hello));
        self.http_config = None;
    }

    fn attach_metric_exporter(&mut self, exporter: capsem_telemetry::export::Exporter) {
        self.metric_exporter = Some(exporter);
    }

    fn traffic_ready(&mut self, capability: ProxyCapability) -> Result<bool> {
        match capability {
            ProxyCapability::HttpTraffic => Ok(self.http_runtime()?.is_some()),
            ProxyCapability::DnsTraffic => Ok(self.dns_runtime().is_some()),
            _ => Ok(true),
        }
    }

    fn http_config(&mut self) -> Result<Option<Arc<capsem_core::net::mitm_proxy::MitmProxyConfig>>> {
        if let Some(config) = &self.http_config {
            return Ok(Some(Arc::clone(config)));
        }
        let (Some(engine), Some(db), Some(upstream_grants)) = (&self.engine, &self.db, &self.upstream_grants) else {
            return Ok(None);
        };
        if !self.credentials_attached {
            return Ok(None);
        }
        let ca = Arc::new(capsem_core::net::cert_authority::CertAuthority::load(
            capsem_core::vm::boot::CA_KEY_PEM,
            capsem_core::vm::boot::CA_CERT_PEM,
        )?);
        let telemetry = Arc::new(capsem_core::net::mitm_proxy::telemetry_hook::TelemetryDeps {
            db: Arc::clone(db),
            credentials: engine.credentials(),
            pricing: Arc::new(capsem_core::net::ai_traffic::pricing::PricingTable::load()),
            trace_state: Arc::new(Mutex::new(capsem_core::net::ai_traffic::TraceState::new())),
        });
        let pipeline = capsem_core::net::mitm_proxy::make_production_pipeline(Arc::clone(&telemetry));
        let upstream_grants: Arc<dyn capsem_core::net::mitm_proxy::TcpUpstreamGrants> = upstream_grants.clone();
        let mcp_endpoint = match &self.mcp_client {
            Some((mcp_client, mcp_hello)) => Some(Arc::new(
                capsem_core::net::mitm_proxy::McpEndpointState::with_proxy_policy(
                    mcp_client.clone(),
                    engine.policy().clone(),
                    Arc::new(tokio::sync::Semaphore::new(usize::from(mcp_hello.inflight_cap))),
                    capsem_core::net::mitm_proxy::McpTimeouts {
                        default_timeout: Duration::from_millis(mcp_hello.default_timeout_ms),
                        tool_call_default: Duration::from_millis(mcp_hello.tool_call_default_ms),
                        tool_call_ceiling: Duration::from_millis(mcp_hello.tool_call_ceiling_ms),
                    },
                )
                .with_builtin_ledger(Arc::clone(db), mcp_hello.builtin_servers.iter().cloned().collect()),
            )),
            None if self.standalone_provider.is_some() => None,
            None => return Ok(None),
        };
        let config = Arc::new(capsem_core::net::mitm_proxy::MitmProxyConfig {
            ca: Arc::clone(&ca),
            server_tls: capsem_core::net::mitm_proxy::make_server_tls_config(&ca),
            engine: Arc::clone(engine),
            db: Arc::clone(db),
            upstream_tls: capsem_core::net::mitm_proxy::make_upstream_tls_config(),
            telemetry,
            pipeline,
            mcp_endpoint,
            upstream_resolver: capsem_core::net::upstream_address::UpstreamResolver::disabled(),
            upstream_grants: Some(upstream_grants),
        });
        self.http_config = Some(Arc::clone(&config));
        Ok(Some(config))
    }

    fn http_runtime(&mut self) -> Result<Option<HttpRuntime>> {
        let Some(config) = self.http_config()? else {
            return Ok(None);
        };
        let target = self
            .standalone_provider
            .as_deref()
            .map(|provider| {
                capsem_core::net::mitm_proxy::StandaloneModelTarget::from_registry(
                    config.engine.policy().snapshot().model_endpoints(),
                    provider,
                )
                .map_err(anyhow::Error::msg)
            })
            .transpose()?;
        Ok(Some(HttpRuntime { config, target }))
    }

    fn dns_runtime(&mut self) -> Option<Arc<DnsRuntime>> {
        if let Some(runtime) = &self.dns_runtime {
            return Some(Arc::clone(runtime));
        }
        let (Some(engine), Some(db), Some(upstream_grants), Some(private_names)) =
            (&self.engine, &self.db, &self.upstream_grants, &self.private_names)
        else {
            return None;
        };
        let grants: Arc<dyn capsem_core::net::dns::DnsUpstreamGrants> = upstream_grants.clone();
        let resolver = Arc::new(capsem_core::net::dns::DnsResolver::with_grants(
            self.dns_upstreams.clone(),
            grants,
        ));
        let policy = engine.policy().clone();
        let handler = Arc::new(
            capsem_core::net::dns::DnsHandler::with_cache(
                policy.clone(),
                resolver,
                Arc::new(capsem_core::net::dns::DnsAnswerCache::default()),
            )
            .with_private_names(Arc::clone(private_names)),
        );
        let runtime = Arc::new(DnsRuntime {
            handler,
            db: Arc::clone(db),
            policy,
        });
        self.dns_runtime = Some(Arc::clone(&runtime));
        Some(runtime)
    }
}

struct DnsRuntime {
    handler: Arc<capsem_core::net::dns::DnsHandler>,
    db: Arc<capsem_logger::DbWriter>,
    policy: ProxyPolicyHandle,
}

struct UnavailableLedger;

impl ProxyLedger for UnavailableLedger {
    fn write(&self, _op: WriteOp) -> ProxyCapabilityFuture<'_, std::result::Result<(), String>> {
        Box::pin(async { Err("proxy ledger capability is not attached".to_string()) })
    }
}

struct UnavailableCredentials;

impl ProxyCredentials for UnavailableCredentials {
    fn capture(&self, _observation: &CredentialObservation) -> std::result::Result<BrokeredCredential, String> {
        Err("proxy credential capability is not attached".to_string())
    }

    fn substitute_upstream(
        &self,
        _domain: &str,
        _ai_provider: Option<ProviderKind>,
        _headers: &mut http::HeaderMap,
        _query: Option<&str>,
    ) -> std::result::Result<BrokeredUpstreamCredentials, String> {
        Err("proxy credential capability is not attached".to_string())
    }
}

async fn serve_policy(stream: UnixStream, state: Arc<Mutex<ProxyRuntimeState>>) -> ProxyChannelCloseReason {
    let Ok((sender, receiver)) = ipc_channel::channel_from_std::<ProxyPolicyResponse, ProxyPolicyRequest>(stream)
    else {
        return ProxyChannelCloseReason::ProtocolError;
    };
    loop {
        let request = match receiver.recv().await {
            Ok(request) => request,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                return ProxyChannelCloseReason::Disconnected;
            }
            Err(error) => {
                tracing::warn!(%error, "proxy policy capability received an invalid frame");
                return ProxyChannelCloseReason::ProtocolError;
            }
        };
        let request_id = request.request_id();
        let response = if let Err(error) = request.validate() {
            ProxyPolicyResponse::rejected(request_id, error.to_string())
        } else {
            let ProxyPolicyRequest::Apply { active_policy, .. } = request;
            match state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .apply(&active_policy)
            {
                Ok(active_policy_digest) => ProxyPolicyResponse::Applied {
                    request_id,
                    active_policy_digest,
                },
                Err(error) => ProxyPolicyResponse::rejected(request_id, format!("{error:#}")),
            }
        };
        match tokio::time::timeout(CONTROL_SEND_TIMEOUT, sender.send(response)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) if error.kind() == io::ErrorKind::BrokenPipe => {
                return ProxyChannelCloseReason::Disconnected;
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "proxy policy capability could not send a response");
                return ProxyChannelCloseReason::ProtocolError;
            }
            Err(_) => return ProxyChannelCloseReason::ProtocolError,
        }
    }
}

async fn read_control(receiver: ControlReceiver, frames: tokio::sync::mpsc::Sender<ControlFrame>) -> io::Result<()> {
    loop {
        match receiver.recv().await {
            Ok(frame) => {
                if frames.send(frame).await.is_err() {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

async fn send_event(sender: &ControlSender, event: ProxyControlEvent) -> Result<()> {
    tokio::time::timeout(
        CONTROL_SEND_TIMEOUT,
        sender.send(&encode_proxy_control_event(event), &[]),
    )
    .await
    .context("proxy control event send timed out")??;
    Ok(())
}

fn parse_generation(value: &str) -> std::result::Result<ProxyGeneration, String> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("generation must be 32 lowercase hexadecimal digits".into());
    }
    let mut bytes = [0; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| "invalid generation")?;
    }
    let generation = ProxyGeneration::new(bytes);
    if generation.as_bytes() == [0; 16] {
        return Err("generation must not be zero".into());
    }
    Ok(generation)
}

#[cfg(test)]
mod tests;

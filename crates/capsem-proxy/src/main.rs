use std::collections::{BTreeMap, HashSet};
use std::io;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
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
    runtime.block_on(run_control(control, args.generation))
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

async fn run_control(control: UnixStream, generation: ProxyGeneration) -> Result<()> {
    let shutdown = control.try_clone()?;
    let sender = ControlSender::new(control.try_clone()?)?;
    let receiver = ControlReceiver::new(control)?;
    let (frames_sender, mut frames) = tokio::sync::mpsc::channel(CONTROL_QUEUE_CAPACITY);
    let reader = tokio::spawn(read_control(receiver, frames_sender));

    let outcome = async {
        send_event(&sender, ProxyControlEvent::Ready { generation }).await?;
        let state = Arc::new(Mutex::new(ProxyRuntimeState::default()));
        let mut grants = BTreeMap::<ProxyCapability, Vec<u64>>::new();
        let mut descriptors = BTreeMap::<u64, UnixStream>::new();
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
                            if frame.fds.len() != 1 {
                                bail!("proxy capability attach requires exactly one descriptor");
                            }
                            let capability = grant.capability();
                            let grant_id = grant.grant_id();
                            let current = grants.get(&capability).map_or(0, Vec::len);
                            let rejection = if grant_ids.contains(&grant_id) {
                                Some(ProxyControlRejection::DuplicateGrant)
                            } else if grant_ids.len() >= GRANT_LIMIT
                                || (capability == ProxyCapability::Traffic && current >= TRAFFIC_GRANT_LIMIT)
                            {
                                Some(ProxyControlRejection::Capacity)
                            } else if capability != ProxyCapability::Traffic && current != 0 {
                                Some(ProxyControlRejection::DuplicateCapability)
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
                            let descriptor = frame.fds.into_iter().next().expect("one checked descriptor");
                            let stream = UnixStream::from(descriptor);
                            grants.entry(capability).or_default().push(grant_id);
                            grant_ids.insert(grant_id);
                            if capability == ProxyCapability::Policy {
                                let state = Arc::clone(&state);
                                tasks.spawn(async move {
                                    let reason = serve_policy(stream, state).await;
                                    (capability, grant_id, reason)
                                });
                            } else {
                                descriptors.insert(grant_id, stream);
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
        drop(descriptors);
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

#[derive(Default)]
struct ProxyRuntimeState {
    engine: Option<ProxyEngine>,
    dns_upstreams: Vec<std::net::SocketAddr>,
    mcp: capsem_core::mcp::policy::McpConfig,
}

impl ProxyRuntimeState {
    fn apply(&mut self, active_policy: &[u8]) -> Result<String> {
        let runtime = ProxyRuntimePolicy::compile(active_policy).map_err(anyhow::Error::msg)?;
        let digest = runtime.snapshot().digest().to_string();
        let (snapshot, dns_upstreams, mcp) = runtime.into_parts();
        if let Some(engine) = &self.engine {
            engine.policy().replace(snapshot);
        } else {
            self.engine = Some(ProxyEngine::new(
                ProxyPolicyHandle::new(snapshot),
                Arc::new(UnavailableLedger),
                Arc::new(UnavailableCredentials),
            ));
        }
        self.dns_upstreams = dns_upstreams;
        self.mcp = mcp;
        Ok(digest)
    }
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

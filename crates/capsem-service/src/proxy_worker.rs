//! Trusted lifecycle and capability coordinator for one confined proxy worker.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use capsem_foundation::ipc_channel;
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::ledger::LedgerChannelGrant;
use capsem_proto::proxy_control::{
    decode_proxy_control_event, encode_proxy_control_request, ProxyCapability, ProxyChannelCloseReason,
    ProxyControlEvent, ProxyControlRequest, ProxyGeneration, PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS,
};
use capsem_proto::proxy_policy::{ProxyPolicyRequest, ProxyPolicyResponse};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch};

use crate::ServiceState;

#[cfg(not(test))]
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(not(test))]
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const OPERATION_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(not(test))]
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(200);
const COMMAND_QUEUE_CAPACITY: usize = 16;

type ControlSender = DescriptorSender<PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS>;
type ControlReceiver = DescriptorReceiver<PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS>;
type ControlFrame = capsem_foundation::unix::router_channel::DescriptorFrame<PROXY_CONTROL_FRAME_SIZE>;

/// A bounded command handle for one fresh proxy-worker generation.
#[derive(Clone)]
pub(crate) struct ProxyWorker {
    generation: ProxyGeneration,
    commands: mpsc::Sender<CommandRequest>,
    stopped: watch::Receiver<Option<String>>,
}

impl ProxyWorker {
    /// Spawn the production worker with an empty environment and no inherited
    /// descriptors other than its coordinator socket on stdin.
    pub(crate) async fn spawn(binary: &Path, active_policy: Vec<u8>, stdout: Stdio, stderr: Stdio) -> Result<Self> {
        Self::spawn_mode(binary, active_policy, stdout, stderr, ProxyWorkerMode::Vm).await
    }

    pub(crate) async fn spawn_standalone(
        binary: &Path,
        active_policy: Vec<u8>,
        stdout: Stdio,
        stderr: Stdio,
        provider_id: String,
    ) -> Result<Self> {
        Self::spawn_mode(
            binary,
            active_policy,
            stdout,
            stderr,
            ProxyWorkerMode::Standalone(provider_id),
        )
        .await
    }

    async fn spawn_mode(
        binary: &Path,
        active_policy: Vec<u8>,
        stdout: Stdio,
        stderr: Stdio,
        mode: ProxyWorkerMode,
    ) -> Result<Self> {
        let generation = fresh_generation();
        let mut command = Command::new(binary);
        command.env_clear().stdout(stdout).stderr(stderr).kill_on_drop(true);
        launch(command, generation, active_policy, move |command, generation| {
            configure_worker_command(command, generation, &mode);
        })
        .await
    }

    pub(crate) fn generation(&self) -> ProxyGeneration {
        self.generation
    }

    pub(crate) async fn grant(&self, capability: ProxyCapability, descriptor: OwnedFd) -> Result<()> {
        self.grant_capability(GrantedCapability::Ordinary(capability), vec![descriptor])
            .await
    }

    pub(crate) async fn grant_ledger(
        &self,
        stream: UnixStream,
        commitment: UnixStream,
        grant: LedgerChannelGrant,
    ) -> Result<()> {
        self.grant_capability(GrantedCapability::Ledger(grant), vec![stream.into(), commitment.into()])
            .await
    }

    async fn grant_capability(&self, capability: GrantedCapability, descriptors: Vec<OwnedFd>) -> Result<()> {
        let (completed, result) = oneshot::channel();
        self.commands
            .send(CommandRequest::Grant {
                capability,
                descriptors,
                completed,
            })
            .await
            .map_err(|_| anyhow!(self.stop_reason("proxy worker supervisor stopped before accepting a grant")))?;
        result
            .await
            .map_err(|_| anyhow!(self.stop_reason("proxy worker supervisor stopped with a grant pending")))?
    }

    pub(crate) async fn apply_policy(&self, active_policy: Vec<u8>) -> Result<String> {
        let (completed, result) = oneshot::channel();
        self.commands
            .send(CommandRequest::ApplyPolicy {
                active_policy,
                completed,
            })
            .await
            .map_err(|_| anyhow!(self.stop_reason("proxy worker supervisor stopped before accepting policy")))?;
        result
            .await
            .map_err(|_| anyhow!(self.stop_reason("proxy worker supervisor stopped with policy pending")))?
    }

    pub(crate) async fn shutdown(self) -> Result<()> {
        let (completed, result) = oneshot::channel();
        self.commands
            .send(CommandRequest::Shutdown { completed })
            .await
            .map_err(|_| anyhow!(self.stop_reason("proxy worker supervisor already stopped")))?;
        result
            .await
            .map_err(|_| anyhow!(self.stop_reason("proxy worker supervisor stopped during shutdown")))?
    }

    pub(crate) fn stop_receiver(&self) -> watch::Receiver<Option<String>> {
        self.stopped.clone()
    }

    fn stop_reason(&self, fallback: &str) -> String {
        self.stopped.borrow().clone().unwrap_or_else(|| fallback.to_string())
    }

    #[cfg(test)]
    pub(crate) fn test_stub() -> Self {
        let generation = fresh_generation();
        let (commands, mut requests) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let (stopped_tx, stopped) = watch::channel(None);
        tokio::spawn(async move {
            while let Some(request) = requests.recv().await {
                match request {
                    CommandRequest::Grant { completed, .. } => {
                        let _ = completed.send(Ok(()));
                    }
                    CommandRequest::Shutdown { completed } => {
                        let _ = completed.send(Ok(()));
                        break;
                    }
                    CommandRequest::ApplyPolicy {
                        active_policy,
                        completed,
                    } => {
                        let _ = completed.send(Ok(capsem_core::net::policy_config::active_policy_digest(
                            &active_policy,
                        )));
                    }
                }
            }
            let _ = stopped_tx.send(Some("test proxy worker stopped".to_string()));
        });
        Self {
            generation,
            commands,
            stopped,
        }
    }
}

/// Start the proxy from a blocking lifecycle worker and retain its log beside
/// the session. Missing workers fail the session; tests use a scoped stub only
/// when their synthetic service state deliberately has no installed binary.
pub(crate) fn spawn_for_session(binary: &Path, active_policy: Vec<u8>, log_path: &Path) -> Result<ProxyWorker> {
    #[cfg(test)]
    if !binary.exists() {
        return Ok(ProxyWorker::test_stub());
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("open {}", log_path.display()))?;
    tokio::runtime::Handle::current().block_on(ProxyWorker::spawn(
        binary,
        active_policy,
        Stdio::from(log.try_clone()?),
        Stdio::from(log),
    ))
}

enum CommandRequest {
    Grant {
        capability: GrantedCapability,
        descriptors: Vec<OwnedFd>,
        completed: oneshot::Sender<Result<()>>,
    },
    ApplyPolicy {
        active_policy: Vec<u8>,
        completed: oneshot::Sender<Result<String>>,
    },
    Shutdown {
        completed: oneshot::Sender<Result<()>>,
    },
}

enum ProxyWorkerMode {
    Vm,
    Standalone(String),
}

fn configure_worker_command(command: &mut Command, generation: ProxyGeneration, mode: &ProxyWorkerMode) {
    command
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--generation")
        .arg(generation_hex(generation));
    if let ProxyWorkerMode::Standalone(provider_id) = mode {
        command.arg("--standalone-provider").arg(provider_id);
    }
}

#[derive(Clone, Copy)]
enum GrantedCapability {
    Ordinary(ProxyCapability),
    Ledger(LedgerChannelGrant),
}

impl GrantedCapability {
    const fn kind(self) -> ProxyCapability {
        match self {
            Self::Ordinary(capability) => capability,
            Self::Ledger(_) => ProxyCapability::Ledger,
        }
    }

    fn channel_grant(
        self,
        generation: ProxyGeneration,
        grant_id: u64,
    ) -> Result<capsem_proto::proxy_control::ProxyChannelGrant> {
        match self {
            Self::Ordinary(capability) => Ok(capsem_proto::proxy_control::ProxyChannelGrant::new(
                generation, grant_id, capability,
            )?),
            Self::Ledger(ledger) => Ok(capsem_proto::proxy_control::ProxyChannelGrant::with_ledger(
                generation, grant_id, ledger,
            )?),
        }
    }
}

struct WorkerProcess {
    generation: ProxyGeneration,
    child: Child,
    control_tx: ControlSender,
    control_events: mpsc::Receiver<io::Result<ControlFrame>>,
    control_reader: tokio::task::JoinHandle<()>,
    policy_tx: ipc_channel::Sender<ProxyPolicyRequest>,
    policy_rx: ipc_channel::Receiver<ProxyPolicyResponse>,
    next_grant_id: u64,
    next_request_id: u64,
    active_grants: HashMap<u64, ProxyCapability>,
}

enum PolicyApply {
    Applied(String),
    Rejected(String),
}

async fn launch(
    mut command: Command,
    generation: ProxyGeneration,
    active_policy: Vec<u8>,
    configure: impl FnOnce(&mut Command, ProxyGeneration),
) -> Result<ProxyWorker> {
    let (coordinator, child_control) = UnixStream::pair().context("create proxy control channel")?;
    configure(&mut command, generation);
    command.stdin(Stdio::from(OwnedFd::from(child_control)));
    let child = command.spawn().context("spawn capsem-proxy")?;
    let control_tx = ControlSender::new(coordinator.try_clone()?).context("open proxy control sender")?;
    let control_rx = ControlReceiver::new(coordinator).context("open proxy control receiver")?;
    let process = WorkerProcess::start(child, control_tx, control_rx, generation, active_policy).await?;
    let (commands, requests) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
    let (stopped_tx, stopped) = watch::channel(None);
    tokio::spawn(supervise(process, requests, stopped_tx));
    Ok(ProxyWorker {
        generation,
        commands,
        stopped,
    })
}

impl WorkerProcess {
    async fn start(
        child: Child,
        control_tx: ControlSender,
        control_rx: ControlReceiver,
        generation: ProxyGeneration,
        active_policy: Vec<u8>,
    ) -> Result<Self> {
        let mut pending = PendingWorker {
            generation,
            child,
            control_tx,
            control_rx,
            next_grant_id: 1,
        };
        let startup = async {
            match pending.recv_event().await? {
                ProxyControlEvent::Ready { generation: ready } if ready == generation => {}
                event => bail!("proxy worker sent {event:?} before readiness"),
            }
            let (policy_peer, policy_worker) = UnixStream::pair().context("create proxy policy channel")?;
            pending.grant(ProxyCapability::Policy, policy_worker).await?;
            Ok(ipc_channel::channel_from_std(policy_peer)?)
        };
        let (policy_tx, policy_rx) = match tokio::time::timeout(STARTUP_TIMEOUT, startup).await {
            Ok(Ok(channel)) => channel,
            Ok(Err(error)) => {
                pending.kill_reap().await;
                return Err(error);
            }
            Err(_) => {
                pending.kill_reap().await;
                bail!("proxy worker readiness timed out");
            }
        };
        let PendingWorker {
            child,
            control_tx,
            control_rx,
            next_grant_id,
            ..
        } = pending;
        let (control_events_tx, control_events) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let control_reader = tokio::spawn(read_control(control_rx, control_events_tx));
        let mut process = Self {
            generation,
            child,
            control_tx,
            control_events,
            control_reader,
            policy_tx,
            policy_rx,
            next_grant_id,
            next_request_id: 1,
            active_grants: HashMap::new(),
        };
        let expected = capsem_core::net::policy_config::active_policy_digest(&active_policy);
        let applied = match process.apply_policy(active_policy).await {
            Ok(PolicyApply::Applied(digest)) => digest,
            Ok(PolicyApply::Rejected(error)) => {
                process.kill_reap().await;
                bail!("proxy worker rejected initial active policy: {error}");
            }
            Err(error) => {
                process.kill_reap().await;
                return Err(error);
            }
        };
        if applied != expected {
            process.kill_reap().await;
            bail!("proxy worker applied active policy {applied}, expected {expected}");
        }
        Ok(process)
    }

    async fn grant(&mut self, capability: GrantedCapability, descriptors: Vec<OwnedFd>) -> Result<()> {
        let grant_id = self.allocate_grant_id()?;
        let grant = capability.channel_grant(self.generation, grant_id)?;
        if descriptors.len() != grant.expected_descriptor_count() {
            bail!("proxy capability descriptor count does not match its grant");
        }
        let raw = descriptors
            .iter()
            .map(|descriptor| descriptor.as_raw_fd())
            .collect::<Vec<_>>();
        self.control_tx
            .send(&encode_proxy_control_request(ProxyControlRequest::Attach(grant)), &raw)
            .await
            .context("send proxy descriptor grant")?;
        drop(descriptors);
        match self.recv_event().await? {
            ProxyControlEvent::Adopted {
                generation,
                grant_id: adopted,
            } if generation == self.generation && adopted == grant_id => {
                self.active_grants.insert(grant_id, capability.kind());
                Ok(())
            }
            ProxyControlEvent::Rejected {
                generation,
                grant_id: rejected,
                reason,
            } if generation == self.generation && rejected == grant_id => {
                bail!(
                    "proxy worker rejected {:?} grant {grant_id}: {reason:?}",
                    capability.kind()
                )
            }
            event => bail!("proxy worker sent {event:?} while adopting grant {grant_id}"),
        }
    }

    async fn apply_policy(&mut self, active_policy: Vec<u8>) -> Result<PolicyApply> {
        let request_id = self.allocate_request_id()?;
        self.policy_tx
            .send(ProxyPolicyRequest::apply(request_id, active_policy))
            .await
            .context("send proxy active policy")?;
        let response = self.recv_policy().await?;
        match response {
            ProxyPolicyResponse::Applied {
                request_id: applied,
                active_policy_digest,
            } if applied == request_id => Ok(PolicyApply::Applied(active_policy_digest)),
            ProxyPolicyResponse::Rejected {
                request_id: rejected,
                error,
            } if rejected == request_id => Ok(PolicyApply::Rejected(error)),
            response => bail!("proxy worker sent mismatched policy response {response:?}"),
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.control_tx
            .send(
                &encode_proxy_control_request(ProxyControlRequest::Shutdown {
                    generation: self.generation,
                }),
                &[],
            )
            .await
            .context("send proxy shutdown")?;
        let stopped = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
            match self.recv_event().await? {
                ProxyControlEvent::Stopped { generation } if generation == self.generation => {}
                event => bail!("proxy worker sent {event:?} during shutdown"),
            }
            let status = self.child.wait().await.context("reap proxy worker")?;
            if !status.success() {
                bail!("proxy worker exited unsuccessfully during shutdown: {status}");
            }
            Ok(())
        })
        .await;
        match stopped {
            Ok(result) => result,
            Err(_) => {
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
                bail!("proxy worker bounded shutdown timed out")
            }
        }
    }

    async fn recv_event(&mut self) -> Result<ProxyControlEvent> {
        tokio::time::timeout(OPERATION_TIMEOUT, async {
            loop {
                let event = tokio::select! {
                    status = self.child.wait() => {
                        let status = status.context("reap proxy worker")?;
                        bail!("proxy worker exited: {status}")
                    }
                    frame = self.control_events.recv() => {
                        let frame = frame.context("proxy control event pump stopped")??;
                        if !frame.fds.is_empty() {
                            bail!("proxy worker returned a descriptor on its control channel");
                        }
                        decode_proxy_control_event(&frame.bytes).map_err(anyhow::Error::from)?
                    }
                };
                if !self.absorb_traffic_close(event)? {
                    break Ok(event);
                }
            }
        })
        .await
        .map_err(|_| anyhow!("proxy worker control operation timed out"))?
    }

    fn absorb_traffic_close(&mut self, event: ProxyControlEvent) -> Result<bool> {
        let ProxyControlEvent::Closed {
            generation,
            grant_id,
            reason,
        } = event
        else {
            return Ok(false);
        };
        if generation != self.generation {
            bail!("proxy worker closed a grant for a stale generation");
        }
        let capability = self
            .active_grants
            .remove(&grant_id)
            .with_context(|| format!("proxy worker closed unknown grant {grant_id}"))?;
        if !matches!(capability, ProxyCapability::HttpTraffic | ProxyCapability::DnsTraffic)
            || reason != ProxyChannelCloseReason::Disconnected
        {
            bail!("proxy worker closed required {capability:?} grant {grant_id}: {reason:?}");
        }
        Ok(true)
    }

    async fn recv_policy(&mut self) -> Result<ProxyPolicyResponse> {
        tokio::time::timeout(OPERATION_TIMEOUT, async {
            tokio::select! {
                status = self.child.wait() => {
                    let status = status.context("reap proxy worker")?;
                    bail!("proxy worker exited with policy pending: {status}")
                }
                response = self.policy_rx.recv() => response.context("proxy worker stopped with policy pending"),
            }
        })
        .await
        .map_err(|_| anyhow!("proxy worker policy operation timed out"))?
    }

    fn allocate_grant_id(&mut self) -> Result<u64> {
        let id = self.next_grant_id;
        self.next_grant_id = id.checked_add(1).context("proxy grant id exhausted")?;
        Ok(id)
    }

    fn allocate_request_id(&mut self) -> Result<u64> {
        let id = self.next_request_id;
        self.next_request_id = id.checked_add(1).context("proxy policy request id exhausted")?;
        Ok(id)
    }

    async fn kill_reap(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        self.control_reader.abort();
        let _ = (&mut self.control_reader).await;
    }
}

struct PendingWorker {
    generation: ProxyGeneration,
    child: Child,
    control_tx: ControlSender,
    control_rx: ControlReceiver,
    next_grant_id: u64,
}

impl PendingWorker {
    async fn recv_event(&mut self) -> Result<ProxyControlEvent> {
        tokio::select! {
            status = self.child.wait() => {
                let status = status.context("reap proxy worker during startup")?;
                bail!("proxy worker exited before readiness: {status}")
            }
            frame = self.control_rx.recv() => {
                let frame = frame.context("receive proxy startup event")?;
                if !frame.fds.is_empty() {
                    bail!("proxy worker returned a descriptor during startup");
                }
                decode_proxy_control_event(&frame.bytes).map_err(anyhow::Error::from)
            }
        }
    }

    async fn grant(&mut self, capability: ProxyCapability, stream: UnixStream) -> Result<()> {
        let grant_id = self.next_grant_id;
        self.next_grant_id = grant_id.checked_add(1).context("proxy startup grant id exhausted")?;
        let grant = capsem_proto::proxy_control::ProxyChannelGrant::new(self.generation, grant_id, capability)?;
        self.control_tx
            .send(
                &encode_proxy_control_request(ProxyControlRequest::Attach(grant)),
                &[stream.as_raw_fd()],
            )
            .await
            .context("send proxy startup grant")?;
        drop(stream);
        match self.recv_event().await? {
            ProxyControlEvent::Adopted {
                generation,
                grant_id: adopted,
            } if generation == self.generation && adopted == grant_id => Ok(()),
            event => bail!("proxy worker sent {event:?} while adopting startup grant {grant_id}"),
        }
    }

    async fn kill_reap(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

async fn supervise(
    mut process: WorkerProcess,
    mut requests: mpsc::Receiver<CommandRequest>,
    stopped: watch::Sender<Option<String>>,
) {
    let reason = loop {
        tokio::select! {
            status = process.child.wait() => {
                break match status {
                    Ok(status) => format!("proxy worker exited unexpectedly: {status}"),
                    Err(error) => format!("proxy worker exit wait failed: {error}"),
                };
            }
            event = process.control_events.recv() => {
                let event = match event {
                    Some(Ok(frame)) if frame.fds.is_empty() => match decode_proxy_control_event(&frame.bytes) {
                        Ok(event) => event,
                        Err(error) => break format!("proxy worker sent invalid control event: {error}"),
                    },
                    Some(Ok(_)) => break "proxy worker sent an unsolicited descriptor".to_string(),
                    Some(Err(error)) => break format!("proxy worker control channel failed: {error}"),
                    None => break "proxy worker control event pump stopped".to_string(),
                };
                match process.absorb_traffic_close(event) {
                    Ok(true) => continue,
                    Ok(false) => break format!("proxy worker sent unsolicited control event {event:?}"),
                    Err(error) => break format!("proxy worker sent invalid control event: {error:#}"),
                }
            }
            request = requests.recv() => {
                let Some(request) = request else {
                    break match process.shutdown().await {
                        Ok(()) => "proxy worker stopped after its owner released it".to_string(),
                        Err(error) => format!("proxy worker release failed: {error:#}"),
                    };
                };
                match request {
                    CommandRequest::Grant {
                        capability,
                        descriptors,
                        completed,
                    } => {
                        let result = process.grant(capability, descriptors).await;
                        let failed = result.is_err();
                        let diagnostic = result.as_ref().err().map(|error| format!("{error:#}"));
                        let _ = completed.send(result);
                        if failed {
                            break diagnostic.expect("failed result has diagnostic");
                        }
                    }
                    CommandRequest::ApplyPolicy { active_policy, completed } => {
                        match process.apply_policy(active_policy).await {
                            Ok(PolicyApply::Applied(digest)) => {
                                let _ = completed.send(Ok(digest));
                            }
                            Ok(PolicyApply::Rejected(error)) => {
                                let _ = completed.send(Err(anyhow!("proxy worker rejected active policy: {error}")));
                            }
                            Err(error) => {
                                let diagnostic = format!("{error:#}");
                                let _ = completed.send(Err(error));
                                break diagnostic;
                            }
                        }
                    }
                    CommandRequest::Shutdown { completed } => {
                        let result = process.shutdown().await;
                        let reason = result
                            .as_ref()
                            .err()
                            .map_or_else(|| "proxy worker stopped by coordinator".to_string(), |error| format!("{error:#}"));
                        let _ = completed.send(result);
                        break reason;
                    }
                }
            }
        }
    };
    process.kill_reap().await;
    fail_pending(&mut requests, &reason);
    let _ = stopped.send(Some(reason));
}

async fn read_control(receiver: ControlReceiver, events: mpsc::Sender<io::Result<ControlFrame>>) {
    loop {
        let event = receiver.recv().await;
        let complete = event.is_err();
        if events.send(event).await.is_err() || complete {
            return;
        }
    }
}

fn fail_pending(requests: &mut mpsc::Receiver<CommandRequest>, reason: &str) {
    while let Ok(request) = requests.try_recv() {
        let error = || Err(anyhow!(reason.to_string()));
        match request {
            CommandRequest::Grant { completed, .. } | CommandRequest::Shutdown { completed } => {
                let _ = completed.send(error());
            }
            CommandRequest::ApplyPolicy { completed, .. } => {
                let _ = completed.send(Err(anyhow!(reason.to_string())));
            }
        }
    }
}

fn fresh_generation() -> ProxyGeneration {
    ProxyGeneration::new(*uuid::Uuid::new_v4().as_bytes())
}

fn generation_hex(generation: ProxyGeneration) -> String {
    let mut encoded = String::with_capacity(32);
    for byte in generation.as_bytes() {
        write!(&mut encoded, "{byte:02x}").expect("write proxy generation to string");
    }
    encoded
}

impl ServiceState {
    pub(crate) async fn grant_proxy_private_names(
        self: &std::sync::Arc<Self>,
        vm_id: &str,
        worker: &ProxyWorker,
    ) -> Result<()> {
        let (service, proxy) = UnixStream::pair().context("create proxy private-name capability")?;
        let serving = tokio::spawn(crate::proxy_private_names::serve(
            std::sync::Arc::clone(self),
            vm_id.to_string(),
            service,
        ));
        if let Err(error) = worker.grant(ProxyCapability::PrivateNames, proxy.into()).await {
            serving.abort();
            let _ = serving.await;
            return Err(error.context("grant proxy private-name capability"));
        }
        tokio::spawn(async move {
            match serving.await {
                Ok(Ok(())) => tracing::debug!("proxy private-name capability disconnected"),
                Ok(Err(error)) => tracing::warn!(%error, "proxy private-name capability failed"),
                Err(error) => tracing::warn!(%error, "proxy private-name capability task failed"),
            }
        });
        Ok(())
    }

    pub(crate) async fn grant_proxy_credentials(&self, worker: &ProxyWorker) -> Result<()> {
        let (service, proxy) = UnixStream::pair().context("create proxy credential capability")?;
        let serving = tokio::spawn(crate::proxy_credentials::serve(service));
        if let Err(error) = worker.grant(ProxyCapability::Credential, proxy.into()).await {
            serving.abort();
            let _ = serving.await;
            return Err(error.context("grant proxy credential capability"));
        }
        tokio::spawn(async move {
            match serving.await {
                Ok(Ok(())) => tracing::debug!("proxy credential capability disconnected"),
                Ok(Err(error)) => tracing::warn!(%error, "proxy credential capability failed"),
                Err(error) => tracing::warn!(%error, "proxy credential capability task failed"),
            }
        });
        Ok(())
    }

    pub(crate) async fn grant_proxy_ledger(
        &self,
        session_id: &str,
        session_dir: &Path,
        worker: &ProxyWorker,
    ) -> Result<()> {
        let client = self
            .ledger_workers
            .acquire(
                session_id,
                &session_dir.join("session.db"),
                &session_dir.join("ledger.log"),
                capsem_proto::ledger::LedgerClientRole::Proxy,
            )
            .await
            .context("acquire proxy ledger channel")?;
        let (stream, commitment, grant) = client.into_parts();
        let commitment = commitment.context("proxy ledger grant omitted its commitment channel")?;
        worker
            .grant_ledger(stream, commitment, grant)
            .await
            .context("grant proxy ledger channel")
    }

    pub(crate) fn register_proxy_worker(
        self: &std::sync::Arc<Self>,
        session_id: &str,
        session_generation: uuid::Uuid,
        worker: ProxyWorker,
    ) -> Result<()> {
        let proxy_generation = generation_hex(worker.generation());
        let mut stopped = worker.stop_receiver();
        {
            let mut workers = self.proxy_workers.lock().unwrap();
            if workers.contains_key(session_id) {
                bail!("proxy worker slot for {session_id} was already occupied");
            }
            workers.insert(session_id.to_string(), (session_generation, worker));
        }
        let state = std::sync::Arc::clone(self);
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            while stopped.borrow().is_none() && stopped.changed().await.is_ok() {}
            let reason = stopped
                .borrow()
                .clone()
                .unwrap_or_else(|| "proxy worker supervisor disappeared".to_string());
            let owner_pid = state
                .instances
                .lock()
                .unwrap()
                .get(&session_id)
                .and_then(|instance| (instance.generation == session_generation).then_some(instance.pid));
            if let Some(pid) = owner_pid {
                tracing::error!(
                    session_id,
                    %session_generation,
                    proxy_generation,
                    pid,
                    reason,
                    "proxy worker stopped while its VM owner remained active"
                );
                if pid > 0 {
                    crate::process_control::send_or_log(
                        pid,
                        crate::process_control::Signal::Terminate,
                        "proxy-worker-death",
                    );
                }
            }
        });
        Ok(())
    }

    pub(crate) fn remove_proxy_worker(&self, session_id: &str, session_generation: uuid::Uuid) {
        let mut workers = self.proxy_workers.lock().unwrap();
        if workers
            .get(session_id)
            .is_some_and(|(generation, _)| *generation == session_generation)
        {
            workers.remove(session_id);
        }
    }

    pub(crate) fn proxy_worker(&self, session_id: &str, session_generation: uuid::Uuid) -> Result<ProxyWorker, String> {
        if let Some((generation, worker)) = self.proxy_workers.lock().unwrap().get(session_id) {
            if *generation == session_generation {
                return Ok(worker.clone());
            }
            return Err("proxy worker belongs to a replaced VM generation".to_string());
        }
        #[cfg(test)]
        {
            Ok(ProxyWorker::test_stub())
        }
        #[cfg(not(test))]
        {
            Err("running VM has no registered proxy worker".to_string())
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;

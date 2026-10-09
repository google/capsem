//! Trusted lifecycle and channel-grant coordinator for one session ledger.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "S09-004 wires producer and reader clients to the supervised ledger"
    )
)]

use std::fmt::Write as _;
use std::io;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};
use capsem_proto::ledger_control::{
    decode_ledger_control_event, encode_ledger_control_request, LedgerControlEvent, LedgerControlRequest,
    LEDGER_CONTROL_FRAME_SIZE, LEDGER_CONTROL_MAX_FDS,
};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch};

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
const COMMAND_QUEUE_CAPACITY: usize = 64;

type ControlSender = DescriptorSender<LEDGER_CONTROL_FRAME_SIZE, LEDGER_CONTROL_MAX_FDS>;
type ControlReceiver = DescriptorReceiver<LEDGER_CONTROL_FRAME_SIZE, LEDGER_CONTROL_MAX_FDS>;
type ControlFrame = capsem_foundation::unix::router_channel::DescriptorFrame<LEDGER_CONTROL_FRAME_SIZE>;

struct LedgerSlot {
    database: PathBuf,
    worker: LedgerWorker,
    commitments: Arc<crate::ledger_commitment::CommitmentAuthority>,
}

/// Service-owned table that serializes each session's ledger lifetime.
pub(crate) struct LedgerWorkers {
    binary: PathBuf,
    commitment_root: PathBuf,
    slots: tokio::sync::Mutex<std::collections::HashMap<String, LedgerSlot>>,
}

impl LedgerWorkers {
    pub(crate) fn new(binary: PathBuf, commitment_root: PathBuf) -> Self {
        Self {
            binary,
            commitment_root,
            slots: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Acquire a role-bound channel from the one worker for `session_id`.
    /// The slot lock deliberately spans readiness: no replacement generation
    /// can start before a stopped generation has been fully reaped.
    pub(crate) async fn acquire(
        self: &Arc<Self>,
        session_id: &str,
        database: &Path,
        log_path: &Path,
        role: LedgerClientRole,
    ) -> Result<LedgerClient> {
        let (worker, commitments) = {
            let mut slots = self.slots.lock().await;
            let mut watcher = None;
            if slots
                .get(session_id)
                .is_some_and(|slot| slot.worker.stop_receiver().borrow().is_some() && slot.database == database)
            {
                slots.remove(session_id);
            }
            let selected = if let Some(slot) = slots.get(session_id) {
                if slot.database != database {
                    bail!(
                        "ledger worker slot for {session_id} owns {}, not {}",
                        slot.database.display(),
                        database.display()
                    );
                }
                (slot.worker.clone(), Arc::clone(&slot.commitments))
            } else {
                let worker = self.spawn(database, log_path).await?;
                let commitments =
                    crate::ledger_commitment::CommitmentAuthority::open(&self.commitment_root, session_id).await?;
                #[cfg(not(test))]
                if let Err(error) = verify_commitments(&worker, database, &commitments).await {
                    let _ = worker.clone().shutdown().await;
                    return Err(error);
                }
                slots.insert(
                    session_id.to_string(),
                    LedgerSlot {
                        database: database.to_path_buf(),
                        worker: worker.clone(),
                        commitments: Arc::clone(&commitments),
                    },
                );
                watcher = Some((worker.generation(), worker.stop_receiver()));
                (worker, commitments)
            };
            drop(slots);
            if let Some((generation, mut stopped)) = watcher {
                let registry = Arc::clone(self);
                let watched_session_id = session_id.to_string();
                tokio::spawn(async move {
                    if stopped.borrow().is_none() {
                        let _ = stopped.changed().await;
                    }
                    let mut slots = registry.slots.lock().await;
                    if slots
                        .get(&watched_session_id)
                        .is_some_and(|slot| slot.worker.generation() == generation)
                    {
                        slots.remove(&watched_session_id);
                    }
                });
            }
            selected
        };
        let mut client = worker.connect(role).await?;
        if role.producer_name().is_some() {
            let (producer, coordinator) = UnixStream::pair().context("create ledger commitment channel")?;
            let grant = client.grant;
            tokio::spawn(async move {
                if let Err(error) = crate::ledger_commitment::serve(commitments, coordinator, grant).await {
                    tracing::warn!(client_id = grant.client_id(), %error, "ledger commitment channel stopped");
                }
            });
            client.commitment = Some(producer);
        }
        Ok(client)
    }

    /// Stop and reap the current generation before vacating its slot.
    pub(crate) async fn shutdown(&self, session_id: &str) -> Result<()> {
        let Some(worker) = self.slots.lock().await.get(session_id).map(|slot| slot.worker.clone()) else {
            return Ok(());
        };
        let generation = worker.generation();
        let result = worker.shutdown().await;
        let mut slots = self.slots.lock().await;
        if slots
            .get(session_id)
            .is_some_and(|slot| slot.worker.generation() == generation)
        {
            slots.remove(session_id);
        }
        result
    }

    async fn spawn(&self, database: &Path, log_path: &Path) -> Result<LedgerWorker> {
        let log = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .await
            .with_context(|| format!("open {}", log_path.display()))?
            .into_std()
            .await;
        LedgerWorker::spawn(&self.binary, database, Stdio::from(log.try_clone()?), Stdio::from(log)).await
    }

    #[cfg(test)]
    async fn generation(&self, session_id: &str) -> Option<LedgerGeneration> {
        self.slots
            .lock()
            .await
            .get(session_id)
            .map(|slot| slot.worker.generation())
    }
}

#[cfg(not(test))]
async fn verify_commitments(
    worker: &LedgerWorker,
    database: &Path,
    commitments: &crate::ledger_commitment::CommitmentAuthority,
) -> Result<()> {
    let channel = worker.connect(LedgerClientRole::Reader).await?;
    let (stream, commitment, grant) = channel.into_parts();
    if commitment.is_some() {
        bail!("reader received a producer commitment channel");
    }
    let client = capsem_logger::ledger_client::LedgerClient::connect(stream, grant, database.to_path_buf())
        .await
        .map_err(anyhow::Error::msg)?;
    commitments
        .verify_ledger(&client)
        .await
        .context("verify session ledger against trusted producer checkpoints")
}

#[derive(Debug)]
pub(crate) struct LedgerClient {
    stream: UnixStream,
    commitment: Option<UnixStream>,
    grant: LedgerChannelGrant,
}

impl LedgerClient {
    pub(crate) const fn grant(&self) -> LedgerChannelGrant {
        self.grant
    }

    pub(crate) fn into_parts(self) -> (UnixStream, Option<UnixStream>, LedgerChannelGrant) {
        (self.stream, self.commitment, self.grant)
    }

    #[cfg(test)]
    pub(crate) fn test_pair(grant: LedgerChannelGrant) -> io::Result<(Self, UnixStream, UnixStream)> {
        let (stream, worker) = UnixStream::pair()?;
        let (commitment, commitment_worker) = UnixStream::pair()?;
        Ok((
            Self {
                stream,
                commitment: Some(commitment),
                grant,
            },
            worker,
            commitment_worker,
        ))
    }
}

#[derive(Clone)]
pub(crate) struct LedgerWorker {
    generation: LedgerGeneration,
    commands: mpsc::Sender<CommandRequest>,
    stopped: watch::Receiver<Option<String>>,
}

impl LedgerWorker {
    pub(crate) async fn spawn(binary: &Path, database: &Path, stdout: Stdio, stderr: Stdio) -> Result<Self> {
        let generation = fresh_generation();
        let mut command = Command::new(binary);
        command.env_clear().stdout(stdout).stderr(stderr).kill_on_drop(true);
        launch(command, database, generation, |command, generation, database| {
            command
                .arg("--parent-pid")
                .arg(std::process::id().to_string())
                .arg("--database")
                .arg(database)
                .arg("--generation")
                .arg(generation_hex(generation));
        })
        .await
    }

    pub(crate) const fn generation(&self) -> LedgerGeneration {
        self.generation
    }

    pub(crate) async fn connect(&self, role: LedgerClientRole) -> Result<LedgerClient> {
        let (completed, result) = oneshot::channel();
        self.commands
            .send(CommandRequest::Connect { role, completed })
            .await
            .map_err(|_| anyhow!(self.stop_reason("ledger worker supervisor stopped before accepting a client")))?;
        result
            .await
            .map_err(|_| anyhow!(self.stop_reason("ledger worker supervisor stopped with a client pending")))?
    }

    pub(crate) async fn shutdown(self) -> Result<()> {
        let (completed, result) = oneshot::channel();
        self.commands
            .send(CommandRequest::Shutdown { completed })
            .await
            .map_err(|_| anyhow!(self.stop_reason("ledger worker supervisor already stopped")))?;
        result
            .await
            .map_err(|_| anyhow!(self.stop_reason("ledger worker supervisor stopped during shutdown")))?
    }

    pub(crate) fn stop_receiver(&self) -> watch::Receiver<Option<String>> {
        self.stopped.clone()
    }

    fn stop_reason(&self, fallback: &str) -> String {
        self.stopped.borrow().clone().unwrap_or_else(|| fallback.to_string())
    }
}

enum CommandRequest {
    Connect {
        role: LedgerClientRole,
        completed: oneshot::Sender<Result<LedgerClient>>,
    },
    Shutdown {
        completed: oneshot::Sender<Result<()>>,
    },
}

struct WorkerProcess {
    generation: LedgerGeneration,
    child: Child,
    control_tx: ControlSender,
    control_events: mpsc::Receiver<io::Result<ControlFrame>>,
    control_reader: Option<tokio::task::JoinHandle<()>>,
    next_client_id: u64,
}

async fn launch(
    mut command: Command,
    database: &Path,
    generation: LedgerGeneration,
    configure: impl FnOnce(&mut Command, LedgerGeneration, &Path),
) -> Result<LedgerWorker> {
    if !database.is_absolute() || database.file_name().and_then(|name| name.to_str()) != Some("session.db") {
        bail!("ledger database must be an absolute session.db path");
    }
    let (coordinator, child_control) = UnixStream::pair().context("create ledger control channel")?;
    configure(&mut command, generation, database);
    command.stdin(Stdio::from(OwnedFd::from(child_control)));
    let child = command.spawn().context("spawn capsem-ledger")?;
    let control_tx = ControlSender::new(coordinator.try_clone()?).context("open ledger control sender")?;
    let control_rx = ControlReceiver::new(coordinator).context("open ledger control receiver")?;
    let process = WorkerProcess::start(child, control_tx, control_rx, generation).await?;
    let (commands, requests) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
    let (stopped_tx, stopped) = watch::channel(None);
    tokio::spawn(supervise(process, requests, stopped_tx));
    Ok(LedgerWorker {
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
        generation: LedgerGeneration,
    ) -> Result<Self> {
        let mut pending = PendingWorker {
            child,
            control_tx,
            control_rx,
        };
        let ready = tokio::time::timeout(STARTUP_TIMEOUT, pending.recv_event()).await;
        match ready {
            Ok(Ok(LedgerControlEvent::Ready { generation: announced })) if announced == generation => {}
            Ok(Ok(LedgerControlEvent::Ready { .. })) => {
                pending.kill_reap().await;
                bail!("ledger worker announced a stale generation before readiness");
            }
            Ok(Ok(event)) => {
                pending.kill_reap().await;
                bail!("ledger worker sent {event:?} before readiness");
            }
            Ok(Err(error)) => {
                pending.kill_reap().await;
                return Err(error);
            }
            Err(_) => {
                pending.kill_reap().await;
                bail!("ledger worker readiness timed out");
            }
        }
        let PendingWorker {
            child,
            control_tx,
            control_rx,
        } = pending;
        let (events_tx, control_events) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let control_reader = tokio::spawn(read_control(control_rx, events_tx));
        Ok(Self {
            generation,
            child,
            control_tx,
            control_events,
            control_reader: Some(control_reader),
            next_client_id: 1,
        })
    }

    async fn connect(&mut self, role: LedgerClientRole) -> Result<LedgerClient> {
        let client_id = self.next_client_id;
        self.next_client_id = client_id.checked_add(1).context("ledger client id exhausted")?;
        let grant = LedgerChannelGrant::new(self.generation, client_id, role)?;
        let (client, worker) = UnixStream::pair().context("create ledger client channel")?;
        self.control_tx
            .send(
                &encode_ledger_control_request(LedgerControlRequest::Attach(grant)),
                &[worker.as_raw_fd()],
            )
            .await
            .context("send ledger descriptor grant")?;
        drop(worker);
        loop {
            match self.recv_event().await? {
                LedgerControlEvent::Adopted {
                    generation,
                    client_id: adopted,
                } if generation == self.generation && adopted == client_id => {
                    return Ok(LedgerClient {
                        stream: client,
                        commitment: None,
                        grant,
                    });
                }
                LedgerControlEvent::Rejected {
                    generation,
                    client_id: rejected,
                    reason,
                } if generation == self.generation && rejected == client_id => {
                    bail!("ledger worker rejected {role:?} client {client_id}: {reason:?}");
                }
                LedgerControlEvent::Closed { generation, .. } if generation == self.generation => {}
                event => bail!("ledger worker sent {event:?} while adopting client {client_id}"),
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.control_tx
            .send(
                &encode_ledger_control_request(LedgerControlRequest::Shutdown {
                    generation: self.generation,
                }),
                &[],
            )
            .await
            .context("send ledger shutdown")?;
        let stopped = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
            loop {
                match self.recv_event().await? {
                    LedgerControlEvent::Stopped { generation } if generation == self.generation => break,
                    LedgerControlEvent::Closed { generation, .. } if generation == self.generation => {}
                    event => bail!("ledger worker sent {event:?} during shutdown"),
                }
            }
            let status = self.child.wait().await.context("reap ledger worker")?;
            if !status.success() {
                bail!("ledger worker exited unsuccessfully during shutdown: {status}");
            }
            Ok(())
        })
        .await;
        match stopped {
            Ok(result) => result,
            Err(_) => {
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
                bail!("ledger worker bounded shutdown timed out")
            }
        }
    }

    async fn recv_event(&mut self) -> Result<LedgerControlEvent> {
        tokio::time::timeout(OPERATION_TIMEOUT, async {
            tokio::select! {
                status = self.child.wait() => {
                    let status = status.context("reap ledger worker")?;
                    bail!("ledger worker exited: {status}")
                }
                frame = self.control_events.recv() => {
                    let frame = frame
                        .context("ledger control event pump stopped")?
                        .context("ledger worker control channel failed")?;
                    if !frame.fds.is_empty() {
                        bail!("ledger worker returned a descriptor on its control channel");
                    }
                    decode_ledger_control_event(&frame.bytes).map_err(anyhow::Error::from)
                }
            }
        })
        .await
        .map_err(|_| anyhow!("ledger worker control operation timed out"))?
    }

    async fn kill_reap(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        if let Some(mut control_reader) = self.control_reader.take() {
            control_reader.abort();
            let _ = (&mut control_reader).await;
        }
    }
}

struct PendingWorker {
    child: Child,
    control_tx: ControlSender,
    control_rx: ControlReceiver,
}

impl PendingWorker {
    async fn recv_event(&mut self) -> Result<LedgerControlEvent> {
        tokio::select! {
            status = self.child.wait() => {
                let status = status.context("reap ledger worker during startup")?;
                bail!("ledger worker exited before readiness: {status}")
            }
            frame = self.control_rx.recv() => {
                let frame = frame.context("receive ledger startup event")?;
                if !frame.fds.is_empty() {
                    bail!("ledger worker returned a descriptor during startup");
                }
                decode_ledger_control_event(&frame.bytes).map_err(anyhow::Error::from)
            }
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
                    Ok(status) => format!("ledger worker exited unexpectedly: {status}"),
                    Err(error) => format!("ledger worker exit wait failed: {error}"),
                };
            }
            event = process.control_events.recv() => {
                match event {
                    Some(Ok(frame)) if frame.fds.is_empty() => match decode_ledger_control_event(&frame.bytes) {
                        Ok(LedgerControlEvent::Closed { generation, client_id, reason })
                            if generation == process.generation => {
                                tracing::debug!(client_id, ?reason, "ledger client channel closed");
                            }
                        Ok(event) => break format!("ledger worker sent unsolicited control event {event:?}"),
                        Err(error) => break format!("ledger worker sent invalid control event: {error}"),
                    },
                    Some(Ok(_)) => break "ledger worker sent an unsolicited descriptor".to_string(),
                    Some(Err(error)) => break format!("ledger worker control channel failed: {error}"),
                    None => break "ledger worker control event pump stopped".to_string(),
                }
            }
            request = requests.recv() => {
                let Some(request) = request else {
                    break match process.shutdown().await {
                        Ok(()) => "ledger worker stopped after its owner released it".to_string(),
                        Err(error) => format!("ledger worker release failed: {error:#}"),
                    };
                };
                match request {
                    CommandRequest::Connect { role, completed } => {
                        let result = process.connect(role).await;
                        let failed = result.is_err();
                        let diagnostic = result.as_ref().err().map(|error| format!("{error:#}"));
                        let _ = completed.send(result);
                        if failed {
                            break diagnostic.expect("failed result has diagnostic");
                        }
                    }
                    CommandRequest::Shutdown { completed } => {
                        let result = process.shutdown().await;
                        // Completion is the replacement barrier: even a send
                        // or protocol failure is reaped before the coordinator
                        // is allowed to vacate this generation's slot.
                        process.kill_reap().await;
                        let reason = result
                            .as_ref()
                            .err()
                            .map_or_else(|| "ledger worker stopped by coordinator".to_string(), |error| format!("{error:#}"));
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
        match request {
            CommandRequest::Connect { completed, .. } => {
                let _ = completed.send(Err(anyhow!(reason.to_string())));
            }
            CommandRequest::Shutdown { completed } => {
                let _ = completed.send(Err(anyhow!(reason.to_string())));
            }
        }
    }
}

fn fresh_generation() -> LedgerGeneration {
    LedgerGeneration::new(*uuid::Uuid::new_v4().as_bytes())
}

fn generation_hex(generation: LedgerGeneration) -> String {
    let mut encoded = String::with_capacity(32);
    for byte in generation.as_bytes() {
        write!(&mut encoded, "{byte:02x}").expect("write ledger generation to string");
    }
    encoded
}

#[cfg(test)]
mod tests;

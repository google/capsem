//! The VM owner's slot registry and guest stdio transport.
use anyhow::{ensure, Context, Result};
use capsem_core::{environment::SandboxMcp, VsockConnection};
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::mcp_worker::{
    decode_spawn_record, encode_spawn_record, McpSpawnRecord, MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS,
};
use capsem_proto::{ExecInputFrame, ExecOutputChannel, HostToGuest, MAX_EXEC_DATA_BYTES};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, watch};

use crate::{guest_control, job_store::JobStore};

const ATTACH_TIMEOUT: Duration = Duration::from_secs(3);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn sync_enabled(
    aggregator: capsem_proto::mcp_aggregator::AggregatorClient,
    mut policy: watch::Receiver<Arc<Vec<SandboxMcp>>>,
    mut previous: Vec<bool>,
) {
    loop {
        let enabled: Vec<_> = policy.borrow_and_update().iter().map(|server| server.enabled).collect();
        if enabled != previous {
            if let Err(error) = aggregator.sandbox_enabled(enabled.clone()).await {
                tracing::warn!(%error, "sandbox MCP availability update failed");
            } else {
                previous = enabled;
            }
        }
        if policy.changed().await.is_err() {
            return;
        }
    }
}

#[derive(Default)]
pub(crate) struct Jobs(Mutex<HashMap<u64, Job>>);

struct Job {
    slot: u32,
    attach: Option<oneshot::Sender<VsockConnection>>,
    done: oneshot::Sender<()>,
}

impl Jobs {
    pub(crate) fn known(&self, id: u64) -> bool {
        self.0.lock().unwrap().contains_key(&id)
    }
    pub(crate) fn accepting(&self, id: u64) -> bool {
        self.0.lock().unwrap().get(&id).is_some_and(|job| job.attach.is_some())
    }

    fn cancel_attach(&self, id: u64) {
        if let Some(job) = self.0.lock().unwrap().get_mut(&id) {
            job.attach = None;
        }
    }
    pub(crate) fn attach(&self, id: u64, connection: VsockConnection) {
        let attach = self.0.lock().unwrap().get_mut(&id).and_then(|job| job.attach.take());
        if let Some(attach) = attach {
            let _ = attach.send(connection);
        }
    }

    pub(crate) fn complete(&self, id: u64) -> bool {
        if let Some(job) = self.0.lock().unwrap().remove(&id) {
            let _ = job.done.send(());
            true
        } else {
            false
        }
    }

    pub(crate) fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

#[derive(Clone)]
pub(crate) struct Owner {
    pub(crate) jobs: Arc<JobStore>,
    pub(crate) hub: guest_control::Sender,
    pub(crate) policy: watch::Receiver<Arc<Vec<SandboxMcp>>>,
    pub(crate) ready: Arc<AtomicBool>,
}

impl Owner {
    /// The caller supplies only a slot. A stream becomes usable after the
    /// matching guest exec connects, and is retired when its peer drops.
    async fn open(&self, slot: u32) -> Result<UnixStream> {
        ensure!(self.ready.load(Ordering::Acquire), "guest control is not ready");
        let mut policy = self.policy.clone();
        let admitted = policy
            .borrow_and_update()
            .get(slot as usize)
            .filter(|server| server.enabled)
            .cloned()
            .context("sandbox MCP slot unavailable")?;
        let id = self.jobs.next_control_id.fetch_add(1, Ordering::Relaxed);
        let (attach, attached) = oneshot::channel();
        let (done, finished) = oneshot::channel();
        let (ready, connected) = oneshot::channel();
        {
            let mut jobs = self.jobs.sandbox_mcps.0.lock().unwrap();
            ensure!(
                !jobs.values().any(|job| job.slot == slot),
                "sandbox MCP slot is still active"
            );
            jobs.insert(
                id,
                Job {
                    slot,
                    attach: Some(attach),
                    done,
                },
            );
        }
        let owner = self.clone();
        tokio::spawn(async move {
            let preface = admitted.clone();
            let retirement = async move {
                loop {
                    if policy.borrow_and_update().get(slot as usize) != Some(&admitted) {
                        return;
                    }
                    if policy.changed().await.is_err() {
                        return;
                    }
                }
            };
            owner.run(id, preface, attached, finished, ready, retirement).await;
        });
        connected.await.context("sandbox MCP launch did not attach")?
    }

    async fn run(
        &self,
        id: u64,
        preface: SandboxMcp,
        attached: oneshot::Receiver<VsockConnection>,
        mut finished: oneshot::Receiver<()>,
        mut ready: oneshot::Sender<Result<UnixStream>>,
        retired: impl std::future::Future<Output = ()>,
    ) {
        let start = guest_control::Prepared::sandbox_mcp(id);
        let mut sent = false;
        let dispatch = async {
            self.hub.send_prepared(start?).await?;
            sent = true;
            attached.await.context("guest exec stream unavailable")
        };
        tokio::pin!(retired);
        let mut completed = false;
        let connection = tokio::select! {
            biased;
            _ = &mut retired => Err(anyhow::anyhow!("sandbox MCP slot retired")),
            _ = ready.closed() => Err(anyhow::anyhow!("sandbox MCP requester stopped")),
            _ = &mut finished => { completed = true; Err(anyhow::anyhow!("sandbox MCP launcher exited")) },
            result = tokio::time::timeout(ATTACH_TIMEOUT, dispatch) => result.context("sandbox MCP attach deadline").and_then(|result| result),
        };
        match connection.and_then(|connection| Relay::new(connection, preface)) {
            Ok((relay, peer)) => {
                if ready.send(Ok(peer)).is_ok() {
                    relay.run(retired).await;
                    completed = !matches!(finished.try_recv(), Err(oneshot::error::TryRecvError::Empty));
                }
            }
            Err(error) => {
                let _ = ready.send(Err(error));
            }
        }
        if !sent {
            self.jobs.sandbox_mcps.complete(id);
        } else if !completed {
            self.jobs.sandbox_mcps.cancel_attach(id);
            self.jobs.pending_acks.lock().unwrap().remove(&id);
            let cancellation_id = self.jobs.next_control_id.fetch_add(1, Ordering::Relaxed);
            let cancel = self.hub.send(HostToGuest::CancelExec { id, cancellation_id });
            let _ = tokio::time::timeout(CLEANUP_TIMEOUT, async {
                cancel.await?;
                let _ = finished.await;
                Ok::<_, io::Error>(())
            })
            .await;
            // Retain an unconfirmed slot until GuestToHost::ExecDone or VM exit.
        }
    }

    pub(crate) async fn serve(self, socket: UnixStream) -> Result<()> {
        let sender = DescriptorSender::<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>::new(socket.try_clone()?)?;
        let receiver = DescriptorReceiver::<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>::new(socket)?;
        let mut previous = 0;
        loop {
            let frame = receiver.recv().await?;
            ensure!(frame.fds.is_empty(), "sandbox spawn request carried descriptors");
            let McpSpawnRecord::Sandbox { request_id, slot } = decode_spawn_record(&frame.bytes)? else {
                anyhow::bail!("invalid sandbox spawn request");
            };
            ensure!(request_id > previous, "sandbox spawn replay");
            previous = request_id;
            // The descriptor reader closes its channel if a record receive is
            // canceled. Await the bounded owner launch before reading again.
            let peer = match self.open(slot).await {
                Ok(peer) => Some(peer),
                Err(error) => {
                    tracing::debug!(slot, %error, "sandbox MCP launch refused");
                    None
                }
            };
            if let Some(peer) = peer {
                tokio::time::timeout(ATTACH_TIMEOUT, async {
                    sender
                        .send(
                            &encode_spawn_record(McpSpawnRecord::Granted { request_id })?,
                            &[peer.as_raw_fd()],
                        )
                        .await?;
                    let receipt = receiver.recv().await?;
                    ensure!(
                        receipt.fds.is_empty()
                            && decode_spawn_record(&receipt.bytes)? == (McpSpawnRecord::Adopted { request_id }),
                        "invalid sandbox spawn receipt"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await??;
            } else {
                sender
                    .send(&encode_spawn_record(McpSpawnRecord::Denied { request_id })?, &[])
                    .await?;
            }
        }
    }
}

struct Relay {
    guest: VsockConnection,
    local: UnixStream,
    input: std::fs::File,
    output: std::fs::File,
    preface: SandboxMcp,
}

impl Relay {
    fn new(guest: VsockConnection, preface: SandboxMcp) -> Result<(Self, UnixStream)> {
        let (local, peer) = UnixStream::pair()?;
        let input = guest.try_clone_file()?;
        let output = guest.try_clone_file()?;
        Ok((
            Self {
                guest,
                local,
                input,
                output,
                preface,
            },
            peer,
        ))
    }

    async fn run(self, retired: impl std::future::Future<Output = ()>) {
        let Ok(mut local_input) = self.local.try_clone() else {
            return;
        };
        let Ok(mut local_output) = self.local.try_clone() else {
            return;
        };
        let Self {
            mut input,
            mut output,
            guest,
            local,
            preface,
        } = self;
        let rx = tokio::task::spawn_blocking(move || -> io::Result<()> {
            for bytes in preface.preface().chunks(MAX_EXEC_DATA_BYTES) {
                capsem_proto::write_exec_input(&mut input, &ExecInputFrame::Data(bytes.to_vec()))?;
            }
            let mut buffer = [0u8; 8192];
            loop {
                let size = local_input.read(&mut buffer)?;
                if size == 0 {
                    return Ok(());
                }
                capsem_proto::write_exec_input(&mut input, &ExecInputFrame::Data(buffer[..size].to_vec()))?;
            }
        });
        let tx = tokio::task::spawn_blocking(move || -> io::Result<()> {
            loop {
                let frame = capsem_proto::read_exec_output(&mut output)?;
                if frame.channel == ExecOutputChannel::Stdout {
                    local_output.write_all(&frame.data)?;
                }
            }
        });
        let (mut rx, mut tx) = (rx, tx);
        tokio::select! {
            biased;
            _ = retired => {},
            _ = &mut rx => {},
            _ = &mut tx => {},
        }
        let _ = local.shutdown(Shutdown::Both);
        let _ = guest.shutdown_both();
        if !rx.is_finished() {
            let _ = (&mut rx).await;
        }
        if !tx.is_finished() {
            let _ = (&mut tx).await;
        }
    }
}

#[cfg(test)]
mod tests;

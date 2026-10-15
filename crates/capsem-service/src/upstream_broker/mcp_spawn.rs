//! Service-owned native peers: workers select only a predefined builtin index.

use super::{mcp_http, BrokerPolicy, WorkerGrant};
use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::{
    fd,
    router_channel::{DescriptorReceiver, DescriptorSender},
};
use capsem_mcp_transport::native_worker::{grant_worker, STARTUP_TIMEOUT};
use capsem_proto::mcp_worker::{
    decode_spawn_record, encode_spawn_record, McpHttpRole, McpSpawnRecord, McpWorkerBootstrap, MCP_SPAWN_FRAME_SIZE,
    MCP_SPAWN_MAX_FDS,
};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

type Sender = DescriptorSender<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>;
type Receiver = DescriptorReceiver<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>;

pub(super) struct Factory {
    pub(super) binary: PathBuf,
    stderr: std::fs::File,
    pub(super) trace_env: Vec<(String, String)>,
    capacity: Arc<Semaphore>,
}

impl Factory {
    pub(super) fn prepare(session: &Path, capacity: Arc<Semaphore>) -> Result<Self> {
        let vm_id = session
            .file_name()
            .context("MCP session identity missing")?
            .to_string_lossy();
        Ok(Self {
            binary: crate::service_runtime::find_sibling_binary("capsem-mcp-builtin"),
            stderr: capsem_foundation::unix::fs::open_private_append_no_follow(
                &session.join("mcp-builtin.stderr.log"),
            )?,
            trace_env: capsem_foundation::telemetry::child_trace_env(&vm_id),
            capacity,
        })
    }

    fn launch(
        &self,
        policy: watch::Receiver<Arc<BrokerPolicy>>,
        authority: WorkerGrant,
    ) -> Result<(Peer, UnixStream, UnixStream)> {
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .context("MCP builtin peer capacity reached")?;
        let (control, child_control) = UnixStream::pair()?;
        let (http, child_http) = UnixStream::pair()?;
        let mut command = tokio::process::Command::new(&self.binary);
        command
            .env_clear()
            .envs(self.trace_env.clone())
            .stdin(Stdio::from(OwnedFd::from(child_control)))
            .stdout(Stdio::null())
            .stderr(Stdio::from(self.stderr.try_clone()?))
            .kill_on_drop(true);
        let child = command.spawn().context("spawn native builtin MCP worker")?;
        drop(command);
        let http = tokio::spawn(mcp_http::serve(http, McpHttpRole::Builtin, policy, authority));
        Ok((
            Peer {
                child,
                http,
                _permit: permit,
            },
            control,
            child_http,
        ))
    }
}

struct Peer {
    child: tokio::process::Child,
    http: tokio::task::JoinHandle<()>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.http.abort();
    }
}

async fn reap(mut peer: Peer) -> Result<()> {
    // Retain the unreaped Child handle throughout signalling; no numeric PID
    // supplied by a worker can become authority over another process.
    peer.child.start_kill().context("stop native MCP peer")?;
    tokio::time::timeout(STARTUP_TIMEOUT, peer.child.wait())
        .await
        .context("native MCP peer exit deadline")??;
    peer.http.abort();
    let _ = (&mut peer.http).await;
    Ok(())
}

pub(super) async fn serve(
    socket: UnixStream,
    factory: Factory,
    policy: watch::Receiver<Arc<BrokerPolicy>>,
    authority: WorkerGrant,
) -> Result<(), String> {
    let mut peers: Vec<Option<Peer>> = (0..capsem_proto::MCP_BUILTIN_PEERS_PER_SESSION).map(|_| None).collect();
    let mut failures = Vec::new();
    let result = run(socket, &factory, policy, authority, &mut peers, &mut failures).await;
    while let Some(peer) = peers.pop() {
        if let Some(peer) = peer {
            if let Err(error) = reap(peer).await {
                failures.push(format!("{error:#}"));
            }
        }
    }
    drop(peers);
    if let Err(error) = result {
        tracing::debug!(%error, "MCP spawn capability closed");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

async fn run(
    socket: UnixStream,
    factory: &Factory,
    policy: watch::Receiver<Arc<BrokerPolicy>>,
    authority: WorkerGrant,
    peers: &mut [Option<Peer>],
    failures: &mut Vec<String>,
) -> Result<()> {
    let sender = Sender::new(UnixStream::from(fd::duplicate(socket.as_fd())?))?;
    let receiver = Receiver::new(socket)?;
    let mut lifetime = policy.clone();
    let closed = async move { while lifetime.changed().await.is_ok() {} };
    tokio::pin!(closed);
    let mut previous_request = 0;
    loop {
        let frame = tokio::select! {
            biased;
            () = authority.revoked() => return Ok(()),
            () = &mut closed => return Ok(()),
            frame = receiver.recv() => frame?,
        };
        ensure!(frame.fds.is_empty(), "MCP spawn request carried descriptors");
        let McpSpawnRecord::Builtin { request_id, peer_index } = decode_spawn_record(&frame.bytes)? else {
            anyhow::bail!("MCP spawn capability received a response");
        };
        ensure!(request_id > previous_request, "MCP spawn request identity replayed");
        previous_request = request_id;
        let Some(slot) = peers.get_mut(peer_index as usize) else {
            respond(&sender, McpSpawnRecord::Denied { request_id }, None).await?;
            continue;
        };
        if let Some(peer) = slot.take() {
            // A refresh may ask again after closing the old MCP connection.
            // The old child must be positively reaped before reusing the slot.
            if let Err(error) = reap(peer).await {
                failures.push(format!("{error:#}"));
                return Err(error);
            }
        }
        let launched = factory.launch(policy.clone(), authority.clone());
        let (peer, control, http) = match launched {
            Ok(launched) => launched,
            Err(error) => {
                tracing::debug!(%error, "MCP builtin launch refused");
                respond(&sender, McpSpawnRecord::Denied { request_id }, None).await?;
                continue;
            }
        };
        *slot = Some(peer);
        let bootstrap = McpWorkerBootstrap {
            role: McpHttpRole::Builtin,
            generation: capsem_crypto::random::uuid_v4().into_bytes(),
        };
        let descriptors = [http.as_fd()];
        let granted = tokio::select! {
            biased;
            () = authority.revoked() => return Ok(()),
            () = &mut closed => return Ok(()),
            result = grant_worker(&control, bootstrap, &descriptors) => result,
        };
        drop(http);
        if let Err(error) = granted {
            tracing::debug!(%error, "native MCP peer did not attest readiness");
            if let Err(error) = reap(slot.take().expect("owned launched peer")).await {
                failures.push(format!("{error:#}"));
                return Err(error);
            }
            respond(&sender, McpSpawnRecord::Denied { request_id }, None).await?;
            continue;
        }
        respond(&sender, McpSpawnRecord::Granted { request_id }, Some(&control)).await?;
    }
}

async fn respond(sender: &Sender, response: McpSpawnRecord, socket: Option<&UnixStream>) -> Result<()> {
    let descriptors: Vec<_> = socket.into_iter().map(AsRawFd::as_raw_fd).collect();
    tokio::time::timeout(
        STARTUP_TIMEOUT,
        sender.send(&encode_spawn_record(response)?, &descriptors),
    )
    .await
    .context("MCP spawn response deadline")??;
    Ok(())
}

#[cfg(test)]
mod tests;

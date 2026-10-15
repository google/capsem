//! Role-bound bootstrap and narrow process-spawn capabilities for native MCP.

use super::http_transport::BrokerHttpClient;
use anyhow::{bail, ensure, Context, Result};
use capsem_foundation::unix::fd::{self, SocketShutdown};
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_foundation::unix::worker_sandbox::{self, Policy, Role};
use capsem_proto::mcp_worker::{
    decode_spawn_record, decode_worker_record, encode_spawn_record, encode_worker_record, McpHttpRole, McpSpawnRecord,
    McpWorkerBootstrap, McpWorkerRecord, MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS, MCP_WORKER_FRAME_SIZE,
    MCP_WORKER_MAX_FDS,
};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(capsem_proto::MCP_STARTUP_TIMEOUT_SECONDS);
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(capsem_proto::MCP_HTTP_TIMEOUT_SECONDS);

/// Give one child only its declared channels and wait for the exact lifetime's
/// post-confinement readiness. The caller owns reaping if this fails.
pub async fn grant_worker(
    control: &UnixStream,
    bootstrap: McpWorkerBootstrap,
    descriptors: &[BorrowedFd<'_>],
) -> Result<()> {
    let record = McpWorkerRecord::Bootstrap(bootstrap);
    ensure!(
        descriptors.len() == record.expected_descriptor_count(),
        "MCP grant descriptor count mismatch"
    );
    for descriptor in descriptors {
        fd::validate_connected_stream(*descriptor)?;
    }
    let sender = DescriptorSender::<MCP_WORKER_FRAME_SIZE, MCP_WORKER_MAX_FDS>::new(UnixStream::from(fd::duplicate(
        control.as_fd(),
    )?))?;
    let receiver = DescriptorReceiver::<MCP_WORKER_FRAME_SIZE, MCP_WORKER_MAX_FDS>::new(UnixStream::from(
        fd::duplicate(control.as_fd())?,
    ))?;
    let descriptors: Vec<_> = descriptors.iter().map(AsRawFd::as_raw_fd).collect();
    tokio::time::timeout(STARTUP_TIMEOUT, async {
        sender.send(&encode_worker_record(record)?, &descriptors).await?;
        let frame = receiver.recv().await?;
        ensure!(frame.fds.is_empty(), "MCP ready carried unexpected descriptors");
        ensure!(
            decode_worker_record(&frame.bytes)? == McpWorkerRecord::Ready(bootstrap),
            "MCP ready generation or role mismatch"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("MCP grant/readiness deadline")?
}

/// This value cannot acknowledge readiness until `confine_and_ready` succeeds.
pub struct PreparedMcpWorker {
    bootstrap: McpWorkerBootstrap,
    control: tokio::net::UnixStream,
    http: BrokerHttpClient,
    spawn: Option<McpSpawnClient>,
}

pub struct NativeMcpWorker {
    pub bootstrap: McpWorkerBootstrap,
    pub control: tokio::net::UnixStream,
    pub http: BrokerHttpClient,
    pub spawn: Option<McpSpawnClient>,
}

impl PreparedMcpWorker {
    pub async fn from_stdin(role: McpHttpRole) -> Result<Self> {
        let descriptor = fd::duplicate(std::io::stdin().as_fd())?;
        Self::adopt(UnixStream::from(descriptor), role).await
    }

    pub async fn adopt(control: UnixStream, role: McpHttpRole) -> Result<Self> {
        fd::validate_connected_stream(control.as_fd())?;
        let receiver = DescriptorReceiver::<MCP_WORKER_FRAME_SIZE, MCP_WORKER_MAX_FDS>::new(UnixStream::from(
            fd::duplicate(control.as_fd())?,
        ))?;
        let frame = tokio::time::timeout(STARTUP_TIMEOUT, receiver.recv())
            .await
            .context("MCP bootstrap deadline")??;
        let record = decode_worker_record(&frame.bytes)?;
        let McpWorkerRecord::Bootstrap(bootstrap) = record else {
            bail!("MCP startup received a ready record")
        };
        ensure!(bootstrap.role == role, "MCP bootstrap role mismatch");
        ensure!(
            frame.fds.len() == record.expected_descriptor_count(),
            "MCP bootstrap descriptor count mismatch"
        );
        let mut descriptors = frame.fds.into_iter();
        let http = BrokerHttpClient::connect(
            UnixStream::from(descriptors.next().expect("validated HTTP descriptor")),
            HTTP_TIMEOUT,
        )
        .await
        .context("adopt MCP HTTP capability")?;
        let spawn = descriptors
            .next()
            .map(|descriptor| McpSpawnClient::new(UnixStream::from(descriptor)))
            .transpose()?;
        drop(receiver);
        fd::set_nonblocking(control.as_fd(), true)?;
        Ok(Self {
            bootstrap,
            control: tokio::net::UnixStream::from_std(control)?,
            http,
            spawn,
        })
    }

    pub async fn confine_and_ready(mut self) -> Result<NativeMcpWorker> {
        let role = match self.bootstrap.role {
            McpHttpRole::Aggregator => Role::McpAggregator,
            McpHttpRole::Builtin => Role::McpBuiltin,
        };
        worker_sandbox::confine(&Policy::new(role)).context("install native MCP sandbox")?;
        worker_sandbox::attest_descriptor_only().context("attest native MCP sandbox")?;
        let ready = encode_worker_record(McpWorkerRecord::Ready(self.bootstrap))?;
        tokio::time::timeout(STARTUP_TIMEOUT, self.control.write_all(&ready))
            .await
            .context("MCP ready deadline")??;
        Ok(NativeMcpWorker {
            bootstrap: self.bootstrap,
            control: self.control,
            http: self.http,
            spawn: self.spawn,
        })
    }
}

struct SpawnWire {
    socket: UnixStream,
    sender: DescriptorSender<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>,
    receiver: DescriptorReceiver<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>,
    next: u64,
    failed: bool,
}

/// Requests only the trusted parent's predefined builtin; never an executable path.
#[derive(Clone)]
pub struct McpSpawnClient(Arc<tokio::sync::Mutex<SpawnWire>>);

impl McpSpawnClient {
    pub fn new(socket: UnixStream) -> Result<Self> {
        fd::validate_connected_stream(socket.as_fd())?;
        let sender = DescriptorSender::new(UnixStream::from(fd::duplicate(socket.as_fd())?))?;
        let receiver = DescriptorReceiver::new(UnixStream::from(fd::duplicate(socket.as_fd())?))?;
        Ok(Self(Arc::new(tokio::sync::Mutex::new(SpawnWire {
            socket,
            sender,
            receiver,
            next: 1,
            failed: false,
        }))))
    }

    pub async fn builtin(&self, peer_index: u32) -> Result<UnixStream> {
        ensure!(
            (peer_index as usize) < capsem_proto::MCP_BUILTIN_PEERS_PER_SESSION,
            "unapproved MCP peer index"
        );
        let mut wire = tokio::time::timeout(STARTUP_TIMEOUT, self.0.lock())
            .await
            .context("MCP spawn queue deadline")?;
        ensure!(!wire.failed, "MCP spawn capability is closed");
        let result = tokio::time::timeout(STARTUP_TIMEOUT, request_builtin(&mut wire, peer_index))
            .await
            .context("MCP spawn capability deadline")
            .and_then(|result| result);
        if result.is_err() {
            wire.failed = true;
            fd::shutdown(wire.socket.as_fd(), SocketShutdown::Both).context("close failed MCP spawn capability")?;
        }
        result
    }
}

async fn request_builtin(wire: &mut SpawnWire, peer_index: u32) -> Result<UnixStream> {
    let request_id = wire.next;
    wire.next = request_id.checked_add(1).context("MCP spawn ids exhausted")?;
    let request = encode_spawn_record(McpSpawnRecord::Builtin { request_id, peer_index })?;
    wire.sender.send(&request, &[]).await?;
    let frame = wire.receiver.recv().await?;
    let record = decode_spawn_record(&frame.bytes)?;
    ensure!(
        frame.fds.len() == record.expected_descriptor_count(),
        "MCP spawn descriptor count mismatch"
    );
    ensure!(
        record == McpSpawnRecord::Granted { request_id },
        "MCP spawn rejected or reply identity changed"
    );
    let descriptor: OwnedFd = frame.fds.into_iter().next().context("MCP spawn missing channel")?;
    fd::validate_connected_stream(descriptor.as_fd())?;
    // Keep our owned socket alive before releasing the sender's copy. Darwin
    // may collect sockets referenced only by queued SCM_RIGHTS messages.
    wire.sender
        .send(&encode_spawn_record(McpSpawnRecord::Adopted { request_id })?, &[])
        .await?;
    Ok(UnixStream::from(descriptor))
}

#[cfg(test)]
mod tests;

//! Steady-state control connection: bounded async I/O and an owned frame reader.
use crate::{ControlFrameTooLarge, VsockConnection};
use anyhow::{Context, Result};
use capsem_proto::{decode_guest_msg, GuestToHost, MAX_FRAME_SIZE};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

const IO_DEADLINE: Duration = Duration::from_secs(5);

pub struct Connection {
    owner: Arc<VsockConnection>,
    writer: OwnedWriteHalf,
    incoming: mpsc::Receiver<Result<GuestToHost>>,
    reader: JoinSet<()>,
}

impl Connection {
    pub fn new(owner: Arc<VsockConnection>) -> Result<Self> {
        let socket = std::os::unix::net::UnixStream::from(owner.try_clone_fd()?);
        socket.set_nonblocking(true)?;
        let (mut read, writer) = tokio::net::UnixStream::from_std(socket)?.into_split();
        let (send, incoming) = mpsc::channel(32);
        let mut reader = JoinSet::new();
        reader.spawn(async move {
            loop {
                let message = read_message(&mut read).await;
                if let Err(error) = &message {
                    if error.downcast_ref::<ControlFrameTooLarge>().is_some() {
                        tracing::error!(%error, "control bridge: oversized guest frame discarded");
                        continue;
                    }
                }
                let failed = message.is_err();
                if send.send(message).await.is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            owner,
            writer,
            incoming,
            reader,
        })
    }

    /// Framing happens before the caller records messages for replay.
    pub async fn write(&mut self, frame: &[u8]) -> Result<()> {
        let result = tokio::time::timeout(IO_DEADLINE, self.writer.write_all(frame))
            .await
            .context("control write timed out")
            .and_then(|result| result.context("control write failed"));
        if result.is_err() {
            self.disconnect();
        }
        result
    }

    /// Cancel safe: the owned reader keeps partial frames across actor selects.
    pub async fn recv(&mut self) -> Option<Result<GuestToHost>> {
        self.incoming.recv().await
    }

    pub async fn close(&mut self) {
        self.disconnect();
        self.reader.shutdown().await;
    }

    fn disconnect(&self) {
        if let Err(error) = self.owner.shutdown_both() {
            tracing::debug!(%error, "control connection shutdown");
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.disconnect();
    }
}

async fn read_message(reader: &mut OwnedReadHalf) -> Result<GuestToHost> {
    let mut length = [0; 4];
    // Quiet idle connections have no deadline. Once a frame starts, bound it.
    reader.read_exact(&mut length[..1]).await?;
    tokio::time::timeout(IO_DEADLINE, async {
        reader.read_exact(&mut length[1..]).await?;
        let len = u32::from_be_bytes(length) as usize;
        if len > MAX_FRAME_SIZE as usize {
            let copied = tokio::io::copy(&mut reader.take(len as u64), &mut tokio::io::sink()).await?;
            anyhow::ensure!(copied == len as u64, "truncated oversized control frame");
            return Err(ControlFrameTooLarge(len).into());
        }
        let mut payload = vec![0; len];
        reader.read_exact(&mut payload).await?;
        decode_guest_msg(&payload)
    })
    .await
    .context("control frame timed out")?
}

#[cfg(test)]
mod tests;

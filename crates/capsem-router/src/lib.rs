//! Data-plane companion protocol. No hypervisor or service-control dependency.
use serde::{Deserialize, Serialize};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_unix_ipc::serde::Handle;

pub const MAX_CONNECTIONS: usize = 128;

/// Only the trusted parent sends descriptor-bearing messages. The parent must
/// never deserialize a variable-sized message from the confined child.
#[derive(Serialize, Deserialize)]
pub enum Grant<Socket = Handle<OwnedFd>> {
    Listen { socket: Socket },
    Connected { id: u64, socket: Socket },
    Refused { id: u64 },
}

/// Borrow descriptors across send; `Handle` serialization consumes ownership
/// but tokio-unix-ipc 0.4 does not close its serialized descriptors afterwards.
/// Keeping ownership with the caller also closes correctly on cancellation.
pub async fn send_grant(
    sender: &capsem_foundation::unix::router_channel::Sender,
    grant: Grant<BorrowedFd<'_>>,
) -> io::Result<()> {
    use tokio_unix_ipc::serde::HandleRef;
    let grant = match grant {
        Grant::Listen { socket } => Grant::Listen {
            socket: HandleRef(socket.as_raw_fd()),
        },
        Grant::Connected { id, socket } => Grant::Connected {
            id,
            socket: HandleRef(socket.as_raw_fd()),
        },
        Grant::Refused { id } => Grant::Refused { id },
    };
    let (payload, fds) = tokio_unix_ipc::serde::serialize((grant, true))?;
    sender.send(&payload, &fds).await?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Ready,
    Open(u64),
    Closed(u64),
    ConfinementFailed,
}

impl Event {
    /// Fixed nine-byte requests bound allocation even for a hostile router.
    pub async fn read(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Self> {
        let mut frame = [0; 9];
        reader.read_exact(&mut frame).await?;
        let id = u64::from_be_bytes(frame[1..].try_into().unwrap());
        match (frame[0], id) {
            (0, 0) => Ok(Self::Ready),
            (1, id) if id != 0 => Ok(Self::Open(id)),
            (2, id) if id != 0 => Ok(Self::Closed(id)),
            (3, 0) => Ok(Self::ConfinementFailed),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid router event")),
        }
    }

    pub async fn write(&self, writer: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
        let (kind, id) = match self {
            Self::Ready => (0, 0u64),
            Self::Open(id) => (1, *id),
            Self::Closed(id) => (2, *id),
            Self::ConfinementFailed => (3, 0),
        };
        let mut frame = [0; 9];
        frame[0] = kind;
        frame[1..].copy_from_slice(&id.to_be_bytes());
        writer.write_all(&frame).await
    }
}

pub async fn relay(
    listener: tokio::net::TcpListener,
    grants: capsem_foundation::unix::router_channel::Receiver<Grant>,
    mut events: tokio::net::UnixStream,
) -> io::Result<()> {
    use std::collections::HashMap;
    use tokio::sync::oneshot;
    use tokio::time::{timeout, Duration};
    let mut jobs = tokio::task::JoinSet::new();
    let mut pending: HashMap<u64, oneshot::Sender<OwnedFd>> = HashMap::new();
    let mut readers = tokio::task::JoinSet::new();
    let (queue, mut messages) = tokio::sync::mpsc::channel(16);
    readers.spawn(async move {
        loop {
            let message = grants.recv().await;
            let failed = message.is_err();
            if queue.send(message).await.is_err() || failed {
                break;
            }
        }
    });
    let mut next_id = 0u64;
    Event::Ready.write(&mut events).await?;
    loop {
        tokio::select! {
            accepted = listener.accept(), if jobs.len() < MAX_CONNECTIONS => {
                let (mut tcp, _) = accepted?;
                tcp.set_nodelay(true)?;
                next_id = next_id.checked_add(1).ok_or_else(|| io::Error::other("router connection id exhausted"))?;
                let id = next_id;
                let (sender, receiver) = oneshot::channel();
                pending.insert(id, sender);
                jobs.spawn(async move {
                    let result = async {
                        let fd = timeout(Duration::from_secs(10), receiver).await
                            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
                            .map_err(|_| io::Error::from(io::ErrorKind::ConnectionRefused))?;
                        // The backend data fd is socket-like on both VZ and KVM;
                        // no Unix address operation is performed on this stream.
                        let stream = std::os::unix::net::UnixStream::from(fd);
                        capsem_foundation::unix::fd::set_nonblocking(stream.as_fd(), true)?;
                        let mut data = tokio::net::UnixStream::from_std(stream)?;
                        tokio::io::copy_bidirectional(&mut tcp, &mut data).await?;
                        Ok::<_, io::Error>(())
                    }.await;
                    (id, result)
                });
                Event::Open(id).write(&mut events).await?;
            }
            message = messages.recv() => match message.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?? {
                Grant::Connected { id, socket } => {
                    if let Some(sender) = pending.remove(&id) {
                        // A late grant is closed immediately after setup timeout.
                        let _ = sender.send(socket.into_inner());
                    }
                }
                Grant::Refused { id } => { pending.remove(&id); }
                Grant::Listen { .. } => return Err(io::Error::new(io::ErrorKind::InvalidData, "duplicate listener grant")),
            },
            completed = jobs.join_next(), if !jobs.is_empty() => {
                let (id, result) = completed.unwrap().map_err(io::Error::other)?;
                pending.remove(&id);
                if let Err(error) = result { eprintln!("router connection {id}: {error}"); }
                Event::Closed(id).write(&mut events).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests;

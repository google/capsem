//! Versioned descriptor-pair grants; policy and destination selection stay in core.
use capsem_foundation::unix::{
    fd,
    router_channel::{Frame, Receiver, Sender, FRAME_SIZE},
    router_stream,
};
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};

pub const MAX_CONNECTIONS: usize = 128;
pub const CONNECTIONS_PER_CLASS: usize = 64;
const VERSION: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Expose,
    Private,
}

pub enum Grant<Socket = OwnedFd> {
    Hello,
    Connected {
        id: u64,
        class: Class,
        source: Socket,
        destination: Socket,
    },
    Abort {
        id: u64,
    },
}

fn encode(kind: u8, id: u64) -> [u8; FRAME_SIZE] {
    let mut frame = [0; FRAME_SIZE];
    frame[0] = VERSION;
    frame[1] = kind;
    frame[2..].copy_from_slice(&id.to_be_bytes());
    frame
}
fn decode(bytes: [u8; FRAME_SIZE]) -> io::Result<(u8, u64)> {
    if bytes[0] != VERSION {
        return Err(invalid("incompatible router protocol"));
    }
    Ok((bytes[1], u64::from_be_bytes(bytes[2..].try_into().unwrap())))
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Grant {
    pub fn decode(mut frame: Frame) -> io::Result<Self> {
        match (decode(frame.bytes)?, frame.fds.len()) {
            ((0, 0), 0) => Ok(Self::Hello),
            ((kind @ (1 | 3), id), 2) if id != 0 => {
                let destination = frame.fds.pop().unwrap();
                let source = frame.fds.pop().unwrap();
                Ok(Self::Connected {
                    id,
                    class: if kind == 1 { Class::Expose } else { Class::Private },
                    source,
                    destination,
                })
            }
            ((2, id), 0) if id != 0 => Ok(Self::Abort { id }),
            _ => Err(invalid("invalid router grant or descriptor count")),
        }
    }
}
pub async fn send_grant(sender: &Sender, grant: Grant<BorrowedFd<'_>>) -> io::Result<()> {
    match grant {
        Grant::Hello => sender.send(&encode(0, 0), &[]).await?,
        Grant::Connected {
            id,
            class,
            source,
            destination,
        } => {
            sender
                .send(
                    &encode(if class == Class::Expose { 1 } else { 3 }, id),
                    &[source.as_raw_fd(), destination.as_raw_fd()],
                )
                .await?
        }
        Grant::Abort { id } => sender.send(&encode(2, id), &[]).await?,
    };
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Ready,
    Accepted(u64),
    Closed(u64),
    ConfinementFailed,
    Refused(u64),
}
impl Event {
    pub async fn read(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Self> {
        let mut frame = [0; FRAME_SIZE];
        reader.read_exact(&mut frame).await?;
        match decode(frame)? {
            (0, 0) => Ok(Self::Ready),
            (1, id) if id != 0 => Ok(Self::Accepted(id)),
            (2, id) if id != 0 => Ok(Self::Closed(id)),
            (3, 0) => Ok(Self::ConfinementFailed),
            (4, id) if id != 0 => Ok(Self::Refused(id)),
            _ => Err(invalid("invalid router event")),
        }
    }
    pub async fn write(&self, writer: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
        let frame = match self {
            Self::Ready => encode(0, 0),
            Self::Accepted(id) => encode(1, *id),
            Self::Closed(id) => encode(2, *id),
            Self::ConfinementFailed => encode(3, 0),
            Self::Refused(id) => encode(4, *id),
        };
        timeout(Duration::from_secs(2), writer.write_all(&frame)).await?
    }
}

// Explicit shutdown wakes holders of duplicate FDs on cancellation and EOF.
struct Stream(UnixStream);
impl Stream {
    fn new(socket: OwnedFd) -> io::Result<Self> {
        fd::validate_connected_stream(socket.as_fd())?;
        fd::set_nonblocking(socket.as_fd(), true)?;
        Ok(Self(UnixStream::from_std(std::os::unix::net::UnixStream::from(
            socket,
        ))?))
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        if let Err(error) = fd::shutdown(self.0.as_fd(), fd::SocketShutdown::Both) {
            tracing::debug!(%error, "router endpoint shutdown");
        }
    }
}

pub async fn relay(grants: Receiver, mut events: UnixStream) -> io::Result<()> {
    let slots = [
        Arc::new(tokio::sync::Semaphore::new(CONNECTIONS_PER_CLASS)),
        Arc::new(tokio::sync::Semaphore::new(CONNECTIONS_PER_CLASS)),
    ];
    let mut jobs = tokio::task::JoinSet::new();
    let mut active = HashMap::new();
    let mut readers = tokio::task::JoinSet::new();
    let (queue, mut messages) = tokio::sync::mpsc::channel(16);
    readers.spawn(async move {
        loop {
            let message = grants.recv().await.and_then(Grant::decode);
            let failed = message.is_err();
            if queue.send(message).await.is_err() || failed {
                break;
            }
        }
    });
    let result = async {
        Event::Ready.write(&mut events).await?;
        let mut last_id = 0;
        loop {
            tokio::select! {
                message = messages.recv() => match message.ok_or_else(|| invalid("router grant channel closed"))?? {
                    Grant::Connected { id, class, source, destination } => {
                        if id <= last_id { return Err(invalid("reused router connection id")); }
                        last_id = id;
                        let index = usize::from(class == Class::Private);
                        let permit = match slots[index].clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                tracing::debug!(connection_id = id, ?class, "router class quota exhausted");
                                Event::Refused(id).write(&mut events).await?;
                                continue;
                            }
                        };
                        let pair = Stream::new(source).and_then(|source| Stream::new(destination).map(|destination| (source, destination)));
                        let (mut source, mut destination) = match pair {
                            Ok(pair) => pair,
                            Err(error) => {
                                tracing::debug!(connection_id = id, %error, "router rejected descriptor pair");
                                Event::Refused(id).write(&mut events).await?;
                                continue;
                            }
                        };
                        // Acknowledgement precedes forwarding and is bounded.
                        Event::Accepted(id).write(&mut events).await?;
                        let task = jobs.spawn(async move {
                            let _permit = permit;
                            let result = router_stream::copy(&mut source.0, &mut destination.0, router_stream::Limits::default()).await;
                            tracing::debug!(connection_id = id, ?class, reason = ?result.reason, from_source = result.from_source,
                                to_source = result.to_source, error = ?result.error, "router stream ended");
                            id
                        });
                        active.insert(id, task);
                    }
                    Grant::Abort { id } => {
                        if let Some(task) = active.get(&id) { task.abort(); }
                    }
                    Grant::Hello => return Err(invalid("duplicate router hello")),
                },
                completed = jobs.join_next(), if !jobs.is_empty() => match completed.unwrap() {
                    Ok(id) => {
                        active.remove(&id);
                        Event::Closed(id).write(&mut events).await?;
                    }
                    Err(error) if error.is_cancelled() => {
                        if let Some(id) = active.iter().find_map(|(&id, task)| (task.id() == error.id()).then_some(id)) {
                            active.remove(&id);
                            Event::Closed(id).write(&mut events).await?;
                        }
                    },
                    Err(error) => return Err(io::Error::other(error)),
                }
            }
        }
    }.await;
    readers.shutdown().await;
    jobs.shutdown().await;
    result
}

#[cfg(test)]
mod tests;

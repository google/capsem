//! Versioned descriptor-pair grants; policy and destination selection stay in core.
use capsem_foundation::unix::{
    fd,
    router_channel::{Frame, Receiver, Sender, FRAME_SIZE},
    router_stream,
};
pub use router_stream::{CloseReason, CloseReport};
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};

pub const MAX_CONNECTIONS: usize = 128;
pub const CONNECTIONS_PER_CLASS: usize = 64;
const VERSION: u8 = 3;

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
    Closed(u64, CloseReport),
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
            (2, id) if id != 0 => {
                let mut report = [0; 17];
                reader.read_exact(&mut report).await?;
                Ok(Self::Closed(
                    id,
                    CloseReport {
                        reason: CloseReason::try_from(report[0]).map_err(invalid)?,
                        from_source: u64::from_be_bytes(report[1..9].try_into().unwrap()),
                        to_source: u64::from_be_bytes(report[9..17].try_into().unwrap()),
                    },
                ))
            }
            (3, 0) => Ok(Self::ConfinementFailed),
            (4, id) if id != 0 => Ok(Self::Refused(id)),
            _ => Err(invalid("invalid router event")),
        }
    }
    pub async fn write(&self, writer: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
        let header = match self {
            Self::Ready => encode(0, 0),
            Self::Accepted(id) => encode(1, *id),
            Self::Closed(id, _) => encode(2, *id),
            Self::ConfinementFailed => encode(3, 0),
            Self::Refused(id) => encode(4, *id),
        };
        let mut frame = [0; 27];
        frame[..FRAME_SIZE].copy_from_slice(&header);
        let length = if let Self::Closed(_, report) = self {
            frame[10] = report.reason as u8;
            frame[11..19].copy_from_slice(&report.from_source.to_be_bytes());
            frame[19..27].copy_from_slice(&report.to_source.to_be_bytes());
            frame.len()
        } else {
            FRAME_SIZE
        };
        timeout(Duration::from_secs(2), writer.write_all(&frame[..length])).await?
    }
}

// Successful FIN drain is graceful. Every other exit, including task abort,
// must not send an early FIN while the parent still holds a TCP descriptor.
struct Stream {
    socket: UnixStream,
    graceful: bool,
}
impl Stream {
    fn new(socket: OwnedFd) -> io::Result<Self> {
        fd::validate_connected_stream(socket.as_fd())?;
        fd::set_stream_buffers(socket.as_fd(), router_stream::SOCKET_BUFFER_SIZE)?;
        fd::set_nonblocking(socket.as_fd(), true)?;
        Ok(Self {
            socket: UnixStream::from_std(std::os::unix::net::UnixStream::from(socket))?,
            graceful: false,
        })
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        if !self.graceful {
            match fd::tcp_reset_on_close(self.socket.as_fd()) {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => tracing::error!(%error, "router TCP reset configuration failed"),
            }
        }
        if let Err(error) = fd::shutdown(self.socket.as_fd(), fd::SocketShutdown::Both) {
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
                        let (stop, stopped) = tokio::sync::oneshot::channel();
                        jobs.spawn(async move {
                            let _permit = permit;
                            let result = router_stream::copy_until(&mut source.socket, &mut destination.socket, router_stream::Limits::default(), async {
                                let _ = stopped.await;
                            }).await;
                            if result.reason == CloseReason::Complete {
                                source.graceful = true;
                                destination.graceful = true;
                            }
                            drop(source);
                            drop(destination);
                            tracing::debug!(connection_id = id, ?class, reason = ?result.reason, from_source = result.from_source,
                                to_source = result.to_source, error = ?result.error, "router stream ended");
                            (id, result.report())
                        });
                        active.insert(id, stop);
                    }
                    Grant::Abort { id } => {
                        if let Some(stop) = active.remove(&id) { let _ = stop.send(()); }
                    }
                    Grant::Hello => return Err(invalid("duplicate router hello")),
                },
                completed = jobs.join_next(), if !jobs.is_empty() => match completed.unwrap() {
                    Ok((id, report)) => {
                        active.remove(&id);
                        Event::Closed(id, report).write(&mut events).await?;
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

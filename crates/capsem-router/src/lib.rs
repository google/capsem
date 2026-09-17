//! Versioned descriptor grants; policy and destination selection stay in core.
//!
//! Two confined companions share this protocol: [`relay`] copies bytes
//! between a published host port's client and the guest for one VM owner,
//! and [`switch::run`] is one network's layer-2 switch, with a port for every
//! cable plugged into it.
use capsem_foundation::unix::{
    fd,
    router_channel::{Frame, Receiver, Sender, FRAME_SIZE},
    router_stream,
};
/// What a [`PortReport`]'s `dropped` counters are indexed by.
pub use capsem_network::switch::DropReason;
pub use router_stream::{CloseReason, CloseReport};
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};

mod preview;

pub const MAX_CONNECTIONS: usize = 128;
/// The default ceiling on a relay's pairs and on a switch's ports.
pub const CONNECTION_LIMIT: usize = 64;
const VERSION: u8 = 7;
/// Preview grant kinds: the admitted request shape is part of the grant.
const PREVIEW_REQUEST: u8 = 3;
const PREVIEW_UPGRADE: u8 = 4;

pub enum Grant<Socket = OwnedFd> {
    Hello,
    /// A published port's host client (`source`, raw TCP) and the guest's
    /// framed VSOCK leg (`destination`).
    Connected {
        id: u64,
        source: Socket,
        destination: Socket,
    },
    /// An authenticated browser socket and its guest leg. HTTP parsing and
    /// control-header filtering happen inside this confined process.
    /// `admission` is the request shape the parent's policy admitted; the
    /// router refuses any other shape on the same connection.
    Preview {
        id: u64,
        admission: capsem_proto::PreviewAdmissionKind,
        source: Socket,
        destination: Socket,
    },
    Abort {
        id: u64,
    },
    /// Plug one cable (a duplicate of the guest's VSOCK stream for it) into
    /// the switch; `port` is a [`port_id`], so it names the attachment's
    /// generation and address, and the address names the port's MAC.
    Plug {
        port: u64,
        socket: Socket,
    },
    Unplug {
        port: u64,
    },
}

/// A port's id: the attachment generation the parent assigned, then the
/// attachment's address, so one `u64` says which cable and whose it is.
pub fn port_id(generation: u32, address: std::net::Ipv4Addr) -> u64 {
    (u64::from(generation) << 32) | u64::from(address.to_bits())
}

pub fn port_address(port: u64) -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::from_bits(port as u32)
}

pub fn port_generation(port: u64) -> u32 {
    (port >> 32) as u32
}

/// What one port carried, reported when it closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortReport {
    pub reason: CloseReason,
    /// Records read from the cable, whatever became of them.
    pub frames_in: u64,
    pub bytes_in: u64,
    /// Records written to the cable.
    pub frames_out: u64,
    pub bytes_out: u64,
    /// Frames lost, indexed by [`capsem_network::switch::DropReason`].
    pub dropped: [u64; DropReason::ALL.len()],
}

const PORT_REPORT_BYTES: usize = 1 + 8 * (4 + DropReason::ALL.len());

impl PortReport {
    fn encode(&self) -> [u8; PORT_REPORT_BYTES] {
        let mut bytes = [0; PORT_REPORT_BYTES];
        bytes[0] = self.reason as u8;
        let counters = [self.frames_in, self.bytes_in, self.frames_out, self.bytes_out];
        for (slot, value) in counters.iter().chain(self.dropped.iter()).enumerate() {
            bytes[1 + slot * 8..9 + slot * 8].copy_from_slice(&value.to_be_bytes());
        }
        bytes
    }

    fn decode(bytes: &[u8; PORT_REPORT_BYTES]) -> io::Result<Self> {
        let counter = |slot: usize| u64::from_be_bytes(bytes[1 + slot * 8..9 + slot * 8].try_into().unwrap());
        Ok(Self {
            reason: CloseReason::try_from(bytes[0]).map_err(invalid)?,
            frames_in: counter(0),
            bytes_in: counter(1),
            frames_out: counter(2),
            bytes_out: counter(3),
            dropped: std::array::from_fn(|reason| counter(4 + reason)),
        })
    }
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
            ((1, id), 2) if id != 0 => {
                let destination = frame.fds.pop().unwrap();
                let source = frame.fds.pop().unwrap();
                Ok(Self::Connected {
                    id,
                    source,
                    destination,
                })
            }
            ((2, id), 0) if id != 0 => Ok(Self::Abort { id }),
            ((kind @ (PREVIEW_REQUEST | PREVIEW_UPGRADE), id), 2) if id != 0 => {
                let destination = frame.fds.pop().unwrap();
                let source = frame.fds.pop().unwrap();
                Ok(Self::Preview {
                    id,
                    admission: if kind == PREVIEW_UPGRADE {
                        capsem_proto::PreviewAdmissionKind::WebsocketUpgrade
                    } else {
                        capsem_proto::PreviewAdmissionKind::Request
                    },
                    source,
                    destination,
                })
            }
            ((5, port), 1) if port_generation(port) != 0 => Ok(Self::Plug {
                port,
                socket: frame.fds.pop().unwrap(),
            }),
            ((6, port), 0) if port_generation(port) != 0 => Ok(Self::Unplug { port }),
            _ => Err(invalid("invalid router grant or descriptor count")),
        }
    }
}
pub async fn send_grant(sender: &Sender, grant: Grant<BorrowedFd<'_>>) -> io::Result<()> {
    match grant {
        Grant::Hello => sender.send(&encode(0, 0), &[]).await?,
        Grant::Connected {
            id,
            source,
            destination,
        } => {
            sender
                .send(&encode(1, id), &[source.as_raw_fd(), destination.as_raw_fd()])
                .await?
        }
        Grant::Abort { id } => sender.send(&encode(2, id), &[]).await?,
        Grant::Preview {
            id,
            admission,
            source,
            destination,
        } => {
            let kind = match admission {
                capsem_proto::PreviewAdmissionKind::Request => PREVIEW_REQUEST,
                capsem_proto::PreviewAdmissionKind::WebsocketUpgrade => PREVIEW_UPGRADE,
            };
            sender
                .send(&encode(kind, id), &[source.as_raw_fd(), destination.as_raw_fd()])
                .await?
        }
        Grant::Plug { port, socket } => sender.send(&encode(5, port), &[socket.as_raw_fd()]).await?,
        Grant::Unplug { port } => sender.send(&encode(6, port), &[]).await?,
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
    PortClosed(u64, PortReport),
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
            (5, port) if port != 0 => {
                let mut report = [0; PORT_REPORT_BYTES];
                reader.read_exact(&mut report).await?;
                Ok(Self::PortClosed(port, PortReport::decode(&report)?))
            }
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
            Self::PortClosed(port, _) => encode(5, *port),
        };
        let mut frame = [0; FRAME_SIZE + PORT_REPORT_BYTES];
        frame[..FRAME_SIZE].copy_from_slice(&header);
        let length = match self {
            Self::Closed(_, report) => {
                frame[10] = report.reason as u8;
                frame[11..19].copy_from_slice(&report.from_source.to_be_bytes());
                frame[19..27].copy_from_slice(&report.to_source.to_be_bytes());
                27
            }
            Self::PortClosed(_, report) => {
                frame[FRAME_SIZE..].copy_from_slice(&report.encode());
                frame.len()
            }
            _ => FRAME_SIZE,
        };
        timeout(Duration::from_secs(2), writer.write_all(&frame[..length])).await?
    }
}

// Successful FIN drain is graceful. Every other exit, including task abort,
// must not send an early FIN while the parent still holds a TCP descriptor.
struct Stream {
    socket: UnixStream,
    graceful: bool,
    tcp: bool,
}
impl Stream {
    fn new(socket: OwnedFd) -> io::Result<Self> {
        let tcp = fd::tcp_reset_on_close(socket.as_fd())?;
        Ok(Self {
            socket: adopt(socket)?,
            graceful: false,
            tcp,
        })
    }
}

/// A granted connected stream, checked and sized, as a tokio socket.
fn adopt(socket: OwnedFd) -> io::Result<UnixStream> {
    fd::validate_connected_stream(socket.as_fd())?;
    fd::set_stream_buffers(socket.as_fd(), router_stream::SOCKET_BUFFER_SIZE)?;
    fd::set_nonblocking(socket.as_fd(), true)?;
    UnixStream::from_std(std::os::unix::net::UnixStream::from(socket))
}
impl Drop for Stream {
    fn drop(&mut self) {
        if self.tcp {
            if !self.graceful {
                return;
            }
            if let Err(error) = fd::tcp_clear_reset_on_close(self.socket.as_fd()) {
                tracing::error!(%error, "router graceful TCP close configuration failed");
            }
        } else if !self.graceful {
            // An abruptly ended VSOCK endpoint is closed, never shut down.
            // With the guest still sending, sixteen shutdown() calls on
            // Virtualization.framework descriptors in one burst made the
            // framework stop the whole VM ("Internal Virtualization error",
            // VZErrorDomain code 1): every stream to the guest ended in the
            // same millisecond, three runs in five. A write-side shutdown
            // alone did it too; closing this duplicate did not, forty bursts
            // running. The owner, which holds the framework object, ends the
            // connection once this copy is gone (S04-018).
            return;
        }
        if let Err(error) = fd::shutdown(self.socket.as_fd(), fd::SocketShutdown::Both) {
            tracing::debug!(%error, "router endpoint shutdown");
        }
    }
}

#[derive(Clone, Copy)]
pub struct ConnectionLimits {
    connections: usize,
}

impl Default for ConnectionLimits {
    fn default() -> Self {
        Self {
            connections: CONNECTION_LIMIT,
        }
    }
}

impl ConnectionLimits {
    pub fn new(connections: u16) -> io::Result<Self> {
        let limits = Self {
            connections: usize::from(connections),
        };
        if !(1..=CONNECTION_LIMIT).contains(&limits.connections) {
            return Err(invalid("router connection limit exceeds its ceiling"));
        }
        Ok(limits)
    }
}

pub async fn relay(grants: Receiver, mut events: UnixStream, limits: ConnectionLimits) -> io::Result<()> {
    let slots = Arc::new(tokio::sync::Semaphore::new(limits.connections));
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
                    grant @ (Grant::Connected { .. } | Grant::Preview { .. }) => {
                        let (id, source, destination, preview) = match grant {
                            Grant::Connected { id, source, destination } => (id, source, destination, None),
                            Grant::Preview { id, admission, source, destination } => (id, source, destination, Some(admission)),
                            _ => unreachable!(),
                        };
                        if id <= last_id { return Err(invalid("reused router connection id")); }
                        last_id = id;
                        let permit = match slots.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                tracing::debug!(connection_id = id, "router connection limit reached");
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
                        // The source is a host client; the destination the guest's VSOCK leg.
                        let framings = router_stream::Framings {
                            source: router_stream::Framing::Raw,
                            destination: router_stream::Framing::Framed,
                        };
                        jobs.spawn(async move {
                            let _permit = permit;
                            let result = if let Some(admission) = preview {
                                preview::relay(&mut source.socket, &mut destination.socket, admission, async {
                                    let _ = stopped.await;
                                })
                                .await
                            } else {
                                router_stream::copy_until(
                                    &mut source.socket,
                                    &mut destination.socket,
                                    framings,
                                    router_stream::Limits::default(),
                                    async { let _ = stopped.await; },
                                )
                                .await
                            };
                            if result.reason == CloseReason::Complete {
                                source.graceful = true;
                                destination.graceful = true;
                            }
                            drop(source);
                            drop(destination);
                            tracing::debug!(connection_id = id, reason = ?result.reason, from_source = result.from_source,
                                to_source = result.to_source, error = ?result.error, "router stream ended");
                            (id, result.report())
                        });
                        active.insert(id, stop);
                    }
                    Grant::Abort { id } => {
                        if let Some(stop) = active.remove(&id) { let _ = stop.send(()); }
                    }
                    Grant::Hello => return Err(invalid("duplicate router hello")),
                    Grant::Plug { port, .. } | Grant::Unplug { port } => {
                        tracing::debug!(port, "switch port grant to a pair relay");
                        Event::Refused(port).write(&mut events).await?;
                    }
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

pub mod switch;

#[cfg(test)]
mod tests;

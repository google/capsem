//! One network's switch: a port for every cable plugged into it.
//!
//! A cable is one stream of `u16`-length ethernet frames (the guest pump's
//! codec). Where a frame goes is [`capsem_network::switch::Table::route`]:
//! MACs, every protocol, broadcast flooded, and each port speaking only as
//! its own MAC and address. The table is published
//! whole on every plug and unplug; a port's reader checks one atomic version
//! per read and otherwise routes without a lock.
//!
//! A port is one job owning both halves of its cable. Its reader splits
//! every whole record out of one large read and hands it on as `Bytes` (a
//! flood clones the handle, not the frame); its writer drains its queue into
//! vectored writes. A reader never waits on anyone else's queue: a member
//! that stops reading loses the frames addressed to it, not everyone else's
//! port. Unplugging cancels the job, which drops both halves -- even a
//! writer blocked on that stalled member -- and closes the descriptor before
//! the port is reported closed and its slot is reused. Descriptors are
//! closed, never shut down (S04-018).
use super::*;
use bytes::{Buf, Bytes, BytesMut};
use capsem_network::frames::HEADER_BYTES;
use capsem_network::switch::{Mac, Route, Station, Table};
use capsem_proto::privatelink::mac_of;
use std::io::IoSlice;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::RwLock;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

/// Frames a port's queue holds before senders start losing frames to it:
/// enough that one full read of small frames (256 KiB at 1500 bytes is 170)
/// fits a single destination.
pub const QUEUE_FRAMES: usize = 256;
/// Bytes a port's queue holds, the writer's batch in flight included: what
/// 64 full frames took before the queue grew, so small frames gained depth
/// and full ones no memory.
pub const QUEUE_BYTES: usize = 64 * (HEADER_BYTES + u16::MAX as usize);
/// Floods one port may send per second. Ordinary ARP is far below it; a
/// storm above it is dropped and counted.
pub const BROADCASTS_PER_SECOND: u32 = 1024;
/// One read takes this many bytes of records at most.
const READ_BUFFER_BYTES: usize = 256 * 1024;

/// One port's queue: frames for its writer, bounded in count by the channel
/// and in bytes by a counter the writer only releases once a batch is
/// written, so a batch in flight still counts.
#[derive(Clone)]
struct Queue {
    frames: mpsc::Sender<Bytes>,
    bytes: Arc<AtomicUsize>,
}

impl Queue {
    fn new() -> (Self, mpsc::Receiver<Bytes>) {
        let (frames, receiver) = mpsc::channel(QUEUE_FRAMES);
        let bytes = Arc::default();
        (Self { frames, bytes }, receiver)
    }

    /// Hand `record` to the writer, or refuse it if either bound is reached.
    fn offer(&self, record: Bytes) -> bool {
        let length = record.len();
        let reserved = self.bytes.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
            (held + length <= QUEUE_BYTES).then_some(held + length)
        });
        if reserved.is_err() {
            return false;
        }
        if self.frames.try_send(record).is_err() {
            self.release(length);
            return false;
        }
        true
    }

    /// The writer wrote `bytes` of what it took.
    fn release(&self, bytes: usize) {
        self.bytes.fetch_sub(bytes, Ordering::Relaxed);
    }
}

/// The table every port routes with, republished on plug and unplug.
#[derive(Default)]
struct Published {
    version: AtomicU64,
    table: RwLock<Arc<Table<Queue>>>,
}

impl Published {
    fn publish(&self, table: &Table<Queue>) {
        *self.table.write().unwrap() = Arc::new(table.clone());
        // After the table: a reader seeing the new version reads a table at
        // least that new.
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    fn refresh(&self, cached: &mut (u64, Arc<Table<Queue>>)) {
        let version = self.version.load(Ordering::Relaxed);
        if version != cached.0 {
            *cached = (version, Arc::clone(&self.table.read().unwrap()));
        }
    }
}

#[derive(Default)]
struct Inbound {
    frames: u64,
    bytes: u64,
    dropped: [u64; DropReason::ALL.len()],
}

#[derive(Default)]
struct Outbound {
    frames: u64,
    bytes: u64,
}

/// Floods allowed in the current one-second window.
struct Storm {
    window: Instant,
    floods: u32,
}

impl Storm {
    fn allow(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.floods = 0;
        }
        self.floods = self.floods.saturating_add(1);
        self.floods <= BROADCASTS_PER_SECOND
    }
}

/// Route every record one cable sends until it ends.
async fn read_port(
    own: Station,
    mut reader: impl AsyncRead + Unpin,
    published: &Published,
    counters: &mut Inbound,
) -> io::Result<()> {
    let mut cached = (u64::MAX, Arc::default());
    let mut buffer = BytesMut::with_capacity(READ_BUFFER_BYTES);
    let mut storm = Storm {
        window: Instant::now(),
        floods: 0,
    };
    loop {
        if buffer.capacity() - buffer.len() < HEADER_BYTES + u16::MAX as usize {
            buffer.reserve(READ_BUFFER_BYTES);
        }
        if reader.read_buf(&mut buffer).await? == 0 {
            return if buffer.is_empty() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "cable ended inside a frame",
                ))
            };
        }
        published.refresh(&mut cached);
        let table = &cached.1;
        while buffer.len() >= HEADER_BYTES {
            let length = usize::from(u16::from_be_bytes([buffer[0], buffer[1]]));
            if length == 0 {
                return Err(invalid("empty cable frame"));
            }
            if buffer.len() < HEADER_BYTES + length {
                break;
            }
            let record = buffer.split_to(HEADER_BYTES + length).freeze();
            counters.frames += 1;
            counters.bytes += record.len() as u64;
            match table.route(&own, &record[HEADER_BYTES..]) {
                Route::Unicast(queue) => {
                    if !queue.offer(record) {
                        counters.dropped[DropReason::QueueFull as usize] += 1;
                    }
                }
                Route::Flood if storm.allow() => {
                    for queue in table.others(&own.mac) {
                        if !queue.offer(record.clone()) {
                            counters.dropped[DropReason::QueueFull as usize] += 1;
                        }
                    }
                }
                Route::Flood => counters.dropped[DropReason::Storm as usize] += 1,
                Route::Drop(reason) => counters.dropped[reason as usize] += 1,
            }
        }
    }
}

/// Write queued records to one cable, as many per write as are waiting; a
/// batch's bytes leave the queue's count only once they are written.
async fn write_port(
    mut writer: impl AsyncWrite + Unpin,
    queue: &Queue,
    mut frames: mpsc::Receiver<Bytes>,
    counters: &mut Outbound,
) -> io::Result<()> {
    let mut batch = Vec::with_capacity(QUEUE_FRAMES);
    while frames.recv_many(&mut batch, QUEUE_FRAMES).await > 0 {
        let count = batch.len() as u64;
        let bytes: usize = batch.iter().map(Bytes::len).sum();
        write_all_vectored(&mut writer, &mut batch).await?;
        queue.release(bytes);
        counters.frames += count;
        counters.bytes += bytes as u64;
    }
    Ok(())
}

async fn write_all_vectored(writer: &mut (impl AsyncWrite + Unpin), batch: &mut Vec<Bytes>) -> io::Result<()> {
    let mut first = 0;
    while first < batch.len() {
        let mut written = {
            let mut slices = [IoSlice::new(&[]); QUEUE_FRAMES];
            let count = (batch.len() - first).min(QUEUE_FRAMES);
            for (slice, record) in slices.iter_mut().zip(&batch[first..first + count]) {
                *slice = IoSlice::new(record);
            }
            writer.write_vectored(&slices[..count]).await?
        };
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        while written > 0 {
            let record = &mut batch[first];
            let taken = written.min(record.len());
            record.advance(taken);
            written -= taken;
            if record.is_empty() {
                first += 1;
            }
        }
    }
    batch.clear();
    Ok(())
}

struct Port {
    mac: Mac,
    /// Taken once the port is unplugged; the entry stays, holding its slot,
    /// until the job has closed the cable.
    stop: Option<oneshot::Sender<()>>,
}

struct Ports {
    limit: usize,
    table: Table<Queue>,
    published: Arc<Published>,
    /// Every port whose job has not ended, unplugged or not.
    ports: HashMap<u64, Port>,
    /// The port currently plugged in for each MAC.
    plugged: HashMap<Mac, u64>,
    /// The newest generation each MAC was plugged with.
    generations: HashMap<Mac, u32>,
}

impl Ports {
    fn unplug(&mut self, port: u64) {
        let Some(entry) = self.ports.get_mut(&port) else { return };
        if let Some(stop) = entry.stop.take() {
            let _ = stop.send(());
        }
        let mac = entry.mac;
        self.forget(port, mac);
    }

    /// Stop routing to `port`, if it is still the one plugged in for `mac`.
    fn forget(&mut self, port: u64, mac: Mac) {
        if self.plugged.get(&mac) == Some(&port) {
            self.plugged.remove(&mac);
            self.table.unplug(&mac);
            self.published.publish(&self.table);
        }
    }
}

pub async fn run(grants: Receiver, mut events: UnixStream, port_limit: usize) -> io::Result<()> {
    let mut state = Ports {
        limit: port_limit,
        table: Table::default(),
        published: Arc::default(),
        ports: HashMap::new(),
        plugged: HashMap::new(),
        generations: HashMap::new(),
    };
    let mut jobs = tokio::task::JoinSet::new();
    let mut readers = tokio::task::JoinSet::new();
    let (queue, mut messages) = mpsc::channel(16);
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
        loop {
            tokio::select! {
                message = messages.recv() => match message.ok_or_else(|| invalid("switch grant channel closed"))?? {
                    Grant::Plug { port, socket } => {
                        let (generation, address) = (port_generation(port), port_address(port));
                        let mac = mac_of(address);
                        if state.generations.get(&mac).is_some_and(|newest| generation <= *newest) {
                            tracing::debug!(port, %address, generation, "stale switch port generation");
                            Event::Refused(port).write(&mut events).await?;
                            continue;
                        }
                        let replaced = state.plugged.get(&mac).copied();
                        if state.ports.len() - usize::from(replaced.is_some()) >= state.limit {
                            tracing::debug!(port, %address, "switch port limit reached");
                            Event::Refused(port).write(&mut events).await?;
                            continue;
                        }
                        let stream = match adopt(socket) {
                            Ok(stream) => stream,
                            Err(error) => {
                                tracing::debug!(port, %address, %error, "switch rejected cable descriptor");
                                Event::Refused(port).write(&mut events).await?;
                                continue;
                            }
                        };
                        if let Some(replaced) = replaced {
                            state.unplug(replaced);
                        }
                        let (queue, receiver) = Queue::new();
                        let (stop, stopped) = oneshot::channel();
                        state.generations.insert(mac, generation);
                        state.ports.insert(port, Port { mac, stop: Some(stop) });
                        state.plugged.insert(mac, port);
                        state.table.plug(mac, queue.clone());
                        state.published.publish(&state.table);
                        let published = Arc::clone(&state.published);
                        jobs.spawn(async move {
                            let mut stream = stream;
                            let (mut inbound, mut outbound) = (Inbound::default(), Outbound::default());
                            let (reader, writer) = stream.split();
                            let outcome = tokio::select! {
                                biased;
                                _ = stopped => Ok(false),
                                result = read_port(Station { mac, address: address.octets() }, reader, &published, &mut inbound) => result.map(|()| true),
                                result = write_port(writer, &queue, receiver, &mut outbound) => result.map(|()| true),
                            };
                            // Both halves are gone with the select; closing the
                            // cable is the last thing before the report.
                            drop(stream);
                            let reason = match &outcome {
                                Ok(false) => CloseReason::Cancelled,
                                Ok(true) => CloseReason::Complete,
                                Err(_) => CloseReason::Io,
                            };
                            let report = PortReport {
                                reason,
                                frames_in: inbound.frames,
                                bytes_in: inbound.bytes,
                                frames_out: outbound.frames,
                                bytes_out: outbound.bytes,
                                dropped: inbound.dropped,
                            };
                            let dropped: Vec<String> = DropReason::ALL.iter().zip(report.dropped).filter(|(_, count)| *count > 0)
                                .map(|(reason, count)| format!("{}={count}", reason.name())).collect();
                            tracing::info!(port, %address, ?reason, frames_in = report.frames_in, bytes_in = report.bytes_in,
                                frames_out = report.frames_out, bytes_out = report.bytes_out, dropped = dropped.join(","),
                                error = ?outcome.err(), "switch port closed");
                            (port, report)
                        });
                        // Reported only once the port routes: the owner may send
                        // the moment it reads this.
                        Event::Accepted(port).write(&mut events).await?;
                    }
                    Grant::Unplug { port } => state.unplug(port),
                    Grant::Hello => return Err(invalid("duplicate switch hello")),
                    Grant::Connected { id, .. } | Grant::Abort { id } => {
                        tracing::debug!(connection_id = id, "relay grant to a switch");
                        Event::Refused(id).write(&mut events).await?;
                    }
                },
                completed = jobs.join_next(), if !jobs.is_empty() => match completed.unwrap() {
                    Ok((port, report)) => {
                        if let Some(entry) = state.ports.remove(&port) {
                            state.forget(port, entry.mac);
                        }
                        Event::PortClosed(port, report).write(&mut events).await?;
                    },
                    Err(error) => return Err(io::Error::other(error)),
                }
            }
        }
    }
    .await;
    readers.shutdown().await;
    jobs.shutdown().await;
    result
}

#[cfg(test)]
mod tests;

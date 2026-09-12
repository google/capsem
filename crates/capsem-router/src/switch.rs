//! One network's switch: every linked member's frames, read, judged and
//! written to exactly one other member.
//!
//! Each link is one stream of `u16`-length ethernet frames (the guest pump's
//! codec) with a reader task and a writer task. The reader hands frames to
//! the destination's writer through a bounded queue and never waits on it:
//! a member that stops reading loses frames addressed to it, not everyone
//! else's link. The verdict is [`capsem_network::switch::classify`]; the
//! member table here is only address to writer. Descriptors are closed when
//! a link ends, never shut down (S04-018).
use super::*;
use bytes::{Bytes, BytesMut};
use capsem_network::frames::{HEADER_BYTES, MAX_FRAME_BYTES};
use capsem_network::switch::{classify, DropReason, Verdict};
use std::net::Ipv4Addr;
use std::sync::RwLock;
use tokio::sync::mpsc;

/// Frames a member's writer holds before its sender starts losing frames
/// to it; each is at most a full frame, so this bounds one link's memory.
pub const QUEUE_FRAMES: usize = 64;

type Table = Arc<RwLock<HashMap<Ipv4Addr, mpsc::Sender<Bytes>>>>;

struct Counters {
    forwarded: u64,
    delivered: Arc<std::sync::atomic::AtomicU64>,
    queue_full: u64,
    dropped: [u64; DropReason::ALL.len()],
}

/// Read one member's frames until the stream ends; returns what it moved.
async fn read_link(
    own: Ipv4Addr,
    mut reader: tokio::net::unix::OwnedReadHalf,
    table: Table,
    counters: &mut Counters,
) -> io::Result<()> {
    let mut buffer = BytesMut::with_capacity(HEADER_BYTES + MAX_FRAME_BYTES);
    loop {
        buffer.clear();
        buffer.resize(HEADER_BYTES, 0);
        match reader.read_exact(&mut buffer[..HEADER_BYTES]).await {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
        let length = usize::from(u16::from_be_bytes([buffer[0], buffer[1]]));
        if length == 0 {
            return Err(invalid("empty link frame"));
        }
        buffer.resize(HEADER_BYTES + length, 0);
        reader.read_exact(&mut buffer[HEADER_BYTES..]).await?;
        enum Fate {
            Forward(mpsc::Sender<Bytes>),
            Reply(Vec<u8>),
            Drop(DropReason),
        }
        let fate = {
            let table = table.read().unwrap();
            match classify(own, |address| table.contains_key(&address), &buffer[HEADER_BYTES..]) {
                // The destination cannot leave between the verdict and the
                // lookup: both happen under the one read lock.
                Verdict::Forward(destination) => Fate::Forward(table[&destination].clone()),
                Verdict::Reply(frame) => Fate::Reply(frame),
                Verdict::Drop(reason) => Fate::Drop(reason),
            }
        };
        match fate {
            Fate::Forward(writer) => match writer.try_send(buffer.split().freeze()) {
                Ok(()) => counters.forwarded += 1,
                Err(_) => counters.queue_full += 1,
            },
            Fate::Reply(frame) => {
                let mut record = BytesMut::with_capacity(HEADER_BYTES + frame.len());
                record.extend_from_slice(&(frame.len() as u16).to_be_bytes());
                record.extend_from_slice(&frame);
                let own_writer = table.read().unwrap().get(&own).cloned();
                if let Some(writer) = own_writer {
                    if writer.try_send(record.freeze()).is_err() {
                        counters.queue_full += 1;
                    }
                }
            }
            Fate::Drop(reason) => counters.dropped[reason as usize] += 1,
        }
    }
}

/// Write queued frames to one member until its queue closes.
async fn write_link(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    mut queue: mpsc::Receiver<Bytes>,
    delivered: Arc<std::sync::atomic::AtomicU64>,
) {
    while let Some(record) = queue.recv().await {
        if writer.write_all(&record).await.is_err() {
            break;
        }
        delivered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    // Closing, never shutting down: the owner ends the guest's connection.
    writer.forget();
}

struct Link {
    address: Ipv4Addr,
    stop: tokio::sync::oneshot::Sender<()>,
}

pub async fn run(grants: Receiver, mut events: UnixStream, link_limit: usize) -> io::Result<()> {
    let table: Table = Arc::default();
    let mut links: HashMap<u64, Link> = HashMap::new();
    let mut by_address: HashMap<Ipv4Addr, u64> = HashMap::new();
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
    let unlink = |id: u64, links: &mut HashMap<u64, Link>, by_address: &mut HashMap<Ipv4Addr, u64>| {
        if let Some(link) = links.remove(&id) {
            if by_address.get(&link.address) == Some(&id) {
                by_address.remove(&link.address);
                table.write().unwrap().remove(&link.address);
            }
            let _ = link.stop.send(());
        }
    };
    let result = async {
        Event::Ready.write(&mut events).await?;
        let mut last_id = 0;
        loop {
            tokio::select! {
                message = messages.recv() => match message.ok_or_else(|| invalid("switch grant channel closed"))?? {
                    Grant::Link { id, socket } => {
                        if id <= last_id { return Err(invalid("reused switch link id")); }
                        last_id = id;
                        let address = link_address(id);
                        if let Some(previous) = by_address.get(&address).copied() {
                            unlink(previous, &mut links, &mut by_address);
                        }
                        if links.len() >= link_limit {
                            tracing::debug!(link_id = id, %address, "switch link quota exhausted");
                            Event::Refused(id).write(&mut events).await?;
                            continue;
                        }
                        let stream = match adopt(socket) {
                            Ok(stream) => stream,
                            Err(error) => {
                                tracing::debug!(link_id = id, %address, %error, "switch rejected link descriptor");
                                Event::Refused(id).write(&mut events).await?;
                                continue;
                            }
                        };
                        Event::Accepted(id).write(&mut events).await?;
                        let (reader, writer) = stream.into_split();
                        let (sender, receiver) = mpsc::channel(QUEUE_FRAMES);
                        let delivered = Arc::new(std::sync::atomic::AtomicU64::new(0));
                        tokio::spawn(write_link(writer, receiver, Arc::clone(&delivered)));
                        table.write().unwrap().insert(address, sender);
                        by_address.insert(address, id);
                        let (stop, stopped) = tokio::sync::oneshot::channel();
                        links.insert(id, Link { address, stop });
                        let table = Arc::clone(&table);
                        jobs.spawn(async move {
                            let mut counters = Counters { forwarded: 0, delivered, queue_full: 0, dropped: [0; DropReason::ALL.len()] };
                            let outcome = tokio::select! {
                                biased;
                                _ = stopped => None,
                                result = read_link(address, reader, table, &mut counters) => Some(result),
                            };
                            let reason = match &outcome {
                                None => CloseReason::Cancelled,
                                Some(Ok(())) => CloseReason::Complete,
                                Some(Err(_)) => CloseReason::Io,
                            };
                            let dropped: Vec<String> = DropReason::ALL.iter().zip(counters.dropped).filter(|(_, count)| *count > 0)
                                .map(|(reason, count)| format!("{}={count}", reason.name())).collect();
                            tracing::info!(link_id = id, %address, ?reason, forwarded = counters.forwarded,
                                delivered = counters.delivered.load(std::sync::atomic::Ordering::Relaxed),
                                queue_full = counters.queue_full, dropped = dropped.join(","),
                                error = ?outcome.and_then(Result::err), "switch link ended");
                            (id, CloseReport { reason, from_source: counters.forwarded,
                                to_source: counters.delivered.load(std::sync::atomic::Ordering::Relaxed) })
                        });
                    }
                    Grant::Abort { id } => unlink(id, &mut links, &mut by_address),
                    Grant::Hello => return Err(invalid("duplicate switch hello")),
                    Grant::Connected { id, .. } => {
                        tracing::debug!(connection_id = id, "pair grant to a switch");
                        Event::Refused(id).write(&mut events).await?;
                    }
                },
                completed = jobs.join_next(), if !jobs.is_empty() => match completed.unwrap() {
                    Ok((id, report)) => {
                        unlink(id, &mut links, &mut by_address);
                        Event::Closed(id, report).write(&mut events).await?;
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

//! Multiplexed DNS sessions to the host over vsock.
//!
//! One session is one persistent vsock connection carrying many queries at
//! once: each `DnsRequest` carries a correlation id and the host echoes it
//! in the `DnsResponse`, so answers may arrive in any order. This replaced
//! a pool of eight threads that each blocked on one query at a time: the
//! pool capped the whole guest at `8 / round_trip` queries per second, and
//! a slow lookup on one worker held every query queued behind it.
//!
//! Before an answer is delivered its transaction id and question are
//! checked against the query that owns the correlation id. A frame that
//! does not answer its own question is discarded and the query keeps
//! waiting: a wrong answer is never handed to a client.

use std::collections::HashMap;
use std::io;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use super::vsock_io::AsyncVsock;
use super::wire::{parse_question, Question};
use capsem_proto::{decode_dns_response, encode_dns_request, DnsRequest, DnsResponse, MAX_FRAME_SIZE};

/// Sessions per forwarder. Two keep the guest resolving while one
/// reconnects, and give the host two independent frame streams.
pub(crate) const DNS_SESSIONS: usize = 2;
/// Queries one session holds in flight before it sheds. Past this the
/// client gets SERVFAIL at once rather than an unbounded queue.
pub(crate) const DNS_SESSION_MAX_IN_FLIGHT: usize = 128;
/// Covers both two-second upstream attempts and the local queue/ledger
/// overhead. Guest libc waits six seconds, so it does not race this reply.
pub(crate) const DNS_QUERY_DEADLINE: Duration = Duration::from_secs(5);
/// Frames queued for the writer before `forward_query` waits.
const WRITER_QUEUE: usize = 256;
const RECONNECT_MIN: Duration = Duration::from_millis(50);
const RECONNECT_MAX: Duration = Duration::from_secs(1);

/// How the session reaches the host. A function so tests can hand it one
/// end of a socket pair.
pub(crate) type Connect = Arc<dyn Fn() -> io::Result<RawFd> + Send + Sync>;

pub(crate) struct DnsForwarder {
    sessions: Vec<Arc<DnsSession>>,
    workers: Vec<tokio::task::JoinHandle<()>>,
    next: AtomicUsize,
}

impl DnsForwarder {
    pub(crate) fn new(session_count: usize, connect: Connect) -> Self {
        assert!(session_count > 0, "DNS forwarder needs at least one session");
        let (sessions, workers) = (0..session_count)
            .map(|index| DnsSession::spawn(index, Arc::clone(&connect)))
            .unzip();
        Self {
            sessions,
            workers,
            next: AtomicUsize::new(0),
        }
    }

    /// Forward one query and wait for its answer, at most
    /// `DNS_QUERY_DEADLINE`. Errors mean "no answer": the caller turns
    /// them into a SERVFAIL for the client.
    pub(crate) async fn forward_query(&self, raw: Vec<u8>, proto: &'static str) -> io::Result<DnsResponse> {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.sessions.len();
        self.sessions[index].forward_query(raw, proto).await
    }
}

impl Drop for DnsForwarder {
    fn drop(&mut self) {
        for worker in &self.workers {
            worker.abort();
        }
    }
}

struct Pending {
    /// `None` when the query itself did not parse; the host answers those
    /// with empty bytes and nothing can be checked.
    expected: Option<Question>,
    reply: oneshot::Sender<io::Result<DnsResponse>>,
}

pub(crate) struct DnsSession {
    index: usize,
    frames: mpsc::Sender<(u32, Vec<u8>)>,
    pending: Mutex<HashMap<u32, Pending>>,
    next_id: AtomicU32,
}

/// Cancellation owns the same cleanup as a reply, error, or timeout.
struct PendingQuery<'a> {
    session: &'a DnsSession,
    id: u32,
}

impl Drop for PendingQuery<'_> {
    fn drop(&mut self) {
        self.session.take_pending(self.id);
    }
}

impl DnsSession {
    fn spawn(index: usize, connect: Connect) -> (Arc<Self>, tokio::task::JoinHandle<()>) {
        let (frames, frames_rx) = mpsc::channel(WRITER_QUEUE);
        let session = Arc::new(Self {
            index,
            frames,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU32::new(1),
        });
        let worker = tokio::spawn(Arc::clone(&session).run(frames_rx, connect));
        (session, worker)
    }

    fn alloc_id(&self) -> u32 {
        loop {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                return id;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    async fn forward_query(&self, raw: Vec<u8>, proto: &'static str) -> io::Result<DnsResponse> {
        self.forward_query_within(raw, proto, DNS_QUERY_DEADLINE).await
    }

    async fn forward_query_within(
        &self,
        raw: Vec<u8>,
        proto: &'static str,
        deadline: Duration,
    ) -> io::Result<DnsResponse> {
        let expected = parse_question(&raw);
        let id = self.alloc_id();
        let (reply, answer) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            if pending.len() >= DNS_SESSION_MAX_IN_FLIGHT {
                return Err(io::Error::other(format!(
                    "dns session {} has {} queries in flight; shedding",
                    self.index,
                    pending.len()
                )));
            }
            pending.insert(id, Pending { expected, reply });
        }
        let _pending = PendingQuery { session: self, id };
        let request = DnsRequest {
            id,
            raw,
            proto: proto.to_string(),
            process_name: None,
        };
        let frame =
            encode_dns_request(&request).map_err(|error| io::Error::other(format!("encode_dns_request: {error:#}")))?;
        tokio::time::timeout(deadline, async {
            self.frames
                .send((id, frame))
                .await
                .map_err(|_| io::Error::other("dns session writer stopped"))?;
            answer
                .await
                .map_err(|_| io::Error::other("dns session dropped the query"))?
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "dns query deadline passed"))?
    }

    fn take_pending(&self, id: u32) -> Option<Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id)
    }

    /// Route one decoded frame to the query that owns its id, after the
    /// answer has been checked against that query's question.
    pub(crate) fn deliver(&self, payload: &[u8]) {
        let response = match decode_dns_response(payload) {
            Ok(response) => response,
            Err(error) => {
                eprintln!(
                    "[capsem-dns-proxy] session {}: undecodable response frame: {error:#}",
                    self.index
                );
                return;
            }
        };
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        // Pre-multiplexing hosts omit the correlation id. Preserve their
        // DNS wire identity checks, and never guess between duplicate queries.
        let id = if response.id == 0 {
            let mut matches = pending.iter().filter(|(_, entry)| {
                entry
                    .expected
                    .as_ref()
                    .is_some_and(|question| question.is_answered_by(&response.raw))
            });
            let Some((&id, _)) = matches.next() else {
                return;
            };
            if matches.next().is_some() {
                return;
            }
            id
        } else {
            response.id
        };
        let Some(entry) = pending.get(&id) else {
            eprintln!(
                "[capsem-dns-proxy] session {}: answer for unknown query id {}; discarded",
                self.index, response.id
            );
            return;
        };
        if !response.raw.is_empty()
            && !entry
                .expected
                .as_ref()
                .is_some_and(|question| question.is_answered_by(&response.raw))
        {
            eprintln!(
                "[capsem-dns-proxy] session {}: answer for id {} does not match its question; discarded",
                self.index, response.id
            );
            return;
        }
        if let Some(entry) = pending.remove(&id) {
            drop(pending);
            let _ = entry.reply.send(Ok(response));
        }
    }

    /// Answer every waiting query with `reason`; the connection is gone.
    fn fail_all(&self, reason: &str) {
        let drained: Vec<Pending> = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, entry)| entry)
            .collect();
        if !drained.is_empty() {
            eprintln!(
                "[capsem-dns-proxy] session {}: failing {} in-flight queries: {reason}",
                self.index,
                drained.len()
            );
        }
        for entry in drained {
            let _ = entry.reply.send(Err(io::Error::other(reason.to_string())));
        }
    }

    async fn run(self: Arc<Self>, mut frames: mpsc::Receiver<(u32, Vec<u8>)>, connect: Connect) {
        let mut backoff = RECONNECT_MIN;
        loop {
            let connect_once = Arc::clone(&connect);
            let fd = match tokio::task::spawn_blocking(move || {
                // If the worker is aborted while connecting, the eventual
                // result still owns and closes the descriptor.
                connect_once().map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
            })
            .await
            {
                Ok(Ok(fd)) => fd,
                Ok(Err(error)) => {
                    eprintln!(
                        "[capsem-dns-proxy] session {}: vsock connect failed: {error}",
                        self.index
                    );
                    self.fail_all("host DNS connection unavailable");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RECONNECT_MAX);
                    continue;
                }
                Err(error) => {
                    eprintln!(
                        "[capsem-dns-proxy] session {}: connect task failed: {error}",
                        self.index
                    );
                    self.fail_all("host DNS connection unavailable");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RECONNECT_MAX);
                    continue;
                }
            };
            let stream = match AsyncVsock::new(fd.into_raw_fd()) {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!(
                        "[capsem-dns-proxy] session {}: cannot register vsock fd: {error}",
                        self.index
                    );
                    self.fail_all("host DNS connection unavailable");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RECONNECT_MAX);
                    continue;
                }
            };
            backoff = RECONNECT_MIN;
            let outcome = self.pump(stream, &mut frames).await;
            self.fail_all(&format!("host DNS connection ended: {outcome}"));
            if frames.is_closed() {
                return;
            }
            tokio::time::sleep(RECONNECT_MIN).await;
        }
    }

    /// Write queued frames and deliver answers until either direction
    /// fails. Returns the error that ended the connection.
    async fn pump(&self, stream: AsyncVsock, frames: &mut mpsc::Receiver<(u32, Vec<u8>)>) -> io::Error {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let write_side = async {
            while let Some((id, frame)) = frames.recv().await {
                if !self.pending.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&id) {
                    continue;
                }
                writer.write_all(&frame).await?;
            }
            Err::<(), io::Error>(io::Error::other("forwarder dropped"))
        };
        let read_side = async {
            loop {
                let mut len_buf = [0u8; 4];
                reader.read_exact(&mut len_buf).await?;
                let len = u32::from_be_bytes(len_buf);
                if len > MAX_FRAME_SIZE {
                    return Err::<(), io::Error>(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("dns response frame too large ({len} > {MAX_FRAME_SIZE})"),
                    ));
                }
                let mut payload = vec![0u8; len as usize];
                reader.read_exact(&mut payload).await?;
                self.deliver(&payload);
            }
        };
        tokio::select! {
            result = write_side => result.unwrap_err(),
            result = read_side => result.unwrap_err(),
        }
    }
}

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;

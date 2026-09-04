//! Multiplexed DNS query sessions over the vsock DNS port.
//!
//! Wire shape:
//!   guest -> host: `[u32 BE length][rmp DnsRequest]`
//!   host -> guest: `[u32 BE length][rmp DnsResponse]`
//!
//! One connection carries many queries at once. Each request names a
//! correlation id; its answer echoes it, so the guest can have hundreds of
//! queries in flight on one connection and take the answers in whatever
//! order the upstreams produce them. The previous shape was lock-step (one
//! query per connection at a time, read on one blocking-pool thread and
//! written on another), which capped the whole guest at eight queries in
//! flight and let one slow lookup hold everything queued behind it.
//!
//! What stays on the reply path: the security decision (inside
//! `DnsHandler::handle`) and the `dns_events` ledger row, which is accepted
//! by the writer before the answer is sent. The rule-ledger rows derived from
//! that event are written on their own task.

use std::io::{Read as _, Write as _};
use std::os::fd::AsFd as _;
use std::sync::Arc;

use anyhow::{Context, Result};
use capsem_core::VsockConnection;
use tokio::io::unix::AsyncFd;
use tokio::sync::{mpsc, Semaphore};
use tracing::{debug, warn};

use crate::helpers::clone_fd;

/// Queries one connection answers concurrently. Matches the guest's
/// per-session in-flight cap; past it the reader simply waits.
pub(super) const DNS_SESSION_MAX_IN_FLIGHT: usize = 128;

type SecurityRulesHandle = Arc<std::sync::RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>;

pub(super) async fn serve_dns_session(
    conn: VsockConnection,
    handler: Arc<capsem_core::net::dns::DnsHandler>,
    db: Arc<capsem_logger::DbWriter>,
    security_rules: SecurityRulesHandle,
) {
    let Some(read_file) = clone_fd(&conn, "duplicate-dns-vsock-reader") else {
        return;
    };
    let Some(write_file) = clone_fd(&conn, "duplicate-dns-vsock-writer") else {
        return;
    };
    let (reader, writer) = match (async_side(read_file), async_side(write_file)) {
        (Ok(reader), Ok(writer)) => (reader, writer),
        (Err(error), _) | (_, Err(error)) => {
            warn!(error = %error, "DNS port: cannot register the session descriptor");
            return;
        }
    };

    let (frames, mut frames_rx) = mpsc::channel::<Vec<u8>>(DNS_SESSION_MAX_IN_FLIGHT);
    let write_task = tokio::spawn(async move {
        while let Some(frame) = frames_rx.recv().await {
            if let Err(error) = write_all(&writer, &frame).await {
                warn!(error = %error, "DNS port: write failed");
                return;
            }
        }
    });
    let in_flight = Arc::new(Semaphore::new(DNS_SESSION_MAX_IN_FLIGHT));

    loop {
        let payload = match read_frame(&reader).await {
            Ok(Some(payload)) => payload,
            Ok(None) => break,
            Err(error) => {
                warn!(error = %error, "DNS port: read failed");
                break;
            }
        };
        let request = match capsem_proto::decode_dns_request(&payload) {
            Ok(request) => request,
            Err(error) => {
                warn!(error = %error, "DNS port: decode_dns_request failed");
                break;
            }
        };
        let Ok(permit) = Arc::clone(&in_flight).acquire_owned().await else {
            break;
        };
        let frames = frames.clone();
        let handler = Arc::clone(&handler);
        let db = Arc::clone(&db);
        let security_rules = Arc::clone(&security_rules);
        tokio::spawn(async move {
            let frame = answer_one(request, &handler, &db, &security_rules).await;
            drop(permit);
            if let Some(frame) = frame {
                // channel-closed-ok: the writer is gone with the connection.
                let _ = frames.send(frame).await;
            }
        });
    }

    drop(frames);
    // Queries already accepted still get their answers written; the writer
    // ends when the last of them has sent its frame.
    let _ = write_task.await;
    drop(conn);
}

/// Evaluate one query, record it, and frame its answer under the request's id.
async fn answer_one(
    request: capsem_proto::DnsRequest,
    handler: &capsem_core::net::dns::DnsHandler,
    db: &Arc<capsem_logger::DbWriter>,
    security_rules: &SecurityRulesHandle,
) -> Option<Vec<u8>> {
    let result = handler.handle(&request.raw).await;

    // T3.3 -- one `dns_events` row per query. trace_id ties it back to the
    // agent action; source_proto distinguishes UDP from TCP DNS at the
    // source. The security emitter is awaited so the audit row is accepted
    // by the single security/logging rail before the answer is returned.
    let event = capsem_core::net::dns::build_dns_event(
        &result,
        Some(request.proto.as_str()),
        request.process_name.clone(),
        capsem_foundation::telemetry::ambient_capsem_trace_id(),
    );
    // The delegated rule rows are not awaited: they are derived from the
    // event row already accepted above and their join handle is dropped.
    let _ = emit_dns_security_write_and_rules(db, security_rules, event).await;

    let response = capsem_proto::DnsResponse {
        id: request.id,
        raw: result.answer_bytes,
        decision: result.decision.as_str().to_string(),
        rcode: result.rcode,
    };
    match capsem_proto::encode_dns_response(&response) {
        Ok(frame) => Some(frame),
        Err(error) => {
            warn!(error = %error, id = request.id, "DNS port: encode_dns_response failed");
            None
        }
    }
}

/// Record the DNS event row (awaited) and write its derived rule-ledger
/// rows on their own task, off the reply path. Returns the event id and the
/// handle of the delegated write, so a caller that needs the rule rows to
/// have landed (a test flushing the ledger) can wait for it.
pub(super) async fn emit_dns_security_write_and_rules(
    db: &Arc<capsem_logger::DbWriter>,
    security_rules: &SecurityRulesHandle,
    event: capsem_logger::DnsEvent,
) -> Option<(
    capsem_core::security_engine::SecurityEventId,
    tokio::task::JoinHandle<()>,
)> {
    let security_event = capsem_core::net::dns::security_event_from_dns_event(&event);
    let event_id =
        capsem_core::security_engine::emit_security_write(db, capsem_logger::WriteOp::DnsEvent(event)).await?;
    let rules = security_rules.read().unwrap().clone();
    let db = Arc::clone(db);
    let delegated_id = event_id.clone();
    let delegated = tokio::spawn(async move {
        if let Err(error) = capsem_core::security_engine::emit_matching_security_rules(
            &db,
            delegated_id,
            capsem_core::security_engine::RuntimeSecurityEventType::DnsQuery,
            &rules,
            &security_event,
            current_unix_ms(),
        )
        .await
        {
            warn!(error = %error, "failed to emit DNS security rule ledger rows");
        }
    });
    Some((event_id, delegated))
}

fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// One duplicated descriptor of the connection, registered with the
/// reactor. Reads and writes go through `try_io`, so a `WouldBlock` clears
/// readiness and the task parks instead of spinning or blocking a thread.
fn async_side(file: std::fs::File) -> std::io::Result<AsyncFd<std::fs::File>> {
    capsem_foundation::unix::fd::set_nonblocking(file.as_fd(), true)?;
    AsyncFd::new(file)
}

async fn read_exact(fd: &AsyncFd<std::fs::File>, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let mut guard = fd.readable().await?;
        match guard.try_io(|inner| inner.get_ref().read(&mut buf[filled..])) {
            Ok(Ok(0)) => return Ok(filled),
            Ok(Ok(n)) => filled += n,
            Ok(Err(error)) => return Err(error),
            Err(_would_block) => continue,
        }
    }
    Ok(filled)
}

/// One length-prefixed frame; `None` at a clean end of stream.
async fn read_frame(fd: &AsyncFd<std::fs::File>) -> Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match read_exact(fd, &mut len_buf).await.context("DNS port: read length")? {
        0 => return Ok(None),
        4 => {}
        short => anyhow::bail!("DNS port: length prefix cut short after {short} bytes"),
    }
    let len = u32::from_be_bytes(len_buf);
    if len > capsem_proto::MAX_FRAME_SIZE {
        anyhow::bail!("DNS port: frame too large ({len} > MAX_FRAME_SIZE)");
    }
    let mut payload = vec![0u8; len as usize];
    let filled = read_exact(fd, &mut payload).await.context("DNS port: read payload")?;
    if filled != payload.len() {
        anyhow::bail!("DNS port: frame cut short ({filled} of {} bytes)", payload.len());
    }
    debug!(len, "DNS port: frame received");
    Ok(Some(payload))
}

async fn write_all(fd: &AsyncFd<std::fs::File>, mut data: &[u8]) -> std::io::Result<()> {
    while !data.is_empty() {
        let mut guard = fd.writable().await?;
        match guard.try_io(|inner| inner.get_ref().write(data)) {
            Ok(Ok(0)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "DNS port: wrote nothing",
                ))
            }
            Ok(Ok(n)) => data = &data[n..],
            Ok(Err(error)) => return Err(error),
            Err(_would_block) => continue,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

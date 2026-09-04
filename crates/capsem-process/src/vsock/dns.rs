//! Persistent DNS query sessions over the vsock DNS port.

use anyhow::{Context, Result};
use capsem_core::VsockConnection;
use std::sync::Arc;
use tracing::warn;

use super::read_bounded_frame;
use crate::helpers::clone_fd;

/// Persistent DNS query handler over the vsock DNS port (T3.2).
///
/// Wire shape:
///   guest -> host: `[u32 BE length][rmp DnsRequest]`
///   host -> guest: `[u32 BE length][rmp DnsResponse]`
///
/// Each connection carries many serialized request/response frames.
/// The guest-side worker pool owns concurrency: one in-flight DNS query
/// per persistent vsock fd. This removes per-query connection churn
/// without introducing response multiplexing ambiguity.
pub(super) async fn serve_dns_session(
    conn: VsockConnection,
    handler: Arc<capsem_core::net::dns::DnsHandler>,
    db: Arc<capsem_logger::DbWriter>,
    security_rules: Arc<std::sync::RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>,
) {
    use std::io::Write as _;

    loop {
        // Move the fd in/out via spawn_blocking so we don't run sync I/O on
        // the tokio runtime. The DNS handler itself is async (UDP forwarder
        // returns Future), so we read one request, run the handler, then
        // write one response.
        let Some(mut read_file) = clone_fd(&conn, "duplicate-dns-vsock-reader") else {
            break;
        };
        let read_res = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>> {
            read_bounded_frame(&mut read_file).context("DNS port: failed to read frame")
        })
        .await;

        let payload = match read_res {
            Ok(Ok(Some(p))) => p,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                warn!(error = %e, "DNS port: read failed");
                break;
            }
            Err(e) => {
                warn!(error = %e, "DNS port: read task panicked");
                break;
            }
        };

        let req = match capsem_proto::decode_dns_request(&payload) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "DNS port: decode_dns_request failed");
                break;
            }
        };

        let result = handler.handle(&req.raw).await;

        // T3.3 -- record one `dns_events` row per query. trace_id ties it
        // back to the agent action; source_proto distinguishes UDP from
        // TCP DNS at the source side. Await the security emitter so DNS audit
        // rows are accepted by the single security/logging rail before the
        // DNS response is returned.
        let event = capsem_core::net::dns::build_dns_event(
            &result,
            Some(req.proto.as_str()),
            req.process_name.clone(),
            capsem_foundation::telemetry::ambient_capsem_trace_id(),
        );
        emit_dns_security_write_and_rules(&db, &security_rules, event).await;

        let response = capsem_proto::DnsResponse {
            raw: result.answer_bytes,
            decision: result.decision.as_str().to_string(),
            rcode: result.rcode,
        };

        let frame = match capsem_proto::encode_dns_response(&response) {
            Ok(f) => f,
            Err(e) => {
                warn!(error = %e, "DNS port: encode_dns_response failed");
                break;
            }
        };

        let Some(mut write_file) = clone_fd(&conn, "duplicate-dns-vsock-writer") else {
            break;
        };
        let write_res = tokio::task::spawn_blocking(move || -> Result<()> {
            write_file
                .write_all(&frame)
                .context("DNS port: failed to write response frame")?;
            Ok(())
        })
        .await;
        match write_res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!(error = %e, "DNS port: write failed");
                break;
            }
            Err(e) => {
                warn!(error = %e, "DNS port: write task panicked");
                break;
            }
        }
    }

    drop(conn);
}

pub(super) async fn emit_dns_security_write_and_rules(
    db: &Arc<capsem_logger::DbWriter>,
    security_rules: &Arc<std::sync::RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>,
    event: capsem_logger::DnsEvent,
) -> Option<capsem_core::security_engine::SecurityEventId> {
    let security_event = capsem_core::net::dns::security_event_from_dns_event(&event);
    let event_id =
        capsem_core::security_engine::emit_security_write(db, capsem_logger::WriteOp::DnsEvent(event)).await?;
    let rules = security_rules.read().unwrap().clone();
    if let Err(error) = capsem_core::security_engine::emit_matching_security_rules(
        db,
        event_id.clone(),
        capsem_core::security_engine::RuntimeSecurityEventType::DnsQuery,
        &rules,
        &security_event,
        current_unix_ms(),
    )
    .await
    {
        warn!(error = %error, "failed to emit DNS security rule ledger rows");
    }
    Some(event_id)
}

fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

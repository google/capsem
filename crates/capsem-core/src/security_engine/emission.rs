//! Security audit admission and its shared async/blocking telemetry.
use super::forensics::{logger_write_credential_ref, logger_write_trace_id, trace_runtime_security_event};
use super::{
    RuntimeSecurityEventFamily, RuntimeSecurityEventType, SecurityEventId, SECURITY_EVENT_EMIT_DURATION_MS,
    SECURITY_EVENT_EMIT_SPAN, SECURITY_EVENT_EMIT_TOTAL,
};
use capsem_logger::{DbWriter, WriteOp};
use std::time::Instant;
use tracing::Instrument;

#[derive(Debug, Clone)]
pub struct RuntimeSecurityEvent {
    pub event_id: Option<SecurityEventId>,
    pub event_type: RuntimeSecurityEventType,
    pub event_family: RuntimeSecurityEventFamily,
    pub credential_ref: Option<String>,
    pub trace_id: Option<String>,
    logger_write: WriteOp,
}

impl RuntimeSecurityEvent {
    pub fn from_logger_write(mut logger_write: WriteOp) -> Self {
        let event_id = logger_write
            .ensure_event_id()
            .and_then(|value| SecurityEventId::parse(value).ok());
        let event_type = RuntimeSecurityEventType::for_write_op(&logger_write);
        let event_family = event_type.family();
        let credential_ref = logger_write_credential_ref(&logger_write);
        let trace_id = logger_write_trace_id(&logger_write);
        Self {
            event_id,
            event_type,
            event_family,
            credential_ref,
            trace_id,
            logger_write,
        }
    }

    pub fn into_logger_write(self) -> WriteOp {
        self.logger_write
    }
}

pub async fn emit_security_write(db: &DbWriter, op: WriteOp) -> Option<SecurityEventId> {
    let event = RuntimeSecurityEvent::from_logger_write(op);
    let event_type = event.event_type.as_str();
    let event_family = event.event_family.as_str();
    let span = tracing::debug_span!(
        target: "capsem.security_event",
        SECURITY_EVENT_EMIT_SPAN,
        event_type,
        event_family,
        status = tracing::field::Empty,
        queue_result = tracing::field::Empty,
    );
    let started = Instant::now();
    span.in_scope(|| trace_runtime_security_event(&event));
    let event_id = event.event_id.clone();
    let result = db
        .write_checked(event.into_logger_write())
        .instrument(span.clone())
        .await;
    finish_emission(event_type, event_family, span, started, result, event_id)
}

pub fn emit_security_write_blocking(db: &DbWriter, op: WriteOp) -> Option<SecurityEventId> {
    let event = RuntimeSecurityEvent::from_logger_write(op);
    let event_type = event.event_type.as_str();
    let event_family = event.event_family.as_str();
    let span = tracing::debug_span!(
        target: "capsem.security_event",
        SECURITY_EVENT_EMIT_SPAN,
        event_type,
        event_family,
        status = tracing::field::Empty,
        queue_result = tracing::field::Empty,
    );
    let started = Instant::now();
    span.in_scope(|| trace_runtime_security_event(&event));
    let event_id = event.event_id.clone();
    let result = span.in_scope(|| db.write_blocking_checked(event.into_logger_write()));
    finish_emission(event_type, event_family, span, started, result, event_id)
}

fn finish_emission(
    event_type: &'static str,
    event_family: &'static str,
    span: tracing::Span,
    started: Instant,
    result: Result<(), String>,
    event_id: Option<SecurityEventId>,
) -> Option<SecurityEventId> {
    let (status, queue_result) = if result.is_ok() {
        ("ok", "queued")
    } else {
        ("error", "failed")
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    ::metrics::counter!(SECURITY_EVENT_EMIT_TOTAL,
        "event_type" => event_type,
        "event_family" => event_family,
        "status" => status,
        "queue_result" => queue_result)
    .increment(1);
    ::metrics::histogram!(SECURITY_EVENT_EMIT_DURATION_MS,
        "event_type" => event_type,
        "event_family" => event_family)
    .record(elapsed_ms);
    span.record("status", status);
    span.record("queue_result", queue_result);
    match result {
        Ok(()) => event_id,
        Err(error) => {
            tracing::warn!(parent: &span, error = %error, "security audit event was not accepted");
            None
        }
    }
}

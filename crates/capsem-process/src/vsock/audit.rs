//! The guest's process-audit relay: one framed record stream per VM, each
//! record written to the ledger and judged against the rules current now.

use std::sync::Arc;

use super::read_bounded_frame;

/// Serve one guest audit connection: decode records until the stream ends.
pub(super) fn serve_audit_records(
    file: &mut impl std::io::Read,
    db: &capsem_logger::DbWriter,
    security_rules: &std::sync::RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>,
) {
    while let Ok(Some(payload)) = read_bounded_frame(file) {
        handle_audit_frame(&payload, db, security_rules);
    }
}

/// Write one audit record and evaluate it against the rules current now.
///
/// The rule set is read through the reload handle per record. The guest opens
/// the audit port exactly once at boot and streams for the life of the VM, so
/// a snapshot taken at connect time froze process-audit policy: a profile
/// edit reloaded every other rail while audit rows kept matching the rules
/// from boot until the VM restarted.
pub(super) fn handle_audit_frame(
    payload: &[u8],
    db: &capsem_logger::DbWriter,
    security_rules: &std::sync::RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>,
) {
    let Ok(record) = capsem_proto::decode_audit_record(payload) else {
        return;
    };
    let rules = Arc::clone(&security_rules.read().unwrap_or_else(|e| e.into_inner()));
    let timestamp = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_micros(record.timestamp_us);
    capsem_core::security_engine::emit_process_audit_security_write_and_rules_blocking(
        db,
        &rules,
        capsem_logger::AuditEvent {
            event_id: None,
            timestamp,
            pid: record.pid,
            ppid: record.ppid,
            uid: record.uid,
            exe: record.exe,
            comm: record.comm,
            argv: record.argv,
            cwd: record.cwd,
            tty: record.tty,
            session_id: record.session_id,
            audit_id: Some(record.audit_id),
            exec_event_id: None,
            parent_exe: record.parent_exe,
            trace_id: capsem_foundation::telemetry::ambient_capsem_trace_id(),
            credential_ref: None,
        },
    );
}

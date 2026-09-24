//! The column lists every read is built from.
//!
//! They are here, together and named, for one reason: `ready()` is where a
//! ledger an older build wrote is supposed to be caught, and it can only be
//! that if `READY_SCHEMA_COLUMNS` demands everything a SELECT asks for. A list
//! inlined into a query is a list nothing compares against the gate, so the
//! column it adds fails later and elsewhere -- as SQLite's own complaint
//! about an unknown column, partway through a route, on a file readiness has
//! already called healthy.
//!
//! `reader_select_columns` is what closes that: `reader/tests.rs` walks it and
//! refuses any column the gate does not already require. Adding a column to a
//! read therefore fails here, in the cheapest place, instead of in production.
use super::MODEL_CALL_COLUMNS_TAIL;

/// The net_events column list, in the order `NetEvent` is read from.
///
/// `credential_ref` and `event_id` used to be selected through
/// `optional_column_expr`, which substituted `NULL AS credential_ref` on a
/// ledger that lacked the column: an older file then read as a current one in
/// which nothing was ever brokered. Both are declared in `schema/ddl.rs` and
/// selected outright, so a file that lacks one says so.
pub(super) const NET_EVENT_COLUMNS: &str = "timestamp, domain, port, decision, process_name, pid,
     method, path, query, status_code,
     bytes_sent, bytes_received, duration_ms, matched_rule,
     request_headers, response_headers,
     request_body_preview, response_body_preview, conn_type,
     policy_mode, policy_action, policy_rule, policy_reason,
     trace_id, credential_ref, event_id";

/// The model_calls column list, in the order `read_model_call_row` reads.
///
/// Every column is required. `protocol`, `credential_ref` and `event_id` used
/// to go through `optional_column_expr`, which quietly substituted `NULL AS
/// protocol` on a ledger that lacked the column, so an older file read as a
/// current one with the fields blank.
pub(super) fn model_call_columns() -> String {
    format!("id, timestamp, provider, protocol, {MODEL_CALL_COLUMNS_TAIL}, credential_ref, usage_details, event_id")
}

/// Every column list a reader selects, against the table it selects from.
///
/// `ready()` is supposed to be where a ledger an older build wrote is caught,
/// and it can only be that if the gate knows every column a route will ask
/// for. `reader_select_columns_are_required` walks this and refuses any entry
/// that `READY_SCHEMA_COLUMNS` does not already demand -- so a new column
/// added to a SELECT is a failing test, not an unknown-column error found by
/// whoever runs the route.
///
/// A list that is not here is not covered, which is why the reads build their
/// SQL from these and never inline a list of their own.
#[cfg(test)]
pub(crate) fn reader_select_columns() -> Vec<(&'static str, String)> {
    vec![
        ("net_events", NET_EVENT_COLUMNS.to_string()),
        ("model_calls", model_call_columns()),
        ("tool_calls", TOOL_CALL_COLUMNS.to_string()),
        ("tool_responses", TOOL_RESPONSE_COLUMNS.to_string()),
        ("exec_events", EXEC_EVENT_COLUMNS.to_string()),
        ("exec_events", EXEC_HISTORY_COLUMNS.to_string()),
        ("audit_events", AUDIT_EVENT_COLUMNS.to_string()),
        ("audit_events", AUDIT_HISTORY_COLUMNS.to_string()),
        ("fs_events", crate::reader::file_events::FILE_EVENT_COLUMNS.to_string()),
    ]
}

/// The tool_calls list `tool_calls_for` reads, in its order.
pub(super) const TOOL_CALL_COLUMNS: &str = "call_index, call_id, tool_name, arguments, origin, event_id";

/// The tool_responses list `tool_responses_for` reads, in its order.
pub(super) const TOOL_RESPONSE_COLUMNS: &str = "call_id, content_preview, is_error, credential_ref, event_id";

/// The exec_events list `recent_exec_events` reads, in its order.
pub(super) const EXEC_EVENT_COLUMNS: &str = "timestamp, exec_id, command, source, trace_id, process_name,
     credential_ref, event_id";

/// The exec_events list `read_exec_history_row` reads, in its order.
pub(super) const EXEC_HISTORY_COLUMNS: &str = "timestamp, exec_id, command, exit_code, duration_ms,
     stdout_preview, stderr_preview, source, trace_id, process_name";

/// The audit_events list `recent_audit_events` reads, in its order.
pub(super) const AUDIT_EVENT_COLUMNS: &str = "timestamp, pid, ppid, uid, exe, comm, argv, cwd,
     tty, session_id, audit_id, exec_event_id, parent_exe,
     trace_id, credential_ref, event_id";

/// The audit_events list `read_audit_history_row` reads, in its order.
pub(super) const AUDIT_HISTORY_COLUMNS: &str = "timestamp, pid, ppid, uid, exe, comm, argv, cwd,
     tty, session_id, audit_id, parent_exe, exit_code";

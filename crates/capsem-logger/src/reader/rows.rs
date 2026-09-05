//! Row-to-record readers shared by the ledger queries.

use super::*;

pub(super) fn read_security_rule_event_row(row: &Row<'_>) -> rusqlite::Result<SecurityRuleEvent> {
    let rule_action: String = row.get(4)?;
    let detection_level: String = row.get(5)?;
    Ok(SecurityRuleEvent {
        timestamp_unix_ms: row.get(0)?,
        event_id: row.get(1)?,
        event_type: row.get(2)?,
        rule_id: row.get(3)?,
        rule_action: SecurityRuleAction::parse_str(&rule_action).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                format!("unknown rule_action {rule_action}").into(),
            )
        })?,
        detection_level: SecurityDetectionLevel::parse_str(&detection_level).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                format!("unknown detection_level {detection_level}").into(),
            )
        })?,
        rule_json: row.get(6)?,
        event_json: row.get(7)?,
        trace_id: row.get(8)?,
        turn_id: row.get(9)?,
        credential_ref: row.get(10)?,
    })
}

pub(super) fn read_security_ask_event_row(row: &Row<'_>) -> rusqlite::Result<SecurityAskEvent> {
    let status: String = row.get(6)?;
    Ok(SecurityAskEvent {
        timestamp_unix_ms: row.get(0)?,
        ask_id: row.get(1)?,
        event_id: row.get(2)?,
        event_type: row.get(3)?,
        rule_id: row.get(4)?,
        rule_name: row.get(5)?,
        status: SecurityAskStatus::parse_str(&status).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                format!("unknown ask status {status}").into(),
            )
        })?,
        rule_json: row.get(7)?,
        event_json: row.get(8)?,
        resolver: row.get(9)?,
        reason: row.get(10)?,
        trace_id: row.get(11)?,
    })
}

/// Parse an fs_events row into FileEvent. Column order must match the SELECT in queries above.
pub(super) fn read_file_event_row(row: &Row<'_>) -> rusqlite::Result<FileEvent> {
    let ts_str: String = row.get(0)?;
    let timestamp = humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
    let action_str: String = row.get(1)?;
    Ok(FileEvent {
        event_id: row.get::<_, Option<String>>(6).ok().flatten(),
        timestamp,
        action: FileAction::parse_str(&action_str),
        path: row.get(2)?,
        size: row.get::<_, Option<i64>>(3)?.map(|s| s as u64),
        trace_id: row.get::<_, Option<String>>(4).ok().flatten(),
        credential_ref: row.get::<_, Option<String>>(5).ok().flatten(),
    })
}

/// Parse an exec_events row into a HistoryEntry for unified history.
pub(super) fn read_exec_history_row(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        timestamp: row.get(0)?,
        layer: "exec".to_string(),
        command: row.get(2)?,
        exit_code: row.get::<_, Option<i64>>(3)?.map(|c| c as i32),
        duration_ms: row.get::<_, Option<i64>>(4)?.map(|d| d as u64),
        stdout_preview: row.get(5)?,
        stderr_preview: row.get(6)?,
        details: serde_json::json!({
            "source": row.get::<_, Option<String>>(7)?,
            "trace_id": row.get::<_, Option<String>>(8)?,
            "process_name": row.get::<_, Option<String>>(9)?,
            "exec_id": row.get::<_, i64>(1)?,
        }),
    })
}

/// Parse an audit_events row into a HistoryEntry for unified history.
pub(super) fn read_audit_history_row(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        timestamp: row.get(0)?,
        layer: "audit".to_string(),
        command: row.get(6)?, // argv
        exit_code: row.get::<_, Option<i64>>(12)?.map(|c| c as i32),
        duration_ms: None,
        stdout_preview: None,
        stderr_preview: None,
        details: serde_json::json!({
            "pid": row.get::<_, i64>(1)?,
            "ppid": row.get::<_, i64>(2)?,
            "uid": row.get::<_, i64>(3)?,
            "exe": row.get::<_, String>(4)?,
            "comm": row.get::<_, Option<String>>(5)?,
            "cwd": row.get::<_, Option<String>>(7)?,
            "tty": row.get::<_, Option<String>>(8)?,
            "session_id": row.get::<_, Option<i64>>(9)?,
            "audit_id": row.get::<_, Option<String>>(10)?,
            "parent_exe": row.get::<_, Option<String>>(11)?,
        }),
    })
}

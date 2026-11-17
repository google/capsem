//! Indexed identity and original timestamps for each archived source.
use super::*;

pub(super) const SOURCE_TABLES: &[&str] = &[
    "net_events",
    "model_calls",
    "tool_calls",
    "tool_responses",
    "exec_events",
    "security_rule_events",
    "security_decision_events",
    "security_ask_events",
    "environment_revisions",
    "environment_requests",
];

pub(super) fn source_metadata(
    conn: &Connection,
    index: &IndexRow,
    created_at: &str,
) -> DbResult<(Option<String>, Option<String>, Option<i64>)> {
    if !SOURCE_TABLES.contains(&index.source_table.as_str()) {
        return Err(format!(
            "archive index has unsupported source table {}",
            index.source_table
        ));
    }
    let text = |sql: &str| -> DbResult<Option<(String, String)>> {
        conn.query_row(sql, [&index.event_id], |row| {
            Ok((bounded_text(row, 0)?, bounded_text(row, 1)?))
        })
        .optional()
        .map_err(|error| error.to_string())
    };
    let millis = |sql: &str| -> DbResult<Option<(String, i64)>> {
        conn.query_row(sql, [&index.event_id], |row| Ok((bounded_text(row, 0)?, row.get(1)?)))
            .optional()
            .map_err(|error| error.to_string())
    };
    match index.source_table.as_str() {
        "net_events" => Ok(text(
            "SELECT 'https://' || domain || COALESCE(path, ''), timestamp
             FROM net_events WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), Some(date), None))),
        "model_calls" => Ok(text(
            "SELECT 'https://' || provider || path, timestamp
             FROM model_calls WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), Some(date), None))),
        "tool_calls" => Ok(text(
            "SELECT 'capsem://tool/' || tool_name, timestamp
             FROM tool_calls WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), Some(date), None))),
        "tool_responses" => {
            let uri = conn
                .query_row(
                    "SELECT 'capsem://tool-response/' || call_id
                     FROM tool_responses WHERE event_id = ?1 ORDER BY id LIMIT 1",
                    [&index.event_id],
                    |row| bounded_text(row, 0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            Ok((uri, Some(created_at.to_string()), None))
        }
        "exec_events" => Ok(text(
            "SELECT 'capsem://exec/' || exec_id, timestamp
             FROM exec_events WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), Some(date), None))),
        "security_rule_events" => Ok(millis(
            "SELECT 'capsem://security/' || rule_id, timestamp_unix_ms
             FROM security_rule_events WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), None, Some(date)))),
        "security_decision_events" => Ok(millis(
            "SELECT 'capsem://security-decision/' || COALESCE(event.actor, run.actor), event.timestamp_unix_ms
             FROM security_decision_events AS event
             LEFT JOIN security_decision_runs AS run ON run.id = event.run_id
             WHERE event.event_id = ?1 ORDER BY event.id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), None, Some(date)))),
        "security_ask_events" => Ok(millis(
            "SELECT 'capsem://security-ask/' || ask_id, timestamp_unix_ms
             FROM security_ask_events WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), None, Some(date)))),
        "environment_revisions" => Ok(millis(
            "SELECT 'capsem://environment-revision/' || environment_id || '/' || revision_id
                || CASE replayable WHEN 0 THEN '#non-replayable' ELSE '' END, timestamp_unix_ms
             FROM environment_revisions WHERE payload_event_id=?1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), None, Some(date)))),
        "environment_requests" => Ok(millis(
            "SELECT 'capsem://environment-request/' || environment_id || '/' || request_id
                || CASE replayable WHEN 0 THEN '#non-replayable' ELSE '' END, timestamp_unix_ms
             FROM environment_requests WHERE payload_event_id=?1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), None, Some(date)))),
        table => unreachable!("{table} was checked against SOURCE_TABLES"),
    }
}

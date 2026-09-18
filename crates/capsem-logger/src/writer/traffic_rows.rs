//! Row inserts for the HTTP, MCP, filesystem and process ledgers.
//!
//! Every body these rows summarize is staged into the session archive here,
//! beside the row that names it; the display columns keep only a preview.

use super::*;

pub(super) fn insert_net_event(
    conn: &Connection,
    event: &NetEvent,
    target: WriteTarget,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    let req_body = body_preview(event.request_body.as_deref());
    let resp_body = body_preview(event.response_body.as_deref());
    let (req_headers, req_headers_cut) = cap_headers(&event.request_headers);
    let (resp_headers, resp_headers_cut) = cap_headers(&event.response_headers);
    let headers_truncated = i64::from(req_headers_cut || resp_headers_cut);
    let event_id = event.event_id.clone().unwrap_or_else(new_event_id);
    execute_cached(
        conn,
        &format!("INSERT INTO {} (
            event_id, timestamp, domain, port, decision, process_name, pid,
            method, path, query, status_code,
            bytes_sent, bytes_received, duration_ms, matched_rule,
            request_headers, response_headers, headers_truncated,
            request_body_preview, response_body_preview, conn_type,
            policy_mode, policy_action, policy_rule, policy_reason,
            trace_id, turn_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)", target.table("net_events")),
        params![
            event_id,
            timestamp,
            event.domain,
            i64::from(event.port),
            event.decision.as_str(),
            event.process_name,
            event.pid.map(i64::from),
            event.method,
            event.path,
            event.query,
            event.status_code.map(i64::from),
            event.bytes_sent as i64,
            event.bytes_received as i64,
            event.duration_ms as i64,
            event.matched_rule,
            req_headers,
            resp_headers,
            headers_truncated,
            req_body,
            resp_body,
            event.conn_type,
            event.policy_mode,
            event.policy_action,
            event.policy_rule,
            event.policy_reason,
            event.trace_id,
            event.trace_id,
            event.credential_ref,
        ],
    )?;
    bodies.stage(EventBodyBlob {
        event_id: &event_id,
        event_type: "http.request",
        source_table: "net_events",
        direction: "request",
        content_type: event.request_headers.as_deref().and_then(content_type_from_headers),
        body: event.request_body.as_deref(),
        original_bytes: None,
        trace_id: event.trace_id.as_deref(),
        turn_id: event.trace_id.as_deref(),
    });
    bodies.stage(EventBodyBlob {
        event_id: &event_id,
        event_type: "http.request",
        source_table: "net_events",
        direction: "response",
        content_type: event.response_headers.as_deref().and_then(content_type_from_headers),
        body: event.response_body.as_deref(),
        original_bytes: None,
        trace_id: event.trace_id.as_deref(),
        turn_id: event.trace_id.as_deref(),
    });
    Ok(())
}

pub(super) fn insert_file_event(conn: &Connection, event: &FileEvent, target: WriteTarget) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    let (directory, name) = split_event_path(&event.path);
    execute_cached(
        conn,
        &format!("INSERT INTO {} (event_id, timestamp, action, path, directory, name, size, kind, trace_id, turn_id, credential_ref)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", target.table("fs_events")),
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.action.as_str(),
            event.path,
            directory,
            name,
            event.size.map(|s| s as i64),
            event.kind.as_str(),
            event.trace_id,
            event.trace_id,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

/// Split a recorded path into the columns routes group and filter on.
///
/// An empty path is not a path in the current directory: the only event that
/// carries one is the overflow marker, which names nothing. Writing `(".", "")`
/// for it put a marker into every "changes under ." grouping, so an empty path
/// gets NULL columns and drops out of those groupings entirely.
fn split_event_path(path: &str) -> (Option<String>, Option<String>) {
    let normalized = path.trim_end_matches('/');
    if normalized.is_empty() {
        return (None, None);
    }
    match normalized.rsplit_once('/') {
        Some(("", name)) => (Some("/".to_string()), Some(name.to_string())),
        Some((dir, name)) if !name.is_empty() => (Some(dir.to_string()), Some(name.to_string())),
        _ => (Some(".".to_string()), Some(normalized.to_string())),
    }
}

pub(super) fn insert_mcp_call(
    conn: &Connection,
    call: &McpCall,
    target: WriteTarget,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(call.timestamp);
    let req_preview = cap_preview(&call.request_preview);
    let resp_preview = cap_preview(&call.response_preview);
    let event_id = call.event_id.clone().unwrap_or_else(new_event_id);
    if call.method == "tools/call" {
        let tool_name = call.tool_name.as_deref().unwrap_or("");
        execute_cached(
        conn,
            &format!("INSERT INTO {} (
                event_id, timestamp, model_call_id, provider, status, call_index, call_id,
                tool_name, arguments, response_preview, origin, transport, server_name, method, request_id,
                decision, duration_ms, error_message, process_name, bytes_sent, bytes_received,
                policy_mode, policy_action, policy_rule, policy_reason, trace_id, turn_id, credential_ref
            )
             VALUES (?1, ?2, NULL, '', ?3, 0, ?4, ?5, ?6, ?7, 'mcp', ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)", target.table("tool_calls")),
            params![
                &event_id,
                &timestamp,
                if call.error_message.is_some() { "error" } else { "responded" },
                call.request_id.as_deref().unwrap_or(&event_id),
                tool_name,
                req_preview.as_deref(),
                resp_preview.as_deref(),
                &call.transport,
                &call.server_name,
                &call.method,
                call.request_id.as_deref(),
                &call.decision,
                call.duration_ms as i64,
                call.error_message.as_deref(),
                call.process_name.as_deref(),
                call.bytes_sent as i64,
                call.bytes_received as i64,
                call.policy_mode.as_deref(),
                call.policy_action.as_deref(),
                call.policy_rule.as_deref(),
                call.policy_reason.as_deref(),
                call.trace_id.as_deref(),
                call.trace_id.as_deref(),
                call.credential_ref.as_deref(),
            ],
        )?;
        bodies.stage(EventBodyBlob {
            event_id: &event_id,
            event_type: "mcp.tool_call",
            source_table: "tool_calls",
            direction: "request",
            content_type: Some("application/json"),
            body: call.request_preview.as_deref().map(str::as_bytes),
            original_bytes: None,
            trace_id: call.trace_id.as_deref(),
            turn_id: call.trace_id.as_deref(),
        });
        bodies.stage(EventBodyBlob {
            event_id: &event_id,
            event_type: "mcp.tool_call",
            source_table: "tool_calls",
            direction: "response",
            content_type: Some("application/json"),
            body: call.response_preview.as_deref().map(str::as_bytes),
            original_bytes: None,
            trace_id: call.trace_id.as_deref(),
            turn_id: call.trace_id.as_deref(),
        });
        return Ok(());
    }
    let _ = (event_id, timestamp, req_preview, resp_preview);
    Ok(())
}

pub(super) fn content_type_from_headers(headers: &str) -> Option<&str> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-type") {
            Some(value.trim())
        } else {
            None
        }
    })
}

pub(super) fn insert_exec_event(conn: &Connection, event: &ExecEvent, target: WriteTarget) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            event_id, timestamp, exec_id, command, source, trace_id, turn_id, process_name, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            target.table("exec_events")
        ),
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.exec_id as i64,
            event.command,
            event.source,
            event.trace_id,
            event.trace_id,
            event.process_name,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

pub(super) fn update_exec_event(
    conn: &Connection,
    complete: &ExecEventComplete,
    exec_floor: i64,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<()> {
    let stdout_preview = cap_preview(&complete.stdout_preview);
    let stderr_preview = cap_preview(&complete.stderr_preview);
    // The exec row was inserted when the command started; its event_id is
    // what the archive rows are keyed on, so the output is reachable from
    // the same id the timeline shows.
    //
    // The archive holds at most the 1 KiB of output the vsock payload
    // carries: capsem-process truncates there (`vsock/exec_completion.rs`)
    // and the rest never reaches this process. `stdout_bytes`/`stderr_bytes`
    // are the true totals, so the index row reports them as `original_bytes`
    // and marks itself truncated rather than claiming the excerpt is all
    // there was.
    let Some(start) = find_exec_start(conn, complete.exec_id, exec_floor)? else {
        // The completion arrived without its start row, so there is no
        // event_id to key the output on and it is not archived. Loud, because
        // it means a producer sent a completion for an exec this ledger never
        // saw begin.
        warn!(
            exec_id = complete.exec_id,
            "exec completion has no start row; its output is not archived"
        );
        return Ok(());
    };
    for (direction, body, produced) in [
        (
            "stdout",
            complete.stdout_preview.as_deref().map(str::as_bytes),
            complete.stdout_bytes,
        ),
        (
            "stderr",
            complete.stderr_preview.as_deref().map(str::as_bytes),
            complete.stderr_bytes,
        ),
    ] {
        bodies.stage(EventBodyBlob {
            event_id: &start.event_id,
            event_type: "process.exec_complete",
            source_table: "exec_events",
            direction,
            content_type: Some("text/plain"),
            body,
            original_bytes: Some(produced),
            trace_id: start.trace_id.as_deref(),
            turn_id: start.trace_id.as_deref(),
        });
    }
    let exec_events = start.table;
    execute_cached(
        conn,
        &format!(
            "UPDATE {exec_events} SET
            exit_code = ?1,
            duration_ms = ?2,
            stdout_preview = ?3,
            stderr_preview = ?4,
            stdout_bytes = ?5,
            stderr_bytes = ?6,
            pid = ?7
         WHERE id = ?8"
        ),
        params![
            i64::from(complete.exit_code),
            complete.duration_ms as i64,
            stdout_preview,
            stderr_preview,
            complete.stdout_bytes as i64,
            complete.stderr_bytes as i64,
            complete.pid.map(i64::from),
            start.id,
        ],
    )?;
    Ok(())
}

/// The exec start row a completion belongs to, and the table it is in now.
struct ExecStart {
    table: String,
    id: i64,
    event_id: String,
    trace_id: Option<String>,
}

/// Find the start row of `exec_id`.
///
/// Memory holds only what the writer has not flushed, so a command that ran
/// past a flush has its start row on disk. It is looked for there only above
/// `exec_floor`, the ledger's last id when this writer opened: exec ids
/// restart with the process, and a resumed session's `7` is not the `7` an
/// earlier boot left behind.
fn find_exec_start(conn: &Connection, exec_id: u64, exec_floor: i64) -> rusqlite::Result<Option<ExecStart>> {
    let memory_table = WriteTarget::Memory.table("exec_events");
    let in_memory = lookup_exec_start(
        conn,
        &memory_table,
        &format!("SELECT id, event_id, trace_id FROM {memory_table} WHERE exec_id = ?1 ORDER BY id DESC LIMIT 1"),
        params![exec_id as i64],
    )?;
    if in_memory.is_some() {
        return Ok(in_memory);
    }
    lookup_exec_start(
        conn,
        "main.exec_events",
        "SELECT id, event_id, trace_id FROM main.exec_events
         WHERE exec_id = ?1 AND id > ?2 ORDER BY id DESC LIMIT 1",
        params![exec_id as i64, exec_floor],
    )
}

fn lookup_exec_start(
    conn: &Connection,
    table: &str,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<Option<ExecStart>> {
    conn.prepare_cached(sql)?
        .query_row(params, |row| {
            Ok(ExecStart {
                table: table.to_string(),
                id: row.get(0)?,
                event_id: row.get(1)?,
                trace_id: row.get(2)?,
            })
        })
        .optional()
}

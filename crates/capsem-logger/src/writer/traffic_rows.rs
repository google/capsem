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
    let req_body = cap_preview(&event.request_body_preview);
    let resp_body = cap_preview(&event.response_body_preview);
    let req_headers = cap_field(&event.request_headers);
    let resp_headers = cap_field(&event.response_headers);
    let event_id = event.event_id.clone().unwrap_or_else(new_event_id);
    execute_cached(
        conn,
        &format!("INSERT INTO {} (
            event_id, timestamp, domain, port, decision, process_name, pid,
            method, path, query, status_code,
            bytes_sent, bytes_received, duration_ms, matched_rule,
            request_headers, response_headers,
            request_body_preview, response_body_preview, conn_type,
            policy_mode, policy_action, policy_rule, policy_reason,
            trace_id, turn_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)", target.table("net_events")),
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
        body: event
            .request_body_full
            .as_deref()
            .or(event.request_body_preview.as_deref()),
        trace_id: event.trace_id.as_deref(),
        turn_id: event.trace_id.as_deref(),
    });
    bodies.stage(EventBodyBlob {
        event_id: &event_id,
        event_type: "http.request",
        source_table: "net_events",
        direction: "response",
        content_type: event.response_headers.as_deref().and_then(content_type_from_headers),
        body: event
            .response_body_full
            .as_deref()
            .or(event.response_body_preview.as_deref()),
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
        &format!("INSERT INTO {} (event_id, timestamp, action, path, directory, name, size, trace_id, turn_id, credential_ref)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)", target.table("fs_events")),
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.action.as_str(),
            event.path,
            directory,
            name,
            event.size.map(|s| s as i64),
            event.trace_id,
            event.trace_id,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

fn split_event_path(path: &str) -> (String, String) {
    let normalized = path.trim_end_matches('/');
    if normalized.is_empty() {
        return (".".to_string(), String::new());
    }
    match normalized.rsplit_once('/') {
        Some(("", name)) => ("/".to_string(), name.to_string()),
        Some((dir, name)) if !name.is_empty() => (dir.to_string(), name.to_string()),
        _ => (".".to_string(), normalized.to_string()),
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
            body: call.request_preview.as_deref(),
            trace_id: call.trace_id.as_deref(),
            turn_id: call.trace_id.as_deref(),
        });
        bodies.stage(EventBodyBlob {
            event_id: &event_id,
            event_type: "mcp.tool_call",
            source_table: "tool_calls",
            direction: "response",
            content_type: Some("application/json"),
            body: call.response_preview.as_deref(),
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
    target: WriteTarget,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<()> {
    let stdout_preview = cap_preview(&complete.stdout_preview);
    let stderr_preview = cap_preview(&complete.stderr_preview);
    // The exec row was inserted when the command started; its event_id is
    // what the archive rows are keyed on, so the output is reachable from
    // the same id the timeline shows.
    //
    // What arrives here is whatever the producer sent: capsem-process caps
    // guest output at 1 KiB before it crosses the vsock boundary
    // (`vsock/exec_completion.rs`), so for guest commands the archive holds
    // that much and no more. It is still the only full copy the ledger has.
    let started: Option<(String, Option<String>)> = conn
        .prepare_cached(&format!(
            "SELECT event_id, trace_id FROM {} WHERE exec_id = ?1",
            target.table("exec_events")
        ))?
        .query_row(params![complete.exec_id as i64], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    if let Some((event_id, trace_id)) = &started {
        for (direction, body) in [
            ("stdout", complete.stdout_preview.as_deref()),
            ("stderr", complete.stderr_preview.as_deref()),
        ] {
            bodies.stage(EventBodyBlob {
                event_id,
                event_type: "process.exec_complete",
                source_table: "exec_events",
                direction,
                content_type: Some("text/plain"),
                body,
                trace_id: trace_id.as_deref(),
                turn_id: trace_id.as_deref(),
            });
        }
    }
    execute_cached(
        conn,
        &format!(
            "UPDATE {} SET
            exit_code = ?1,
            duration_ms = ?2,
            stdout_preview = ?3,
            stderr_preview = ?4,
            stdout_bytes = ?5,
            stderr_bytes = ?6,
            pid = ?7
         WHERE exec_id = ?8",
            target.table("exec_events")
        ),
        params![
            i64::from(complete.exit_code),
            complete.duration_ms as i64,
            stdout_preview,
            stderr_preview,
            complete.stdout_bytes as i64,
            complete.stderr_bytes as i64,
            complete.pid.map(i64::from),
            complete.exec_id as i64,
        ],
    )?;
    Ok(())
}

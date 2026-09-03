use rusqlite::{params, Connection};

use super::{
    blake3_ref, cap_field, format_timestamp, insert_event_body_blob, new_event_id, EventBodyBlob, WriteTarget,
};
use crate::events::ModelCall;

pub(super) fn insert_model_call(conn: &Connection, call: &ModelCall, target: WriteTarget) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(call.timestamp);
    let req_body = cap_field(&call.request_body_preview);
    let text_content = cap_field(&call.text_content);
    let thinking_content = cap_field(&call.thinking_content);
    let sys_prompt = cap_field(&call.system_prompt_preview);
    let event_id = call.event_id.clone().unwrap_or_else(new_event_id);
    conn.execute(
        &format!("INSERT INTO {} (
            event_id, timestamp, provider, protocol, model, process_name, pid,
            method, path, stream,
            system_prompt_preview, messages_count, tools_count,
            request_bytes, request_body_preview,
            message_id, status_code, text_content, thinking_content,
            stop_reason, input_tokens, output_tokens,
            duration_ms, response_bytes, estimated_cost_usd, trace_id,
            usage_details, credential_ref, turn_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)", target.table("model_calls")),
        params![
            event_id,
            timestamp,
            call.provider,
            call.protocol,
            call.model,
            call.process_name,
            call.pid.map(i64::from),
            call.method,
            call.path,
            i64::from(call.stream),
            sys_prompt,
            call.messages_count as i64,
            call.tools_count as i64,
            call.request_bytes as i64,
            req_body,
            call.message_id,
            call.status_code.map(i64::from),
            text_content,
            thinking_content,
            call.stop_reason,
            call.input_tokens.map(|t| t as i64),
            call.output_tokens.map(|t| t as i64),
            call.duration_ms as i64,
            call.response_bytes as i64,
            call.estimated_cost_usd,
            call.trace_id,
            if call.usage_details.is_empty() { None } else { Some(serde_json::to_string(&call.usage_details).unwrap_or_default()) },
            call.credential_ref,
            call.trace_id,
        ],
    )?;
    let model_call_id = conn.last_insert_rowid();
    insert_event_body_blob(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type: "model.call",
            source_table: "model_calls",
            direction: "request",
            content_type: Some("application/json"),
            body: call
                .request_body_full
                .as_deref()
                .or(call.request_body_preview.as_deref()),
            trace_id: call.trace_id.as_deref(),
            turn_id: call.trace_id.as_deref(),
        },
    )?;
    insert_event_body_blob(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type: "model.call",
            source_table: "model_calls",
            direction: "response",
            content_type: None,
            body: call.response_body_full.as_deref().or(call.text_content.as_deref()),
            trace_id: call.trace_id.as_deref(),
            turn_id: call.trace_id.as_deref(),
        },
    )?;
    insert_model_items(conn, model_call_id, call, &timestamp, target)?;

    for tc in &call.tool_calls {
        // W6: tool_calls.trace_id falls back to the parent model_call's
        // trace_id (they belong to the same agent turn).
        let tc_trace = tc.trace_id.clone().or_else(|| call.trace_id.clone());
        conn.execute(
            &format!(
                "INSERT INTO {} (
                event_id, timestamp, model_call_id, provider, status, call_index, call_id,
                tool_name, arguments, origin, transport, server_name, decision, duration_ms,
                trace_id, turn_id, credential_ref
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                target.table("tool_calls")
            ),
            params![
                new_event_id(),
                timestamp,
                model_call_id,
                call.provider,
                "observed",
                i64::from(tc.call_index),
                tc.call_id,
                tc.tool_name,
                tc.arguments,
                tc.origin,
                model_tool_transport(call),
                "model",
                "allowed",
                call.duration_ms as i64,
                tc_trace,
                call.trace_id,
                call.credential_ref,
            ],
        )?;
    }

    for tr in &call.tool_responses {
        let tr_trace = tr.trace_id.clone().or_else(|| call.trace_id.clone());
        let tr_credential_ref = tr.credential_ref.clone().or_else(|| call.credential_ref.clone());
        conn.execute(
            &format!(
                "INSERT INTO {} (model_call_id, call_id, content_preview, is_error, trace_id, turn_id, credential_ref)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                target.table("tool_responses")
            ),
            params![
                model_call_id,
                tr.call_id,
                tr.content_preview,
                i64::from(tr.is_error),
                tr_trace,
                call.trace_id,
                tr_credential_ref,
            ],
        )?;
    }

    Ok(())
}

fn insert_model_items(
    conn: &Connection,
    model_call_id: i64,
    call: &ModelCall,
    timestamp: &str,
    target: WriteTarget,
) -> rusqlite::Result<()> {
    let table = target.table("model_items");
    let mut item_index = 0_i64;
    let mut insert_item = |kind: &str,
                           call_id: Option<&str>,
                           tool_name: Option<&str>,
                           arguments: Option<&str>,
                           content: Option<String>|
     -> rusqlite::Result<()> {
        item_index += 1;
        let call_id = call_id.unwrap_or_default();
        let content = cap_field(&content);
        let hash_material = serde_json::json!({
            "kind": kind,
            "call_id": call_id,
            "tool_name": tool_name,
            "arguments": arguments,
            "content": content,
        })
        .to_string();
        let content_hash = blake3_ref(&hash_material);
        conn.execute(
            &format!(
                "INSERT OR IGNORE INTO {table} (
                event_id, model_call_id, timestamp, provider, model, path, trace_id,
                kind, item_index, call_id, tool_name, arguments, content,
                content_hash, credential_ref, turn_id
             )
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
             WHERE NOT EXISTS (
                SELECT 1 FROM {table}
                WHERE trace_id IS ?7
                  AND kind = ?8
                  AND content_hash = ?14
                  AND call_id = ?10
             )"
            ),
            params![
                new_event_id(),
                model_call_id,
                timestamp,
                call.provider,
                call.model,
                call.path,
                call.trace_id,
                kind,
                item_index,
                call_id,
                tool_name,
                arguments,
                content,
                content_hash,
                call.credential_ref,
                call.trace_id,
            ],
        )?;
        Ok(())
    };

    // A tool-result continuation request is represented by tool_response rows;
    // do not also log it as another user request for the same trace.
    if call.tool_responses.is_empty() {
        if let Some(content) = &call.request_body_preview {
            insert_item("request", None, None, None, Some(content.clone()))?;
        }
    }
    if let Some(content) = &call.thinking_content {
        insert_item("reasoning", None, None, None, Some(content.clone()))?;
    }
    if let Some(content) = &call.text_content {
        insert_item("response", None, None, None, Some(content.clone()))?;
    }
    for tool_call in &call.tool_calls {
        insert_item(
            "tool_call",
            Some(&tool_call.call_id),
            Some(&tool_call.tool_name),
            tool_call.arguments.as_deref(),
            tool_call.arguments.clone(),
        )?;
    }
    for tool_response in &call.tool_responses {
        insert_item(
            "tool_response",
            Some(&tool_response.call_id),
            None,
            None,
            tool_response.content_preview.clone(),
        )?;
    }
    Ok(())
}

fn model_tool_transport(call: &ModelCall) -> &'static str {
    if call.stream {
        "sse"
    } else {
        "http"
    }
}

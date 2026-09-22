use rusqlite::{params, Connection};

use super::bodies::{BodyArchive, EventBodyBlob};
use super::{blake3_ref, body_preview, cap_field, cap_preview, format_timestamp, new_event_id, WriteTarget};
use crate::events::ModelCall;

pub(super) fn insert_model_call(
    conn: &Connection,
    call: &ModelCall,
    target: WriteTarget,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(call.timestamp);
    let req_body = body_preview(call.request_body.as_deref());
    let text_content = cap_field(&call.text_content);
    let thinking_content = cap_field(&call.thinking_content);
    let sys_prompt = cap_preview(&call.system_prompt_preview);
    let event_id = call.event_id.clone().unwrap_or_else(new_event_id);
    super::execute_cached(
        conn,
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
    bodies.stage(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type: "model.call",
            source_table: "model_calls",
            direction: "request",
            content_type: Some("application/json"),
            body: call.request_body.as_deref(),
            original_bytes: None,
            trace_id: call.trace_id.as_deref(),
            turn_id: call.trace_id.as_deref(),
        },
    );
    bodies.stage(
        conn,
        EventBodyBlob {
            event_id: &event_id,
            event_type: "model.call",
            source_table: "model_calls",
            direction: "response",
            content_type: None,
            body: call
                .response_body
                .as_deref()
                .or_else(|| call.text_content.as_deref().map(str::as_bytes)),
            original_bytes: None,
            trace_id: call.trace_id.as_deref(),
            turn_id: call.trace_id.as_deref(),
        },
    );
    insert_model_items(conn, model_call_id, call, &timestamp, target)?;

    for tc in &call.tool_calls {
        // W6: tool_calls.trace_id falls back to the parent model_call's
        // trace_id (they belong to the same agent turn).
        let tc_trace = tc.trace_id.clone().or_else(|| call.trace_id.clone());
        super::execute_cached(
            conn,
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
                tc.event_id.clone().unwrap_or_else(new_event_id),
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
        // The full content is archived below; this column is the display
        // excerpt, reached from the same event_id the archive row carries.
        let tr_content_preview = cap_preview(&tr.content_preview);
        let tr_event_id = tr.event_id.clone().unwrap_or_else(new_event_id);
        bodies.stage(
            conn,
            EventBodyBlob {
                event_id: &tr_event_id,
                // A tool result is part of the model exchange it continues;
                // `source_table` is what distinguishes it from the call body.
                event_type: "model.call",
                source_table: "tool_responses",
                direction: "response",
                content_type: None,
                body: tr.content_preview.as_deref().map(str::as_bytes),
                original_bytes: None,
                trace_id: tr_trace.as_deref(),
                turn_id: call.trace_id.as_deref(),
            },
        );
        super::execute_cached(
            conn,
            &format!(
                "INSERT INTO {} (event_id, model_call_id, call_id, content_preview, is_error, trace_id, turn_id, credential_ref)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                target.table("tool_responses")
            ),
            params![
                tr_event_id,
                model_call_id,
                tr.call_id,
                tr_content_preview,
                i64::from(tr.is_error),
                tr_trace,
                call.trace_id,
                tr_credential_ref,
            ],
        )?;
    }

    Ok(())
}

/// A model item's `event_id`, derived from the identity the row already has.
///
/// Every other column of a `model_items` row comes from the `ModelCall` it is
/// derived from, so minting this one from a random UUID made it the single
/// value in a rebuilt ledger that a replay of the same session could not
/// reproduce -- which is what kept the fixture regenerator from being
/// byte-reproducible, and with it the digest that is supposed to let a
/// reviewer rerun the tool and diff.
///
/// `(trace_id, kind, content_hash, call_id)` is the tuple the `UNIQUE` on this
/// table already treats as the row's identity, so two rows can only share a
/// derived id if one of them cannot exist. That makes the id an answer about
/// the item rather than an arbitrary label, and two writes of the same item
/// now agree on it instead of disagreeing by construction. The width matches
/// `new_event_id`: 12 lowercase hex, as the column's CHECK requires.
fn model_item_event_id(trace_id: Option<&str>, kind: &str, content_hash: &str, call_id: &str) -> String {
    let material = format!("{}\0{kind}\0{content_hash}\0{call_id}", trace_id.unwrap_or_default());
    blake3::hash(material.as_bytes()).to_hex()[..12].to_string()
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
        // Hash the ORIGINAL, uncapped content: content_hash feeds the
        // UNIQUE(trace_id, kind, content_hash, call_id) dedup guard below,
        // and two distinct turns whose bodies only differ after the cap
        // point (e.g. share the same system prompt for the first 2 KB)
        // must not collapse into one row. Only the stored value is capped.
        let hash_material = serde_json::json!({
            "kind": kind,
            "call_id": call_id,
            "tool_name": tool_name,
            "arguments": arguments,
            "content": content,
        })
        .to_string();
        let content_hash = blake3_ref(&hash_material);
        // Request and response bodies are archived under the parent model
        // event; tool responses are archived under their own event row. These
        // columns are display excerpts, while the original content above
        // remains the dedup identity. Reasoning and tool-call arguments have
        // no complete archive representation of their own.
        let content = if matches!(kind, "request" | "response" | "tool_response") {
            cap_preview(&content)
        } else {
            cap_field(&content)
        };
        // First write wins, and the first write may already have been
        // flushed out of memory: the flush copies with INSERT OR REPLACE, so
        // a repeat that reached memory would replace the disk row.
        super::execute_cached(
            conn,
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
             )
             AND NOT EXISTS (
                SELECT 1 FROM main.model_items
                WHERE trace_id IS ?7
                  AND kind = ?8
                  AND content_hash = ?14
                  AND call_id = ?10
             )"
            ),
            params![
                model_item_event_id(call.trace_id.as_deref(), kind, &content_hash, call_id),
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
        // The whole captured body reaches `insert_item`: it is archived and
        // hashed before only its display value is capped to a preview.
        if let Some(body) = call.request_body.as_deref().filter(|body| !body.is_empty()) {
            let content = String::from_utf8_lossy(body).into_owned();
            insert_item("request", None, None, None, Some(content))?;
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

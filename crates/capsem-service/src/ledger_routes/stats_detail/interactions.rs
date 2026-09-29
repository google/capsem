//! Interaction query intent. All execution stays with the logger DB handle.
use super::*;

mod mcp;
mod payload;
use payload::{cut, preview_payload, text_payload};

pub(crate) const MODEL_ITEMS_SQL: &str = r#"
SELECT mi.event_id, mi.timestamp, mi.model_call_id, mc.event_id AS model_event_id,
       mi.trace_id, mi.turn_id, mi.item_index, mi.kind, mi.call_id,
       substr(mi.content, 1, ?1) AS content, COALESCE(length(mi.content) > ?1, 0) AS content_truncated,
       (SELECT CASE WHEN MAX(tr.is_error NOT IN (0, 1)) = 1 THEN 2
                    WHEN COUNT(DISTINCT tr.is_error) = 1 THEN MIN(tr.is_error) ELSE NULL END
        FROM tool_responses tr
        WHERE tr.model_call_id = mi.model_call_id AND tr.call_id = mi.call_id) AS is_error
FROM model_items mi
LEFT JOIN model_calls mc ON mc.id = mi.model_call_id
WHERE mi.kind != 'tool_call'
ORDER BY mi.id DESC LIMIT 200
"#;

pub(crate) const TOOL_CALLS_SQL: &str = r#"
SELECT tc.event_id, COALESCE(NULLIF(tc.timestamp, ''), mc.timestamp, '') AS timestamp,
       tc.model_call_id, mc.event_id AS model_event_id, tc.trace_id, tc.turn_id,
       tc.call_id, tc.tool_name, tc.server_name, tc.origin, tc.decision, tc.method,
       substr(tc.arguments, 1, ?1) AS arguments, COALESCE(length(tc.arguments) > ?1, 0) AS arguments_truncated,
       tc.response_preview, tc.error_message
FROM tool_calls tc
LEFT JOIN model_calls mc ON mc.id = tc.model_call_id
ORDER BY tc.id DESC LIMIT 200
"#;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModelItemKind {
    Request,
    Reasoning,
    Response,
    ToolResponse,
}

#[derive(Deserialize)]
pub(super) struct ModelItemRow {
    event_id: String,
    timestamp: String,
    model_call_id: i64,
    model_event_id: Option<String>,
    trace_id: Option<String>,
    turn_id: Option<String>,
    item_index: u64,
    kind: ModelItemKind,
    call_id: String,
    content: Option<String>,
    content_truncated: bool,
    is_error: Option<u8>,
}

#[derive(Deserialize)]
pub(super) struct ToolRow {
    event_id: String,
    timestamp: String,
    model_call_id: Option<i64>,
    model_event_id: Option<String>,
    trace_id: Option<String>,
    turn_id: Option<String>,
    call_id: String,
    tool_name: String,
    server_name: Option<String>,
    origin: ToolOrigin,
    method: Option<String>,
    decision: ToolDecision,
    arguments: Option<String>,
    arguments_truncated: bool,
    response_preview: Option<String>,
    error_message: Option<String>,
}

/// The interaction report over the rows [`MODEL_ITEMS_SQL`] and
/// [`TOOL_CALLS_SQL`] read, which the stats detail batch fetched.
pub(super) fn read_interactions(
    vm_id: &str,
    db_path: &StdPath,
    models: Vec<ModelItemRow>,
    tools: Vec<ToolRow>,
    bodies: &std::collections::BTreeMap<String, Vec<EventBody>>,
) -> Result<InteractionReport, AppError> {
    let mut items = Vec::with_capacity(models.len() + tools.len());
    for row in models {
        let is_error = match row.is_error {
            None => None,
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => {
                return Err(ledger_route_error(
                    vm_id,
                    "stats_detail",
                    "interaction_models",
                    db_path,
                    "invalid tool result error flag",
                ))
            }
        };
        let content = match row.kind {
            ModelItemKind::Request => InteractionContent::Request(InteractionRequest {
                kind: InteractionRequestKind::RequestPreview,
                payload: row.content.map(|raw| cut(preview_payload(raw), row.content_truncated)),
            }),
            ModelItemKind::Reasoning | ModelItemKind::Response => {
                let kind = match row.kind {
                    ModelItemKind::Reasoning => InteractionBlockKind::Reasoning,
                    _ => InteractionBlockKind::Text,
                };
                InteractionContent::Message(InteractionMessage {
                    kind: InteractionMessageKind::Message,
                    role: InteractionRole::Assistant,
                    blocks: vec![InteractionBlock {
                        kind,
                        payload: row.content.map(|text| cut(text_payload(text), row.content_truncated)),
                    }],
                })
            }
            ModelItemKind::ToolResponse => InteractionContent::ToolResult(InteractionToolResult {
                kind: InteractionToolResultKind::ToolResult,
                call_id: row.call_id,
                payload: row.content.map(|raw| cut(preview_payload(raw), row.content_truncated)),
                response: None,
                is_error,
                error_message: None,
            }),
        };
        items.push(Interaction {
            event_id: row.event_id,
            timestamp: row.timestamp,
            model_call_id: Some(row.model_call_id),
            model_event_id: row.model_event_id,
            trace_id: row.trace_id,
            turn_id: row.turn_id,
            item_index: Some(row.item_index),
            content,
        });
    }
    for row in tools {
        let content = mcp::tool_content(&row);
        items.push(Interaction {
            event_id: row.event_id,
            timestamp: row.timestamp,
            model_call_id: row.model_call_id,
            model_event_id: row.model_event_id,
            trace_id: row.trace_id,
            turn_id: row.turn_id,
            item_index: None,
            content: InteractionContent::ToolCall(content),
        });
    }
    items.sort_by(|a, b| {
        (&a.timestamp, a.model_call_id, a.item_index, &a.event_id).cmp(&(
            &b.timestamp,
            b.model_call_id,
            b.item_index,
            &b.event_id,
        ))
    });
    let bodies = bodies
        .values()
        .flatten()
        .map(|body| InteractionBody {
            event_id: body.event_id.clone(),
            source_table: body.source_table.clone(),
            direction: body.direction.clone(),
            content_type: body.content_type.clone(),
            original_bytes: body.original_bytes,
            stored_bytes: body.stored_bytes,
            truncated: body.truncated,
            body_hash: body.body_hash.clone(),
        })
        .collect();
    Ok(InteractionReport { items, bodies })
}

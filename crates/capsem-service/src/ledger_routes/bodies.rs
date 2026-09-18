//! Reading archived bodies back out over HTTP.
//!
//! The list views carry body *metadata* -- what was captured, how big it was,
//! what it hashes to. The bytes stay in the archive until somebody asks for
//! one event, because a page of two hundred rows that inlined its bodies would
//! be a response whose size two hundred remote servers chose.
//!
//! So there is one route for the bytes, `GET /vms/{id}/bodies/{event_id}`, and
//! it is the same route for the stats view and for a forensic client: one
//! shape, one bound, one place that decides what "I gave you less than all of
//! it" is spelled like. A second route with its own cut-off would be a second
//! answer to that question, and a caller cannot tell a short body from a
//! truncated one unless every route says so the same way.

use base64::Engine;
use capsem_logger::StoredBody;

use super::*;

/// Body index metadata for the stats detail view: what was captured for the
/// recent events of each layer, never the bytes. The bytes are read one event
/// at a time through [`handle_event_bodies`].
pub(crate) const STATS_DETAIL_BODY_BLOBS_SQL: &str = r#"
SELECT event_id, source_table, direction, content_type, original_bytes, stored_bytes, truncated, body_hash
FROM event_body_blobs
WHERE event_id IN (
    SELECT event_id FROM net_events WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200
)
OR event_id IN (
    SELECT event_id FROM model_calls WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200
)
OR event_id IN (
    SELECT event_id FROM tool_calls WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200
)
OR event_id IN (
    SELECT event_id FROM security_rule_events ORDER BY id DESC LIMIT 200
)
ORDER BY event_id, direction
"#;

/// Index rows grouped by the event they describe, so the detail view can look
/// up one event's metadata without scanning the list.
pub(crate) fn body_blob_map(rows: Vec<serde_json::Value>) -> serde_json::Value {
    let mut by_event = serde_json::Map::new();
    for row in rows {
        let Some(id) = row.get("event_id").and_then(|value| value.as_str()) else {
            continue;
        };
        let entry = by_event
            .entry(id.to_string())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if let serde_json::Value::Array(rows) = entry {
            rows.push(row);
        }
    }
    serde_json::Value::Object(by_event)
}

/// Twelve lowercase hex characters, which is what the ledger's CHECK
/// constraints enforce on the way in.
const EVENT_ID_LEN: usize = 12;

/// What a caller gets if it asks for nothing in particular. Enough for a
/// detail pane to render a whole body in the overwhelming majority of cases,
/// small enough that a webview asking about a table row cannot be handed ten
/// megabytes it never wanted.
const DEFAULT_TRANSPORT_BYTES: usize = 1024 * 1024;

/// The most any single request may ask for, clamped rather than refused: a
/// forensic client that wants everything should get everything the archive can
/// hold, not an error it has to learn a number to avoid.
const MAX_TRANSPORT_BYTES: usize = 16 * 1024 * 1024;

/// How much of each body to send back.
#[derive(Deserialize, Debug, Default)]
pub(crate) struct EventBodiesQuery {
    /// Bytes per body. Absent means [`DEFAULT_TRANSPORT_BYTES`]; anything
    /// above [`MAX_TRANSPORT_BYTES`] is clamped to it.
    pub(crate) max_bytes: Option<usize>,
}

impl EventBodiesQuery {
    pub(crate) fn transport_budget(&self) -> usize {
        self.max_bytes
            .unwrap_or(DEFAULT_TRANSPORT_BYTES)
            .min(MAX_TRANSPORT_BYTES)
    }
}

/// An `{event_id}` from a URL, checked before anything is done with it.
///
/// One validator, called first by every handler whose path carries an event
/// id. The id in a URL is an untrusted string, and the constraint the ledger
/// enforces on the way in is not enforced on the way out; without one place
/// that says what twelve hex characters means, each route would have its own
/// idea and each would spend a query before finding out it was given `../`.
///
/// # Errors
///
/// A 400 naming the constraint, for anything that is not exactly twelve
/// lowercase hex characters.
pub(crate) fn validate_event_id(raw: &str) -> Result<&str, AppError> {
    let hex = raw.len() == EVENT_ID_LEN && raw.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if hex {
        return Ok(raw);
    }
    Err(AppError(
        StatusCode::BAD_REQUEST,
        format!(
            "event id must be exactly {EVENT_ID_LEN} lowercase hex characters; got {} character(s)",
            raw.chars().count()
        ),
    ))
}

/// One archived body, cut to what this response agreed to carry.
///
/// The single place a route decides how much of a body it hands back. Text is
/// cut at a UTF-8 character boundary so the content is still a string; bytes
/// that are not text go out as base64 and are cut wherever the budget lands.
///
/// `truncated_for_transport` is what this function did. `truncated` is what
/// the capture did, upstream, before the archive ever saw the body -- the two
/// are reported separately because a reviewer looking at a partial body needs
/// to know whether the rest of it exists anywhere.
pub(crate) fn bounded_body_response(stored: StoredBody, max_bytes: usize) -> api::bodies::EventBody {
    let stored_bytes = stored.bytes.len();
    let (encoding, content, cut_at) = match std::str::from_utf8(&stored.bytes) {
        Ok(text) => {
            let cut = floor_char_boundary(text, max_bytes);
            (api::bodies::BodyEncoding::Utf8, text[..cut].to_string(), cut)
        }
        Err(_) => {
            let cut = max_bytes.min(stored_bytes);
            let encoded = base64::engine::general_purpose::STANDARD.encode(&stored.bytes[..cut]);
            (api::bodies::BodyEncoding::Base64, encoded, cut)
        }
    };
    api::bodies::EventBody {
        event_id: stored.event_id,
        source_table: stored.source_table,
        direction: stored.direction.as_str().to_string(),
        content_type: stored.content_type,
        original_bytes: stored.original_bytes,
        stored_bytes: stored_bytes as u64,
        truncated: stored.truncated,
        truncated_for_transport: cut_at < stored_bytes,
        body_hash: stored.body_hash,
        encoding,
        content,
    }
}

/// The largest index at or below `at` that starts a character.
///
/// `str::floor_char_boundary` is still unstable, and cutting a multi-byte
/// character in half would make `content` something that is not a string.
fn floor_char_boundary(text: &str, at: usize) -> usize {
    if at >= text.len() {
        return text.len();
    }
    let mut cut = at;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// `GET /vms/{id}/bodies/{event_id}?max_bytes=<N>` -- every archived body of
/// one event.
///
/// One index query and one inflate per block, because the request and the
/// response of an exchange are staged together and almost always share one.
///
/// An event with no archived body answers with an empty list rather than a
/// 404: the event may simply have had no body, and a 404 would say instead
/// that the event does not exist.
pub(crate) async fn handle_event_bodies(
    State(state): State<Arc<ServiceState>>,
    Path((id, event_id)): Path<(String, String)>,
    Query(params): Query<EventBodiesQuery>,
) -> Result<Json<api::bodies::EventBodiesResponse>, AppError> {
    let event_id = validate_event_id(&event_id)?;
    let max_bytes = params.transport_budget();
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    let db = open_ready_session_db(&state, &id, "bodies", &db_path).await?;
    let stored = db
        .read_bodies(event_id)
        .await
        .map_err(|error| ledger_route_error(&id, "bodies", "read bodies", &db_path, error))?;
    let bodies: Vec<api::bodies::EventBody> = stored
        .into_iter()
        .map(|body| bounded_body_response(body, max_bytes))
        .collect();
    info!(
        route = "/vms/{id}/bodies/{event_id}",
        vm_id = id.as_str(),
        max_bytes,
        body_count = bodies.len(),
        "event_bodies"
    );
    Ok(Json(api::bodies::EventBodiesResponse {
        event_id: event_id.to_string(),
        bodies,
    }))
}

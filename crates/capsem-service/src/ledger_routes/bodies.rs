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
///
/// Every kind the detail pane can render a body section for belongs here,
/// `exec_events` included. A kind left out has no metadata in the list
/// response, so its section can only appear once its own body fetch resolves
/// -- and when that fetch fails there is nothing at all to key on, not even
/// the hash that would say a body was captured. The cost is a slightly larger
/// list response, which is the trade for the section rendering from the list
/// like every other one. Each window matches the list it annotates: exec rows
/// are bound to [`STATS_DETAIL_PROCESS_EVENTS_LIMIT`] as `?1`, the same number
/// the process list is bound to, so the two cannot drift into metadata for
/// rows the response does not carry or rows with none.
///
/// Decisions and asks archive a payload under the same event id as the rule
/// match they came from, and no list here carries a decision or an ask row, so
/// their index rows annotate nothing -- and left in, three `payload` rows for
/// one event would leave the pane to guess which was the rule's.
pub(crate) const STATS_DETAIL_BODY_BLOBS_SQL: &str = r#"
SELECT event_id, source_table, direction, content_type, original_bytes, stored_bytes, truncated, body_hash
FROM event_body_blobs
WHERE source_table NOT IN ('security_decision_events', 'security_ask_events')
AND (event_id IN (
    SELECT event_id FROM net_events WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200
)
OR event_id IN (
    SELECT event_id FROM model_calls WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200
)
OR event_id IN (
    SELECT event_id FROM tool_calls WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200
)
OR event_id IN (
    SELECT event_id FROM exec_events ORDER BY id DESC LIMIT ?1
)
OR event_id IN (
    SELECT event_id FROM security_rule_events ORDER BY id DESC LIMIT 200
))
ORDER BY event_id, direction
"#;

/// How many exec rows the stats detail view lists, and so how many it carries
/// body metadata for. `STATS_DETAIL_PROCESS_EVENTS_SQL` and
/// [`STATS_DETAIL_BODY_BLOBS_SQL`] both bind it; neither spells the number.
pub(crate) const STATS_DETAIL_PROCESS_EVENTS_LIMIT: usize = 100;

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
///
/// Two things `max_bytes` is not, both of which a caller sizing a buffer would
/// otherwise get wrong:
///
/// - It bounds the **decoded** bytes, not the bytes on the wire. A body that
///   is not valid UTF-8 goes out as base64, which is about 4/3 the size plus
///   JSON escaping, so `max_bytes=16777216` can produce a response of roughly
///   21 MiB.
/// - It is **per body**, not per response. An event has at most one body per
///   direction, so a response carries at most `directions * max_bytes`.
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
///
/// **`max_bytes` bounds the response, not the read.** `read_bodies` inflates
/// every body of the event before [`bounded_body_response`] cuts any of them,
/// so peak service memory for one request is what the archive holds for this
/// event, not what the caller asked to be sent.
///
/// That is deliberate, and it is why this read takes no `max_total_bytes` the
/// way [`capsem_logger::DbHandle::read_bodies_for_events`] does. The index is
/// `UNIQUE(event_id, source_table, direction)`, so one event has at most one
/// body per direction -- five, and the writer stages at most
/// `MAX_BODY_BLOB_BYTES` (10 MiB) of each. The ceiling is a constant nobody
/// can raise from a URL. The page read is the opposite shape: the caller names
/// up to two hundred events and the same reasoning gives two hundred times the
/// cap, which is a budget precisely because the caller chooses the multiplier.
///
/// If a direction is ever added, or the capture cap raised much, that constant
/// stops being small and this read needs the budget too.
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

/// Chunks in flight between the blocking export and the HTTP response.
///
/// Small on purpose: the channel is backpressure, not a buffer. A slow client
/// stalls the export thread instead of letting a whole session's bodies pile
/// up in the service's memory.
const EXPORT_CHANNEL_CHUNKS: usize = 4;

/// The export's writer: each `write` hands a chunk to the response stream and
/// waits with a deadline while the client is behind. `export_warc` moves this
/// writer onto a blocking task, so entering the runtime here cannot block one
/// of its worker threads.
struct ExportChannelWriter {
    chunks: tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
    runtime: tokio::runtime::Handle,
    deadline: std::time::Instant,
}

/// What a dropped receiver is reported as.
///
/// The export flattens its writer's `io::Error` into a message on the way out,
/// so the kind is gone by the time the spawned task sees it and this string is
/// all that is left to tell a cancelled download from a broken one. It is a
/// constant rather than two literals precisely because a match on a message
/// written twice is a match that drifts.
const EXPORT_CLIENT_GONE: &str = "the export client went away";
const EXPORT_BLOCKED_WRITE: std::time::Duration = std::time::Duration::from_secs(30);
const EXPORT_TOTAL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15 * 60);

impl std::io::Write for ExportChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "WARC export exceeded 15 minutes"))?;
        let wait = remaining.min(EXPORT_BLOCKED_WRITE);
        match self.runtime.block_on(tokio::time::timeout(
            wait,
            self.chunks.send(Ok(Bytes::copy_from_slice(buf))),
        )) {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, EXPORT_CLIENT_GONE)),
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "WARC export response was blocked for 30 seconds",
                ))
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `GET /vms/{id}/bodies/export.warc.gz` -- the whole session's archived
/// bodies as a WARC 1.1 file any web-archive tool can read.
///
/// Streamed rather than buffered: the export runs on a blocking task writing
/// into a small bounded channel and the response *is* that channel. A session
/// with a gigabyte of bodies costs a few chunks of resident memory, and a
/// client that reads slowly stalls the export rather than the service.
///
/// There is no event id to check here. The VM id is resolved exactly as every
/// other session route resolves it, and nothing else in the path is caller
/// input.
///
/// No `content-encoding: gzip`: the gzip framing is part of the WARC file the
/// caller asked for, not a transfer encoding, and announcing it would have
/// browsers hand the client a decompressed file under a `.gz` name.
///
/// **A failure after the first byte cannot be reported.** HTTP has no way to
/// send a status once the body has begun, so an export that fails midway ends
/// the stream and logs at error level; the client sees a truncated file. That
/// is the honest outcome available -- the WARC stops after its last complete
/// record rather than carrying a wrong one -- and the service log is the only
/// place the reason exists.
///
/// A *skipped body* is not that case and needs no log to be seen: the export
/// leaves out rows it cannot honestly describe, keeps going, and closes the
/// file with a `warcinfo` record counting the omissions by reason. The summary
/// logged below is a convenience; the file itself is the record.
pub(crate) async fn handle_bodies_warc_export(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    let db = open_ready_session_db(&state, &id, "bodies_warc_export", &db_path).await?;

    let (chunks, receiver) = tokio::sync::mpsc::channel(EXPORT_CHANNEL_CHUNKS);
    let vm_id = id.clone();
    let runtime = tokio::runtime::Handle::current();
    tokio::spawn(async move {
        match db
            .export_warc(ExportChannelWriter {
                chunks,
                runtime,
                deadline: std::time::Instant::now() + EXPORT_TOTAL_DEADLINE,
            })
            .await
        {
            Ok(summary) => info!(
                route = "/vms/{id}/bodies/export.warc.gz",
                vm_id = vm_id.as_str(),
                records = summary.records,
                bytes_written = summary.bytes_written,
                skipped = summary.skipped_count,
                "bodies_warc_export"
            ),
            // A client that closes the connection mid-download is the normal
            // way this ends -- a reviewer who saw enough, a page navigated
            // away from -- and logging it at error level would fill the log
            // with the one outcome nobody needs to investigate, beside the one
            // they do.
            Err(error) if error.contains(EXPORT_CLIENT_GONE) => info!(
                route = "/vms/{id}/bodies/export.warc.gz",
                vm_id = vm_id.as_str(),
                "session body WARC export was cancelled by the client"
            ),
            Err(error) => error!(
                route = "/vms/{id}/bodies/export.warc.gz",
                vm_id = vm_id.as_str(),
                error = %error,
                "session body WARC export failed after the response began; the client has a truncated file"
            ),
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(receiver);
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/warc")
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"capsem-session-{id}.warc.gz\""),
        )
        .body(axum::body::Body::from_stream(stream))
        .map_err(|error| ledger_route_error(&id, "bodies", "build the export response", &db_path, error))
}

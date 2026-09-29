//! What one request to a ledger-backed route may cost, and carry.
//!
//! The TUI, the tray and the web app poll `/vms/list` (behind the gateway's
//! `/status`) and `stats/summary` on a timer; `/info` and the status routes
//! report the same totals. Those are answered from the ledger handle's memory
//! and are held to zero reader round trips. `stats/detail` is fetched when a
//! user opens a session's stats; it is held to one batch, and its payload to
//! a bound.

use super::*;
use crate::ledger_routes::stats_detail::STATS_DETAIL_FIELD_CHARS;

const VM_ID: &str = "poll-cost-vm";
/// Longer than any listed field may be, so the bound shows.
const LONG_FIELD_CHARS: usize = STATS_DETAIL_FIELD_CHARS * 4;

fn model_call(arguments: String, thinking: String) -> capsem_logger::WriteOp {
    capsem_logger::WriteOp::ModelCall(capsem_logger::ModelCall {
        event_id: Some("abc123abc123".to_string()),
        timestamp: std::time::SystemTime::now(),
        provider: "anthropic".to_string(),
        protocol: Some("anthropic".to_string()),
        model: Some("poll-model".to_string()),
        process_name: Some("claude".to_string()),
        pid: Some(42),
        method: "POST".to_string(),
        path: "/v1/messages".to_string(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 1,
        tools_count: 1,
        request_bytes: 32,
        request_body: None,
        message_id: Some("msg-poll".to_string()),
        status_code: Some(200),
        text_content: Some("done".to_string()),
        thinking_content: Some(thinking),
        response_body: None,
        stop_reason: Some("tool_use".to_string()),
        input_tokens: Some(12),
        output_tokens: Some(7),
        usage_details: BTreeMap::new(),
        duration_ms: 25,
        response_bytes: 64,
        estimated_cost_usd: 0.001,
        trace_id: Some("trace-poll".to_string()),
        credential_ref: None,
        tool_calls: vec![capsem_logger::ToolCallEntry {
            event_id: Some("def456def456".to_string()),
            call_index: 0,
            call_id: "tool-poll".to_string(),
            tool_name: "Write".to_string(),
            arguments: Some(arguments),
            origin: "model".to_string(),
            trace_id: Some("trace-poll".to_string()),
        }],
        tool_responses: vec![],
    })
}

/// A running VM whose ledger holds one model call with a tool call whose
/// arguments, and a reasoning block, are far past the listed-field bound.
async fn state_with_long_fields() -> (Arc<ServiceState>, tempfile::TempDir) {
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions").join(VM_ID);
    std::fs::create_dir_all(&session_dir).unwrap();
    let db_path = session_dir.join("session.db");
    let arguments = format!(r#"{{"content":"{}"}}"#, "a".repeat(LONG_FIELD_CHARS));
    let thinking = "t".repeat(LONG_FIELD_CHARS);
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
        writer.write_blocking(model_call(arguments, thinking));
        writer.shutdown_blocking();
    })
    .await
    .unwrap();
    insert_fake_instance_with_session_dir(&state, VM_ID, 4242, session_dir);
    (state, dir)
}

/// Every route a client polls on a timer, and every route that only reports
/// a session's totals. Each is answered from the ledger handles' memory.
fn hot_routes() -> Vec<String> {
    let mut routes: Vec<String> = [
        "info",
        "status",
        "stats/summary",
        "security/status",
        "detection/status",
        "enforcement/status",
        "history/counts",
        "history/processes",
    ]
    .iter()
    .map(|route| format!("/vms/{VM_ID}/{route}"))
    .collect();
    routes.extend(
        [
            "/vms/list",
            "/status",
            "/stats",
            "/security/status",
            "/detection/status",
            "/enforcement/status",
        ]
        .map(String::from),
    );
    routes
}

async fn get(state: &Arc<ServiceState>, route: &str) -> serde_json::Value {
    let (status, body) = route_request(
        build_service_router(Arc::clone(state)),
        axum::http::Method::GET,
        route,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{route}: {body}");
    body
}

/// Serving a hot route opens no ledger, asks no reader worker and reads
/// nothing from SQLite, however many times it is polled.
///
/// A poll's cost used to scale with how often clients asked: `/info` readied
/// its session's handle twice and read the counter snapshot back from the
/// file on every request, and the service-wide security status read every
/// session's last 2000 rule matches to report six counts.
#[tokio::test]
async fn hot_routes_never_open_or_query_a_ledger() {
    let (state, _dir) = state_with_long_fields().await;
    // The first request for a session opens its handle; that is the one open.
    let routes = hot_routes();
    let mut first = Vec::new();
    for route in &routes {
        first.push(get(&state, route).await);
    }
    let session = state.session_db_handle(VM_ID).expect("the session's handle");
    let session_requests = session.reader_requests();
    let main_requests = state.profile_mutation_db.reader_requests();

    for _ in 0..8 {
        for (route, first) in routes.iter().zip(&first) {
            let polled = get(&state, route).await;
            if route.ends_with("/info") || route == "/vms/list" {
                // Uptime and the like move; the ledger's totals must not.
                continue;
            }
            assert_eq!(&polled, first, "{route} changed on an idle ledger");
        }
    }
    let after = state.session_db_handle(VM_ID).expect("the session's handle");
    assert!(
        Arc::ptr_eq(&session, &after),
        "a hot route must reuse the session's one ledger handle, never open another"
    );
    assert_eq!(
        session.reader_requests(),
        session_requests,
        "a hot route must be answered from the ledger handle's memory: no reader \
         round trip, no SQLite read, per request"
    );
    assert_eq!(
        state.profile_mutation_db.reader_requests(),
        main_requests,
        "a hot route must not read main.db while it has not changed"
    );
}

/// The totals a hot route reports are the ones the writer committed, not a
/// frozen copy: after the handle's `ready()` barrier the next poll has them.
#[tokio::test]
async fn hot_routes_report_commits_without_reading_the_ledger_per_poll() {
    let (state, _dir) = state_with_long_fields().await;
    let Json(before) = handle_info(State(Arc::clone(&state)), Path(VM_ID.into()))
        .await
        .expect("first info");
    assert_eq!(before.model_call_count, Some(1));

    let db_path = state.run_dir.join("sessions").join(VM_ID).join("session.db");
    let capsem_logger::WriteOp::ModelCall(mut call) = model_call("{}".into(), "t".into()) else {
        unreachable!("model_call builds a model call");
    };
    call.event_id = Some("abc123abc124".into());
    call.tool_calls[0].event_id = Some("def456def457".into());
    call.tool_calls[0].call_id = "tool-poll-2".into();
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
        writer.write_blocking(capsem_logger::WriteOp::ModelCall(call));
        writer.shutdown_blocking();
    })
    .await
    .unwrap();
    let session = state.session_db_handle(VM_ID).expect("the session's handle");
    session.ready().await.expect("barrier");

    let requests = session.reader_requests();
    let Json(after) = handle_info(State(Arc::clone(&state)), Path(VM_ID.into()))
        .await
        .expect("info after commit");
    assert_eq!(after.model_call_count, Some(2), "the commit must reach /info");
    assert_eq!(
        session.reader_requests(),
        requests,
        "and still without a reader round trip"
    );
}

#[tokio::test]
async fn a_stats_detail_fetch_is_one_batch() {
    let (state, _dir) = state_with_long_fields().await;
    let route = format!("/vms/{VM_ID}/stats/detail");
    let first = get(&state, &route).await;
    let session = state.session_db_handle(VM_ID).expect("the session's handle");
    let before = session.reader_requests();
    let second = get(&state, &route).await;
    assert_eq!(
        session.reader_requests() - before,
        1,
        "stats/detail is fetched on demand, not polled; a fetch is one batch of its lists, \
         which carries its own readiness check and which the handle answers from its cache \
         while idle"
    );
    assert_eq!(second, first, "the batch must answer exactly what the lists read");
}

#[tokio::test]
async fn stats_detail_bounds_every_listed_free_text_field() {
    let (state, _dir) = state_with_long_fields().await;
    let (status, body) = route_request(
        build_service_router(Arc::clone(&state)),
        axum::http::Method::GET,
        &format!("/vms/{VM_ID}/stats/detail"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let tool = &body["tool_events"][0];
    let arguments = tool["arguments"].as_str().expect("tool arguments listed");
    assert_eq!(arguments.chars().count(), STATS_DETAIL_FIELD_CHARS, "{tool}");
    assert!(
        tool["bytes"].as_u64().unwrap() > LONG_FIELD_CHARS as u64,
        "bytes keeps measuring the whole call, so a cut argument is visible: {tool}"
    );

    let items = body["interactions"]["items"].as_array().expect("interactions");
    let call = items
        .iter()
        .find(|item| item["content"]["kind"] == "tool_call")
        .expect("tool call interaction");
    let payload = &call["content"]["arguments"];
    assert_eq!(payload["status"], "truncated", "{payload}");
    assert_eq!(
        payload["content"]["raw"].as_str().unwrap().chars().count(),
        STATS_DETAIL_FIELD_CHARS
    );
    let reasoning = items
        .iter()
        .find(|item| item["content"]["blocks"][0]["kind"] == "reasoning")
        .expect("reasoning interaction");
    let block = &reasoning["content"]["blocks"][0]["payload"];
    assert_eq!(block["status"], "truncated", "{block}");
    assert_eq!(
        block["content"]["text"].as_str().unwrap().chars().count(),
        STATS_DETAIL_FIELD_CHARS
    );

    let whole = serde_json::to_vec(&body).unwrap().len();
    assert!(
        whole < LONG_FIELD_CHARS,
        "a listing is bounded by its field bound, not by the fields it lists: {whole} bytes"
    );
}

//! `GET /vms/{id}/bodies/{event_id}`: the validator, the bound, and the route.

use super::*;
use crate::ledger_routes::bodies::{bounded_body_response, validate_event_id, EventBodiesQuery};
use base64::Engine;
use capsem_logger::{BodyDirection, StoredBody};

const MIB: usize = 1024 * 1024;

fn stored(bytes: Vec<u8>, truncated: bool) -> StoredBody {
    let original_bytes = bytes.len() as u64;
    StoredBody {
        event_id: "0123456789ab".to_string(),
        source_table: "net_events".to_string(),
        direction: BodyDirection::Response,
        content_type: Some("application/json".to_string()),
        original_bytes,
        truncated,
        body_hash: format!("blake3:{}", blake3::hash(&bytes).to_hex()),
        bytes,
    }
}

#[test]
fn an_event_id_is_twelve_lowercase_hex_characters_or_a_bad_request() {
    assert_eq!(validate_event_id("0123456789ab").unwrap(), "0123456789ab");
    assert_eq!(validate_event_id("ffffffffffff").unwrap(), "ffffffffffff");

    for rejected in [
        "0123456789a",   // eleven
        "0123456789abc", // thirteen
        "0123456789AB",  // uppercase is not what the ledger writes
        "../",
        "",
        "0123456789ag", // 'g' is not hex
        "0123456789 b",
    ] {
        let AppError(status, message) = validate_event_id(rejected).expect_err(rejected);
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
        assert!(
            message.contains("12 lowercase hex characters"),
            "the refusal must name the constraint, got {message:?}"
        );
    }
}

#[test]
fn a_body_is_cut_to_the_default_budget_and_reports_its_true_size() {
    let body = "a".repeat(3 * MIB);
    let bounded = bounded_body_response(
        stored(body.into_bytes(), false),
        EventBodiesQuery::default().transport_budget(),
    );

    assert_eq!(bounded.original_bytes, 3 * MIB as u64);
    assert_eq!(bounded.stored_bytes, 3 * MIB as u64);
    assert_eq!(bounded.content.len(), MIB, "the default budget is one mebibyte");
    assert!(bounded.truncated_for_transport, "the route cut it");
    assert!(!bounded.truncated, "the capture did not");
    assert_eq!(bounded.encoding, api::bodies::BodyEncoding::Utf8);
}

#[test]
fn a_larger_budget_returns_the_whole_body() {
    let body = "a".repeat(3 * MIB);
    let budget = EventBodiesQuery {
        max_bytes: Some(4 * MIB),
    }
    .transport_budget();
    let bounded = bounded_body_response(stored(body.into_bytes(), false), budget);

    assert_eq!(budget, 4 * MIB);
    assert_eq!(bounded.content.len(), 3 * MIB);
    assert!(!bounded.truncated_for_transport);
}

#[test]
fn a_budget_above_the_ceiling_is_clamped_rather_than_refused() {
    assert_eq!(
        EventBodiesQuery {
            max_bytes: Some(usize::MAX)
        }
        .transport_budget(),
        16 * MIB
    );
    assert_eq!(
        EventBodiesQuery {
            max_bytes: Some(64 * MIB)
        }
        .transport_budget(),
        16 * MIB
    );
    // Below the ceiling the caller's number is the caller's number.
    assert_eq!(EventBodiesQuery { max_bytes: Some(7) }.transport_budget(), 7);
}

#[test]
fn capture_truncation_and_transport_truncation_are_reported_independently() {
    // Captured whole, sent whole.
    let whole = bounded_body_response(stored(b"short".to_vec(), false), MIB);
    assert!(!whole.truncated && !whole.truncated_for_transport);

    // Captured whole, cut for transport.
    let cut = bounded_body_response(stored(b"0123456789".to_vec(), false), 4);
    assert!(!cut.truncated && cut.truncated_for_transport);
    assert_eq!(cut.content, "0123");

    // Captured short, sent whole: the archive holds all there is.
    let capped = bounded_body_response(stored(b"partial".to_vec(), true), MIB);
    assert!(capped.truncated && !capped.truncated_for_transport);

    // Both.
    let both = bounded_body_response(stored(b"0123456789".to_vec(), true), 4);
    assert!(both.truncated && both.truncated_for_transport);
}

#[test]
fn text_is_cut_at_a_character_boundary() {
    // Four three-byte characters; a budget of 7 lands inside the third.
    let bounded = bounded_body_response(stored("水水水水".as_bytes().to_vec(), false), 7);
    assert_eq!(bounded.content, "水水");
    assert_eq!(bounded.encoding, api::bodies::BodyEncoding::Utf8);
    assert!(bounded.truncated_for_transport);
}

#[test]
fn bytes_that_are_not_text_come_back_as_base64() {
    let raw = vec![0xff, 0xfe, 0x00, 0x01];
    let bounded = bounded_body_response(stored(raw.clone(), false), MIB);
    assert_eq!(bounded.encoding, api::bodies::BodyEncoding::Base64);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&bounded.content)
            .expect("the route promised base64"),
        raw
    );
    assert!(!bounded.truncated_for_transport);
}

/// A session with one net event carrying both a request and a response body.
async fn session_with_bodies(state: &ServiceState, vm_id: &str, session_dir: &std::path::Path) {
    std::fs::create_dir_all(session_dir).unwrap();
    insert_fake_instance_with_session_dir(state, vm_id, std::process::id(), session_dir.to_path_buf());

    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
            event_id: Some("0123456789ab".to_string()),
            timestamp: std::time::SystemTime::now(),
            domain: "answers.example".to_string(),
            port: 443,
            decision: capsem_logger::Decision::Allowed,
            process_name: Some("curl".to_string()),
            pid: Some(12),
            method: Some("POST".to_string()),
            path: Some("/api".to_string()),
            query: None,
            status_code: Some(200),
            bytes_sent: 9,
            bytes_received: 16,
            duration_ms: 4,
            matched_rule: None,
            request_headers: Some("content-type: application/json".to_string()),
            response_headers: Some("content-type: application/json".to_string()),
            request_body: Some(br#"{"ask":1}"#.to_vec()),
            response_body: Some(br#"{"answer":"yes"}"#.to_vec()),
            conn_type: Some("http".to_string()),
            policy_mode: None,
            policy_action: Some("allow".to_string()),
            policy_rule: None,
            policy_reason: None,
            trace_id: None,
            credential_ref: None,
        }))
        .await;
    writer.shutdown_blocking();
}

#[tokio::test]
async fn the_route_returns_one_entry_per_stored_direction() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("bodies-vm");
    session_with_bodies(&state, "bodies-vm", &session_dir).await;

    let (status, body) = route_request(app, axum::http::Method::GET, "/vms/bodies-vm/bodies/0123456789ab", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let response: api::bodies::EventBodiesResponse = serde_json::from_value(body).unwrap();
    assert_eq!(response.event_id, "0123456789ab");
    let by_direction: std::collections::BTreeMap<_, _> = response
        .bodies
        .iter()
        .map(|body| (body.direction.as_str(), body))
        .collect();
    assert_eq!(
        by_direction.keys().copied().collect::<Vec<_>>(),
        vec!["request", "response"]
    );

    for (direction, expected) in [("request", r#"{"ask":1}"#), ("response", r#"{"answer":"yes"}"#)] {
        let body = by_direction[direction];
        assert_eq!(body.content, expected, "{direction}");
        assert_eq!(body.encoding, api::bodies::BodyEncoding::Utf8);
        assert_eq!(body.source_table, "net_events");
        assert_eq!(body.original_bytes, expected.len() as u64);
        assert_eq!(body.stored_bytes, expected.len() as u64);
        assert!(!body.truncated && !body.truncated_for_transport);
        assert_eq!(
            body.body_hash,
            format!("blake3:{}", blake3::hash(expected.as_bytes()).to_hex()),
            "{direction}"
        );
    }
}

#[tokio::test]
async fn an_event_with_no_archived_body_is_an_empty_list_not_a_404() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("empty-vm");
    session_with_bodies(&state, "empty-vm", &session_dir).await;

    let (status, body) = route_request(app, axum::http::Method::GET, "/vms/empty-vm/bodies/ffffffffffff", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let response: api::bodies::EventBodiesResponse = serde_json::from_value(body).unwrap();
    assert_eq!(response.event_id, "ffffffffffff");
    assert!(response.bodies.is_empty());
}

#[tokio::test]
async fn a_malformed_event_id_is_refused_before_the_database_is_touched() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    // A session directory with no session.db in it. A handler that reached
    // the ledger would fail there; the 400 below is only reachable if the id
    // was checked first.
    let session_dir = dir.path().join("sessions").join("no-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "no-db-vm", std::process::id(), session_dir.clone());

    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/no-db-vm/bodies/NOTHEXATALL",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // And the same route with a well-formed id does reach the ledger, which
    // is what makes the assertion above mean something.
    let (status, _) = route_request(app, axum::http::Method::GET, "/vms/no-db-vm/bodies/0123456789ab", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn the_route_honours_max_bytes_and_says_it_cut_the_body() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("cut-vm");
    session_with_bodies(&state, "cut-vm", &session_dir).await;

    let (status, body) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/cut-vm/bodies/0123456789ab?max_bytes=4",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let response: api::bodies::EventBodiesResponse = serde_json::from_value(body).unwrap();
    for body in &response.bodies {
        assert_eq!(body.content.len(), 4);
        assert!(body.truncated_for_transport);
        assert!(body.original_bytes > 4, "the true size is still reported");
    }
}

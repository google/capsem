//! What the writer stores for a request's headers, and what it refuses to.
//!
//! Headers are the one text column an upstream fills directly and a reader
//! trusts as a record of the exchange. They shared `MAX_FIELD_BYTES` with
//! model text until these tests existed -- 256 KB apiece, for a field that
//! averages about 300 bytes -- so what is tested here is the cap and the
//! marker that admits to it, not the round trip.
use super::*;

/// A hostile upstream cannot spend more than HEADER_BYTES of the ledger.
///
/// Headers shared `MAX_FIELD_BYTES` with model text until this test existed:
/// 256 KB per request, of a field that really averages about 300 bytes,
/// mirrored into RAM by two processes. A megabyte of padding is what the
/// attack looks like, so a megabyte of padding is what the test sends.
#[tokio::test]
async fn a_padded_header_is_capped_and_the_cut_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();

    let padded = format!("x-pad: {}", "A".repeat(1024 * 1024));
    assert!(padded.len() > 1024 * 1024);
    writer
        .write(WriteOp::NetEvent(crate::events::NetEvent {
            event_id: Some("abcdef123456".into()),
            timestamp: std::time::SystemTime::now(),
            domain: "hostile.example".into(),
            port: 443,
            decision: crate::events::Decision::Allowed,
            process_name: None,
            pid: None,
            method: Some("GET".into()),
            path: Some("/".into()),
            query: None,
            status_code: Some(200),
            bytes_sent: 0,
            bytes_received: 0,
            duration_ms: 1,
            matched_rule: None,
            request_headers: Some("host: hostile.example".into()),
            response_headers: Some(padded),
            request_body: None,
            response_body: None,
            conn_type: Some("https".into()),
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
            trace_id: None,
            credential_ref: None,
        }))
        .await;
    writer.flush().await;
    writer.shutdown_blocking();

    let conn = Connection::open(&db_path).unwrap();
    let (stored, request_len, truncated): (i64, i64, i64) = conn
        .query_row(
            "SELECT length(response_headers), length(request_headers), headers_truncated
             FROM net_events WHERE event_id = 'abcdef123456'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored, HEADER_BYTES as i64, "the pad must be cut at HEADER_BYTES");
    assert_eq!(truncated, 1, "a cut header set must say it was cut");
    assert_eq!(
        request_len,
        "host: hostile.example".len() as i64,
        "the header that fit must be stored whole"
    );
}

/// The ordinary case: nothing is cut, and the row says so.
#[tokio::test]
async fn headers_that_fit_are_not_marked_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let writer = DbWriter::open(&db_path, 64).unwrap();
    writer
        .write(WriteOp::NetEvent(crate::events::NetEvent {
            event_id: Some("abcdef654321".into()),
            timestamp: std::time::SystemTime::now(),
            domain: "example.test".into(),
            port: 443,
            decision: crate::events::Decision::Allowed,
            process_name: None,
            pid: None,
            method: Some("GET".into()),
            path: Some("/".into()),
            query: None,
            status_code: Some(200),
            bytes_sent: 0,
            bytes_received: 0,
            duration_ms: 1,
            matched_rule: None,
            request_headers: Some("host: example.test".into()),
            response_headers: Some("content-type: text/plain".into()),
            request_body: None,
            response_body: None,
            conn_type: Some("https".into()),
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
            trace_id: None,
            credential_ref: None,
        }))
        .await;
    writer.flush().await;
    writer.shutdown_blocking();

    let conn = Connection::open(&db_path).unwrap();
    let truncated: i64 = conn
        .query_row(
            "SELECT headers_truncated FROM net_events WHERE event_id = 'abcdef654321'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(truncated, 0);
}

#[test]
fn cap_headers_reports_only_a_real_cut() {
    assert_eq!(cap_headers(&None), (None, false));
    let exact = Some("h".repeat(HEADER_BYTES));
    let (value, cut) = cap_headers(&exact);
    assert_eq!(value.unwrap().len(), HEADER_BYTES);
    assert!(!cut, "a blob exactly at the limit is not truncated");
    let over = Some("h".repeat(HEADER_BYTES + 1));
    let (value, cut) = cap_headers(&over);
    assert_eq!(value.unwrap().len(), HEADER_BYTES);
    assert!(cut);
}

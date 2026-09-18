//! Security payloads in the body archive, and reading a page of them.
//!
//! Rule matches, decisions and asks each carried the event they were about
//! inline, in a table the owning process also mirrors in RAM. They archive it
//! instead, the three under their own tables for one event id, and whoever
//! wants a page of them reads it keyed and bounded.

use super::correctness::make_correctness_security_event;
use super::*;
use crate::db::BodyDirection;

/// A rule match's forensic payload averaged a kilobyte and peaked at 297 KB in
/// one real session, and every one of those bytes sat in the RAM mirror as
/// well as on disk. The row keeps what routes filter on; the payload is a body
/// like any other and belongs in the archive, fetched when someone asks.
#[tokio::test]
async fn security_rule_payload_is_archived_not_inlined() {
    let p = temp_db_path("security-rule-payload");
    let db = DbHandle::open(&p).expect("open handle");
    let mut event = make_correctness_security_event(&credential_reference("test", "not-a-real-secret"));
    event.event_id = "0123456789ab".into();
    event.event_json = r#"{"rule":"x"}"#.repeat(200);

    db.write(WriteOp::SecurityRuleEvent(event))
        .await
        .expect("write security rule event");
    db.flush().await.expect("flush");

    let columns = query_json(
        &db.query(
            "SELECT name FROM pragma_table_info('security_rule_events') ORDER BY name",
            &[],
        )
        .await
        .expect("read security_rule_events columns"),
    );
    let names: Vec<String> = columns["rows"]
        .as_array()
        .expect("column rows")
        .iter()
        .map(|row| row[0].as_str().expect("column name").to_string())
        .collect();
    assert!(
        !names.iter().any(|name| name == "event_json"),
        "the payload must leave SQLite, not sit in every row and every RAM mirror: {names:?}"
    );

    let body = db
        .read_body("0123456789ab", "security_rule_events", BodyDirection::Payload)
        .await
        .expect("read the archived payload")
        .expect("a rule match's payload is archived");
    assert_eq!(body.source_table, "security_rule_events");
    assert_eq!(body.content_type.as_deref(), Some("application/json"));
    assert!(
        body.bytes.starts_with(br#"{"rule""#),
        "the archived payload must be the payload that was written"
    );
    assert_eq!(body.bytes.len(), 2400, "and all of it, not a preview");
}

/// The decision table was measured at 7.5 MB of a 10.5 MB ledger, about 6 KB a
/// row with ~25 decisions per request, all of it mirrored in RAM -- the same
/// event a rule match carries, once more per transition. Decisions and asks
/// store their payload the way rule matches do, so there is one way to store a
/// security payload.
#[tokio::test]
async fn security_decision_and_ask_payloads_are_archived_not_inlined() {
    use crate::events::{
        SecurityAskEvent, SecurityAskPending, SecurityDecision, SecurityDecisionEvent, SecurityDecisionStage,
    };

    let p = temp_db_path("security-decision-ask-payload");
    let db = DbHandle::open(&p).expect("open handle");
    let payload = r#"{"event_type":"process.audit","process":{"command":"cat"}}"#.repeat(100);

    db.write(WriteOp::SecurityDecisionEvent(SecurityDecisionEvent {
        timestamp_unix_ms: 1_789_000_000_000,
        event_id: "0123456789ab".into(),
        event_type: "process.audit".into(),
        stage: SecurityDecisionStage::Rule,
        actor: "profiles.rules.audit".into(),
        rule_id: Some("profiles.rules.audit".into()),
        plugin_id: None,
        previous_decision: SecurityDecision::Allow,
        requested_decision: SecurityDecision::Allow,
        effective_decision: SecurityDecision::Allow,
        reason: None,
        event_json: payload.clone(),
        trace_id: Some("trace-decision".into()),
        turn_id: Some("turn-decision".into()),
        credential_ref: None,
    }))
    .await
    .expect("write security decision");
    db.write(WriteOp::SecurityAskEvent(SecurityAskEvent::pending(
        SecurityAskPending {
            timestamp_unix_ms: 1_789_000_000_001,
            ask_id: "0123456789ac".into(),
            event_id: "0123456789ab".into(),
            event_type: "process.audit".into(),
            rule_id: "profiles.rules.ask_audit".into(),
            rule_name: "ask_audit".into(),
            rule_json: "{}".into(),
            event_json: payload.clone(),
        },
    )))
    .await
    .expect("write security ask");
    db.flush().await.expect("flush");

    for table in ["security_decision_events", "security_ask_events"] {
        let columns = query_json(
            &db.query(&format!("SELECT name FROM pragma_table_info('{table}')"), &[])
                .await
                .expect("read columns"),
        );
        let names: Vec<&str> = columns["rows"]
            .as_array()
            .expect("column rows")
            .iter()
            .map(|row| row[0].as_str().expect("column name"))
            .collect();
        assert!(
            !names.contains(&"event_json"),
            "{table} must not keep the payload inline and in RAM: {names:?}"
        );

        let body = db
            .read_body("0123456789ab", table, BodyDirection::Payload)
            .await
            .expect("read the archived payload")
            .unwrap_or_else(|| panic!("{table} archives its payload"));
        assert_eq!(body.source_table, table);
        assert_eq!(body.content_type.as_deref(), Some("application/json"));
        assert_eq!(body.bytes, payload.as_bytes(), "{table}: all of it, not a preview");
    }
}

/// Write `count` security rule matches whose payloads are `{"match":N}`, and
/// return their event ids in the order they were written.
async fn write_security_payloads(db: &DbHandle, count: usize) -> Vec<String> {
    let credential_ref = credential_reference("test", "not-a-real-secret");
    let mut event_ids = Vec::with_capacity(count);
    for index in 0..count {
        let mut event = make_correctness_security_event(&credential_ref);
        event.event_id = format!("{index:012x}");
        event.event_json = format!(r#"{{"match":{index}}}"#);
        event_ids.push(event.event_id.clone());
        db.write(WriteOp::SecurityRuleEvent(event))
            .await
            .expect("write security rule event");
    }
    db.flush().await.expect("flush");
    event_ids
}

/// A route with a page of rules in hand needs a payload per row. Asking event
/// by event is a query and a blocking task each, and inflates the block that
/// holds them once per body; one call reads the page the caller named, in the
/// order the archive holds them.
#[tokio::test]
async fn bodies_for_events_read_the_named_page_in_one_pass() {
    let p = temp_db_path("security-rule-payload-page");
    let db = DbHandle::open(&p).expect("open handle");
    let event_ids = write_security_payloads(&db, 5).await;

    // Deliberately out of archive order, and a subset: the caller names rows,
    // the archive decides the read order.
    let asked: Vec<&str> = vec![event_ids[3].as_str(), event_ids[0].as_str(), event_ids[2].as_str()];
    let archived = db
        .read_bodies_for_events(&asked, "security_rule_events", BodyDirection::Payload, 1 << 20)
        .await
        .expect("read the named payloads");

    let seen: Vec<String> = archived
        .bodies
        .iter()
        .map(|body| String::from_utf8(body.bytes.clone()).expect("payloads are JSON text"))
        .collect();
    assert_eq!(
        seen,
        vec![r#"{"match":0}"#, r#"{"match":2}"#, r#"{"match":3}"#],
        "the read must return exactly the named rows, in the order the archive holds them"
    );
    assert_eq!(archived.truncated_rows, 0, "a budget this large refuses nothing");
    assert!(archived
        .bodies
        .iter()
        .all(|body| body.source_table == "security_rule_events"));
    assert_eq!(
        db.archive_blocks_inflated_for_tests(),
        1,
        "a page of payloads that share a block must cost one inflate"
    );
}

/// A row count alone is not a memory bound: 2000 rows of a 10 MiB body is
/// 20 GiB. The byte budget stops accumulating, and what it refused is counted
/// -- a caller that reported nothing would be saying the ledger was empty.
#[tokio::test]
async fn the_payload_budget_stops_accumulating_and_counts_what_it_refused() {
    let p = temp_db_path("security-rule-payload-budget");
    let db = DbHandle::open(&p).expect("open handle");
    let event_ids = write_security_payloads(&db, 3).await;
    let asked: Vec<&str> = event_ids.iter().map(String::as_str).collect();

    // Every payload is the same length, so a budget of two admits exactly two.
    let one_payload = r#"{"match":0}"#.len();
    let archived = db
        .read_bodies_for_events(&asked, "security_rule_events", BodyDirection::Payload, one_payload * 2)
        .await
        .expect("read within the budget");

    assert_eq!(archived.bodies.len(), 2, "the budget admits two of the three");
    assert_eq!(archived.truncated_rows, 1, "and says the third was refused");
    let seen: Vec<String> = archived
        .bodies
        .iter()
        .map(|body| String::from_utf8(body.bytes.clone()).expect("payloads are JSON text"))
        .collect();
    assert_eq!(
        seen,
        vec![r#"{"match":0}"#, r#"{"match":1}"#],
        "the budget takes them in archive order, not an arbitrary two"
    );
}

/// A page longer than SQLite's variable limit is read in chunks, and the two
/// bounds have to survive the seam: the budget is one budget across chunks, not
/// one per chunk, and every chunk's rows reach the result.
#[tokio::test]
async fn a_page_past_the_parameter_limit_is_one_budget_across_its_chunks() {
    let p = temp_db_path("security-rule-payload-chunks");
    let db = DbHandle::open(&p).expect("open handle");
    // 1000 events is one full chunk of 997 plus a second of 3, so anything
    // that resets per chunk or drops the tail shows up here.
    let event_ids = write_security_payloads(&db, 1000).await;
    let asked: Vec<&str> = event_ids.iter().map(String::as_str).collect();

    let whole_page = db
        .read_bodies_for_events(&asked, "security_rule_events", BodyDirection::Payload, 1 << 20)
        .await
        .expect("read the whole page");
    assert_eq!(whole_page.bodies.len(), 1000, "every chunk's rows reach the result");
    assert_eq!(whole_page.truncated_rows, 0);
    assert!(
        whole_page.bodies.iter().any(|body| body.event_id == event_ids[999]),
        "the short trailing chunk is read too, not just the full one"
    );
    // Archive order within a chunk is the order the writer staged them, and
    // the writer staged them in the order they were written.
    let first_chunk: Vec<&str> = whole_page.bodies[..997]
        .iter()
        .map(|body| body.event_id.as_str())
        .collect();
    let expected_first: Vec<&str> = event_ids[..997].iter().map(String::as_str).collect();
    assert_eq!(first_chunk, expected_first, "a chunk comes back in archive order");
    let second_chunk: Vec<&str> = whole_page.bodies[997..]
        .iter()
        .map(|body| body.event_id.as_str())
        .collect();
    let expected_second: Vec<&str> = event_ids[997..].iter().map(String::as_str).collect();
    assert_eq!(second_chunk, expected_second, "and so does the next one");

    // A budget smaller than the first chunk must stay spent when the second
    // chunk starts: a per-chunk budget would read 997 more bodies here.
    let payload_bytes: usize = whole_page.bodies.iter().map(|body| body.bytes.len()).sum();
    let half = db
        .read_bodies_for_events(
            &asked,
            "security_rule_events",
            BodyDirection::Payload,
            payload_bytes / 2,
        )
        .await
        .expect("read within half the budget");
    assert_eq!(
        half.bodies.len() + half.truncated_rows,
        1000,
        "every row is either read or counted, across the chunk seam"
    );
    assert!(
        half.truncated_rows > 0 && half.bodies.len() < 1000,
        "half the bytes cannot buy the whole page: {half:?}"
    );
    let held: usize = half.bodies.iter().map(|body| body.bytes.len()).sum();
    assert!(
        held <= payload_bytes / 2,
        "the budget is the ceiling on resident bytes, across chunks: {held} > {}",
        payload_bytes / 2
    );
}

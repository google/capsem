//! Identical bodies in one block are stored once.
//!
//! A rule match's payload, the decision it drove and an ask it raised carry
//! byte-identical events, and an event matching several rules archived its
//! payload once per rule with only one of those copies indexed. What these hold:
//! the repeats share a span and read back hash-verified, a large repeat reuses a
//! committed span in an earlier block, and everything that walks the archive --
//! retention, the WARC export -- still sees one body per index row.

use std::collections::BTreeSet;

use super::bodies::count;
use super::correctness::make_correctness_security_event;
use super::warc_export::{block, body_members, body_record_id, export_to_bytes, header};
use super::*;
use crate::db::BodyDirection;
use crate::events::{
    SecurityAskEvent, SecurityAskPending, SecurityDecision, SecurityDecisionEvent, SecurityDecisionStage,
};

const PAYLOAD: &str =
    r#"{"event_type":"http.request","http":{"host":"dedup.example"},"decision":{"effective":"allow"}}"#;

async fn write_rule(db: &DbHandle, event_id: &str, rule_id: &str, payload: &str) {
    let mut event = make_correctness_security_event(&credential_reference("test", "dedup-not-a-secret"));
    event.event_id = event_id.into();
    event.rule_id = rule_id.into();
    event.event_json = payload.into();
    db.write(WriteOp::SecurityRuleEvent(event)).await.expect("write rule");
}

async fn write_decision(db: &DbHandle, event_id: &str, payload: &str) {
    db.write(WriteOp::SecurityDecisionEvent(SecurityDecisionEvent {
        timestamp_unix_ms: 1_789_000_000_000,
        event_id: event_id.into(),
        event_type: "http.request".into(),
        stage: SecurityDecisionStage::Rule,
        actor: "profiles.rules.dedup".into(),
        rule_id: Some("profiles.rules.dedup".into()),
        plugin_id: None,
        previous_decision: SecurityDecision::Allow,
        requested_decision: SecurityDecision::Allow,
        effective_decision: SecurityDecision::Allow,
        reason: None,
        event_json: payload.into(),
        trace_id: None,
        turn_id: None,
        credential_ref: None,
    }))
    .await
    .expect("write decision");
}

async fn write_ask(db: &DbHandle, event_id: &str, ask_id: &str, payload: &str) {
    db.write(WriteOp::SecurityAskEvent(SecurityAskEvent::pending(
        SecurityAskPending {
            timestamp_unix_ms: 1_789_000_000_001,
            ask_id: ask_id.into(),
            event_id: event_id.into(),
            event_type: "http.request".into(),
            rule_id: "profiles.rules.dedup".into(),
            rule_name: "dedup".into(),
            rule_json: "{}".into(),
            event_json: payload.into(),
        },
    )))
    .await
    .expect("write ask");
}

/// Raw bytes the archive holds, summed over every block: what was appended,
/// before compression, which is exactly what deduplication saves.
async fn archived_raw_bytes(db: &DbHandle) -> i64 {
    query_json(
        &db.query("SELECT COALESCE(SUM(raw_len), 0) FROM body_blocks", &[])
            .await
            .expect("sum block sizes"),
    )["rows"][0][0]
        .as_i64()
        .expect("raw byte total")
}

/// `(source_table, block_offset, body_offset, body_len)` for every payload row
/// of one event, in table order.
async fn payload_spans(db: &DbHandle, event_id: &str) -> Vec<(String, i64, i64, i64)> {
    let value = query_json(
        &db.query(
            "SELECT source_table, block_offset, body_offset, body_len FROM event_body_blobs
             WHERE event_id = ? AND direction = 'payload' ORDER BY source_table",
            &[json!(event_id)],
        )
        .await
        .expect("read payload spans"),
    );
    value["rows"]
        .as_array()
        .expect("span rows")
        .iter()
        .map(|row| {
            (
                row[0].as_str().expect("table").to_string(),
                row[1].as_i64().expect("block offset"),
                row[2].as_i64().expect("body offset"),
                row[3].as_i64().expect("body length"),
            )
        })
        .collect()
}

async fn payload(db: &DbHandle, event_id: &str, table: &str) -> Vec<u8> {
    db.read_body(event_id, table, BodyDirection::Payload)
        .await
        .expect("read a payload: the hash check passes")
        .unwrap_or_else(|| panic!("{table} archived a payload for {event_id}"))
        .bytes
}

/// A rule match and the decision it drove carry one event. They are two index
/// rows naming one span, each read back and verified against its own row's
/// hash, and the archive holds the bytes once.
#[tokio::test]
async fn a_rule_payload_and_its_decision_share_one_copy() {
    let p = temp_db_path("dedup-rule-and-decision");
    let db = DbHandle::open(&p).expect("open handle");
    write_rule(&db, "0123456789a0", "profiles.rules.dedup", PAYLOAD).await;
    write_decision(&db, "0123456789a0", PAYLOAD).await;
    db.flush().await.expect("flush");

    let spans = payload_spans(&db, "0123456789a0").await;
    assert_eq!(spans.len(), 2, "one index row per table: {spans:?}");
    assert_eq!(
        (spans[0].1, spans[0].2, spans[0].3),
        (spans[1].1, spans[1].2, spans[1].3),
        "both rows name the same block, offset and length"
    );
    for table in ["security_rule_events", "security_decision_events"] {
        assert_eq!(payload(&db, "0123456789a0", table).await, PAYLOAD.as_bytes(), "{table}");
    }
    assert_eq!(
        archived_raw_bytes(&db).await,
        PAYLOAD.len() as i64,
        "the archive grew by one copy, not two"
    );
}

/// The orphans: an event matching three rules staged its payload three times
/// and indexed one, since the index keeps one row per (event, table,
/// direction). About a quarter of what one measured session stored was these.
#[tokio::test]
async fn an_event_matching_three_rules_stores_its_payload_once() {
    let p = temp_db_path("dedup-three-rules");
    let db = DbHandle::open(&p).expect("open handle");
    for rule_id in ["profiles.rules.one", "profiles.rules.two", "profiles.rules.three"] {
        write_rule(&db, "0123456789a1", rule_id, PAYLOAD).await;
    }
    db.flush().await.expect("flush");

    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM security_rule_events").await,
        3,
        "every match is still a row"
    );
    assert_eq!(
        archived_raw_bytes(&db).await,
        PAYLOAD.len() as i64,
        "and the payload is stored once"
    );
    assert_eq!(
        payload(&db, "0123456789a1", "security_rule_events").await,
        PAYLOAD.as_bytes()
    );
}

/// A payload big enough to be worth an index lookup (at least 512 bytes).
fn large_payload() -> String {
    format!(
        r#"{{"event_type":"http.request","http":{{"host":"dedup.example","body":"{}"}}}}"#,
        "a".repeat(700)
    )
}

/// A large body already committed in an earlier block is not stored again:
/// the newer row points into the older block and reads back hash-verified.
#[tokio::test]
async fn a_large_body_in_an_earlier_block_is_reused() {
    let p = temp_db_path("dedup-across-blocks");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    let large = large_payload();
    write_rule(&db, "0123456789a2", "profiles.rules.dedup", &large).await;
    db.flush().await.expect("flush closes the first block");
    write_decision(&db, "0123456789a2", &large).await;
    db.flush().await.expect("flush commits the second row");

    let spans = payload_spans(&db, "0123456789a2").await;
    assert_eq!(spans.len(), 2);
    assert_eq!(
        (spans[0].1, spans[0].2),
        (spans[1].1, spans[1].2),
        "one span: {spans:?}"
    );
    assert_eq!(archived_raw_bytes(&db).await, large.len() as i64, "stored once");
    for table in ["security_rule_events", "security_decision_events"] {
        assert_eq!(payload(&db, "0123456789a2", table).await, large.as_bytes(), "{table}");
    }
}

/// Below the lookup floor, a repeat in a later block is stored again: deflate
/// already absorbs it, and the lookup would cost more than it saves.
#[tokio::test]
async fn a_small_body_in_an_earlier_block_is_stored_again() {
    let p = temp_db_path("dedup-small-across-blocks");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_rule(&db, "0123456789a2", "profiles.rules.dedup", PAYLOAD).await;
    db.flush().await.expect("flush closes the first block");
    write_decision(&db, "0123456789a2", PAYLOAD).await;
    db.flush().await.expect("flush commits the second row");

    assert_eq!(archived_raw_bytes(&db).await, 2 * PAYLOAD.len() as i64);
}

/// A block past its own age is kept while a newer row still points into it,
/// and goes once that row ages out too.
#[tokio::test]
async fn retention_keeps_an_old_block_a_newer_row_still_reads() {
    let p = temp_db_path("dedup-retention-across-blocks");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    let large = large_payload();
    write_rule(&db, "0123456789b1", "profiles.rules.old", &large).await;
    db.flush().await.expect("flush the old block");
    std::thread::sleep(std::time::Duration::from_millis(20));
    let cutoff = crate::writer::format_timestamp(std::time::SystemTime::now());
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_decision(&db, "0123456789b1", &large).await;
    db.flush().await.expect("flush the newer row");

    db.retain_bodies_since(&cutoff).await.expect("retain");
    assert_eq!(
        payload(&db, "0123456789b1", "security_decision_events").await,
        large.as_bytes(),
        "the newer row still reads its bytes from the older block"
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM pragma_foreign_key_check").await, 0);

    db.retain_bodies_since("2999-01-01T00:00:00Z")
        .await
        .expect("retain nothing");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM event_body_blobs").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM body_blocks").await, 0);
}

/// Keyed by hash, not by anything weaker: two bodies of the same length that
/// differ are two bodies.
#[tokio::test]
async fn equal_length_different_bytes_are_not_reused() {
    let p = temp_db_path("dedup-equal-length");
    let db = DbHandle::open(&p).expect("open handle");
    let other: String = PAYLOAD.replace("dedup.example", "other.example");
    assert_eq!(other.len(), PAYLOAD.len(), "the fixture needs equal lengths");
    assert_ne!(other, PAYLOAD);
    write_rule(&db, "0123456789a3", "profiles.rules.dedup", PAYLOAD).await;
    write_decision(&db, "0123456789a3", &other).await;
    db.flush().await.expect("flush");

    assert_eq!(archived_raw_bytes(&db).await, 2 * PAYLOAD.len() as i64);
    assert_eq!(
        payload(&db, "0123456789a3", "security_rule_events").await,
        PAYLOAD.as_bytes()
    );
    assert_eq!(
        payload(&db, "0123456789a3", "security_decision_events").await,
        other.as_bytes(),
        "each row reads its own bytes, verified against its own hash"
    );
}

/// Retention moves and drops blocks whole. A block whose span two rows share
/// must survive a compaction with both rows still reading, and go with both of
/// them when it ages out -- leaving no row, and no foreign-key violation, behind.
#[tokio::test]
async fn retention_keeps_and_drops_a_shared_span_with_every_row_that_names_it() {
    let p = temp_db_path("dedup-retention");
    crate::writer::close_blocks_at_every_flush_for_tests(&p);
    let db = DbHandle::open(&p).expect("open handle");
    write_rule(&db, "0123456789a4", "profiles.rules.old", r#"{"old":true}"#).await;
    db.flush().await.expect("flush the old block");
    write_rule(&db, "0123456789a5", "profiles.rules.dedup", PAYLOAD).await;
    write_decision(&db, "0123456789a5", PAYLOAD).await;
    db.flush().await.expect("flush the shared block");

    let sealed = query_json(
        &db.query("SELECT sealed_at FROM body_blocks ORDER BY block_offset", &[])
            .await
            .expect("read seal times"),
    );
    let sealed: Vec<String> = sealed["rows"]
        .as_array()
        .expect("seal rows")
        .iter()
        .map(|row| row[0].as_str().expect("sealed at").to_string())
        .collect();
    assert_eq!(sealed.len(), 2);
    assert_ne!(
        sealed[0], sealed[1],
        "the flushes must be apart in time for a cutoff to sit between"
    );

    // Drop the old block; the shared one moves up and both rows follow it.
    db.retain_bodies_since(&sealed[1])
        .await
        .expect("retain the shared block");
    let spans = payload_spans(&db, "0123456789a5").await;
    assert_eq!(spans.len(), 2);
    assert_eq!((spans[0].1, spans[0].2), (spans[1].1, spans[1].2), "still one span");
    for table in ["security_rule_events", "security_decision_events"] {
        assert_eq!(payload(&db, "0123456789a5", table).await, PAYLOAD.as_bytes(), "{table}");
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM pragma_foreign_key_check").await, 0);

    // Age the shared block out too: both rows go with it.
    db.retain_bodies_since("2999-01-01T00:00:00Z")
        .await
        .expect("retain nothing");
    assert!(
        payload_spans(&db, "0123456789a5").await.is_empty(),
        "no row outlives its block"
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM pragma_foreign_key_check").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM event_body_blobs").await, 0);
}

/// The export walks index rows, not bytes: three rows sharing one span are
/// three records, each under its own id, each carrying the whole payload.
#[tokio::test]
async fn the_export_writes_every_row_of_a_shared_span_once() {
    let p = temp_db_path("dedup-warc");
    let db = DbHandle::open(&p).expect("open handle");
    write_rule(&db, "0123456789a6", "dedup-rule", PAYLOAD).await;
    write_decision(&db, "0123456789a6", PAYLOAD).await;
    write_ask(&db, "0123456789a6", "0123456789a7", PAYLOAD).await;
    db.flush().await.expect("flush");
    assert_eq!(archived_raw_bytes(&db).await, PAYLOAD.len() as i64, "one stored copy");

    let (summary, out) = export_to_bytes(&db, &p).await;
    assert!(summary.skipped.is_empty(), "{:?}", summary.skipped);
    assert_eq!(summary.records, 3);
    let members = body_members(&out);
    let ids: BTreeSet<String> = members
        .iter()
        .map(|member| header(member, "WARC-Record-ID").expect("record id"))
        .collect();
    assert_eq!(ids.len(), 3, "one record per (table, event, direction): {ids:?}");
    for table in [
        "security_rule_events",
        "security_decision_events",
        "security_ask_events",
    ] {
        let id = body_record_id(&p, table, "0123456789a6", "payload");
        let member = members
            .iter()
            .find(|member| header(member, "WARC-Record-ID").as_deref() == Some(id.as_str()))
            .unwrap_or_else(|| panic!("{table}'s record is exported as {id}"));
        assert_eq!(
            block(member),
            PAYLOAD.as_bytes(),
            "{table}: the shared bytes, read whole"
        );
    }
}

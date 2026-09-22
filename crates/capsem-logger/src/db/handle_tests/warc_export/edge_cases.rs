use super::*;

/// A body whose source row is gone has no URI it was a capture of. Inventing
/// one would put a record in the export that claims something the ledger does
/// not, so it is skipped -- and counted, because an export that silently wrote
/// fewer records than the index has is one nobody can audit.
#[tokio::test]
async fn a_body_whose_source_row_is_missing_is_skipped_and_counted() {
    let p = temp_db_path("warc-export-orphan");
    let db = DbHandle::open(&p).expect("open handle");
    for (event_id, domain) in [("0123456789ab", "kept.example"), ("0123456789ac", "orphan.example")] {
        db.write(WriteOp::NetEvent(net_event_with_response(event_id, domain, "body")))
            .await
            .expect("write event");
    }
    db.flush().await.expect("flush");
    let db = rewrite_and_reopen(db, &p, "DELETE FROM net_events WHERE event_id = '0123456789ac'").await;

    let (summary, out) = export_to_bytes(&db, &p).await;

    assert_eq!(summary.records, 1, "only the body with a source row is described");
    assert_eq!(
        body_members(&out).len(),
        1,
        "and the file holds exactly that one body record"
    );
    assert_eq!(summary.skipped.len(), 1, "{:?}", summary.skipped);
    let skipped = &summary.skipped[0];
    assert_eq!(skipped.event_id, "0123456789ac");
    assert_eq!(skipped.source_table, "net_events");
    assert_eq!(skipped.direction, "response");
    assert_eq!(skipped.reason, SkipReason::MissingSourceRow);
    assert!(
        skipped.reason.to_string().contains("target URI"),
        "the reason must be readable: {}",
        skipped.reason
    );
}

/// A source row whose timestamp is not a ledger timestamp has no WARC date
/// that would be true, and a guessed one is indistinguishable afterwards from
/// a real one.
#[tokio::test]
async fn a_body_whose_timestamp_will_not_parse_is_skipped_and_counted() {
    let p = temp_db_path("warc-export-bad-date");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789ab",
        "undated.example",
        "body",
    )))
    .await
    .expect("write event");
    db.flush().await.expect("flush");
    let db = rewrite_and_reopen(
        db,
        &p,
        "UPDATE net_events SET timestamp = 'yesterday afternoon' WHERE event_id = '0123456789ab'",
    )
    .await;

    let (summary, out) = export_to_bytes(&db, &p).await;
    assert_eq!(summary.records, 0);
    assert!(body_members(&out).is_empty(), "a skipped row writes no body record");
    assert_eq!(
        summary.skipped[0].reason,
        SkipReason::UnreadableTimestamp("yesterday afternoon".into())
    );
}

/// Archive order is the whole reason the export is one query: a block
/// holding a dozen bodies must inflate once, not a dozen times.
#[tokio::test]
async fn the_export_inflates_each_block_once() {
    let p = temp_db_path("warc-export-one-inflate");
    let db = DbHandle::open(&p).expect("open handle");
    for i in 0..12 {
        db.write(WriteOp::NetEvent(net_event_with_response(
            &format!("{i:012x}"),
            "shared.example",
            &format!("body number {i}"),
        )))
        .await
        .expect("write event");
    }
    db.flush().await.expect("flush");
    let blocks = count(&db, "SELECT COUNT(*) FROM body_blocks").await;
    assert_eq!(blocks, 1, "twelve small bodies share one block");

    db.archive_reader_reset();
    let (summary, out) = export_to_bytes(&db, &p).await;
    assert_eq!(summary.records, 12);
    assert_eq!(body_members(&out).len(), 12, "twelve body records in one file");
    assert_eq!(
        db.archive_blocks_inflated_for_tests(),
        1,
        "the export must walk the archive in its own order and inflate each block once"
    );
}

#[tokio::test]
async fn an_empty_session_exports_an_empty_file_rather_than_failing() {
    let p = temp_db_path("warc-export-empty");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(make_net_event("nobody.example", Decision::Allowed)))
        .await
        .expect("write a body-less event");
    db.flush().await.expect("flush");

    let (summary, out) = export_to_bytes(&db, &p).await;
    assert_eq!(summary.records, 0);
    assert!(summary.skipped.is_empty());
    assert!(
        body_members(&out).is_empty(),
        "no bodies is a file of two warcinfo records and nothing else"
    );
    assert_eq!(warcinfo_records(&out).len(), 2, "which still say so for themselves");
    assert_eq!(summary.bytes_written, out.len() as u64);
}

/// A request that matches two rules has two rule rows and one archived
/// payload. The export joined the body to every row sharing its event id, so
/// one body became two records under one id -- which WARC forbids, and which
/// counted the body twice. One body, one record, named by the first rule.
#[tokio::test]
async fn a_body_several_rows_share_is_exported_once() {
    let p = temp_db_path("warc-export-shared-body");
    let db = DbHandle::open(&p).expect("open handle");
    for rule_id in ["first-rule", "second-rule"] {
        let mut security = make_correctness_security_event(&credential_reference("test", "warc-shared"));
        security.event_id = "0123456789ae".into();
        security.rule_id = rule_id.into();
        security.event_json = r#"{"matched":"twice"}"#.into();
        db.write(WriteOp::SecurityRuleEvent(security))
            .await
            .expect("write security rule event");
    }
    db.flush().await.expect("flush");
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM security_rule_events").await,
        2,
        "the fixture needs two rows naming one event"
    );

    let (summary, out) = export_to_bytes(&db, &p).await;
    assert!(summary.skipped.is_empty(), "{:?}", summary.skipped);
    assert_eq!(summary.records, 1, "one archived body is one record");
    let members = body_members(&out);
    assert_eq!(members.len(), 1);
    assert_eq!(
        header(&members[0], "WARC-Target-URI").as_deref(),
        Some("capsem://security/first-rule"),
        "the payload is named by the first rule that matched it"
    );
    assert_eq!(block(&members[0]), br#"{"matched":"twice"}"#);
}

/// A rule match, the decision it drove and the ask it raised all archive a
/// `payload` for one event. Each is its own record, and the ids stay distinct
/// because the id carries the table: without it the three shared one id.
#[tokio::test]
async fn decision_and_ask_payloads_are_exported_under_their_own_ids() {
    use crate::events::{
        SecurityAskEvent, SecurityAskPending, SecurityDecision, SecurityDecisionEvent, SecurityDecisionStage,
    };

    let p = temp_db_path("warc-export-security-payloads");
    let db = DbHandle::open(&p).expect("open handle");
    let event_id = "0123456789af";

    let mut rule = make_correctness_security_event(&credential_reference("test", "warc-security"));
    rule.event_id = event_id.into();
    rule.rule_id = "ask-rule".into();
    rule.event_json = r#"{"seen_by":"rule"}"#.into();
    db.write(WriteOp::SecurityRuleEvent(rule)).await.expect("write rule");
    db.write(WriteOp::SecurityDecisionEvent(SecurityDecisionEvent {
        timestamp_unix_ms: 1_789_000_000_000,
        event_id: event_id.into(),
        event_type: "http.request".into(),
        stage: SecurityDecisionStage::Rule,
        actor: "profiles.rules.ask_rule".into(),
        rule_id: Some("profiles.rules.ask_rule".into()),
        plugin_id: None,
        previous_decision: SecurityDecision::Allow,
        requested_decision: SecurityDecision::Ask,
        effective_decision: SecurityDecision::Ask,
        reason: None,
        event_json: r#"{"seen_by":"decision"}"#.into(),
        trace_id: None,
        turn_id: None,
        credential_ref: None,
    }))
    .await
    .expect("write decision");
    db.write(WriteOp::SecurityAskEvent(SecurityAskEvent::pending(
        SecurityAskPending {
            timestamp_unix_ms: 1_789_000_000_001,
            ask_id: "0123456789b0".into(),
            event_id: event_id.into(),
            event_type: "http.request".into(),
            rule_id: "profiles.rules.ask_rule".into(),
            rule_name: "ask_rule".into(),
            rule_json: "{}".into(),
            event_json: r#"{"seen_by":"ask"}"#.into(),
        },
    )))
    .await
    .expect("write ask");
    db.flush().await.expect("flush");

    let (summary, out) = export_to_bytes(&db, &p).await;
    assert!(summary.skipped.is_empty(), "{:?}", summary.skipped);
    assert_eq!(summary.records, 3, "three archived payloads, three records");

    let members = body_members(&out);
    let ids: BTreeSet<String> = members
        .iter()
        .map(|member| header(member, "WARC-Record-ID").expect("every record has an id"))
        .collect();
    assert_eq!(ids.len(), 3, "the three records must not share an id: {ids:?}");

    for (table, uri, bytes) in [
        (
            "security_rule_events",
            "capsem://security/ask-rule",
            br#"{"seen_by":"rule"}"#.as_slice(),
        ),
        (
            "security_decision_events",
            "capsem://security-decision/profiles.rules.ask_rule",
            br#"{"seen_by":"decision"}"#.as_slice(),
        ),
        (
            "security_ask_events",
            "capsem://security-ask/0123456789b0",
            br#"{"seen_by":"ask"}"#.as_slice(),
        ),
    ] {
        let id = body_record_id(&p, table, event_id, "payload");
        let member = members
            .iter()
            .find(|member| header(member, "WARC-Record-ID").as_deref() == Some(id.as_str()))
            .unwrap_or_else(|| panic!("{table}'s payload is exported as {id}"));
        assert_eq!(header(member, "WARC-Target-URI").as_deref(), Some(uri), "{table}");
        assert_eq!(
            block(member),
            bytes,
            "{table}: the record's block is the archived payload"
        );
    }
}

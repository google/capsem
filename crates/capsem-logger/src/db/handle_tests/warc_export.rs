//! Exporting a session's bodies as a WARC file.
//!
//! The format itself is proved in `capsem-archive`, and against `warcio` in
//! `external_warc_reader.rs`. What these hold is the mapping: which ledger row
//! becomes which record, which rows are refused and counted, and that the walk
//! still costs one inflate per block.

use std::collections::BTreeSet;
use std::io::Read;

use super::*;
use crate::db::SkipReason;
use crate::events::{ExecEvent, ExecEventComplete};

use super::bodies::{count, net_event_with_response};
use super::correctness::{make_correctness_security_event, make_correctness_tool_response_model_call};

/// Split a `.warc.gz` into its gzip members and inflate each one, the way a
/// tool seeking to a single record does.
pub(super) fn members(mut bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let mut decoder = flate2::bufread::GzDecoder::new(bytes);
        let mut member = Vec::new();
        decoder.read_to_end(&mut member).expect("a complete gzip member");
        bytes = decoder.into_inner();
        out.push(member);
    }
    out
}

/// The value of one header in a decoded record, or `None` when absent.
pub(super) fn header(member: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(member);
    let prefix = format!("{name}: ");
    text.split("\r\n")
        .take_while(|line| !line.is_empty())
        .find_map(|line| line.strip_prefix(prefix.as_str()).map(str::to_string))
}

/// Everything after the blank line, minus the two CRLFs that end the record.
pub(super) fn block(member: &[u8]) -> Vec<u8> {
    let start = member
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a blank line ends the headers")
        + 4;
    member[start..member.len() - 4].to_vec()
}

/// Only the body records: the file is bracketed by two `warcinfo` records that
/// describe it and correspond to no index row.
pub(super) fn body_members(bytes: &[u8]) -> Vec<Vec<u8>> {
    members(bytes)
        .into_iter()
        .filter(|member| header(member, "WARC-Type").as_deref() == Some("resource"))
        .collect()
}

/// The `application/warc-fields` block of a `warcinfo` record, as a map.
pub(super) fn warcinfo_fields(member: &[u8]) -> BTreeMap<String, String> {
    String::from_utf8(block(member))
        .expect("warc-fields are text")
        .split("\r\n")
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// The leading and trailing `warcinfo` records, in file order.
pub(super) fn warcinfo_records(bytes: &[u8]) -> Vec<Vec<u8>> {
    members(bytes)
        .into_iter()
        .filter(|member| header(member, "WARC-Type").as_deref() == Some("warcinfo"))
        .collect()
}

/// Export to a file beside the ledger and read it back.
///
/// A file rather than a `Vec`, because `export_warc` moves its writer onto a
/// blocking thread and a borrowed buffer cannot go there -- which is also how
/// every real caller will use it.
pub(super) async fn export_to_bytes(db: &DbHandle, db_path: &std::path::Path) -> (crate::ExportSummary, Vec<u8>) {
    let out = db_path.with_extension("warc.gz");
    let _ = std::fs::remove_file(&out);
    let file = std::fs::File::create(&out).expect("create the export file");
    let summary = db.export_warc(file).await.expect("export the session");
    let bytes = std::fs::read(&out).expect("read the export back");
    let _ = std::fs::remove_file(&out);
    (summary, bytes)
}

/// The `WARC-Record-ID` header a body of the ledger at `db_path` is exported
/// under, angle brackets included. The session is the ledger's directory.
pub(super) fn body_record_id(db_path: &std::path::Path, source_table: &str, event_id: &str, direction: &str) -> String {
    let session = db_path
        .parent()
        .and_then(std::path::Path::file_name)
        .expect("a ledger lives in a session directory")
        .to_string_lossy();
    format!("<urn:capsem:{session}:{source_table}:{event_id}:{direction}>")
}

/// Close the owning handle, edit the ledger on disk, and reopen it as an
/// external reader.
///
/// The fixtures below need a ledger in a state the writer will not produce --
/// a body whose source row is gone, a timestamp that is not one. The owning
/// handle answers reads from its RAM mirror, so an edit made underneath it is
/// invisible to it and the fixture would silently not exist; closing it first
/// and reading the disk afterwards is what makes the edit the thing under
/// test.
pub(super) async fn rewrite_and_reopen(db: DbHandle, path: &std::path::Path, sql: &str) -> DbHandle {
    drop(db);
    let conn = rusqlite::Connection::open(path).expect("open the ledger directly to build the fixture");
    conn.execute_batch(sql).expect("apply the fixture edit");
    drop(conn);
    let reopened = DbHandle::open_external_reader(path).expect("reopen the edited ledger");
    reopened.ready().await.expect("the edited ledger is readable");
    reopened
}

/// A session with a body in each of the four tables that carry one in normal
/// use. Returns the tool response's event id, which the ledger assigns.
async fn write_a_session_of_every_kind(db: &DbHandle) -> String {
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789ab",
        "answers.example",
        r#"{"answer":"yes"}"#,
    )))
    .await
    .expect("write net event");

    let mut model = make_correctness_tool_response_model_call(&credential_reference("test", "warc-not-a-secret"));
    model.event_id = Some("0123456789ac".into());
    model.provider = "api.model.example".into();
    model.path = "/v1/messages".into();
    model.request_body = Some(br#"{"model":"a-model","messages":[]}"#.to_vec());
    model.tool_responses = vec![ToolResponseEntry {
        event_id: None,
        call_id: "warc-tool-call-1".into(),
        content_preview: Some("t".repeat(64 * 1024)),
        is_error: false,
        trace_id: None,
        credential_ref: None,
    }];
    db.write(WriteOp::ModelCall(model)).await.expect("write model call");

    let mut security = make_correctness_security_event(&credential_reference("test", "warc-not-a-secret"));
    security.event_id = "0123456789ad".into();
    security.rule_id = "block-secrets".into();
    security.event_json = r#"{"rule":"matched"}"#.repeat(60);
    db.write(WriteOp::SecurityRuleEvent(security))
        .await
        .expect("write security rule event");

    db.flush().await.expect("flush");

    let rows = query_json(
        &db.query(
            "SELECT event_id FROM tool_responses WHERE call_id = ?",
            &[json!("warc-tool-call-1")],
        )
        .await
        .expect("read tool response row"),
    );
    rows["rows"][0][0].as_str().expect("tool response event_id").to_string()
}

#[tokio::test]
async fn every_archived_body_becomes_one_record_named_by_where_it_came_from() {
    let p = temp_db_path("warc-export-every-kind");
    let db = DbHandle::open(&p).expect("open handle");
    let tool_response_event_id = write_a_session_of_every_kind(&db).await;

    let (summary, out) = export_to_bytes(&db, &p).await;

    let total = count(&db, "SELECT COUNT(*) FROM event_body_blobs").await;
    assert!(summary.skipped.is_empty(), "{:?}", summary.skipped);
    assert_eq!(
        summary.records, total as u64,
        "every index row must become exactly one record"
    );

    let members = body_members(&out);
    assert_eq!(
        members.len(),
        summary.records as usize,
        "one gzip member per body record"
    );
    assert_eq!(
        summary.bytes_written,
        out.len() as u64,
        "the summary must count what was actually written"
    );

    let described: BTreeMap<String, String> = members
        .iter()
        .map(|member| {
            (
                header(member, "WARC-Record-ID").expect("every record has an id"),
                header(member, "WARC-Target-URI").expect("every record has a target"),
            )
        })
        .collect();
    assert_eq!(
        described
            .get(&body_record_id(&p, "net_events", "0123456789ab", "response"))
            .map(String::as_str),
        Some("https://answers.example/api"),
        "{described:?}"
    );
    assert_eq!(
        described
            .get(&body_record_id(&p, "model_calls", "0123456789ac", "request"))
            .map(String::as_str),
        Some("https://api.model.example/v1/messages"),
        "{described:?}"
    );
    assert_eq!(
        described
            .get(&body_record_id(&p, "security_rule_events", "0123456789ad", "payload"))
            .map(String::as_str),
        Some("capsem://security/block-secrets"),
        "{described:?}"
    );
    assert_eq!(
        described
            .get(&body_record_id(
                &p,
                "tool_responses",
                &tool_response_event_id,
                "response"
            ))
            .map(String::as_str),
        Some("capsem://tool-response/warc-tool-call-1"),
        "{described:?}"
    );

    for member in &members {
        let date = header(member, "WARC-Date").expect("every record has a date");
        assert_eq!(date.len(), 20, "a WARC date is whole seconds in UTC: {date}");
        assert!(date.ends_with('Z') && date.contains('T'), "{date}");
        assert_eq!(header(member, "WARC-Type").as_deref(), Some("resource"));
    }

    let net = members
        .iter()
        .find(|member| {
            header(member, "WARC-Record-ID") == Some(body_record_id(&p, "net_events", "0123456789ab", "response"))
        })
        .expect("the net event's record");
    assert_eq!(block(net), br#"{"answer":"yes"}"#, "the block is the archived body");
}

/// WARC requires record ids to be globally unique, and an event id is only
/// unique within its ledger. Two sessions that happen to assign the same one
/// must still export records a merged collection can tell apart.
#[tokio::test]
async fn two_sessions_with_the_same_event_id_export_distinct_record_ids() {
    let mut ids = Vec::new();
    for session in ["warc-session-a", "warc-session-b"] {
        let dir = std::env::temp_dir().join(format!("capsem-test-{session}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the session directory");
        let p = dir.join("session.db");
        let db = DbHandle::open(&p).expect("open handle");
        db.write(WriteOp::NetEvent(net_event_with_response(
            "0123456789ab",
            "same.example",
            "the same event id in both sessions",
        )))
        .await
        .expect("write net event");
        db.flush().await.expect("flush");

        let (summary, out) = export_to_bytes(&db, &p).await;
        assert_eq!(summary.records, 1, "{:?}", summary.skipped);
        let id = header(&body_members(&out)[0], "WARC-Record-ID").expect("the record has an id");
        assert_eq!(
            id,
            format!(
                "<urn:capsem:{}:net_events:0123456789ab:response>",
                dir.file_name().unwrap().to_string_lossy()
            ),
            "the id names the session, then the table, the event and the direction"
        );
        ids.push(id);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert_ne!(ids[0], ids[1], "a merged collection must not see one id twice");
}

/// A caller streaming the export to a client drops the summary on the floor,
/// so the file has to say for itself what it holds and what it left out.
#[tokio::test]
async fn the_file_describes_itself_with_a_warcinfo_at_each_end() {
    let p = temp_db_path("warc-export-warcinfo");
    let db = DbHandle::open(&p).expect("open handle");
    write_a_session_of_every_kind(&db).await;

    let (summary, out) = export_to_bytes(&db, &p).await;
    let all = members(&out);
    assert_eq!(
        header(&all[0], "WARC-Type").as_deref(),
        Some("warcinfo"),
        "the file opens by describing itself"
    );
    assert_eq!(
        header(all.last().expect("records"), "WARC-Type").as_deref(),
        Some("warcinfo"),
        "and closes by saying what it left out"
    );
    assert_eq!(all.len(), summary.records as usize + 2);

    let opening = warcinfo_fields(&all[0]);
    assert!(opening["software"].starts_with("capsem/"), "{:?}", opening["software"]);
    assert_eq!(opening["format"], "WARC File Format 1.1");
    assert_eq!(opening["capsem-block-digest-algorithm"], "blake3");
    assert!(opening.contains_key("capsem-session"), "{opening:?}");
    assert!(
        opening["capsem-exported-at"].ends_with('Z'),
        "{:?}",
        opening["capsem-exported-at"]
    );

    let closing = warcinfo_fields(all.last().expect("records"));
    assert_eq!(closing["capsem-records"], summary.records.to_string());
    assert_eq!(closing["capsem-skipped"], "0");
    assert!(
        !closing.keys().any(|name| name.starts_with("capsem-skipped-")),
        "a clean export names no reasons: {closing:?}"
    );

    // Neither warcinfo claims to have captured anything, because it did not.
    for record in warcinfo_records(&out) {
        assert_eq!(header(&record, "WARC-Target-URI"), None);
        assert_eq!(
            header(&record, "Content-Type").as_deref(),
            Some("application/warc-fields")
        );
    }
}

/// `tool_calls.tool_name` and `tool_responses.call_id` are written verbatim
/// from model and MCP JSON, with no CHECK forbidding a newline, so a
/// counterparty chooses them. A URI built from one cannot be a WARC header --
/// and before this was a skip, the refusal aborted the whole export and left a
/// reviewer a truncated file and no summary. A hostile name must cost exactly
/// the records it names.
#[tokio::test]
async fn a_hostile_tool_name_costs_its_own_records_not_the_export() {
    let p = temp_db_path("warc-export-hostile-tool-name");
    let db = DbHandle::open(&p).expect("open handle");
    write_a_session_of_every_kind(&db).await;
    db.write(WriteOp::McpCall(mcp_call_with_bodies())).await.expect("write");
    db.flush().await.expect("flush");
    let clean = export_to_bytes(&db, &p).await.0.records;

    let forged = "char(13) || char(10) || 'WARC-Type: revisit'";
    let db = rewrite_and_reopen(
        db,
        &p,
        &format!(
            "UPDATE tool_calls SET tool_name = 'search' || {forged};
             UPDATE tool_responses SET call_id = 'call-1' || {forged};"
        ),
    )
    .await;

    let (summary, out) = export_to_bytes(&db, &p).await;
    // Two tool_calls bodies (request and response) and one tool_response body.
    assert_eq!(
        summary.records,
        clean - 3,
        "every other body must still reach the reviewer"
    );
    assert_eq!(body_members(&out).len(), summary.records as usize);
    assert_eq!(summary.skipped.len(), 3, "{:?}", summary.skipped);
    let tables: BTreeSet<&str> = summary.skipped.iter().map(|body| body.source_table.as_str()).collect();
    assert_eq!(
        tables,
        BTreeSet::from(["tool_calls", "tool_responses"]),
        "both verbatim columns must be covered"
    );
    for body in &summary.skipped {
        assert!(
            matches!(&body.reason, SkipReason::UnrepresentableUri(uri) if uri.contains("WARC-Type: revisit")),
            "{:?}",
            body.reason
        );
    }
    assert!(
        !members(&out)
            .iter()
            .any(|member| String::from_utf8_lossy(member).contains("WARC-Type: revisit")),
        "the forged header must not reach the file by any path"
    );
    assert_eq!(
        warcinfo_fields(members(&out).last().expect("records"))["capsem-skipped-unrepresentable-uri"],
        "3"
    );
}

fn mcp_call_with_bodies() -> crate::events::McpCall {
    crate::events::McpCall {
        event_id: Some("0123456789b7".into()),
        timestamp: SystemTime::now(),
        server_name: "files".into(),
        method: "tools/call".into(),
        tool_name: Some("search".into()),
        request_id: Some("1".into()),
        request_preview: Some(r#"{"query":"anything"}"#.into()),
        response_preview: Some(r#"{"results":[]}"#.into()),
        decision: "allowed".into(),
        duration_ms: 4,
        error_message: None,
        process_name: Some("agent".into()),
        bytes_sent: 20,
        bytes_received: 14,
        transport: "vsock_frame".into(),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: Some("trace-warc-mcp".into()),
        credential_ref: None,
    }
}

/// One corrupt block must not deny a reviewer the other records. The body is
/// left out, because it is not that body, and the file says so at the end.
#[tokio::test]
async fn a_body_that_fails_its_hash_check_is_skipped_and_the_file_says_so() {
    let p = temp_db_path("warc-export-corrupt-body");
    let db = DbHandle::open(&p).expect("open handle");
    for (event_id, domain) in [("0123456789ab", "good.example"), ("0123456789ac", "bad.example")] {
        db.write(WriteOp::NetEvent(net_event_with_response(event_id, domain, "a body")))
            .await
            .expect("write event");
    }
    db.flush().await.expect("flush");

    // The index row now claims a hash the archived bytes do not have, which is
    // what a damaged block looks like from the reader's side.
    let db = rewrite_and_reopen(
        db,
        &p,
        "UPDATE event_body_blobs SET body_hash = 'blake3:' || hex(zeroblob(32))
         WHERE event_id = '0123456789ac'",
    )
    .await;

    let (summary, out) = export_to_bytes(&db, &p).await;
    assert_eq!(summary.records, 1, "the undamaged body still reaches the reviewer");
    assert_eq!(body_members(&out).len(), 1);
    assert_eq!(summary.skipped.len(), 1, "{:?}", summary.skipped);
    assert_eq!(summary.skipped[0].event_id, "0123456789ac");
    assert!(
        matches!(summary.skipped[0].reason, SkipReason::CorruptBody(_)),
        "{:?}",
        summary.skipped[0].reason
    );

    let closing = warcinfo_fields(members(&out).last().expect("records"));
    assert_eq!(closing["capsem-records"], "1");
    assert_eq!(closing["capsem-skipped"], "1");
    assert_eq!(
        closing["capsem-skipped-corrupt-body"], "1",
        "a reader holding only the file must be able to see the omission: {closing:?}"
    );
}

/// Two index rows damaged the same way, one step apart in how far the edit
/// went: one span still lands inside its block and comes back as bytes that
/// fail the hash, the other lands outside it and the archive refuses. They are
/// the same damage and must cost the same thing -- one record each. Before
/// this, the first was a counted skip and the second ended the whole export.
#[tokio::test]
async fn neighbouring_corruption_costs_two_rows_not_the_export() {
    let p = temp_db_path("warc-export-neighbouring-corruption");
    let db = DbHandle::open(&p).expect("open handle");
    for index in 0..4 {
        db.write(WriteOp::NetEvent(net_event_with_response(
            &format!("{index:012x}"),
            "neighbours.example",
            &format!("body number {index} is long enough to have an inside"),
        )))
        .await
        .expect("write event");
    }
    db.flush().await.expect("flush");

    let db = rewrite_and_reopen(
        db,
        &p,
        // 000000000001: still inside its block, so the archive returns bytes
        // and the hash check is what catches it.
        // 000000000002: past the end of the block, so the archive refuses.
        "UPDATE event_body_blobs SET body_offset = body_offset + 1
           WHERE event_id = '000000000001';
         UPDATE event_body_blobs SET body_offset = 1000000000
           WHERE event_id = '000000000002';",
    )
    .await;

    let (summary, out) = export_to_bytes(&db, &p).await;
    assert_eq!(summary.records, 2, "the two undamaged bodies still reach the reviewer");
    assert_eq!(body_members(&out).len(), 2);
    assert_eq!(summary.skipped.len(), 2, "{:?}", summary.skipped);

    let by_event: BTreeMap<&str, &SkipReason> = summary
        .skipped
        .iter()
        .map(|body| (body.event_id.as_str(), &body.reason))
        .collect();
    assert!(
        matches!(by_event["000000000001"], SkipReason::CorruptBody(_)),
        "an in-range edit is caught by the hash: {by_event:?}"
    );
    assert!(
        matches!(by_event["000000000002"], SkipReason::UnreadableBody(_)),
        "an out-of-range edit is refused by the archive: {by_event:?}"
    );

    let closing = warcinfo_fields(members(&out).last().expect("records"));
    assert_eq!(closing["capsem-skipped"], "2");
    assert_eq!(closing["capsem-skipped-corrupt-body"], "1");
    assert_eq!(closing["capsem-skipped-unreadable-body"], "1");
}

/// The `warcinfo` ids were once byte-identical in every export ever produced,
/// which WARC forbids and which collides the moment two exports are merged
/// into one collection.
#[tokio::test]
async fn two_exports_of_one_session_do_not_share_a_warcinfo_id() {
    let p = temp_db_path("warc-export-unique-warcinfo-ids");
    let db = DbHandle::open(&p).expect("open handle");
    write_a_session_of_every_kind(&db).await;

    let ids = |bytes: &[u8]| -> Vec<String> {
        warcinfo_records(bytes)
            .iter()
            .map(|record| header(record, "WARC-Record-ID").expect("every record has an id"))
            .collect()
    };
    let first = ids(&export_to_bytes(&db, &p).await.1);
    let second = ids(&export_to_bytes(&db, &p).await.1);

    assert_eq!(first.len(), 2);
    assert_ne!(first[0], first[1], "the two ends of one export differ: {first:?}");
    let shared: Vec<&String> = first.iter().filter(|id| second.contains(id)).collect();
    assert!(
        shared.is_empty(),
        "two exports of one session must share no record id: {shared:?}"
    );
    for id in first.iter().chain(&second) {
        assert!(id.contains("warcinfo"), "the id must still say what it is: {id}");
    }
}

/// The wire format of the two `warcinfo` records. These names sit in an
/// exported artifact, so a reader parsing last month's file must find this
/// month's spelling -- there is no version negotiation and nobody to ask.
#[tokio::test]
async fn the_warcinfo_field_names_and_skip_labels_are_the_documented_wire_format() {
    assert_eq!(
        [
            SkipReason::MissingSourceRow,
            SkipReason::UnreadableTimestamp(String::new()),
            SkipReason::UnrepresentableUri(String::new()),
            SkipReason::CorruptBody(String::new()),
            SkipReason::UnreadableBody(String::new()),
        ]
        .map(|reason| reason.label()),
        [
            "missing-source-row",
            "unreadable-timestamp",
            "unrepresentable-uri",
            "corrupt-body",
            "unreadable-body",
        ],
        "every skip reason's label is wire format and may not be renamed silently"
    );

    let p = temp_db_path("warc-export-wire-format");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789ab",
        "wire.example",
        "a body",
    )))
    .await
    .expect("write event");
    db.flush().await.expect("flush");
    let db = rewrite_and_reopen(
        db,
        &p,
        "UPDATE net_events SET timestamp = 'not a timestamp' WHERE event_id = '0123456789ab'",
    )
    .await;

    let (_, out) = export_to_bytes(&db, &p).await;
    let all = members(&out);
    assert_eq!(
        warcinfo_fields(&all[0]).keys().cloned().collect::<Vec<_>>(),
        vec![
            "capsem-block-digest-algorithm",
            "capsem-exported-at",
            "capsem-session",
            "format",
            "software",
        ],
        "the opening record's field names are wire format"
    );
    assert_eq!(
        warcinfo_fields(all.last().expect("records"))
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "capsem-records",
            "capsem-skipped",
            "capsem-skipped-unreadable-timestamp",
            "software",
        ],
        "the closing record's field names are wire format, one per reason that fired"
    );
}

/// An exec event's output is a body like any other, and its URI names the
/// exec it came from rather than pretending to be a URL.
#[tokio::test]
async fn exec_output_is_exported_under_a_capsem_uri_per_stream() {
    let p = temp_db_path("warc-export-exec");
    let db = DbHandle::open(&p).expect("open handle");
    let big = "o".repeat(64 * 1024);
    db.write(WriteOp::ExecEvent(ExecEvent {
        event_id: Some("0123456789b1".into()),
        timestamp: SystemTime::now(),
        exec_id: 4242,
        command: "print a lot".into(),
        source: "api".into(),
        trace_id: None,
        process_name: Some("bash".into()),
        credential_ref: None,
    }))
    .await
    .expect("write exec start");
    db.write(WriteOp::ExecEventComplete(ExecEventComplete {
        exec_id: 4242,
        exit_code: 0,
        duration_ms: 12,
        stdout_preview: Some(big.clone()),
        stderr_preview: Some(big.clone()),
        stdout_bytes: big.len() as u64,
        stderr_bytes: big.len() as u64,
        pid: Some(99),
    }))
    .await
    .expect("write exec completion");
    db.flush().await.expect("flush");

    let (summary, out) = export_to_bytes(&db, &p).await;
    assert_eq!(summary.records, 2, "stdout and stderr are two bodies");

    for member in body_members(&out) {
        assert_eq!(
            header(&member, "WARC-Target-URI").as_deref(),
            Some("capsem://exec/4242"),
            "both streams name the exec they came from"
        );
        assert_eq!(block(&member), big.as_bytes());
    }
}

/// A body that was cut carries the spec's own field for it, and a
/// `Content-Length` that still equals what the record actually holds -- a
/// reader told to expect the original length would look for bytes that are not
/// there.
#[tokio::test]
async fn a_truncated_body_says_so_and_still_measures_itself() {
    let p = temp_db_path("warc-export-truncated");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789ab",
        "big.example",
        "x",
    )))
    .await
    .expect("write event");
    db.flush().await.expect("flush");

    // The writer only truncates at the 10 MiB body cap, and a 10 MiB fixture
    // to observe one header is not worth it. The index row is what the export
    // reads, so the fixture marks the row.
    let db = rewrite_and_reopen(
        db,
        &p,
        "UPDATE event_body_blobs SET truncated = 1, original_bytes = 4096 WHERE event_id = '0123456789ab'",
    )
    .await;

    let (summary, out) = export_to_bytes(&db, &p).await;
    assert_eq!(summary.records, 1);
    let member = body_members(&out).pop().expect("one record");
    assert_eq!(header(&member, "WARC-Truncated").as_deref(), Some("length"));
    assert_eq!(
        header(&member, "Content-Length").as_deref(),
        Some("1"),
        "the length is what the record carries, not what it was cut from"
    );
}

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

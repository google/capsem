//! Exporting a session's bodies as a WARC file.
//!
//! The format itself is proved in `capsem-archive`, and against `warcio` in
//! `external_warc_reader.rs`. What these hold is the mapping: which ledger row
//! becomes which record, which rows are refused and counted, and that the walk
//! still costs one inflate per block.

use std::collections::BTreeSet;
use std::io::{Read, Write};

use super::*;

mod edge_cases;
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

struct CancelledWriter;

impl Write for CancelledWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "cancelled test export",
        ))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct HeldWriter {
    reached: Option<std::sync::mpsc::SyncSender<()>>,
    resume: Option<std::sync::mpsc::Receiver<()>>,
}

impl Write for HeldWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(reached) = self.reached.take() {
            reached.send(()).unwrap();
            self.resume.take().unwrap().recv().unwrap();
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn held_writer() -> (
    HeldWriter,
    std::sync::mpsc::Receiver<()>,
    std::sync::mpsc::SyncSender<()>,
) {
    let (reached_tx, reached_rx) = std::sync::mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(0);
    (
        HeldWriter {
            reached: Some(reached_tx),
            resume: Some(resume_rx),
        },
        reached_rx,
        resume_tx,
    )
}

async fn wait_until_held(reached: std::sync::mpsc::Receiver<()>, which: &'static str) {
    tokio::task::spawn_blocking(move || reached.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap_or_else(|error| panic!("{which} export did not reach its writer: {error}"));
}

async fn session_with_one_body(name: &str, event_id: &str) -> DbHandle {
    let path = temp_db_path(name);
    let db = DbHandle::open(&path).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        event_id,
        "permit.example",
        "body",
    )))
    .await
    .expect("write event");
    db.flush().await.expect("flush");
    db
}

#[tokio::test]
async fn a_second_export_for_one_session_is_rejected_without_a_queue_and_the_permit_is_released() {
    let first = session_with_one_body("warc-permit-first", "0e0e0e0e0e01").await;

    let (first_writer, first_reached, first_resume) = held_writer();
    let first_export = {
        let db = first.clone();
        tokio::spawn(async move { db.export_warc(first_writer).await })
    };
    wait_until_held(first_reached, "first").await;
    let same_session = first
        .export_warc(std::io::sink())
        .await
        .expect_err("a second export for one session is rejected");
    assert!(
        same_session.contains("already active for this session"),
        "{same_session}"
    );

    first_resume.send(()).unwrap();
    first_export.await.unwrap().unwrap();
    first
        .export_warc(std::io::sink())
        .await
        .expect("the released session permit is immediately reusable");
}

#[tokio::test]
async fn a_cancelled_export_releases_its_descriptor_spool_and_capture_lock() {
    let p = temp_db_path("warc-cancel-release");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0c0c0c0c0c01",
        "cancel.example",
        "body",
    )))
    .await
    .expect("write event");
    db.flush().await.expect("flush");

    let error = db
        .export_warc(CancelledWriter)
        .await
        .expect_err("the output was cancelled");
    assert!(error.contains("cancelled test export"), "{error}");
    db.retain_bodies_since("2999-01-01T00:00:00Z")
        .await
        .expect("publication immediately acquires EX after cancellation");
    let generations = std::fs::read_dir(p.with_extension("bodies"))
        .expect("list archive generations")
        .count();
    assert_eq!(generations, 1, "the cancelled capture pins no old generation");
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
    security.event_json = format!(r#"{{"rule":"{}"}}"#, "matched".repeat(60));
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

    let security = members
        .iter()
        .find(|member| {
            header(member, "WARC-Record-ID")
                == Some(body_record_id(&p, "security_rule_events", "0123456789ad", "payload"))
        })
        .expect("the security event's record");
    assert_eq!(header(security, "Content-Type").as_deref(), Some("application/json"));
    let forensic: serde_json::Value = serde_json::from_slice(&block(security)).unwrap();
    assert_eq!(forensic["event_type"], "http.request");
    assert_eq!(forensic["rule"], "matched".repeat(60));
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

//! Exporting a session's bodies as a WARC file.
//!
//! The format itself is proved in `capsem-archive`, and against `warcio` in
//! `external_warc_reader.rs`. What these hold is the mapping: which ledger row
//! becomes which record, which rows are refused and counted, and that the walk
//! still costs one inflate per block.

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

/// Close the owning handle, edit the ledger on disk, and reopen it as an
/// external reader.
///
/// The fixtures below need a ledger in a state the writer will not produce --
/// a body whose source row is gone, a timestamp that is not one. The owning
/// handle answers reads from its RAM mirror, so an edit made underneath it is
/// invisible to it and the fixture would silently not exist; closing it first
/// and reading the disk afterwards is what makes the edit the thing under
/// test.
async fn rewrite_and_reopen(db: DbHandle, path: &std::path::Path, sql: &str) -> DbHandle {
    drop(db);
    let conn = rusqlite::Connection::open(path).expect("open the ledger directly to build the fixture");
    conn.execute(sql, []).expect("apply the fixture edit");
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

    let members = members(&out);
    assert_eq!(members.len(), summary.records as usize, "one gzip member per record");
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
        described.get("<urn:capsem:0123456789ab:response>").map(String::as_str),
        Some("https://answers.example/api"),
        "{described:?}"
    );
    assert_eq!(
        described.get("<urn:capsem:0123456789ac:request>").map(String::as_str),
        Some("https://api.model.example/v1/messages"),
        "{described:?}"
    );
    assert_eq!(
        described.get("<urn:capsem:0123456789ad:payload>").map(String::as_str),
        Some("capsem://security/block-secrets"),
        "{described:?}"
    );
    assert_eq!(
        described
            .get(&format!("<urn:capsem:{tool_response_event_id}:response>"))
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
        .find(|member| header(member, "WARC-Record-ID").as_deref() == Some("<urn:capsem:0123456789ab:response>"))
        .expect("the net event's record");
    assert_eq!(block(net), br#"{"answer":"yes"}"#, "the block is the archived body");
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

    for member in members(&out) {
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
    let member = members(&out).pop().expect("one record");
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
    assert_eq!(members(&out).len(), 1, "and the file holds exactly that one record");
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
    assert!(out.is_empty(), "a refused record writes nothing");
    assert_eq!(
        summary.skipped[0].reason,
        SkipReason::UnreadableTimestamp("yesterday afternoon".into())
    );
}

/// Archive order is the whole reason the export is one query: a 256 KiB block
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
    assert_eq!(members(&out).len(), 12, "twelve records in one file");
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
    assert_eq!(summary.bytes_written, 0);
    assert!(summary.skipped.is_empty());
    assert!(out.is_empty(), "no bodies is an empty file, not an error");
}

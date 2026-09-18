//! The export, read by a WARC reader that is not ours.
//!
//! Everything else about the format is checked by code in this repository,
//! which is the one kind of proof that cannot fail the way the format actually
//! fails: our writer and our parser can agree perfectly on a file no standard
//! tool will open. The whole point of exporting WARC is that a reviewer uses
//! `warcio`, `pywb` or `wget --warc`, so `warcio` is what reads it here.
//!
//! **This does not skip.** It used to, when `uv` was missing or the machine was
//! offline, and `#[test]` has no way to report a skip -- the `eprintln!` was
//! swallowed without `--nocapture`, so the one proof the format is real could
//! quietly stop running and the suite would still be green. `uv` is not
//! optional in this repository: every gate command runs through it and
//! `just doctor fix` installs it. `warcio` is pinned in `build_system`'s dev
//! dependency group, so `--frozen` resolves it from the checked-in lock
//! without reaching the network. Both are therefore hard requirements, and a
//! machine without them fails here with the command that would fix it.

use std::path::Path;
use std::process::Command;

use super::bodies::{count, net_event_with_response};
use super::warc_export::{export_to_bytes, rewrite_and_reopen};
use super::*;

/// `--frozen` and `--project build_system`: the interpreter and `warcio` both
/// come from the checked-in lock, so this resolves offline and cannot drift to
/// whatever version a machine happens to have.
const UV: &str = "uv";
const UV_ARGS: [&str; 4] = ["run", "--project", "build_system", "--frozen"];

const MISSING_UV: &str = "`uv` is required to run this repository's Python; install it with `just doctor fix`. \
                          It is not optional: every gate command runs through it";

/// Print one JSON object per record, as `warcio` sees it.
const READ_WITH_WARCIO: &str = r#"
import json, sys
from warcio.archiveiterator import ArchiveIterator

records = []
with open(sys.argv[1], "rb") as handle:
    for record in ArchiveIterator(handle):
        headers = record.rec_headers
        records.append(
            {
                "type": record.rec_type,
                "id": headers.get_header("WARC-Record-ID"),
                "uri": headers.get_header("WARC-Target-URI"),
                "date": headers.get_header("WARC-Date"),
                "content_type": headers.get_header("Content-Type"),
                "body": record.content_stream().read().decode("utf-8", "replace"),
            }
        )
print(json.dumps(records))
"#;

/// Every record of `path`, as `warcio` parsed it.
fn read_with_warcio(path: &Path) -> Vec<serde_json::Value> {
    let output = Command::new(UV)
        .args(UV_ARGS)
        .args(["python", "-c", READ_WITH_WARCIO])
        .arg(path)
        .current_dir(repository_root())
        .output()
        .unwrap_or_else(|error| panic!("{MISSING_UV}: {error}"));
    assert!(
        output.status.success(),
        "warcio could not read the export: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("warcio prints JSON")
}

/// `--project build_system` is relative, and `cargo test` runs from the crate.
fn repository_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate sits two levels below the repository root")
        .to_path_buf()
}

/// The fields of a `warcinfo` record's `application/warc-fields` block.
fn fields(record: &serde_json::Value) -> BTreeMap<String, String> {
    record["body"]
        .as_str()
        .expect("warc-fields are text")
        .split("\r\n")
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

#[tokio::test]
async fn warcio_reads_every_record_the_export_wrote() {
    let p = temp_db_path("warc-export-warcio");
    let db = DbHandle::open(&p).expect("open handle");
    db.write(WriteOp::NetEvent(net_event_with_response(
        "0123456789ab",
        "answers.example",
        r#"{"answer":"yes"}"#,
    )))
    .await
    .expect("write net event");
    for index in 0..4 {
        db.write(WriteOp::NetEvent(net_event_with_response(
            &format!("{index:012x}"),
            "bulk.example",
            &format!("body number {index}"),
        )))
        .await
        .expect("write net event");
    }
    db.flush().await.expect("flush");

    // One body is given a hash its bytes do not have, so the export has
    // something real to leave out and the trailing warcinfo has something to
    // report. A file that only ever describes a clean session cannot show that
    // the omission is visible in the artifact.
    let db = rewrite_and_reopen(
        db,
        &p,
        "UPDATE event_body_blobs SET body_hash = 'blake3:' || hex(zeroblob(32))
         WHERE event_id = '000000000002'",
    )
    .await;

    // Exported once and kept, because the tool needs a path rather than bytes.
    let (summary, bytes) = export_to_bytes(&db, &p).await;
    let export = p.with_extension("read-by-warcio.warc.gz");
    std::fs::write(&export, &bytes).expect("write the export for warcio");

    let indexed = count(&db, "SELECT COUNT(*) FROM event_body_blobs").await as usize;
    let records = read_with_warcio(&export);
    let _ = std::fs::remove_file(&export);

    assert_eq!(summary.skipped.len(), 1, "{:?}", summary.skipped);
    let bodies: Vec<&serde_json::Value> = records.iter().filter(|r| r["type"] == "resource").collect();
    assert_eq!(
        bodies.len(),
        indexed - summary.skipped.len(),
        "warcio must find one record per index row the export did not skip"
    );
    assert_eq!(bodies.len(), summary.records as usize, "and as many as we counted");

    let known = bodies
        .iter()
        .find(|record| record["id"] == "<urn:capsem:0123456789ab:response>")
        .expect("the net event's record, as warcio identifies it");
    assert_eq!(known["uri"], "https://answers.example/api");
    assert_eq!(
        known["body"], r#"{"answer":"yes"}"#,
        "the payload warcio hands back must be the body the session captured"
    );
    assert_eq!(known["content_type"], "application/json");
    assert!(
        known["date"].as_str().expect("a date").ends_with('Z'),
        "{:?}",
        known["date"]
    );

    // The file describes itself, as warcio reads it: a warcinfo at each end,
    // and the trailing one accounting for the body that was left out.
    let info: Vec<&serde_json::Value> = records.iter().filter(|r| r["type"] == "warcinfo").collect();
    assert_eq!(info.len(), 2, "one warcinfo at each end: {records:?}");
    assert_eq!(records[0]["type"], "warcinfo", "the first record describes the file");
    assert_eq!(
        records.last().expect("records")["type"],
        "warcinfo",
        "the last record says what was left out"
    );

    let opening = fields(info[0]);
    assert!(opening["software"].starts_with("capsem/"), "{opening:?}");
    assert_eq!(opening["format"], "WARC File Format 1.1");

    let closing = fields(info[1]);
    assert_eq!(closing["capsem-records"], summary.records.to_string());
    assert_eq!(closing["capsem-skipped"], "1");
    assert_eq!(
        closing["capsem-skipped-corrupt-body"], "1",
        "the injected omission must be visible to a reader holding only the file: {closing:?}"
    );
}

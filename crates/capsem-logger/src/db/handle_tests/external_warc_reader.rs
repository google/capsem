//! The export, read by a WARC reader that is not ours.
//!
//! Everything else about the format is checked by code in this repository,
//! which is the one kind of proof that cannot fail the way the format actually
//! fails: our writer and our parser can agree perfectly on a file no standard
//! tool will open. The whole point of exporting WARC is that a reviewer uses
//! `warcio`, `pywb` or `wget --warc`, so `warcio` is what reads it here.
//!
//! It shells out, which is how the other optional-tool tests in this tree work
//! -- `capsem-admin` runs `pkgbuild`, `capsem-guard` runs `perl`. The
//! difference is that `warcio` is not on a machine by default, so this probes
//! for it first and skips with a reason rather than failing: a developer
//! offline on a train must not see a red test about a Python package.

use std::path::Path;
use std::process::Command;

use super::bodies::{count, net_event_with_response};
use super::warc_export::export_to_bytes;
use super::*;

/// The tool, and the exact invocation used for both the probe and the read.
/// `--no-project` because this is not a `build_system` script: it is a
/// throwaway interpreter with one package in it.
const UV: &str = "uv";
const UV_ARGS: [&str; 4] = ["run", "--with", "warcio", "--no-project"];

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

/// Whether `uv` can produce an interpreter with `warcio` in it right now.
///
/// A probe rather than a `which`: the package still has to be fetched or found
/// in the cache, and an offline machine fails there, not at the executable.
fn warcio_is_available() -> bool {
    Command::new(UV)
        .args(UV_ARGS)
        .args(["python", "-c", "import warcio"])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Every record of `path`, as `warcio` parsed it.
fn read_with_warcio(path: &Path) -> Vec<serde_json::Value> {
    let output = Command::new(UV)
        .args(UV_ARGS)
        .args(["python", "-c", READ_WITH_WARCIO])
        .arg(path)
        .output()
        .expect("run warcio");
    assert!(
        output.status.success(),
        "warcio could not read the export: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("warcio prints JSON")
}

#[tokio::test]
async fn warcio_reads_every_record_the_export_wrote() {
    if !warcio_is_available() {
        eprintln!(
            "skipping: `{UV} {} python -c 'import warcio'` did not succeed, so this machine \
             has no warcio to check the export against (offline, or uv is not installed)",
            UV_ARGS.join(" ")
        );
        return;
    }

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

    // Exported once and kept, because the tool needs a path rather than bytes.
    let (summary, bytes) = export_to_bytes(&db, &p).await;
    let export = p.with_extension("read-by-warcio.warc.gz");
    std::fs::write(&export, &bytes).expect("write the export for warcio");

    let indexed = count(&db, "SELECT COUNT(*) FROM event_body_blobs").await as usize;
    let records = read_with_warcio(&export);
    let _ = std::fs::remove_file(&export);

    assert_eq!(
        records.len(),
        indexed - summary.skipped.len(),
        "warcio must find one record per index row the export did not skip"
    );
    assert_eq!(records.len(), summary.records as usize, "and as many as we counted");
    assert!(
        records.iter().all(|record| record["type"] == "resource"),
        "every record is a resource record: {records:?}"
    );

    let known = records
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
}

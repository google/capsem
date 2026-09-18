//! `GET /vms/{id}/bodies/export.warc.gz`: the streamed session export.
//!
//! What matters about a WARC export is that code which is not ours can read
//! it, so `warcio` is what reads it here -- the same probe-and-skip pattern
//! `crates/capsem-logger/src/db/handle_tests/external_warc_reader.rs` uses, so
//! a developer offline on a train sees a skip rather than a red test. What the
//! route adds over the logger's own export test is the streaming path: these
//! bytes came through a bounded channel and `Body::from_stream`, and a framing
//! mistake there would produce a file no reader can open.

use super::bodies::session_with_bodies;
use super::*;
use std::process::Command;

/// The tool, and the exact invocation used for both the probe and the read.
const UV: &str = "uv";
const UV_ARGS: [&str; 4] = ["run", "--with", "warcio", "--no-project"];

const COUNT_WITH_WARCIO: &str = r#"
import json, sys
from warcio.archiveiterator import ArchiveIterator

types = []
with open(sys.argv[1], "rb") as handle:
    for record in ArchiveIterator(handle):
        types.append(record.rec_type)
print(json.dumps(types))
"#;

/// Whether `uv` can produce an interpreter with `warcio` in it right now. A
/// probe rather than a `which`: the package still has to be fetched or found
/// in the cache, and an offline machine fails there, not at the executable.
fn warcio_is_available() -> bool {
    Command::new(UV)
        .args(UV_ARGS)
        .args(["python", "-c", "import warcio"])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[tokio::test]
async fn the_export_route_streams_a_warc_a_standard_reader_can_open() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("export-vm");
    session_with_bodies(&state, "export-vm", &session_dir).await;

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/export-vm/bodies/export.warc.gz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("the export route should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[axum::http::header::CONTENT_TYPE], "application/warc");
    assert_eq!(
        headers[axum::http::header::CONTENT_DISPOSITION],
        "attachment; filename=\"capsem-session-export-vm.warc.gz\""
    );
    // No transfer-level gzip: the framing belongs to the file the caller asked
    // for, and announcing it would hand browsers a decompressed `.gz`.
    assert!(!headers.contains_key(axum::http::header::CONTENT_ENCODING));

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(!bytes.is_empty(), "the export sent no bytes at all");

    // The ledger's own count of what the export had to describe. The event
    // written by `session_with_bodies` has a request body and a response body.
    let indexed = capsem_logger::DbHandle::open_external_reader(&session_dir.join("session.db"))
        .unwrap()
        .query("SELECT COUNT(*) FROM event_body_blobs", &[])
        .await
        .unwrap();
    let indexed: serde_json::Value = serde_json::from_str(&indexed).unwrap();
    let indexed = indexed["rows"][0][0].as_u64().expect("a count");
    assert_eq!(indexed, 2, "the fixture archives a request and a response");

    if !warcio_is_available() {
        eprintln!(
            "skipping the warcio read: `{UV} {} python -c 'import warcio'` did not succeed, so \
             this machine has no warcio to check the export against (offline, or uv is missing)",
            UV_ARGS.join(" ")
        );
        return;
    }

    let export = session_dir.join("route-export.warc.gz");
    std::fs::write(&export, &bytes).expect("write the streamed export for warcio");
    let output = Command::new(UV)
        .args(UV_ARGS)
        .args(["python", "-c", COUNT_WITH_WARCIO])
        .arg(&export)
        .output()
        .expect("run warcio");
    assert!(
        output.status.success(),
        "warcio could not read what the route streamed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let types: Vec<String> = serde_json::from_slice(&output.stdout).expect("warcio prints JSON");
    assert_eq!(
        types.len() as u64,
        indexed,
        "one record per index row; warcio found {types:?}"
    );
    assert!(
        types.iter().all(|kind| kind == "resource"),
        "every record is a resource record: {types:?}"
    );
}

//! `GET /vms/{id}/bodies/export.warc.gz`: the streamed session export.
//!
//! What matters about a WARC export is that code which is not ours can read
//! it, so `warcio` is what reads it here, exactly as
//! `crates/capsem-logger/src/db/handle_tests/external_warc_reader.rs` does.
//! What the route adds over the logger's own export test is the streaming
//! path: these bytes came through a bounded channel and `Body::from_stream`,
//! and a framing mistake there would produce a file no reader can open.
//!
//! **This does not skip.** It used to probe for `warcio` and return quietly
//! when the probe failed, which meant the one proof the streamed file is
//! readable could stop running without turning anything red. `uv` is not
//! optional here and `warcio` is pinned in `build_system`, so `--frozen`
//! resolves it from the checked-in lock without the network.

use super::bodies::session_with_bodies;
use super::*;
use std::process::Command;

/// `--frozen` and `--project build_system`: the interpreter and `warcio` both
/// come from the checked-in lock, so this resolves offline.
const UV: &str = "uv";
const UV_ARGS: [&str; 4] = ["run", "--project", "build_system", "--frozen"];

const MISSING_UV: &str = "`uv` is required to run this repository's Python; install it with `just doctor fix`. \
                          It is not optional: every gate command runs through it";

/// `--project build_system` is relative, and `cargo test` runs from the crate.
fn repository_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate sits two levels below the repository root")
        .to_path_buf()
}

const COUNT_WITH_WARCIO: &str = r#"
import json, sys
from warcio.archiveiterator import ArchiveIterator

types = []
with open(sys.argv[1], "rb") as handle:
    for record in ArchiveIterator(handle):
        types.append(record.rec_type)
print(json.dumps(types))
"#;

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

    let export = session_dir.join("route-export.warc.gz");
    std::fs::write(&export, &bytes).expect("write the streamed export for warcio");
    let output = Command::new(UV)
        .args(UV_ARGS)
        .args(["python", "-c", COUNT_WITH_WARCIO])
        .arg(&export)
        .current_dir(repository_root())
        .output()
        .unwrap_or_else(|error| panic!("{MISSING_UV}: {error}"));
    assert!(
        output.status.success(),
        "warcio could not read what the route streamed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let types: Vec<String> = serde_json::from_slice(&output.stdout).expect("warcio prints JSON");
    assert_eq!(
        types.iter().filter(|kind| *kind == "resource").count() as u64,
        indexed,
        "one resource record per index row; warcio found {types:?}"
    );
    // The export brackets its bodies with a warcinfo at each end, so the file
    // says for itself what it holds and what it left out. The streaming path
    // must carry those through like any other record.
    assert_eq!(
        types.first().map(String::as_str),
        Some("warcinfo"),
        "the streamed file must open by describing itself: {types:?}"
    );
    assert_eq!(
        types.last().map(String::as_str),
        Some("warcinfo"),
        "and close by saying what it left out: {types:?}"
    );
    assert_eq!(types.len() as u64, indexed + 2, "{types:?}");
}

//! `GET /vms/{id}/history/transcript` documents a `tail_lines` query
//! parameter (default 500) and ignored it: the route read the whole
//! `pty.log` synchronously on a tokio worker and base64-encoded every byte.

use super::*;
use base64::Engine;

fn transcript_state(lines: usize) -> (Arc<ServiceState>, tempfile::TempDir, String) {
    let (state, dir) = make_test_state_with_tempdir();
    let id = "transcript-vm";
    let session_dir = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let body = (0..lines).fold(String::new(), |mut body, n| {
        use std::fmt::Write;
        let _ = write!(body, "line-{n}\r\n");
        body
    });
    std::fs::write(session_dir.join("pty.log"), body).unwrap();
    insert_fake_instance_with_session_dir(&state, id, std::process::id(), session_dir);
    (state, dir, id.to_string())
}

async fn transcript(state: &Arc<ServiceState>, id: &str, query: &str) -> (usize, String) {
    let app = build_service_router(Arc::clone(state));
    let (status, body) = route_request(
        app,
        axum::http::Method::GET,
        &format!("/vms/{id}/history/transcript{query}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let bytes = body["bytes"].as_u64().unwrap() as usize;
    let content = base64::engine::general_purpose::STANDARD
        .decode(body["content"].as_str().unwrap())
        .unwrap();
    (bytes, String::from_utf8(content).unwrap())
}

#[tokio::test]
async fn tail_lines_returns_only_the_last_lines() {
    let (state, _dir, id) = transcript_state(10);
    let (bytes, content) = transcript(&state, &id, "?tail_lines=3").await;
    assert_eq!(content, "line-7\r\nline-8\r\nline-9\r\n");
    assert_eq!(bytes, content.len(), "bytes reports the returned slice, not the file");
}

#[tokio::test]
async fn tail_lines_defaults_to_five_hundred() {
    let (state, _dir, id) = transcript_state(600);
    let (_, content) = transcript(&state, &id, "").await;
    assert_eq!(content.lines().count(), 500);
    assert!(content.starts_with("line-100\r\n"));
    assert!(content.ends_with("line-599\r\n"));
}

#[tokio::test]
async fn tail_lines_larger_than_the_file_returns_everything() {
    let (state, _dir, id) = transcript_state(4);
    let (bytes, content) = transcript(&state, &id, "?tail_lines=1000").await;
    assert_eq!(content.lines().count(), 4);
    assert_eq!(bytes, content.len());
}

#[tokio::test]
async fn an_unterminated_last_line_counts_as_a_line() {
    let (state, _dir, id) = transcript_state(0);
    let session_dir = state.run_dir.join("sessions").join(&id);
    std::fs::write(session_dir.join("pty.log"), b"first\nsecond\nno-newline").unwrap();
    let (_, content) = transcript(&state, &id, "?tail_lines=2").await;
    assert_eq!(content, "second\nno-newline");
    let (_, content) = transcript(&state, &id, "?tail_lines=0").await;
    assert_eq!(content, "");
}

#[tokio::test]
async fn transcript_survives_bytes_that_are_not_utf8() {
    let (state, _dir, id) = transcript_state(0);
    let session_dir = state.run_dir.join("sessions").join(&id);
    std::fs::write(session_dir.join("pty.log"), [0xff, 0xfe, b'\n', 0x1b, b'[', b'm', 0xc3]).unwrap();
    let app = build_service_router(Arc::clone(&state));
    let (status, body) = route_request(
        app,
        axum::http::Method::GET,
        &format!("/vms/{id}/history/transcript?tail_lines=1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let content = base64::engine::general_purpose::STANDARD
        .decode(body["content"].as_str().unwrap())
        .unwrap();
    assert_eq!(content, [0x1b, b'[', b'm', 0xc3]);
}

use super::*;
use std::io::Write;
use std::sync::Mutex;

fn test_magika() -> Mutex<magika::Session> {
    Mutex::new(
        magika::Session::builder()
            .with_inter_threads(1)
            .with_intra_threads(1)
            .build()
            .expect("magika init"),
    )
}

// ---- sanitize_file_path ----

#[test]
fn sanitize_strips_script_tags() {
    // The `/` inside `</script>` is in the allowlist and survives, so the
    // output keeps it. The < > ( ) are dropped.
    let r = sanitize_file_path("<script>alert(1)</script>.txt").unwrap();
    assert_eq!(r, "scriptalert1/script.txt");
}

#[test]
fn sanitize_strips_null_bytes() {
    let r = sanitize_file_path("foo\0bar.txt").unwrap();
    assert_eq!(r, "foobar.txt");
}

#[test]
fn sanitize_strips_unicode() {
    let r = sanitize_file_path("foo\u{200B}bar.txt").unwrap();
    assert_eq!(r, "foobar.txt");
}

#[test]
fn sanitize_rejects_dot_dot() {
    let err = sanitize_file_path("../etc/passwd").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[test]
fn sanitize_rejects_embedded_dot_dot() {
    let err = sanitize_file_path("foo/../bar").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[test]
fn sanitize_collapses_slashes() {
    let result = sanitize_file_path("foo//bar///baz");
    assert_eq!(result.unwrap(), "foo/bar/baz");
}

#[test]
fn sanitize_strips_leading_slash() {
    let result = sanitize_file_path("/foo/bar");
    assert_eq!(result.unwrap(), "foo/bar");
}

#[test]
fn sanitize_rejects_empty() {
    let err = sanitize_file_path("").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[test]
fn sanitize_preserves_valid_path() {
    let result = sanitize_file_path("foo/bar.txt");
    assert_eq!(result.unwrap(), "foo/bar.txt");
}

#[test]
fn sanitize_preserves_hyphens_underscores_dots() {
    let result = sanitize_file_path("my-file_v2.tar.gz");
    assert_eq!(result.unwrap(), "my-file_v2.tar.gz");
}

// ---- new tests ----

#[test]
fn sanitize_rejects_only_slashes() {
    // Several slashes collapse + leading-strip to empty, then the empty
    // check fires. Confirms the order: collapse → strip → reject empty.
    let err = sanitize_file_path("///").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(err.1, "empty path after sanitization");
}

#[test]
fn sanitize_rejects_dot_dot_after_filter() {
    // Disallowed characters drop out before the `..` check, so `.<>.`
    // collapses to `..` and is correctly rejected as traversal -- proves
    // the filter runs before the traversal check, not after it.
    let err = sanitize_file_path(".<>.").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(err.1, "path traversal rejected");
}

// ---- magika helpers ----

#[test]
fn extract_magika_info_smoke() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("hello.txt");
    std::fs::write(&txt, b"hello world\n").unwrap();
    let session = test_magika();
    let mut s = session.lock().unwrap();
    let ft = s.identify_file_sync(&txt).expect("magika identify");
    let (label, mime, group, is_text) = extract_magika_info(&ft);
    assert!(!label.is_empty());
    assert!(!mime.is_empty());
    assert!(!group.is_empty());
    assert!(
        is_text,
        "ASCII text should be recognized as text, got label={label}"
    );
}

#[test]
fn identify_file_sync_returns_unknown_for_missing_file() {
    let session = test_magika();
    let missing = std::path::Path::new("/nonexistent/path/that/does/not/exist.bin");
    let (label, mime, group, is_text) = identify_file_sync(&session, missing);
    assert_eq!(label, "unknown");
    assert_eq!(mime, "application/octet-stream");
    assert_eq!(group, "unknown");
    assert!(!is_text);
}

#[test]
fn identify_file_sync_round_trips_real_file() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("a.txt");
    let mut f = std::fs::File::create(&txt).unwrap();
    f.write_all(b"plain text content\n").unwrap();
    drop(f);
    let session = test_magika();
    let (label, mime, _group, is_text) = identify_file_sync(&session, &txt);
    assert!(
        is_text,
        "ASCII text not recognized as text, got label={label}"
    );
    assert_eq!(mime, "text/plain");
}

#[test]
fn identify_file_sync_uses_extension_and_utf8_fallback_for_small_text() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("tiny.txt");
    std::fs::write(&txt, b"x\n").unwrap();
    let detected = normalize_file_type(
        &txt,
        (
            "unknown".into(),
            "application/octet-stream".into(),
            "unknown".into(),
            false,
        ),
    );
    assert_eq!(
        detected,
        ("text".into(), "text/plain".into(), "text".into(), true)
    );
}

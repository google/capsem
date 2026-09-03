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
    assert!(is_text, "ASCII text should be recognized as text, got label={label}");
    drop(s);
}

#[test]
fn identify_file_sync_round_trips_real_file() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("a.txt");
    let mut f = std::fs::File::create(&txt).unwrap();
    f.write_all(b"plain text content\n").unwrap();
    drop(f);
    let session = test_magika();
    let mut file = std::fs::File::open(&txt).unwrap();
    let (label, mime, _group, is_text) = identify_file_sync(&session, &txt, &mut file);
    assert!(is_text, "ASCII text not recognized as text, got label={label}");
    assert_eq!(mime, "text/plain");

    let (label, mime, _group, is_text) = identify_bytes_sync(&session, &txt, b"plain text content\n");
    assert!(is_text, "bytes path disagrees with file path, got label={label}");
    assert_eq!(mime, "text/plain");
}

#[test]
fn identify_file_sync_uses_extension_and_utf8_fallback_for_small_text() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("tiny.txt");
    std::fs::write(&txt, b"x\n").unwrap();
    let detected = normalize_file_type(
        &txt,
        b"x\n",
        (
            "unknown".into(),
            "application/octet-stream".into(),
            "unknown".into(),
            false,
        ),
    );
    assert_eq!(detected, ("text".into(), "text/plain".into(), "text".into(), true));
}

/// Magika classifies by content, and the extension is deliberately not a vote.
///
/// The ironbank ledger fixture uploaded `upload:<random hex>` into a `.txt`
/// file, which is the shape of a CSS declaration -- `property: value` -- and
/// Magika sometimes agreed, so a complete gate failed on one nonce in some
/// runs and not others. `normalize_file_type` cannot help: it only rescues a
/// file Magika gave up on, and Magika had not given up.
///
/// Recorded here rather than only fixed in the fixture, because the next
/// person to assert an exact mime for a short generated payload needs to know
/// what they are asking of a classifier.
#[test]
fn a_short_key_value_line_is_not_reliably_plain_text() {
    let dir = tempfile::tempdir().unwrap();
    let magika = std::sync::Mutex::new(magika::Session::new().unwrap());

    // A nonce found by search, kept so this is a reproduction rather than a
    // claim. The gate saw `text/css` on a different one; this one gives
    // `text/csv`. Which wrong answer it is does not matter -- that the answer
    // depends on the random part does.
    let ambiguous = dir.path().join("ambiguous.txt");
    std::fs::write(&ambiguous, b"upload:fps-0000000000000000b135823cc1684885\n").unwrap();
    let (_, ambiguous_mime, _, _) =
        identify_file_sync(&magika, &ambiguous, &mut std::fs::File::open(&ambiguous).unwrap());
    assert_ne!(
        ambiguous_mime, "text/plain",
        "a `key: value` line is what a classifier is entitled to read as \
         structured data, whatever the extension says"
    );

    // The same nonce inside ordinary prose, and seven more, because one
    // sample proves nothing about the next -- which is how the fixture
    // reached a complete gate and failed there.
    for nonce in [
        "0000000000000000b135823cc1684885",
        "1a2b3c4d5e6f708192a3b4c5d6e7f809",
        "ffffffffffffffffffffffffffffffff",
        "00112233445566778899aabbccddeeff",
        "deadbeefdeadbeefdeadbeefdeadbeef",
        "9f86d081884c7d659a2feaa0c55ad015",
        "5e884898da28047151d0e56f8dc62927",
        "6b86b273ff34fce19d6b804eff5a3f57",
    ] {
        let prose = dir.path().join(format!("{nonce}.txt"));
        std::fs::write(
            &prose,
            format!(
                "This is the ironbank upload fixture for the file, process and \
                 snapshot ledger.\nCorrelation nonce fps-{nonce} identifies this \
                 run so the ledger rows can be matched back to it.\n"
            ),
        )
        .unwrap();
        let (_, mime, _, is_text) = identify_file_sync(&magika, &prose, &mut std::fs::File::open(&prose).unwrap());
        assert_eq!(mime, "text/plain", "prose must classify as prose: {nonce}");
        assert!(is_text);
    }
}

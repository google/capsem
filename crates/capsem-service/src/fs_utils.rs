//! Files-API helpers that don't depend on `ServiceState`.
//!
//! `sanitize_file_path` is the allowlist-based input gate; the Magika helpers
//! adapt the `magika` crate's API for use in `spawn_blocking` contexts.
//! `resolve_workspace_path` stays in `main.rs` because it borrows
//! `&ServiceState` and moving it now would force `ServiceState` out of
//! `main.rs` too -- that's the next sprint's job.

use std::sync::Mutex;

use axum::http::StatusCode;

use crate::errors::AppError;

/// Allowlist-based path sanitization for the files API.
/// Strips any character NOT in `[a-zA-Z0-9._\-/]`, collapses consecutive
/// slashes, strips leading `/`, and rejects `..` or empty results.
pub fn sanitize_file_path(raw: &str) -> Result<String, AppError> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-' || *c == '/')
        .collect();
    let mut collapsed = String::with_capacity(cleaned.len());
    let mut prev_slash = false;
    for ch in cleaned.chars() {
        if ch == '/' {
            if !prev_slash {
                collapsed.push(ch);
            }
            prev_slash = true;
        } else {
            collapsed.push(ch);
            prev_slash = false;
        }
    }
    let trimmed = collapsed.trim_start_matches('/');
    if trimmed.is_empty() {
        return Err(AppError(StatusCode::BAD_REQUEST, "empty path after sanitization".into()));
    }
    if trimmed.contains("..") {
        return Err(AppError(StatusCode::BAD_REQUEST, "path traversal rejected".into()));
    }
    Ok(trimmed.to_string())
}

/// Extract file-type info from Magika `FileType` as `(label, mime, group, is_text)`.
pub fn extract_magika_info(ft: &magika::FileType) -> (String, String, String, bool) {
    let info = ft.info();
    (
        info.label.to_string(),
        info.mime_type.to_string(),
        info.group.to_string(),
        info.is_text,
    )
}

/// Identify a file using Magika. Runs synchronously under the session mutex --
/// callers wrap in `spawn_blocking` because `Session::identify_file_sync` takes
/// `&mut self`. Returns the `unknown`/`application/octet-stream` tuple on any
/// error so handlers don't have to plumb errors through for best-effort typing.
pub fn identify_file_sync(
    magika: &Mutex<magika::Session>,
    path: &std::path::Path,
) -> (String, String, String, bool) {
    let mut session = magika.lock().unwrap();
    match session.identify_file_sync(path) {
        Ok(ft) => extract_magika_info(&ft),
        Err(_) => ("unknown".into(), "application/octet-stream".into(), "unknown".into(), false),
    }
}

#[cfg(test)]
mod tests {
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
        let (label, _mime, _group, is_text) = identify_file_sync(&session, &txt);
        assert!(is_text, "ASCII text not recognized as text, got label={label}");
    }
}

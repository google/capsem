//! Files-API helpers that don't depend on `ServiceState`.
//!
//! `sanitize_file_path` is the allowlist-based input gate; the Magika helpers
//! adapt the `magika` crate's API for use in `spawn_blocking` contexts.
//! `resolve_workspace_path` stays in `main.rs` because it borrows
//! `&ServiceState` and moving it now would force `ServiceState` out of
//! `main.rs` too -- that's the next sprint's job.

use std::io::Read;
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
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "empty path after sanitization".into(),
        ));
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
pub fn identify_file_sync(magika: &Mutex<magika::Session>, path: &std::path::Path) -> (String, String, String, bool) {
    let mut session = magika.lock().unwrap();
    match session.identify_file_sync(path) {
        Ok(ft) => normalize_file_type(path, extract_magika_info(&ft)),
        Err(_) => (
            "unknown".into(),
            "application/octet-stream".into(),
            "unknown".into(),
            false,
        ),
    }
}

fn normalize_file_type(
    path: &std::path::Path,
    detected: (String, String, String, bool),
) -> (String, String, String, bool) {
    let (label, mime, group, is_text) = detected;
    if is_text || mime != "application/octet-stream" {
        return (label, mime, group, is_text);
    }
    if has_plain_text_extension(path) && file_looks_utf8(path) {
        return ("text".into(), "text/plain".into(), "text".into(), true);
    }
    (label, mime, group, is_text)
}

fn has_plain_text_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "txt"
                    | "text"
                    | "md"
                    | "markdown"
                    | "log"
                    | "json"
                    | "toml"
                    | "yaml"
                    | "yml"
                    | "csv"
                    | "tsv"
                    | "sh"
                    | "py"
                    | "js"
                    | "ts"
                    | "rs"
            )
        })
        .unwrap_or(false)
}

fn file_looks_utf8(path: &std::path::Path) -> bool {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut buf = Vec::with_capacity(8192);
    match file.by_ref().take(8192).read_to_end(&mut buf) {
        Ok(_) => std::str::from_utf8(&buf).is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests;

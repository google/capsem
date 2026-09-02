//! Files-API helpers that don't depend on `ServiceState`.
//!
//! `sanitize_file_path` is the allowlist-based input gate; the Magika helpers
//! adapt the `magika` crate's API for use in `spawn_blocking` contexts.
//! `resolve_workspace_target` lives in `vm_files.rs` because it borrows
//! `&ServiceState`.

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

/// The tuple for a file that could not be typed. Typing is best-effort, so
/// handlers never plumb its errors.
pub fn unknown_file_type() -> (String, String, String, bool) {
    (
        "unknown".into(),
        "application/octet-stream".into(),
        "unknown".into(),
        false,
    )
}

/// Identify an already-open regular file with Magika. Taking the handle rather
/// than a path means the bytes classified are the bytes the caller opened --
/// with `O_NOFOLLOW` -- and not whatever a guest has since put at that name.
/// Runs synchronously under the session mutex; callers wrap in `spawn_blocking`.
pub fn identify_file_sync(
    magika: &Mutex<magika::Session>,
    name: &std::path::Path,
    file: &mut std::fs::File,
) -> (String, String, String, bool) {
    let mut head = Vec::with_capacity(UTF8_PROBE_BYTES);
    if file
        .by_ref()
        .take(UTF8_PROBE_BYTES as u64)
        .read_to_end(&mut head)
        .is_err()
    {
        return unknown_file_type();
    }
    let mut session = magika.lock().unwrap();
    match session.identify_content_sync(&mut *file) {
        Ok(ft) => normalize_file_type(name, &head, extract_magika_info(&ft)),
        Err(_) => unknown_file_type(),
    }
}

/// Identify bytes already read into memory. See `identify_file_sync`.
pub fn identify_bytes_sync(
    magika: &Mutex<magika::Session>,
    name: &std::path::Path,
    data: &[u8],
) -> (String, String, String, bool) {
    let mut session = magika.lock().unwrap();
    match session.identify_content_sync(data) {
        Ok(ft) => normalize_file_type(
            name,
            &data[..data.len().min(UTF8_PROBE_BYTES)],
            extract_magika_info(&ft),
        ),
        Err(_) => unknown_file_type(),
    }
}

const UTF8_PROBE_BYTES: usize = 8192;

/// Rescue a file Magika gave up on: a plain-text extension plus a UTF-8 head
/// is text.
fn normalize_file_type(
    name: &std::path::Path,
    head: &[u8],
    detected: (String, String, String, bool),
) -> (String, String, String, bool) {
    let (label, mime, group, is_text) = detected;
    if is_text || mime != "application/octet-stream" {
        return (label, mime, group, is_text);
    }
    if has_plain_text_extension(name) && std::str::from_utf8(head).is_ok() {
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

#[cfg(test)]
mod tests;

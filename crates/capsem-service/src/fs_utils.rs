//! Files-API helpers that don't depend on `ServiceState`.
//!
//! `sanitize_file_path` is the allowlist-based input gate; `identify_file`
//! and `identify_bytes` type a workspace file for the files API.
//! `resolve_workspace_target` lives in `vm_files.rs` because it borrows
//! `&ServiceState`.

use std::io::Read;

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

#[derive(serde::Deserialize)]
pub struct FileListQuery {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_file_depth")]
    pub depth: u32,
    /// Take `path` literally inside the workspace; see [`resolve_file_path`].
    #[serde(default)]
    pub exact: bool,
}

fn default_file_depth() -> u32 {
    1
}

#[derive(serde::Deserialize)]
pub struct FileContentQuery {
    pub path: String,
    /// Take `path` literally inside the workspace; see [`resolve_file_path`].
    #[serde(default)]
    pub exact: bool,
}

/// Where a files-API path lands: the workspace-relative path on disk, and the
/// paths the VM and, when one runs, its container see for that same file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePath {
    pub relative: String,
    pub vm_path: String,
    pub container_path: Option<String>,
}

/// Resolve a caller's path. A relative path is workspace-relative. An absolute
/// path is the path the caller sees: under `/root` in a VM, or under the
/// container's workspace mount when the VM runs a container; anything else is
/// refused rather than silently placed somewhere else, which is what stripping
/// the leading `/` used to do. `exact` keeps that literal workspace form.
pub fn resolve_file_path(raw: &str, exact: bool, container: bool) -> Result<FilePath, AppError> {
    let relative = sanitize_file_path(workspace_relative(raw, exact, container)?)?;
    Ok(FilePath {
        vm_path: format!("{}/{relative}", capsem_proto::GUEST_WORKSPACE),
        container_path: container.then(|| format!("{}/{relative}", capsem_core::container::CONTAINER_WORKSPACE)),
        relative,
    })
}

/// Like [`resolve_file_path`] for a directory listing, which may name the
/// workspace root itself (empty result).
pub fn resolve_dir_path(raw: &str, exact: bool, container: bool) -> Result<String, AppError> {
    match workspace_relative(raw, exact, container)?.trim_matches('/') {
        "" => Ok(String::new()),
        _ => Ok(resolve_file_path(raw, exact, container)?.relative),
    }
}

/// The part of `raw` below the workspace, before sanitizing.
fn workspace_relative(raw: &str, exact: bool, container: bool) -> Result<&str, AppError> {
    if exact || !raw.starts_with('/') {
        return Ok(raw);
    }
    let root = if container {
        capsem_core::container::CONTAINER_WORKSPACE
    } else {
        capsem_proto::GUEST_WORKSPACE
    };
    match raw.strip_prefix(root) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => Ok(rest),
        _ if container => Err(AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "{raw} is not reachable: this VM runs a container, which sees the workspace at {root}; \
                 pass a path under {root}, a relative path, or exact=true to place the path as written in the workspace"
            ),
        )),
        _ => Err(AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "{raw} is outside the workspace: only paths under {root} are reachable; \
                 pass a relative path, or exact=true to place the path as written in the workspace"
            ),
        )),
    }
}

/// What the files API reports about a file's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileType {
    /// Short name the UI uses as a syntax-highlighting hint (`python`, `png`).
    pub label: &'static str,
    pub mime: &'static str,
    pub group: &'static str,
    pub is_text: bool,
}

impl FileType {
    pub const TEXT: Self = Self::new("text", "text/plain", "text", true);
    pub const UNKNOWN: Self = Self::new("unknown", "application/octet-stream", "unknown", false);

    const fn new(label: &'static str, mime: &'static str, group: &'static str, is_text: bool) -> Self {
        Self {
            label,
            mime,
            group,
            is_text,
        }
    }
}

/// Bytes read from the head of a file to decide whether it is text.
const TEXT_PROBE_BYTES: usize = 8192;

/// Type an already-open regular file from its name and the head of its
/// content. Taking the handle rather than a path means the bytes classified
/// are the bytes the caller opened -- with `O_NOFOLLOW` -- and not whatever a
/// guest has since put at that name.
pub fn identify_file(name: &std::path::Path, file: &mut std::fs::File) -> FileType {
    let mut head = Vec::with_capacity(TEXT_PROBE_BYTES);
    match file.take(TEXT_PROBE_BYTES as u64).read_to_end(&mut head) {
        Ok(_) => identify_bytes(name, &head),
        Err(_) => FileType::UNKNOWN,
    }
}

/// Type bytes already in memory. See `identify_file`.
//
// TODO(magika-v2): content-based typing returns once Magika v2 (pure Rust)
// is production-ready, google/capsem#234. Magika v1 pulled in ONNX Runtime,
// a native library downloaded at build time, for a UI hint.
pub fn identify_bytes(name: &std::path::Path, data: &[u8]) -> FileType {
    let head = &data[..data.len().min(TEXT_PROBE_BYTES)];
    let text = looks_like_text(head);
    let extension = name
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match by_extension(&extension) {
        // A text extension on binary content is not believed.
        Some(known) if known.is_text && !text => FileType::UNKNOWN,
        Some(known) => known,
        None if text => FileType::TEXT,
        None => FileType::UNKNOWN,
    }
}

/// No NUL byte, and valid UTF-8 up to a character the probe may have cut.
fn looks_like_text(head: &[u8]) -> bool {
    !head.contains(&0)
        && match std::str::from_utf8(head) {
            Ok(_) => true,
            Err(error) => error.error_len().is_none(),
        }
}

fn by_extension(extension: &str) -> Option<FileType> {
    let (label, mime, group, is_text) = match extension {
        "txt" | "text" | "log" | "ini" | "cfg" | "conf" => ("text", "text/plain", "text", true),
        "md" | "markdown" => ("markdown", "text/markdown", "text", true),
        "csv" => ("csv", "text/csv", "text", true),
        "tsv" => ("tsv", "text/tab-separated-values", "text", true),
        "json" => ("json", "application/json", "code", true),
        "toml" => ("toml", "application/toml", "code", true),
        "yaml" | "yml" => ("yaml", "application/yaml", "code", true),
        "xml" => ("xml", "text/xml", "code", true),
        "html" | "htm" => ("html", "text/html", "code", true),
        "css" => ("css", "text/css", "code", true),
        "sh" | "bash" | "zsh" => ("shell", "text/x-shellscript", "code", true),
        "py" => ("python", "text/x-python", "code", true),
        "js" | "mjs" | "cjs" | "jsx" => ("javascript", "text/javascript", "code", true),
        "ts" | "tsx" => ("typescript", "application/typescript", "code", true),
        "rs" => ("rust", "text/x-rust", "code", true),
        "go" => ("go", "text/x-go", "code", true),
        "c" | "h" => ("c", "text/x-c", "code", true),
        "cc" | "cpp" | "hpp" => ("cpp", "text/x-c++", "code", true),
        "java" => ("java", "text/x-java", "code", true),
        "rb" => ("ruby", "text/x-ruby", "code", true),
        "sql" => ("sql", "application/sql", "code", true),
        "svg" => ("svg", "image/svg+xml", "image", true),
        "png" => ("png", "image/png", "image", false),
        "jpg" | "jpeg" => ("jpeg", "image/jpeg", "image", false),
        "gif" => ("gif", "image/gif", "image", false),
        "webp" => ("webp", "image/webp", "image", false),
        "ico" => ("ico", "image/vnd.microsoft.icon", "image", false),
        "pdf" => ("pdf", "application/pdf", "document", false),
        "zip" => ("zip", "application/zip", "archive", false),
        "gz" | "tgz" => ("gzip", "application/gzip", "archive", false),
        "tar" => ("tar", "application/x-tar", "archive", false),
        "zst" => ("zstd", "application/zstd", "archive", false),
        "wasm" => ("wasm", "application/wasm", "executable", false),
        _ => return None,
    };
    Some(FileType::new(label, mime, group, is_text))
}

#[cfg(test)]
mod tests;

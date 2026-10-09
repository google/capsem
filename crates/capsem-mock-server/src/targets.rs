//! Where a replayed model tool call writes, and with which token.
use super::EXPECTED_POEM;
use serde_json::Value;

const TARGET_ROOTS: [&str; 2] = ["/root/", "/workspace/"];

pub(super) fn write_target(payload: &Value, default_prefix: &str) -> (String, String) {
    let raw = serde_json::to_string(payload).unwrap_or_default();
    let token = find_hex32(&raw).unwrap_or_else(|| EXPECTED_POEM.to_string());
    let path = find_target_txt_path(&raw).unwrap_or_else(|| format!("/root/{default_prefix}-output.txt"));
    (token, path)
}

pub(super) fn find_hex32(raw: &str) -> Option<String> {
    raw.as_bytes()
        .windows(32)
        .find(|window| window.iter().all(u8::is_ascii_hexdigit))
        .and_then(|window| std::str::from_utf8(window).ok())
        .map(ToOwned::to_owned)
}

/// Where a client asks a model to write: the last `.txt` path under a VM's
/// `/root` or a container workload's `/workspace` (its root is read-only).
pub(super) fn find_target_txt_path(raw: &str) -> Option<String> {
    TARGET_ROOTS
        .iter()
        .flat_map(|root| raw.match_indices(root))
        .filter_map(|(start, _)| {
            let tail = &raw[start..];
            let end = tail
                .char_indices()
                .find_map(|(index, ch)| {
                    let allowed = ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-');
                    (!allowed).then_some(index)
                })
                .unwrap_or(tail.len());
            let candidate = &tail[..end];
            candidate.find(".txt").map(|index| {
                let end = index + 4;
                (start, candidate[..end].replace("\\/", "/"))
            })
        })
        .max_by_key(|(start, _)| *start)
        .map(|(_, path)| path)
}

/// The directory a tool that writes `path` runs in.
pub(super) fn target_dir(path: &str) -> &str {
    path.rsplit_once('/')
        .map_or("/", |(dir, _)| if dir.is_empty() { "/" } else { dir })
}

pub(super) fn shell_write_command(token: &str, path: &str) -> String {
    let token = token.replace('\'', "'\\''");
    let path = path.replace('\'', "'\\''");
    format!("printf '%s\\n' '{token}' > '{path}'")
}

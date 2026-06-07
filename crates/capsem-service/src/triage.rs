//! Triage helpers: scan host log files for panics, errors, and slow
//! operations. Used by `/panics` and `/triage` HTTP endpoints (which the
//! capsem_panics + capsem_triage MCP tools call).
//!
//! Design intent: an AI agent or developer can run ONE MCP tool call and
//! get a ranked list of suspect events from the last N minutes across
//! `service.log`, `mcp.log`, `gateway.log`, `tray.log`, and the latest
//! `~/.capsem/logs/<ts>.jsonl`. No follow-up grep needed.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

/// Parse a "since" string into an absolute SystemTime.
///
/// Accepts:
///   - duration form: `30m`, `2h`, `24h`, `7d`, `300s`
///   - RFC3339 form: `2026-05-02T17:30:00Z` (extracted via cheap parsing)
pub fn parse_since(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(d) = parse_duration_suffix(s) {
        return Some(SystemTime::now() - d);
    }
    parse_rfc3339_seconds(s).map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

fn parse_duration_suffix(s: &str) -> Option<Duration> {
    if s.len() < 2 {
        return None;
    }
    let (num, unit) = s.split_at(s.len() - 1);
    let n: u64 = num.parse().ok()?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86400,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

/// Best-effort RFC3339-seconds parser: `2026-05-02T17:30:00Z` ->
/// unix epoch seconds. Avoids a chrono dep; we only need the second
/// granularity to filter logs.
fn parse_rfc3339_seconds(s: &str) -> Option<u64> {
    if s.len() < 20 || !s.ends_with('Z') {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let mo: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    let h: u32 = s.get(11..13)?.parse().ok()?;
    let mi: u32 = s.get(14..16)?.parse().ok()?;
    let se: u32 = s.get(17..19)?.parse().ok()?;
    Some(civil_to_secs(y, mo, d, h, mi, se))
}

fn civil_to_secs(y: i64, m: u32, d: u32, h: u32, mi: u32, s: u32) -> u64 {
    // Howard Hinnant days_from_civil
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era * 146097) as u64 + doe - 719_468;
    days * 86400 + h as u64 * 3600 + mi as u64 * 60 + s as u64
}

/// One panic event extracted from a log file.
#[derive(Debug, Clone, Serialize)]
pub struct PanicEvent {
    pub ts: String,
    pub binary: String,
    pub thread: Option<String>,
    pub location: Option<String>,
    pub message: String,
    pub frames: Vec<String>,
}

/// One generic error/warning event with structured fields.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorEvent {
    pub ts: String,
    pub binary: String,
    pub level: String,
    pub target: Option<String>,
    pub message: String,
}

/// One slow-operation event surfaced by `target=fs op=fsync` or similar
/// timing markers added by W4.
#[derive(Debug, Clone, Serialize)]
pub struct SlowOpEvent {
    pub ts: String,
    pub binary: String,
    pub op: String,
    pub duration_ms: u64,
}

/// Scan one file's tail for panics. Returns a list of `PanicEvent`s.
///
/// Two passes:
/// 1. JSON tracing line with a `panic` or `panicked` substring.
/// 2. Plain-text fallback regex for `thread '<name>' panicked at <loc>`.
pub fn scan_panics_in_file(
    path: &Path,
    binary_name: &str,
    since_unix_secs: u64,
) -> Vec<PanicEvent> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    let mut pending: Option<PanicEvent> = None;

    for line in text.lines() {
        // JSON tracing line first.
        if line.starts_with('{') {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                let ts = json
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let level = json.get("level").and_then(|v| v.as_str()).unwrap_or("");
                let msg = json
                    .get("fields")
                    .and_then(|f| f.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                if (msg.contains("panicked at") || msg.contains("PANIC"))
                    || level == "ERROR" && msg.contains("panic")
                {
                    if let Some(p) = pending.take() {
                        out.push(p);
                    }
                    if !ts.is_empty()
                        && parse_rfc3339_seconds(&ts)
                            .map(|s| s < since_unix_secs)
                            .unwrap_or(false)
                    {
                        continue;
                    }
                    let loc = json
                        .get("fields")
                        .and_then(|f| f.get("location"))
                        .and_then(|m| m.as_str())
                        .map(redact_home_path);
                    out.push(PanicEvent {
                        ts,
                        binary: binary_name.to_string(),
                        thread: None,
                        location: loc,
                        message: msg.to_string(),
                        frames: Vec::new(),
                    });
                    continue;
                }
            }
        }

        // Plain-text fallback: `thread '<name>' panicked at <loc>`. The
        // following lines are the message + indented frame lines until
        // a non-frame, non-message line breaks the run.
        if let Some(rest) = line.strip_prefix("thread '") {
            if let Some(end) = rest.find('\'') {
                let thread = &rest[..end];
                let after = &rest[end + 1..];
                if let Some(loc_start) = after.find("panicked at ") {
                    let loc_and_tail = &after[loc_start + 12..];
                    let location = loc_and_tail
                        .trim_end_matches(':')
                        .split(':')
                        .next()
                        .map(redact_home_path);
                    if let Some(p) = pending.take() {
                        out.push(p);
                    }
                    pending = Some(PanicEvent {
                        ts: String::new(),
                        binary: binary_name.to_string(),
                        thread: Some(thread.to_string()),
                        location,
                        message: String::new(),
                        frames: Vec::new(),
                    });
                    continue;
                }
            }
        }

        if let Some(p) = pending.as_mut() {
            let trimmed = line.trim_start();
            // First non-empty line after the panic header is the
            // message body. Treat as message if `message` is still
            // empty AND this isn't a frame.
            let looks_like_frame = !trimmed.is_empty()
                && (trimmed.starts_with("at ")
                    || trimmed
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false));
            if p.message.is_empty() && !looks_like_frame && !trimmed.is_empty() {
                p.message = redact_home_path(trimmed);
                continue;
            }
            if looks_like_frame {
                if p.frames.len() < 16 {
                    p.frames.push(redact_home_path(trimmed));
                }
                continue;
            }
            // Non-frame, non-empty, message-already-set -> end of panic.
            out.push(pending.take().unwrap());
        }
    }

    if let Some(p) = pending.take() {
        out.push(p);
    }

    out
}

/// Scan a tail of a file for level>=WARN events that fall within the
/// `since` window. Plain-text lines are skipped (this is post-W2; all
/// long-lived binaries emit JSON).
pub fn scan_errors_in_file(
    path: &Path,
    binary_name: &str,
    since_unix_secs: u64,
    limit: usize,
) -> Vec<ErrorEvent> {
    let bytes = match read_tail(path, 1024 * 1024) {
        Some(b) => b,
        None => return Vec::new(),
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for line in text.lines() {
        if !line.starts_with('{') {
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let level = json.get("level").and_then(|v| v.as_str()).unwrap_or("");
        if level != "ERROR" && level != "WARN" {
            continue;
        }
        let ts = json
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !ts.is_empty()
            && parse_rfc3339_seconds(&ts)
                .map(|s| s < since_unix_secs)
                .unwrap_or(false)
        {
            continue;
        }
        let target = json
            .get("target")
            .and_then(|v| v.as_str())
            .map(String::from);
        let msg = json
            .get("fields")
            .and_then(|f| f.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        out.push(ErrorEvent {
            ts,
            binary: binary_name.to_string(),
            level: level.to_string(),
            target,
            message: msg,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Scan a file for `target=fs op=<op> duration_ms=<n>` markers added by
/// W4's hot-path instrumentation. Returns ops > threshold_ms.
pub fn scan_slow_ops_in_file(
    path: &Path,
    binary_name: &str,
    since_unix_secs: u64,
    threshold_ms: u64,
) -> Vec<SlowOpEvent> {
    let bytes = match read_tail(path, 1024 * 1024) {
        Some(b) => b,
        None => return Vec::new(),
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for line in text.lines() {
        if !line.starts_with('{') {
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let fields = json.get("fields");
        let dur = fields
            .and_then(|f| f.get("duration_ms"))
            .and_then(|v| v.as_u64());
        let op = fields.and_then(|f| f.get("op")).and_then(|v| v.as_str());
        let (Some(duration_ms), Some(op)) = (dur, op) else {
            continue;
        };
        if duration_ms < threshold_ms {
            continue;
        }
        let ts = json
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !ts.is_empty()
            && parse_rfc3339_seconds(&ts)
                .map(|s| s < since_unix_secs)
                .unwrap_or(false)
        {
            continue;
        }
        out.push(SlowOpEvent {
            ts,
            binary: binary_name.to_string(),
            op: op.to_string(),
            duration_ms,
        });
    }
    out
}

/// Resolve the symbolic log name to a path under `~/.capsem/run/`.
/// Hard-coded allowlist; never honor a relative path or `..`.
pub fn host_log_path(run_dir: &Path, name: &str) -> Option<PathBuf> {
    match name {
        "service" => Some(run_dir.join("service.log")),
        "mcp" => Some(run_dir.join("mcp.log")),
        "gateway" => Some(run_dir.join("gateway.log")),
        "tray" => Some(run_dir.join("tray.log")),
        _ => None,
    }
}

/// Find the latest `*.jsonl` in `~/.capsem/logs/` (capsem-app's per-session
/// log file). Returns None if the dir is missing/empty.
pub fn latest_app_log(home: &Path) -> Option<PathBuf> {
    let dir = home.join("logs");
    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
        match &latest {
            None => latest = Some((modified, p)),
            Some((t, _)) if modified > *t => latest = Some((modified, p)),
            _ => {}
        }
    }
    latest.map(|(_, p)| p)
}

fn read_tail(path: &Path, max_bytes: u64) -> Option<Vec<u8>> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let len = metadata.len();
    let bytes = std::fs::read(path).ok()?;
    if len <= max_bytes {
        return Some(bytes);
    }
    let start = (len - max_bytes) as usize;
    let mut tail = bytes[start..].to_vec();
    if let Some(idx) = tail.iter().position(|b| *b == b'\n') {
        tail.drain(..=idx);
    }
    Some(tail)
}

fn redact_home_path(s: &str) -> String {
    // Cheap path-prefix collapse without pulling in regex. Only the two
    // common forms; covers Linux + macOS dev machines.
    if let Some(idx) = s.find("/Users/") {
        let mut owned = s.to_string();
        if let Some(end) = owned[idx + 7..].find('/') {
            let abs_end = idx + 7 + end + 1;
            owned.replace_range(idx..abs_end, "~/");
        }
        owned
    } else if let Some(idx) = s.find("/home/") {
        let mut owned = s.to_string();
        if let Some(end) = owned[idx + 6..].find('/') {
            let abs_end = idx + 6 + end + 1;
            owned.replace_range(idx..abs_end, "~/");
        }
        owned
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests;

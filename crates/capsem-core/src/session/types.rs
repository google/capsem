/// Session management: unique session IDs, session index DB, and lifecycle.
///
/// Each VM boot creates a new session with a unique ID (YYYYMMDD-HHMMSS-XXXX).
/// The session index (`main.db`) tracks metadata across sessions. Per-session
/// telemetry lives in `<session_dir>/session.db`.
///
/// Session lifecycle:
///   running -> stopped    (graceful shutdown, rollup done)
///   running -> crashed    (ungraceful, backfill on next startup)
///   stopped/crashed -> vacuumed   (DB checkpointed + vacuumed + gzipped)
///   vacuumed -> terminated        (disk artifacts deleted, only main.db record)
use serde::{Deserialize, Serialize};

/// Generate a unique session ID: YYYYMMDD-HHMMSS-XXXX (4 random hex chars).
pub fn generate_session_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let (y, m, d, hours, minutes, seconds) = epoch_to_parts(secs);

    // 4 random hex chars from timestamp nanos + XOR with a counter.
    let nanos = now.subsec_nanos();
    let rand_bits = nanos ^ std::process::id().wrapping_mul(2654435761);
    let suffix = rand_bits & 0xFFFF;

    format!(
        "{y:04}{m:02}{d:02}-{hours:02}{minutes:02}{seconds:02}-{suffix:04x}",
    )
}

/// Validate that a string looks like a valid session ID.
pub fn is_valid_session_id(s: &str) -> bool {
    // YYYYMMDD-HHMMSS-XXXX = 20 chars
    if s.len() != 20 {
        return false;
    }
    let bytes = s.as_bytes();
    // Check structure: 8 digits, dash, 6 digits, dash, 4 hex
    bytes[0..8].iter().all(|b| b.is_ascii_digit())
        && bytes[8] == b'-'
        && bytes[9..15].iter().all(|b| b.is_ascii_digit())
        && bytes[15] == b'-'
        && bytes[16..20].iter().all(|b| b.is_ascii_hexdigit())
}

/// A session record stored in main.db.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub mode: String,
    pub command: Option<String>,
    pub status: String,
    pub created_at: String,
    pub stopped_at: Option<String>,
    pub scratch_disk_size_gb: u32,
    pub ram_bytes: u64,
    pub total_requests: u64,
    pub allowed_requests: u64,
    pub denied_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_estimated_cost: f64,
    pub total_tool_calls: u64,
    pub total_mcp_calls: u64,
    pub total_file_events: u64,
    pub compressed_size_bytes: Option<u64>,
    pub vacuumed_at: Option<String>,
    /// "block" (legacy) or "virtiofs" (VirtioFS overlay).
    pub storage_mode: String,
    /// BLAKE3 hash of the rootfs squashfs used by this session.
    pub rootfs_hash: Option<String>,
    /// Version string of the rootfs (e.g., "0.9.1").
    pub rootfs_version: Option<String>,
}

/// Aggregated statistics across all sessions.
#[derive(Debug, Clone, Serialize)]
pub struct GlobalStats {
    pub total_sessions: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_estimated_cost: f64,
    pub total_tool_calls: u64,
    pub total_mcp_calls: u64,
    pub total_file_events: u64,
    pub total_requests: u64,
    pub total_allowed: u64,
    pub total_denied: u64,
}

/// Per-provider AI usage summary across sessions.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderSummary {
    pub provider: String,
    pub call_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost: f64,
    pub total_duration_ms: u64,
}

/// Per-tool usage summary across sessions.
#[derive(Debug, Clone, Serialize)]
pub struct ToolSummary {
    pub tool_name: String,
    pub call_count: u64,
    pub total_bytes: u64,
    pub total_duration_ms: u64,
}

/// Per-MCP-tool usage summary across sessions.
#[derive(Debug, Clone, Serialize)]
pub struct McpToolSummary {
    pub tool_name: String,
    pub server_name: String,
    pub call_count: u64,
    pub total_bytes: u64,
    pub total_duration_ms: u64,
}

/// Break epoch seconds into (year, month, day, hour, minute, second) UTC components.
pub(crate) fn epoch_to_parts(secs: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let mut y = 1970i64;
    let mut remaining_days = days as i64;
    loop {
        let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining_days < year_days {
            break;
        }
        remaining_days -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut m = 0u32;
    for md in &month_days {
        if remaining_days < *md {
            break;
        }
        remaining_days -= md;
        m += 1;
    }
    (y, m + 1, remaining_days as u32 + 1, hours, minutes, seconds)
}

/// Convert epoch seconds to ISO 8601 string (YYYY-MM-DDTHH:MM:SSZ).
pub fn epoch_to_iso(secs: u64) -> String {
    let (y, m, d, hours, minutes, seconds) = epoch_to_parts(secs);
    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Current UTC time as ISO 8601 string.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    epoch_to_iso(secs)
}

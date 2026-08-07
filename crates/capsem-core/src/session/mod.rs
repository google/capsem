//! Session management: unique session IDs, session index DB, and lifecycle.

mod maintenance;

#[cfg(test)]
mod tests;

pub use capsem_logger::{
    epoch_to_iso, generate_session_id, is_valid_session_id, now_iso, GlobalStats, McpToolSummary,
    ProviderSummary, SessionIndex, SessionRecord, ToolSummary,
};
pub use maintenance::*;

/// Distil a captured `process.log`/`serial.log` tail down to the one line
/// worth putting in front of a human.
///
/// A crashed boot's tail is dozens of lines of routine startup followed by
/// the error that ended it, so the last non-empty line is the cause. Callers
/// that have room for the whole tail (an HTTP error body, `capsem logs`)
/// should keep using it; this is for the single-line surfaces -- a
/// `service.log` entry, a `capsem list` row -- where the alternative has
/// historically been to log the fact of the crash and drop the reason.
pub fn boot_failure_summary(tail: &str) -> &str {
    tail.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("(log empty)")
}

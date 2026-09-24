//! The file-event rail's reads.
//!
//! Its own module because the rail has a vocabulary the rest of the reader
//! does not share: a column list whose order is a contract with
//! `read_file_event_row`.

use super::*;

/// The fs_events column list `read_file_event_row` expects, in its order.
///
/// Every column here is required. These three used to be selected through
/// `optional_column_expr`, which substituted `NULL AS trace_id` when the
/// column was absent: a ledger an older build wrote then read as a current
/// one with nothing to correlate, instead of failing. `schema/ddl.rs`
/// declares them, so a file that lacks one is broken schema and SQLite says
/// so by name.
pub(super) const FILE_EVENT_COLUMNS: &str = "timestamp, action, path, size, trace_id, credential_ref, event_id, kind";

impl DbReader {
    /// Query the most recent N file events, ordered newest first.
    pub fn recent_file_events(&self, limit: usize) -> rusqlite::Result<Vec<FileEvent>> {
        let sql = format!("SELECT {FILE_EVENT_COLUMNS} FROM fs_events ORDER BY id DESC LIMIT ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], read_file_event_row)?;
        rows.collect()
    }

    /// Search file events by path substring.
    pub fn search_file_events(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<FileEvent>> {
        let pattern = format!("%{query}%");
        let sql = format!("SELECT {FILE_EVENT_COLUMNS} FROM fs_events WHERE path LIKE ?1 ORDER BY id DESC LIMIT ?2");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, limit as i64], read_file_event_row)?;
        rows.collect()
    }
}

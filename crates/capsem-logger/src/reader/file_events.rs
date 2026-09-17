//! The file-event rail's reads.
//!
//! Its own module because the rail has a vocabulary the rest of the reader
//! does not share: a column list whose order is a contract with
//! `read_file_event_row`, and a `total` that deliberately excludes the
//! overflow marker, which is a record of absence rather than a change to a
//! path.

use super::*;

impl DbReader {
    /// The fs_events column list `read_file_event_row` expects. `kind` is
    /// required and selected outright: a ledger without it is broken schema.
    fn file_event_columns(&self) -> String {
        format!(
            "timestamp, action, path, size, {}, {}, {}, kind",
            self.optional_column_expr("fs_events", "trace_id"),
            self.optional_column_expr("fs_events", "credential_ref"),
            self.optional_column_expr("fs_events", "event_id"),
        )
    }

    /// Query the most recent N file events, ordered newest first.
    pub fn recent_file_events(&self, limit: usize) -> rusqlite::Result<Vec<FileEvent>> {
        let sql = format!(
            "SELECT {} FROM fs_events ORDER BY id DESC LIMIT ?1",
            self.file_event_columns()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], read_file_event_row)?;
        rows.collect()
    }

    /// Search file events by path substring.
    pub fn search_file_events(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<FileEvent>> {
        let pattern = format!("%{query}%");
        let sql = format!(
            "SELECT {} FROM fs_events WHERE path LIKE ?1 ORDER BY id DESC LIMIT ?2",
            self.file_event_columns()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, limit as i64], read_file_event_row)?;
        rows.collect()
    }

    /// Aggregate file event statistics. All aggregation done in SQL.
    ///
    /// `total` counts changes to paths. An overflow marker is not one -- it is
    /// the record saying some changes went unrecorded in a window -- so
    /// counting it as a file event would inflate the very number a reader
    /// checks to see how much happened. It has its own count instead.
    pub fn file_event_stats(&self) -> rusqlite::Result<FileEventStats> {
        self.conn.query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN action != 'overflow' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN action = 'created' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN action = 'modified' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN action = 'deleted' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN action = 'restored' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN action = 'overflow' THEN 1 ELSE 0 END), 0)
             FROM fs_events",
            [],
            |row| {
                Ok(FileEventStats {
                    total: row.get::<_, i64>(0)? as u64,
                    created: row.get::<_, i64>(1)? as u64,
                    modified: row.get::<_, i64>(2)? as u64,
                    deleted: row.get::<_, i64>(3)? as u64,
                    restored: row.get::<_, i64>(4)? as u64,
                    overflow_windows: row.get::<_, i64>(5)? as u64,
                })
            },
        )
    }
}

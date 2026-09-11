//! Reader readiness and reconciliation against the writer-owned disk schema.
use super::*;

impl DbReader {
    /// Validate that this ledger is structurally ready for route reads.
    ///
    /// Empty ledgers are valid. Missing tables or route-critical columns are
    /// DB contract failures and must not be converted into empty route payloads.
    pub fn ready(&self) -> Result<(), String> {
        schema::validate_ready_schema(&self.conn)
    }

    /// Refresh DB-owned hot memory tables from disk for externally-written DBs.
    ///
    /// Normal writer-owned handles do not call this on read because their
    /// memory tables are the write truth. Service session route handles use it
    /// because capsem-process owns the writes and disk is the process boundary.
    pub(crate) fn sync_from_disk(&self) -> rusqlite::Result<()> {
        // Skip copying when no external connection has committed.
        // `data_version` is SQLite's own answer to "did another connection
        // commit since I last looked"; a writer in another process moves it,
        // this connection's own memory-schema writes do not.
        let data_version: i64 = self.conn.query_row("PRAGMA main.data_version", [], |row| row.get(0))?;
        if self.synced_data_version.get() == Some(data_version) {
            return Ok(());
        }
        let schema_version: i64 = self
            .conn
            .query_row("PRAGMA main.schema_version", [], |row| row.get(0))?;
        let schema_changed = self.synced_schema_version.get() != Some(schema_version);
        self.conn.pragma_update(None, "query_only", "OFF")?;
        let result = schema::with_memory_schema_lock(|| {
            if schema_changed {
                schema::reconcile_memory_tables_from_disk(&self.conn)?;
                schema::validate_ready_schema(&self.conn).map_err(rusqlite::Error::InvalidParameterName)?;
            }
            schema::sync_memory_tables_from_disk(&self.conn, schema::hot_ledger_tables())?;
            if schema_changed {
                schema::create_memory_read_views(&self.conn)?;
            }
            Ok(())
        });
        let restore = self.conn.pragma_update(None, "query_only", "ON");
        if result.is_ok() {
            // Recorded as read before the copy: a commit that lands during
            // the copy leaves a newer version behind and triggers the next one.
            self.synced_data_version.set(Some(data_version));
            self.synced_schema_version.set(Some(schema_version));
            self.disk_syncs.set(self.disk_syncs.get() + 1);
        }
        result.and(restore)
    }

    /// How many times the memory tables were rebuilt from disk.
    pub fn disk_syncs(&self) -> u64 {
        self.disk_syncs.get()
    }
}

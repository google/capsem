//! Reader readiness, and the one question a cross-process reader must ask.
use super::*;

impl DbReader {
    /// Validate that this ledger is structurally ready for route reads.
    ///
    /// Empty ledgers are valid. Missing tables or route-critical columns are
    /// DB contract failures and must not be converted into empty route payloads.
    pub fn ready(&self) -> Result<(), String> {
        schema::validate_ready_schema(&self.conn, self.memory_mirror)
    }

    /// The ledger's `data_version` when it differs from the last one committed
    /// here, or `None` when nothing has changed since.
    ///
    /// `data_version` is SQLite's own answer to "did another connection commit
    /// since I last looked". Neither the file's size nor its mtime answers that
    /// question -- a WAL-only commit moves neither -- and a reader in another
    /// process has nothing else to go on.
    ///
    /// Observing is deliberately separate from recording: the observation is a
    /// promise that everything derived from it will be redone, so it is only
    /// committed once that work has actually succeeded. See
    /// `commit_observed_version`.
    pub(crate) fn observe_data_version(&self) -> rusqlite::Result<Option<i64>> {
        let data_version: i64 = self.conn.query_row("PRAGMA main.data_version", [], |row| row.get(0))?;
        Ok((self.synced_data_version.get() != Some(data_version)).then_some(data_version))
    }

    /// Record an observation whose dependent work completed successfully.
    ///
    /// Committing it earlier would lose the change: a query that then failed
    /// would leave this reader believing it had already accounted for a commit
    /// it never read, and every later poll would answer from a cache built
    /// before it -- until the writer happened to commit again.
    pub(crate) fn commit_observed_version(&self, data_version: i64) {
        self.synced_data_version.set(Some(data_version));
        self.disk_syncs.set(self.disk_syncs.get() + 1);
    }

    /// How many observed ledger changes this reader has committed.
    #[cfg(test)]
    pub(crate) fn disk_syncs(&self) -> u64 {
        self.disk_syncs.get()
    }

    /// How many caller-owned queries this reader actually executed.
    ///
    /// A batch served from the handle's `query_many` cache never reaches the
    /// reader, so this is what distinguishes a cache hit from a re-execution.
    #[cfg(test)]
    pub(crate) fn queries_executed(&self) -> u64 {
        self.queries_executed.get()
    }

    pub(crate) fn record_query_executed(&self) {
        self.queries_executed.set(self.queries_executed.get() + 1);
    }

    /// Schema names attached to this reader's connection (`main`, `temp`, and
    /// `mem` only when this reader mirrors the hot tables).
    #[cfg(test)]
    pub(crate) fn attached_schemas(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("PRAGMA database_list")?;
        let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
        names.collect()
    }

    /// How long this reader waits out a file lock before failing, in ms.
    #[cfg(test)]
    pub(crate) fn busy_timeout_ms(&self) -> rusqlite::Result<i64> {
        self.conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))
    }
}

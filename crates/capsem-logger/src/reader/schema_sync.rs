//! Reader readiness, and the one question a cross-process reader must ask.
use super::*;

/// What this reader has seen of the file, and what it did about it.
#[derive(Default)]
pub(super) struct SyncState {
    /// `PRAGMA main.data_version` as of the last change this reader both saw
    /// and finished acting on. It moves only when another connection commits
    /// to the file, so an unchanged value means results derived from it are
    /// still current.
    synced_data_version: Cell<Option<i64>>,
    disk_syncs: Cell<u64>,
    queries_executed: Cell<u64>,
    /// `PRAGMA schema_version` of the last schema this reader found ready.
    /// Readiness is a property of the schema, so it holds until DDL moves the
    /// version; a row commit leaves it where it is.
    ready_schema_version: Cell<Option<i64>>,
    shape_validations: Cell<u64>,
}

impl DbReader {
    /// Validate that this ledger is structurally ready for route reads.
    ///
    /// Empty ledgers are valid. Missing tables or route-critical columns are
    /// DB contract failures and must not be converted into empty route payloads.
    ///
    /// The shape is validated once per `schema_version`: only DDL can change
    /// it, and DDL moves the version, including DDL from another connection.
    /// Only success is remembered. A failure is re-validated on the next call,
    /// which is cheap because validation reads the schema, never the rows --
    /// and must stay so, since every poller retries a failure.
    ///
    /// This says nothing about page integrity, and an unchanged version is no
    /// evidence against corruption. A damaged page fails the query that reads
    /// it; a full scan belongs to the ledger copy, never to readiness.
    pub fn ready(&self) -> Result<(), String> {
        let schema_version: i64 = self
            .conn
            .prepare_cached("PRAGMA main.schema_version")
            .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
            .map_err(|error| format!("session db schema version unreadable: {error}"))?;
        if self.sync.ready_schema_version.get() == Some(schema_version) {
            return Ok(());
        }
        self.sync.shape_validations.set(self.sync.shape_validations.get() + 1);
        schema::validate_ready_schema(&self.conn)?;
        self.sync.ready_schema_version.set(Some(schema_version));
        Ok(())
    }

    /// How many times this reader validated the schema's shape.
    #[cfg(test)]
    pub(crate) fn shape_validations(&self) -> u64 {
        self.sync.shape_validations.get()
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
        let data_version: i64 = self
            .conn
            .prepare_cached("PRAGMA main.data_version")?
            .query_row([], |row| row.get(0))?;
        Ok((self.sync.synced_data_version.get() != Some(data_version)).then_some(data_version))
    }

    /// Record an observation whose dependent work completed successfully.
    ///
    /// Committing it earlier would lose the change: a query that then failed
    /// would leave this reader believing it had already accounted for a commit
    /// it never read, and every later poll would answer from a cache built
    /// before it -- until the writer happened to commit again.
    pub(crate) fn commit_observed_version(&self, data_version: i64) {
        self.sync.synced_data_version.set(Some(data_version));
        self.sync.disk_syncs.set(self.sync.disk_syncs.get() + 1);
    }

    /// How many observed ledger changes this reader has committed.
    #[cfg(test)]
    pub(crate) fn disk_syncs(&self) -> u64 {
        self.sync.disk_syncs.get()
    }

    /// How many caller-owned queries this reader actually executed.
    ///
    /// A batch served from the handle's `query_many` cache never reaches the
    /// reader, so this is what distinguishes a cache hit from a re-execution.
    #[cfg(test)]
    pub(crate) fn queries_executed(&self) -> u64 {
        self.sync.queries_executed.get()
    }

    pub(crate) fn record_query_executed(&self) {
        self.sync.queries_executed.set(self.sync.queries_executed.get() + 1);
    }

    /// Schema names attached to this reader's connection.
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

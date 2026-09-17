//! How a query-only reader connection is opened.
//!
//! Two shapes, and the difference is which process owns the writes. Sharing a
//! process with the writer means sharing its table locks, so that reader
//! mirrors the hot tables into `mem` and reads the mirror. A reader in another
//! process has WAL between it and the writer already, so it reads `main` and
//! holds no copy at all.

use super::*;

impl DbReader {
    /// Open a query-only connection that mirrors the hot tables into `mem`.
    ///
    /// This is the in-process reader: the writer lives in the same process and
    /// the mirror is what keeps the two off each other's table locks.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::open_with(path, true)
    }

    /// Open a query-only connection that reads `main` through WAL.
    ///
    /// This is the cross-process reader. The writer is another process, so WAL
    /// already gives lock-free reads of the file and a RAM mirror would only
    /// duplicate every hot table of every session the service watches.
    pub fn open_disk_only(path: &Path) -> rusqlite::Result<Self> {
        Self::open_with(path, false)
    }

    /// Nothing on this path writes `main` -- `upgrade_legacy` was the last
    /// thing that did -- so `SQLITE_OPEN_READ_ONLY` looks like the obvious
    /// tightening. It is not, and the reason is worth keeping: a WAL database
    /// opened read-only needs its `-shm` index to already exist, because it
    /// may not create one. A live session has it, so this would pass every
    /// test that opens a ledger beside its writer, and fail on exactly the
    /// case that matters -- reading a retained session whose writer is gone
    /// and whose sidecars were checkpointed away. `apply_reader_pragmas` keeps
    /// the connection query-only; the flag stays read-write so the file can be
    /// opened at all.
    fn open_with(path: &Path, memory_mirror: bool) -> rusqlite::Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(path, flags)?;
        schema::transport::assert_current(&conn)?;
        if memory_mirror {
            let memory_uri = schema::memory_uri_for_path(path);
            schema::with_memory_schema_lock(|| {
                schema::create_memory_tables(&conn, &memory_uri)?;
                schema::rehydrate_memory_tables_from_disk_once(&conn, schema::hot_ledger_tables())?;
                schema::create_memory_read_views(&conn)
            })?;
        }
        schema::apply_reader_pragmas(&conn, memory_mirror)?;
        schema::record_sqlite_mmap_telemetry(&conn, path, "reader", "open");
        Ok(Self {
            conn,
            memory_mirror,
            synced_data_version: Cell::new(None),
            disk_syncs: Cell::new(0),
            queries_executed: Cell::new(0),
        })
    }

    /// Open an in-memory database (for testing; typically unused since
    /// in-memory DBs can't be shared between connections).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::apply_pragmas(&conn)?; // in-memory is read-write, pragmas are fine
        schema::create_tables(&conn)?;
        let memory_uri = schema::memory_uri_for_name(&format!(
            "reader-open-in-memory-{}-{}",
            std::process::id(),
            IN_MEMORY_READER_ID.fetch_add(1, Ordering::Relaxed)
        ));
        schema::with_memory_schema_lock(|| {
            schema::create_memory_tables(&conn, &memory_uri)?;
            schema::rehydrate_memory_tables_from_disk_once(&conn, schema::hot_ledger_tables())
        })?;
        Ok(Self {
            conn,
            memory_mirror: true,
            synced_data_version: Cell::new(None),
            disk_syncs: Cell::new(0),
            queries_executed: Cell::new(0),
        })
    }
}

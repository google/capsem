//! How a query-only reader connection is opened.
//!
//! Every reader reads `main` through WAL, whether its writer is in this
//! process or another. The writer's memory schema holds only rows it has not
//! flushed yet, so there is nothing in it a reader could want (#213).

use super::*;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

impl super::rawquery::QueryDeadline {
    pub(super) fn install(conn: &mut Connection) -> rusqlite::Result<Self> {
        let epoch = Instant::now();
        let expires = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&expires);
        conn.progress_handler(
            1_000,
            Some(move || {
                let deadline = observed.load(Ordering::Relaxed);
                deadline != 0 && epoch.elapsed().as_nanos() >= u128::from(deadline)
            }),
        )?;
        Ok(Self { epoch, expires })
    }
}

impl DbReader {
    /// Open a query-only connection that reads the ledger file.
    ///
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
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(path, flags)?;
        schema::transport::assert_current(&conn)?;
        schema::apply_reader_pragmas(&conn)?;
        schema::record_sqlite_mmap_telemetry(&conn, path, "reader", "open");
        Self::with_connection(conn)
    }

    /// Open an in-memory database (for testing; typically unused since
    /// in-memory DBs can't be shared between connections).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::apply_pragmas(&conn)?; // in-memory is read-write, pragmas are fine
        schema::create_tables(&conn)?;
        Self::with_connection(conn)
    }

    fn with_connection(mut conn: Connection) -> rusqlite::Result<Self> {
        // Route SQL is a fixed set of statements polled on a timer; compiling
        // each one again on every poll was a measurable share of a route's
        // CPU. Room for every polled route's statements, as the writer has.
        conn.set_prepared_statement_cache_capacity(capsem_proto::CACHE_SQLITE_PREPARED_STATEMENTS);
        let query_deadline = super::rawquery::QueryDeadline::install(&mut conn)?;
        Ok(Self {
            conn,
            sync: Default::default(),
            query_deadline,
        })
    }
}

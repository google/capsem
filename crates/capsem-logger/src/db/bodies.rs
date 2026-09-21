//! Reading archived bodies back through the DB handle.
//!
//! The index lives in SQLite and the bytes live in `session.bodies`; both are
//! DB-owned. A route asks for a body by event id and gets bytes, exactly as it
//! asks for rows and gets rows. It never learns there is a second file, never
//! opens one, and never sees an `ArchiveError`: a body the index names and the
//! file cannot produce is a broken ledger and fails loudly.

use std::time::{Duration, Instant};

use capsem_archive::{ArchiveError, BlockExtent, BodyLogReader, BodyRef};
use capsem_foundation::unix::contained::ContainedDir;
use capsem_foundation::unix::lock::{self, LockMode};
use rusqlite::{Connection, Row};
use serde_json::Value;

use super::{DbHandle, DbResult};
use crate::writer::RetainOutcome;

/// Which side of an exchange a stored body is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyDirection {
    Request,
    Response,
    Payload,
    Stdout,
    Stderr,
}

impl BodyDirection {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Response => "response",
            Self::Payload => "payload",
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "request" => Some(Self::Request),
            "response" => Some(Self::Response),
            "payload" => Some(Self::Payload),
            "stdout" => Some(Self::Stdout),
            "stderr" => Some(Self::Stderr),
            _ => None,
        }
    }
}

/// One archived body and the index row that describes it.
#[derive(Debug, Clone)]
pub struct StoredBody {
    pub event_id: String,
    pub source_table: String,
    pub direction: BodyDirection,
    pub content_type: Option<String>,
    pub original_bytes: u64,
    pub truncated: bool,
    pub body_hash: String,
    pub bytes: Vec<u8>,
}

/// Archived bodies, and what a budget left behind.
///
/// `truncated_rows` is not an error: the index named that many more bodies and
/// the budget refused them. A caller that reports nothing has told its user the
/// ledger was empty, which is a different statement.
#[derive(Debug, Default, Clone)]
pub struct ArchivedBodies {
    pub bodies: Vec<StoredBody>,
    pub truncated_rows: usize,
}

/// SQLite's default `SQLITE_MAX_VARIABLE_NUMBER` is 999, and two of them are
/// the source table and the direction. A longer event list is chunked rather
/// than interpolated: the ids come from ledger rows, and a query built by
/// concatenation is a query one careless change makes injectable.
const MAX_EVENT_IDS_PER_QUERY: usize = 997;

/// An index row, before its bytes are fetched.
pub(super) struct IndexRow {
    pub(super) event_id: String,
    pub(super) source_table: String,
    pub(super) direction: BodyDirection,
    pub(super) content_type: Option<String>,
    pub(super) original_bytes: u64,
    pub(super) truncated: bool,
    pub(super) body_hash: String,
    pub(super) reference: BodyRef,
    pub(super) extent: BlockExtent,
}

pub(super) struct CapturedBodies {
    pub(super) reader: BodyLogReader,
    pub(super) rows: Vec<IndexRow>,
}

pub(super) struct ArchiveSqlDeadline<'a> {
    conn: &'a Connection,
    deadline: Instant,
}

impl<'a> ArchiveSqlDeadline<'a> {
    pub(super) fn install(conn: &'a Connection, deadline: Instant) -> Self {
        conn.progress_handler(1_000, Some(move || Instant::now() >= deadline));
        Self { conn, deadline }
    }

    pub(super) fn check(&self, operation: &str) -> DbResult<()> {
        if Instant::now() >= self.deadline {
            Err(format!("{operation} exceeded its deadline"))
        } else {
            Ok(())
        }
    }
}

impl Drop for ArchiveSqlDeadline<'_> {
    fn drop(&mut self) {
        self.conn.progress_handler(0, None::<fn() -> bool>);
    }
}

const INTERACTIVE_CAPTURE_MAX_IDS: usize = 10_000;
const INTERACTIVE_CAPTURE_MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const INTERACTIVE_CAPTURE_DEADLINE: Duration = Duration::from_secs(5);

#[cfg(test)]
struct CapturePause {
    reached: std::sync::mpsc::SyncSender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
static CAPTURE_PAUSES: std::sync::Mutex<Option<std::collections::HashMap<std::path::PathBuf, CapturePause>>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn pause_next_archive_capture_for_tests(
    db_path: &std::path::Path,
) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::SyncSender<()>) {
    let (reached_tx, reached_rx) = std::sync::mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(0);
    CAPTURE_PAUSES
        .lock()
        .unwrap()
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(
            db_path.to_path_buf(),
            CapturePause {
                reached: reached_tx,
                resume: resume_rx,
            },
        );
    (reached_rx, resume_tx)
}

#[cfg(test)]
pub(super) fn pause_archive_capture_for_tests(db_path: &std::path::Path) {
    let pause = CAPTURE_PAUSES
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|map| map.remove(db_path));
    if let Some(pause) = pause {
        pause.reached.send(()).unwrap();
        pause.resume.recv().unwrap();
    }
}

#[cfg(not(test))]
pub(super) fn pause_archive_capture_for_tests(_db_path: &std::path::Path) {}

/// The ten index columns, in the order [`index_row`] reads them. A query that
/// carries extra columns of its own -- the WARC export joins each body to its
/// source row -- puts them after these.
pub(super) const INDEX_COLUMNS: &str = "b.event_id, b.source_table, b.direction, b.content_type, b.original_bytes, \
                                        b.truncated, b.body_hash, b.block_offset, b.body_offset, b.body_len, \
                                        blocks.disk_len, blocks.raw_len";
pub(super) const INDEX_FROM: &str =
    "event_body_blobs AS b JOIN body_blocks AS blocks ON blocks.block_offset = b.block_offset";

/// Rows are ordered by block so the reader inflates each block once: the
/// request and response of one exchange are staged together and almost always
/// share a block.
const INDEX_ORDER: &str = "ORDER BY b.block_offset, b.body_offset";

impl DbHandle {
    /// Read one archived body, or `None` when the ledger has no such row.
    ///
    /// Named by the index's whole unique key. It used to take only the event
    /// and the direction and return the first match, which was one row until
    /// the security ledgers each began archiving a `payload`: a rule match, the
    /// decision it drove and the ask it raised name the same event, and "the
    /// payload of this event" stopped meaning one body. A read that could hand
    /// back the decision's bytes to a caller asking for the rule's is a read
    /// that answers a different question than it was asked.
    pub async fn read_body(
        &self,
        event_id: &str,
        source_table: &str,
        direction: BodyDirection,
    ) -> DbResult<Option<StoredBody>> {
        let sql = format!(
            "SELECT {INDEX_COLUMNS} FROM {INDEX_FROM}
             WHERE b.event_id = ?1 AND b.source_table = ?2 AND b.direction = ?3 {INDEX_ORDER}"
        );
        let captured = self
            .capture_body_rows(
                vec![(
                    sql,
                    vec![event_id.into(), source_table.into(), direction.as_str().into()],
                )],
                1,
            )
            .await?;
        Ok(self.read_archived(captured).await?.into_iter().next())
    }

    /// Read every archived body of one event, in one index query.
    pub async fn read_bodies(&self, event_id: &str) -> DbResult<Vec<StoredBody>> {
        let sql = format!("SELECT {INDEX_COLUMNS} FROM {INDEX_FROM} WHERE b.event_id = ?1 {INDEX_ORDER}");
        let captured = self.capture_body_rows(vec![(sql, vec![event_id.into()])], 1).await?;
        self.read_archived(captured).await
    }

    /// Read one direction's archived body for a named set of events.
    ///
    /// The per-event reads above answer "show me this exchange". This answers
    /// "I have a page of rows and I need the body of each" -- asking event by
    /// event would cost an index query and a blocking task per row, and would
    /// inflate the same block once for every body that sits in it. Here it is
    /// one query per chunk, each read in archive order.
    ///
    /// The caller names the events rather than asking for "the newest N",
    /// because the two windows are chosen by different orderings and a page of
    /// rows whose payloads came from a different page is a projection that
    /// lies. Whatever the caller listed is what it gets back.
    ///
    /// `max_total_bytes` is the second bound, and the one that matters: a row
    /// count alone permits `event_ids.len()` times the 10 MiB body cap in
    /// resident memory. A body that does not fit what is left of the budget is
    /// skipped and counted rather than silently dropped, so a caller can say
    /// so; the walk continues, so a smaller body later in the page may still
    /// fit. What the budget guarantees is the ceiling on resident bytes, not
    /// that the result is a prefix of the page.
    ///
    /// # Errors
    ///
    /// The same as the per-event reads: a row whose bytes the archive cannot
    /// produce, or does not produce intact, is a broken ledger and fails.
    pub async fn read_bodies_for_events(
        &self,
        event_ids: &[&str],
        source_table: &str,
        direction: BodyDirection,
        max_total_bytes: usize,
    ) -> DbResult<ArchivedBodies> {
        let mut archived = ArchivedBodies::default();
        let mut queries = Vec::new();
        // One budget for the whole page, not one per chunk: the chunking is a
        // SQLite parameter limit, not a unit of memory anyone agreed to.
        let mut budget = max_total_bytes;
        for chunk in event_ids.chunks(MAX_EVENT_IDS_PER_QUERY) {
            let placeholders = (3..3 + chunk.len())
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT {INDEX_COLUMNS} FROM {INDEX_FROM}
                 WHERE b.source_table = ?1 AND b.direction = ?2 AND b.event_id IN ({placeholders})
                 {INDEX_ORDER}"
            );
            let mut params: Vec<Value> = Vec::with_capacity(chunk.len() + 2);
            params.push(source_table.into());
            params.push(direction.as_str().into());
            params.extend(chunk.iter().map(|event_id| Value::from(*event_id)));
            queries.push((sql, params));
        }
        let mut captured = self.capture_body_rows(queries, event_ids.len()).await?;
        // Split before reading, so the budget bounds what is inflated and
        // held rather than what is thrown away afterwards.
        let mut affordable = Vec::with_capacity(captured.rows.len());
        for row in captured.rows.drain(..) {
            let cost = row.reference.len as usize;
            if cost > budget {
                archived.truncated_rows += 1;
                continue;
            }
            budget -= cost;
            affordable.push(row);
        }
        captured.rows = affordable;
        archived.bodies.extend(self.read_archived(captured).await?);
        Ok(archived)
    }

    /// Drop every archived body whose block was last written before `cutoff` (RFC 3339),
    /// compacting `session.bodies` and rewriting the index rows that name it.
    ///
    /// Writer-owning handles only. `capsem-process` owns every write to a
    /// session ledger, retention included; the service's handles are external
    /// disk readers and get the same refusal `write` gives them, because a
    /// second process rewriting the archive under the writer is exactly the
    /// thing the single-writer rule exists to prevent.
    pub async fn retain_bodies_since(&self, cutoff: &str) -> DbResult<RetainOutcome> {
        let Some(writer) = &self.inner.writer else {
            let error =
                "db handle is read-only; session body retention must use the owning process DB handle".to_string();
            tracing::error!(
                db_path = %self.inner.path.display(),
                operation = "retain_bodies_since",
                error = %error,
                "session db handle operation failed"
            );
            return Err(error);
        };
        writer.retain_bodies_since(cutoff).await
    }

    async fn capture_body_rows(
        &self,
        queries: Vec<super::DbQueryOwned>,
        requested_ids: usize,
    ) -> DbResult<CapturedBodies> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.inner
            .reader_tx
            .send(super::ReadRequest::CaptureBodies {
                queries,
                requested_ids,
                reply,
            })
            .map_err(|error| format!("db reader worker closed: {error}"))?;
        rx.await
            .map_err(|error| format!("db reader worker dropped archive capture reply: {error}"))?
            .map(|observed| self.take_observed(observed))
    }

    /// Resolve index rows to bytes on a blocking thread: inflating a block is
    /// CPU work on a file, and neither belongs on the async runtime.
    async fn read_archived(&self, captured: CapturedBodies) -> DbResult<Vec<StoredBody>> {
        let handle = self.clone();
        tokio::task::spawn_blocking(move || handle.read_archived_blocking(captured))
            .await
            .map_err(|error| format!("session body archive read task failed: {error}"))?
    }

    fn read_archived_blocking(&self, captured: CapturedBodies) -> DbResult<Vec<StoredBody>> {
        let CapturedBodies { reader, rows } = captured;
        let result = rows.into_iter().map(|row| read_one(&reader, row)).collect();
        self.inner
            .archive_blocks_inflated
            .fetch_add(reader.blocks_inflated(), std::sync::atomic::Ordering::Relaxed);
        result
    }

    /// Drop the cached archive reader, so the next read opens the file again.
    ///
    /// Test-only: production discards a stale reader by noticing the archive
    /// was replaced, which needs no one to remember to call anything.
    #[cfg(test)]
    pub(crate) fn archive_reader_reset(&self) {}

    /// How many blocks this handle's reader has inflated.
    #[cfg(test)]
    pub(crate) fn archive_blocks_inflated_for_tests(&self) -> u64 {
        self.inner
            .archive_blocks_inflated
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Why one archived body could not be produced, and how far the damage reaches.
///
/// The distinction is what a caller reading many rows needs. Almost every way
/// a read fails is local: `RefOutOfRange` is this one row, whose recorded span
/// falls outside a block that inflated perfectly well, and a bad header, a
/// truncated payload or a failed inflate are that one block. Only a seek
/// failure is about the file, and a file that cannot be opened at all never
/// reaches here -- `with_archive_reader` fails on the open before any row is
/// read.
///
/// Getting this wrong was a real asymmetry: an index row edited to a
/// wrong-but-in-range span returned bytes, failed the hash and was skipped,
/// while the same row edited a little further aborted the whole export. The
/// reader's own module doc names index tampering as the expected shape.
///
/// The interactive reads below flatten all of it into one error, because a
/// route asked for that body and has nothing to show without it. The export
/// tells them apart.
#[derive(Debug)]
pub(super) enum BodyFault {
    /// Local to this row or its block: an out-of-range span, a bad block
    /// header, a truncated payload, a block that will not inflate or does not
    /// match its own hash. Every other row is still worth trying.
    Unreadable(String),
    /// The file itself would not seek. The handle is in no state to answer the
    /// next row either.
    FileIo(String),
    /// The bytes came back and are not the bytes the index recorded.
    Corrupt(String),
}

impl BodyFault {
    pub(super) fn into_message(self) -> String {
        match self {
            Self::Unreadable(message) | Self::FileIo(message) | Self::Corrupt(message) => message,
        }
    }
}

pub(super) fn read_one(reader: &BodyLogReader, row: IndexRow) -> DbResult<StoredBody> {
    read_one_checked(reader, row).map_err(BodyFault::into_message)
}

/// Resolve one index row to its bytes, keeping the two failure kinds apart.
pub(super) fn read_one_checked(reader: &BodyLogReader, row: IndexRow) -> Result<StoredBody, BodyFault> {
    let bytes = reader.read_bounded(row.reference, row.extent).map_err(|error| {
        let message = format!(
            "session body archive could not resolve {}/{} of event {}: {error}",
            row.source_table,
            row.direction.as_str(),
            row.event_id
        );
        match error {
            // The only variant the read raises about the file rather than
            // about one row or one block: the reader seeks before it reads
            // a header or a segment, and every other failure it can produce
            // is named after the offset it happened at.
            ArchiveError::Io(_) => BodyFault::FileIo(message),
            _ => BodyFault::Unreadable(message),
        }
    })?;
    // The archive verifies its own block; this verifies the span of it the
    // index row picked out. A block's hash cannot notice an index row that
    // was edited to name a different offset inside the same valid block, and
    // that row would otherwise be served as this event's body.
    let hash = format!("blake3:{}", blake3::hash(&bytes).to_hex());
    if hash != row.body_hash {
        return Err(BodyFault::Corrupt(format!(
            "session body archive returned the wrong bytes for {}/{} of event {}: index says {}, bytes hash to {hash}",
            row.source_table,
            row.direction.as_str(),
            row.event_id,
            row.body_hash
        )));
    }
    Ok(StoredBody {
        event_id: row.event_id,
        source_table: row.source_table,
        direction: row.direction,
        content_type: row.content_type,
        original_bytes: row.original_bytes,
        truncated: row.truncated,
        body_hash: row.body_hash,
        bytes,
    })
}

pub(super) fn capture_body_rows(
    conn: &Connection,
    db_path: &std::path::Path,
    queries: Vec<super::DbQueryOwned>,
    requested_ids: usize,
) -> DbResult<CapturedBodies> {
    if requested_ids > INTERACTIVE_CAPTURE_MAX_IDS {
        return Err(format!(
            "archive capture requested {requested_ids} ids; limit is {INTERACTIVE_CAPTURE_MAX_IDS}"
        ));
    }
    let deadline = Instant::now() + INTERACTIVE_CAPTURE_DEADLINE;
    let archive_lock = lock::acquire_existing_until(
        &crate::writer::archive_lock_path_for_db(db_path),
        LockMode::Shared,
        deadline,
    )
    .map_err(|error| format!("acquire archive capture lock: {error}"))?;
    let sql_deadline = ArchiveSqlDeadline::install(conn, deadline);
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("begin archive capture snapshot: {error}"))?;
    let state = crate::schema::archive_state(&tx).map_err(|error| format!("read archive_state: {error}"))?;
    let directory = ContainedDir::open_root(&crate::writer::archive_path_for_db(db_path))
        .and_then(|directory| {
            directory.validate_private()?;
            Ok(directory)
        })
        .map_err(|error| format!("open archive generation directory: {error}"))?;
    let reader = BodyLogReader::open_generation(&directory, state.header, state.committed_end)
        .map_err(|error| format!("open captured archive generation: {error}"))?;
    drop(archive_lock);
    pause_archive_capture_for_tests(db_path);

    const MAX_CAPTURED_ROWS: usize = 10_000;
    let mut captured = Vec::new();
    let mut metadata_bytes = 0usize;
    for (sql, params) in queries {
        sql_deadline.check("archive metadata capture")?;
        let mut statement = tx.prepare(&sql).map_err(|error| error.to_string())?;
        let owned = sqlite_params(&params);
        let refs: Vec<&dyn rusqlite::types::ToSql> = owned.iter().map(|value| value.as_ref()).collect();
        let mut rows = statement.query(refs.as_slice()).map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            if captured.len() == MAX_CAPTURED_ROWS {
                return Err(format!("archive capture exceeds {MAX_CAPTURED_ROWS} rows"));
            }
            let index = index_row_sql(row)?;
            metadata_bytes = checked_interactive_metadata_bytes(metadata_bytes, index_metadata_bytes(&index))?;
            captured.push(index);
        }
    }
    tx.commit()
        .map_err(|error| format!("commit archive capture snapshot: {error}"))?;
    Ok(CapturedBodies { reader, rows: captured })
}

pub(super) fn checked_interactive_metadata_bytes(current: usize, additional: usize) -> DbResult<usize> {
    let next = current
        .checked_add(additional)
        .ok_or_else(|| "archive capture metadata size overflow".to_string())?;
    if next > INTERACTIVE_CAPTURE_MAX_METADATA_BYTES {
        return Err(format!(
            "archive capture metadata exceeds {INTERACTIVE_CAPTURE_MAX_METADATA_BYTES} bytes"
        ));
    }
    Ok(next)
}

fn index_metadata_bytes(index: &IndexRow) -> usize {
    std::mem::size_of::<IndexRow>()
        + index.event_id.len()
        + index.source_table.len()
        + index.content_type.as_ref().map_or(0, String::len)
        + index.body_hash.len()
}

pub(super) fn sqlite_params(params: &[Value]) -> Vec<Box<dyn rusqlite::types::ToSql>> {
    params
        .iter()
        .map(|value| -> Box<dyn rusqlite::types::ToSql> {
            match value {
                Value::Null => Box::new(rusqlite::types::Null),
                Value::Bool(value) => Box::new(i64::from(*value)),
                Value::Number(value) => value.as_i64().map_or_else(
                    || Box::new(value.as_f64()) as Box<dyn rusqlite::types::ToSql>,
                    |v| Box::new(v),
                ),
                Value::String(value) => Box::new(value.clone()),
                Value::Array(_) | Value::Object(_) => Box::new(rusqlite::types::Null),
            }
        })
        .collect()
}

pub(super) fn index_row_sql(row: &Row<'_>) -> DbResult<IndexRow> {
    let direction_text = bounded_sql_text(row, 2, 32)?;
    let direction = BodyDirection::parse(&direction_text)
        .ok_or_else(|| format!("body index row has unknown direction {direction_text}"))?;
    let block_offset = unsigned(row, 7, "block offset")?;
    let body_offset = u32::try_from(unsigned(row, 8, "body offset")?)
        .map_err(|_| "body index row has an unreadable offset".to_string())?;
    let body_len = u32::try_from(unsigned(row, 9, "body length")?)
        .map_err(|_| "body index row has an unreadable length".to_string())?;
    let raw_len = u32::try_from(unsigned(row, 11, "block raw extent")?)
        .map_err(|_| "body index row has an unreadable raw extent".to_string())?;
    Ok(IndexRow {
        event_id: bounded_sql_text(row, 0, 256)?,
        source_table: bounded_sql_text(row, 1, 256)?,
        direction,
        content_type: bounded_optional_sql_text(row, 3, 64 * 1024)?,
        original_bytes: unsigned(row, 4, "original byte count")?,
        truncated: row.get::<_, i64>(5).map_err(|error| error.to_string())? != 0,
        body_hash: bounded_sql_text(row, 6, 256)?,
        reference: BodyRef {
            block_offset,
            offset: body_offset,
            len: body_len,
        },
        extent: BlockExtent {
            disk_len: unsigned(row, 10, "block disk extent")?,
            raw_len,
        },
    })
}

fn bounded_sql_text(row: &Row<'_>, index: usize, max: usize) -> DbResult<String> {
    match row.get_ref(index).map_err(|error| error.to_string())? {
        rusqlite::types::ValueRef::Text(bytes) if bytes.len() <= max => std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|error| format!("body index column {index} is not UTF-8: {error}")),
        rusqlite::types::ValueRef::Text(bytes) => Err(format!(
            "body index column {index} exceeds its {max}-byte bound: {} bytes",
            bytes.len()
        )),
        value => Err(format!(
            "body index column {index} is {:?}, not text",
            value.data_type()
        )),
    }
}

fn bounded_optional_sql_text(row: &Row<'_>, index: usize, max: usize) -> DbResult<Option<String>> {
    match row.get_ref(index).map_err(|error| error.to_string())? {
        rusqlite::types::ValueRef::Null => Ok(None),
        _ => bounded_sql_text(row, index, max).map(Some),
    }
}

fn unsigned(row: &Row<'_>, index: usize, field: &str) -> DbResult<u64> {
    let value: i64 = row.get(index).map_err(|error| error.to_string())?;
    u64::try_from(value).map_err(|_| format!("body index row has a negative {field}"))
}

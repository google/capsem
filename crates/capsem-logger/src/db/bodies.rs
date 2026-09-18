//! Reading archived bodies back through the DB handle.
//!
//! The index lives in SQLite and the bytes live in `session.bodies`; both are
//! DB-owned. A route asks for a body by event id and gets bytes, exactly as it
//! asks for rows and gets rows. It never learns there is a second file, never
//! opens one, and never sees an `ArchiveError`: a body the index names and the
//! file cannot produce is a broken ledger and fails loudly.

use std::path::Path;

use capsem_archive::{BodyLogReader, BodyRef};
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
}

/// The ten index columns, in the order [`index_row`] reads them. A query that
/// carries extra columns of its own -- the WARC export joins each body to its
/// source row -- puts them after these.
pub(super) const INDEX_COLUMNS: &str = "event_id, source_table, direction, content_type, original_bytes, truncated, \
                                        body_hash, block_offset, body_offset, body_len";

/// Rows are ordered by block so the reader inflates each block once: the
/// request and response of one exchange are staged together and almost always
/// share a block.
const INDEX_ORDER: &str = "ORDER BY block_offset, body_offset";

impl DbHandle {
    /// Read one archived body, or `None` when the ledger has no such row.
    pub async fn read_body(&self, event_id: &str, direction: BodyDirection) -> DbResult<Option<StoredBody>> {
        let sql = format!(
            "SELECT {INDEX_COLUMNS} FROM event_body_blobs WHERE event_id = ?1 AND direction = ?2 {INDEX_ORDER}"
        );
        let rows = self
            .body_index_rows(&sql, &[event_id.into(), direction.as_str().into()])
            .await?;
        Ok(self.read_archived(rows).await?.into_iter().next())
    }

    /// Read every archived body of one event, in one index query.
    pub async fn read_bodies(&self, event_id: &str) -> DbResult<Vec<StoredBody>> {
        let sql = format!("SELECT {INDEX_COLUMNS} FROM event_body_blobs WHERE event_id = ?1 {INDEX_ORDER}");
        let rows = self.body_index_rows(&sql, &[event_id.into()]).await?;
        self.read_archived(rows).await
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
        // One budget for the whole page, not one per chunk: the chunking is a
        // SQLite parameter limit, not a unit of memory anyone agreed to.
        let mut budget = max_total_bytes;
        for chunk in event_ids.chunks(MAX_EVENT_IDS_PER_QUERY) {
            let placeholders = (3..3 + chunk.len())
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT {INDEX_COLUMNS} FROM event_body_blobs
                 WHERE source_table = ?1 AND direction = ?2 AND event_id IN ({placeholders})
                 {INDEX_ORDER}"
            );
            let mut params: Vec<Value> = Vec::with_capacity(chunk.len() + 2);
            params.push(source_table.into());
            params.push(direction.as_str().into());
            params.extend(chunk.iter().map(|event_id| Value::from(*event_id)));
            let rows = self.body_index_rows(&sql, &params).await?;
            // Split before reading, so the budget bounds what is inflated and
            // held rather than what is thrown away afterwards.
            let mut affordable = Vec::with_capacity(rows.len());
            for row in rows {
                let cost = row.reference.len as usize;
                if cost > budget {
                    archived.truncated_rows += 1;
                    continue;
                }
                budget -= cost;
                affordable.push(row);
            }
            archived.bodies.extend(self.read_archived(affordable).await?);
        }
        Ok(archived)
    }

    /// Drop every archived body whose block sealed before `cutoff` (RFC 3339),
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
        // The cached reader is not reset here. Retention replaces the
        // archive file, and `read_archived_blocking` notices that for every
        // handle rather than only for the one that asked -- the handles that
        // most need noticing are in the other process.
        writer.retain_bodies_since(cutoff).await
    }

    async fn body_index_rows(&self, sql: &str, params: &[Value]) -> DbResult<Vec<IndexRow>> {
        self.body_index_values(sql, params)
            .await?
            .iter()
            .map(index_row)
            .collect()
    }

    /// The raw JSON rows of an index query, for a caller that selects more
    /// than [`INDEX_COLUMNS`] and reads the rest of each row itself.
    pub(super) async fn body_index_values(&self, sql: &str, params: &[Value]) -> DbResult<Vec<Value>> {
        let raw = self.query(sql, params).await?;
        let value: Value =
            serde_json::from_str(&raw).map_err(|error| format!("body index rows were not decodable: {error}"))?;
        value
            .get("rows")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "body index query returned no rows array".to_string())
    }

    /// Resolve index rows to bytes on a blocking thread: inflating a block is
    /// CPU work on a file, and neither belongs on the async runtime.
    async fn read_archived(&self, rows: Vec<IndexRow>) -> DbResult<Vec<StoredBody>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let handle = self.clone();
        tokio::task::spawn_blocking(move || handle.read_archived_blocking(rows))
            .await
            .map_err(|error| format!("session body archive read task failed: {error}"))?
    }

    fn read_archived_blocking(&self, rows: Vec<IndexRow>) -> DbResult<Vec<StoredBody>> {
        self.with_archive_reader(|reader| rows.into_iter().map(|row| read_one(reader, row)).collect())
    }

    /// Take the reader out of its slot, do the work with the lock released,
    /// and put it back. The lock guards the cached reader, not the file:
    /// holding it across the inflate would make one slow read block every
    /// other one.
    ///
    /// Everything that resolves a `BodyRef` goes through here, so the archive
    /// is opened once per batch and each block inflates once for rows given in
    /// archive order -- and so the staleness check below is not something a
    /// new caller has to remember.
    ///
    /// Blocking: the caller is already on a blocking thread.
    pub(super) fn with_archive_reader<T>(&self, work: impl FnOnce(&BodyLogReader) -> DbResult<T>) -> DbResult<T> {
        let cached = self.take_archive_reader();
        let reader = match cached {
            // A cached reader that is still on the archive, which is every
            // read but the first one after a retention.
            //
            // This handle may be an external reader in the service, watching
            // a ledger `capsem-process` owns. That process compacts the
            // archive when a persistent VM stops, by renaming the new file
            // over the old one, and this handle's descriptor stays on the old
            // inode -- where every surviving block has moved and the index it
            // is about to be asked with names the new offsets. Without this
            // check its next read returns another body's bytes and fails the
            // hash comparison below: correct, in that nothing wrong is
            // served, and useless, in that the body is there and readable.
            //
            // One `stat` per read batch, against the file identity the reader
            // recorded when it opened, so the one-block cache survives
            // everything except an actual replacement. Keying it to the
            // ledger's own change signal instead would throw that cache away
            // on every commit during a live session.
            Some(reader) if !reader.file_was_replaced() => reader,
            _ => open_archive(&crate::writer::archive_path_for_db(&self.inner.path))?,
        };
        let done = work(&reader);
        self.put_archive_reader(reader);
        done
    }

    fn take_archive_reader(&self) -> Option<BodyLogReader> {
        self.inner
            .archive_reader
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }

    fn put_archive_reader(&self, reader: BodyLogReader) {
        *self
            .inner
            .archive_reader
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(reader);
    }

    /// Drop the cached archive reader, so the next read opens the file again.
    ///
    /// Test-only: production discards a stale reader by noticing the archive
    /// was replaced, which needs no one to remember to call anything.
    #[cfg(test)]
    pub(crate) fn archive_reader_reset(&self) {
        *self
            .inner
            .archive_reader
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
    }

    /// How many blocks this handle's reader has inflated.
    #[cfg(test)]
    pub(crate) fn archive_blocks_inflated_for_tests(&self) -> u64 {
        self.inner
            .archive_reader
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map_or(0, BodyLogReader::blocks_inflated)
    }
}

pub(super) fn read_one(reader: &BodyLogReader, row: IndexRow) -> DbResult<StoredBody> {
    let bytes = reader.read(row.reference).map_err(|error| {
        format!(
            "session body archive could not resolve {}/{} of event {}: {error}",
            row.source_table,
            row.direction.as_str(),
            row.event_id
        )
    })?;
    // The archive verifies its own block; this verifies the span of it the
    // index row picked out. A block's hash cannot notice an index row that
    // was edited to name a different offset inside the same valid block, and
    // that row would otherwise be served as this event's body.
    let hash = format!("blake3:{}", blake3::hash(&bytes).to_hex());
    if hash != row.body_hash {
        return Err(format!(
            "session body archive returned the wrong bytes for {}/{} of event {}: index says {}, bytes hash to {hash}",
            row.source_table,
            row.direction.as_str(),
            row.event_id,
            row.body_hash
        ));
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

fn open_archive(path: &Path) -> DbResult<BodyLogReader> {
    BodyLogReader::open(path)
        .map_err(|error| format!("session body archive {} could not be opened: {error}", path.display()))
}

pub(super) fn index_row(row: &Value) -> DbResult<IndexRow> {
    let text = |index: usize| -> DbResult<String> {
        row.get(index)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("body index row is missing column {index}"))
    };
    let number = |index: usize| -> DbResult<u64> {
        row.get(index)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("body index row is missing column {index}"))
    };
    let direction_text = text(2)?;
    let direction = BodyDirection::parse(&direction_text)
        .ok_or_else(|| format!("body index row has unknown direction {direction_text}"))?;
    let len = u32::try_from(number(9)?).map_err(|_| "body index row has an unreadable length".to_string())?;
    let offset = u32::try_from(number(8)?).map_err(|_| "body index row has an unreadable offset".to_string())?;
    Ok(IndexRow {
        event_id: text(0)?,
        source_table: text(1)?,
        direction,
        content_type: row.get(3).and_then(Value::as_str).map(str::to_string),
        original_bytes: number(4)?,
        truncated: number(5)? != 0,
        body_hash: text(6)?,
        reference: BodyRef {
            block_offset: number(7)?,
            offset,
            len,
        },
    })
}

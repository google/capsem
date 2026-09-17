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

/// An index row, before its bytes are fetched.
struct IndexRow {
    event_id: String,
    source_table: String,
    direction: BodyDirection,
    content_type: Option<String>,
    original_bytes: u64,
    truncated: bool,
    body_hash: String,
    reference: BodyRef,
}

const INDEX_COLUMNS: &str = "event_id, source_table, direction, content_type, original_bytes, truncated, \
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

    /// Read the newest archived bodies of one source table and direction.
    ///
    /// The per-event reads above answer "show me this exchange". This answers
    /// "I have a page of rows and I need the body of each" -- asking event by
    /// event would cost an index query and a blocking task per row, and would
    /// inflate the same block once for every body that sits in it. Here it is
    /// one query and one pass, in archive order.
    ///
    /// # Errors
    ///
    /// The same as the per-event reads: a row whose bytes the archive cannot
    /// produce, or does not produce intact, is a broken ledger and fails.
    pub async fn read_recent_bodies(
        &self,
        source_table: &str,
        direction: BodyDirection,
        limit: usize,
    ) -> DbResult<Vec<StoredBody>> {
        let sql = format!(
            "SELECT {INDEX_COLUMNS} FROM (
                 SELECT {INDEX_COLUMNS}, block_offset AS block_order, body_offset AS body_order
                 FROM event_body_blobs
                 WHERE source_table = ?1 AND direction = ?2
                 ORDER BY id DESC LIMIT ?3
             ) ORDER BY block_order, body_order"
        );
        let rows = self
            .body_index_rows(
                &sql,
                &[source_table.into(), direction.as_str().into(), (limit as u64).into()],
            )
            .await?;
        self.read_archived(rows).await
    }

    async fn body_index_rows(&self, sql: &str, params: &[Value]) -> DbResult<Vec<IndexRow>> {
        let raw = self.query(sql, params).await?;
        let value: Value =
            serde_json::from_str(&raw).map_err(|error| format!("body index rows were not decodable: {error}"))?;
        let rows = value
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| "body index query returned no rows array".to_string())?;
        rows.iter().map(index_row).collect()
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

    /// Take the reader out of its slot, read with the lock released, and put
    /// it back. The lock guards the cached reader, not the file: holding it
    /// across the inflate would make one slow read block every other one.
    fn read_archived_blocking(&self, rows: Vec<IndexRow>) -> DbResult<Vec<StoredBody>> {
        let cached = self.take_archive_reader();
        let reader = match cached {
            Some(reader) => reader,
            None => open_archive(&crate::writer::archive_path_for_db(&self.inner.path))?,
        };
        let bodies = rows
            .into_iter()
            .map(|row| read_one(&reader, row))
            .collect::<DbResult<Vec<StoredBody>>>();
        self.put_archive_reader(reader);
        bodies
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

    /// Drop the cached archive reader. The archive is append-only, so nothing
    /// in the product rewrites it today; a reader that has seen a rewritten
    /// file would be holding a stale block, and this is how it is discarded.
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

fn read_one(reader: &BodyLogReader, row: IndexRow) -> DbResult<StoredBody> {
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

fn index_row(row: &Value) -> DbResult<IndexRow> {
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

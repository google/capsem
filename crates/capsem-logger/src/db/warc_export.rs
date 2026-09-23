//! Exporting a session's archived bodies as a WARC file.
//!
//! The point is to need none of our code to read the result. `capsem-archive`
//! owns the record format; this module owns what a Capsem ledger row means in
//! it -- which URI a body was a capture of, when it happened, and which rows
//! cannot honestly be described at all.
//!
//! One SQLite snapshot is walked in indexed `(block_offset, body_offset, id)`
//! pages, with each source row resolved by its event-id index. The typed rows
//! are framed into a bounded private spool before network streaming begins.
//! The body walk then runs on one blocking thread through one
//! `BodyLogReader`. Archive order is what makes the export cost one inflate
//! per block instead of one per body; anything else would inflate a block
//! again for every body that happens to sit in it.
//!
//! **Nothing is invented, and one bad row does not cost the rest.** A body
//! whose source row is gone has no URI; a row whose timestamp will not parse
//! has no date; a URI built from a `tool_name` with a newline in it cannot be
//! written as a header at all; a body whose bytes fail the hash the index
//! recorded is not that body, and one the archive cannot produce at all is
//! nothing. Every one of them is skipped and counted with its reason rather
//! than described with a guess or allowed to abort the walk. A record carrying
//! a plausible guess is worse than one that is absent, because a reviewer
//! cannot tell the two apart afterwards; and one damaged row must not deny a
//! reviewer the other four thousand.
//!
//! The last two are deliberately the same answer. An index row edited to a
//! span that still lands inside its block comes back as bytes that fail the
//! hash, and the same row edited one step further lands outside it and the
//! archive refuses -- the same damage, and no reading on which one of them
//! should cost a reviewer the session and the other should not. Only a failed
//! seek is about the file rather than a row, and it is the one thing here that
//! is still an error.
//!
//! **The omissions are in the artifact, not only in the summary.** A caller
//! that streams this to a client (`GET /vms/{id}/bodies/export.warc.gz`) drops
//! the summary on the floor, so the file says for itself what it holds: a
//! leading `warcinfo` record naming the software, the session and the export
//! date, and a trailing one carrying the skip counts by reason. The spec
//! permits several `warcinfo` records per file and this is what they are for.
//!
//! The closing record is also the completion signal. An export that fails hard
//! has already written the opening record and streamed whatever it got
//! through, so the file simply ends after its last complete record. **A file
//! with no closing `warcinfo` is an export that did not finish**, and its
//! record list is not the session -- which is the one thing a reader cannot
//! work out from the records themselves.
//!
//! Every body goes out through the same hash-verified read path a route uses.
//! An export is evidence, and evidence that skipped the check the interactive
//! path performs would be the one copy nobody verified.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use capsem_archive::{warc, BlockExtent, BodyLogReader, BodyRef, WarcRecord};
use capsem_foundation::unix::contained::ContainedDir;
use capsem_foundation::unix::lock::{self, LockMode};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::bodies::{
    index_row_sql, read_one_checked, ArchiveSqlDeadline, BodyFault, IndexRow, INDEX_COLUMNS, INDEX_FROM,
};
use super::{DbHandle, DbResult};

/// What an export did, and what it could not describe.
#[derive(Debug, Default, Clone)]
pub struct ExportSummary {
    /// Records written, one per exported body.
    pub records: u64,
    /// Bytes handed to the writer, gzip members included.
    pub bytes_written: u64,
    /// Exact number of index rows that were not exported. An empty export and
    /// an export that refused every row are different statements.
    pub skipped_count: u64,
    /// A bounded diagnostic sample of rows that were not exported, each with
    /// why. Exact totals live in `skipped_count` and `skipped_by_reason`.
    pub skipped: Vec<SkippedBody>,
    skipped_by_reason: BTreeMap<&'static str, u64>,
}

impl ExportSummary {
    /// How many rows each reason accounted for.
    ///
    /// This is what the trailing `warcinfo` carries, and it is deliberately a
    /// count per reason rather than a list of event ids: the file is handed to
    /// a reviewer, and "11 bodies omitted, all of them corrupt" is the thing
    /// they need to know before they conclude anything from what is there.
    #[must_use]
    pub fn counts_by_reason(&self) -> BTreeMap<&'static str, u64> {
        self.skipped_by_reason.clone()
    }

    fn record_skip(&mut self, body: SkippedBody) {
        self.skipped_count += 1;
        *self.skipped_by_reason.entry(body.reason.label()).or_insert(0) += 1;
        if self.skipped.len() < MAX_SKIPPED_SAMPLES {
            self.skipped.push(body);
        }
    }
}

const MAX_SKIPPED_SAMPLES: usize = 32;

#[derive(Default)]
struct ActiveWarcExports {
    sessions: HashSet<PathBuf>,
}

impl ActiveWarcExports {
    fn reserve(&mut self, session: &std::path::Path, process_limit: usize) -> DbResult<()> {
        if self.sessions.contains(session) {
            return Err("a WARC export is already active for this session".into());
        }
        if self.sessions.len() >= process_limit {
            return Err(format!("the process already has {process_limit} active WARC exports"));
        }
        self.sessions.insert(session.to_path_buf());
        Ok(())
    }
}

static ACTIVE_WARC_EXPORTS: LazyLock<Mutex<ActiveWarcExports>> = LazyLock::new(Mutex::default);

struct WarcExportPermit {
    session: PathBuf,
}

impl WarcExportPermit {
    fn acquire(session: &std::path::Path) -> DbResult<Self> {
        let mut active = ACTIVE_WARC_EXPORTS.lock().unwrap_or_else(|error| error.into_inner());
        active.reserve(session, active_warc_export_process_limit())?;
        drop(active);
        let session = session.to_path_buf();
        Ok(Self { session })
    }
}

impl Drop for WarcExportPermit {
    fn drop(&mut self) {
        ACTIVE_WARC_EXPORTS
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sessions
            .remove(&self.session);
    }
}

/// One index row the export could not turn into an honest record.
#[derive(Debug, Clone)]
pub struct SkippedBody {
    pub event_id: String,
    pub source_table: String,
    pub direction: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The join found no row in the body's source table, so there is no URI
    /// this body was a capture of.
    MissingSourceRow,
    /// The source row's timestamp is not a ledger timestamp, so there is no
    /// `WARC-Date` that would be true.
    UnreadableTimestamp(String),
    /// The target URI has a line break in it, which a WARC header block cannot
    /// carry.
    ///
    /// Not hypothetical: `tool_calls.tool_name` and `tool_responses.call_id`
    /// are written verbatim from model and MCP JSON, with no CHECK forbidding
    /// a newline, so a counterparty chooses this string. `write_record`
    /// refuses it -- correctly, since the alternative is a forged header --
    /// and before this was a skip, one such name aborted the whole export and
    /// left the reviewer a truncated file and no summary.
    UnrepresentableUri(String),
    /// The archived bytes do not hash to what the index recorded for them.
    ///
    /// The body is not exported, because it is not that body. The walk goes
    /// on: a damaged block must not cost a reviewer every other record in the
    /// session, and the trailing `warcinfo` is what makes the loss visible in
    /// the file itself.
    CorruptBody(String),
    /// The archive could not produce the bytes at all: this row's recorded
    /// span falls outside its block, or the block itself has a bad header, a
    /// truncated payload, or will not inflate.
    ///
    /// The same class of damage as `CorruptBody` and treated the same way. A
    /// row edited to a wrong-but-in-range span lands there and a row edited a
    /// little further lands here; there is no reading on which one of those
    /// should cost a reviewer the whole session and the other should not.
    UnreadableBody(String),
}

impl SkipReason {
    /// The stable name this reason is counted under in the trailing
    /// `warcinfo`. Stable because it is in an exported artifact: a reader
    /// parsing last month's export must find this month's spelling.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::MissingSourceRow => "missing-source-row",
            Self::UnreadableTimestamp(_) => "unreadable-timestamp",
            Self::UnrepresentableUri(_) => "unrepresentable-uri",
            Self::CorruptBody(_) => "corrupt-body",
            Self::UnreadableBody(_) => "unreadable-body",
        }
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSourceRow => write!(f, "its source row is missing, so the body has no target URI"),
            Self::UnreadableTimestamp(stamp) => {
                write!(f, "its source row's timestamp {stamp:?} is not a ledger timestamp")
            }
            Self::UnrepresentableUri(uri) => {
                write!(f, "its target URI {uri:?} has a line break a WARC header cannot carry")
            }
            Self::CorruptBody(detail) => write!(f, "its archived bytes failed their hash check: {detail}"),
            Self::UnreadableBody(detail) => write!(f, "the archive could not produce its bytes: {detail}"),
        }
    }
}

/// How a body's identity is spelled in `WARC-Record-ID`.
///
/// WARC requires record ids to be globally unique, and an event id is only
/// unique within its ledger: twelve hex digits the session assigns, so two
/// sessions merged into one collection can carry the same one. The session
/// names the ledger, and the rest is the index's own unique key --
/// `UNIQUE(event_id, source_table, direction)` on `event_body_blobs` -- so the
/// id is unique by construction and still points straight back at the row.
///
/// The source table is not decoration. This id used to leave it out, on the
/// stated grounds that one event has at most one body per direction; that was
/// never what the index says, and it stopped being true in practice when the
/// security ledgers began archiving their payloads: a rule match, the decision
/// it drove and an ask it raised all name the same event, each with a
/// `payload`, and without the table the three records shared one id.
fn record_id(session: &str, row: &IndexRow) -> String {
    format!(
        "urn:capsem:{session}:{}:{}:{}",
        row.source_table,
        row.event_id,
        row.direction.as_str()
    )
}

/// One body's index row plus the two facts only its source row can supply.
struct ExportRow {
    index: IndexRow,
    /// `None` when the join found no source row.
    target_uri: Option<String>,
    /// The source row's time, as text and/or unix milliseconds depending on
    /// which column the table keeps. Exactly one is present per row.
    date_text: Option<String>,
    date_unix_ms: Option<i64>,
}

pub(super) struct CapturedWarc {
    reader: BodyLogReader,
    spool: File,
}

#[derive(Serialize, Deserialize)]
struct SpoolRow {
    event_id: String,
    source_table: String,
    direction: String,
    content_type: Option<String>,
    original_bytes: u64,
    truncated: bool,
    body_hash: String,
    block_offset: u64,
    body_offset: u32,
    body_len: u32,
    disk_len: u64,
    raw_len: u32,
    target_uri: Option<String>,
    date_text: Option<String>,
    date_unix_ms: Option<i64>,
}

impl From<ExportRow> for SpoolRow {
    fn from(row: ExportRow) -> Self {
        Self {
            event_id: row.index.event_id,
            source_table: row.index.source_table,
            direction: row.index.direction.as_str().to_string(),
            content_type: row.index.content_type,
            original_bytes: row.index.original_bytes,
            truncated: row.index.truncated,
            body_hash: row.index.body_hash,
            block_offset: row.index.reference.block_offset,
            body_offset: row.index.reference.offset,
            body_len: row.index.reference.len,
            disk_len: row.index.extent.disk_len,
            raw_len: row.index.extent.raw_len,
            target_uri: row.target_uri,
            date_text: row.date_text,
            date_unix_ms: row.date_unix_ms,
        }
    }
}

impl TryFrom<SpoolRow> for ExportRow {
    type Error = String;

    fn try_from(row: SpoolRow) -> Result<Self, Self::Error> {
        let direction = super::bodies::BodyDirection::parse(&row.direction)
            .ok_or_else(|| format!("WARC metadata spool has unknown direction {}", row.direction))?;
        Ok(Self {
            index: IndexRow {
                event_id: row.event_id,
                source_table: row.source_table,
                direction,
                content_type: row.content_type,
                original_bytes: row.original_bytes,
                truncated: row.truncated,
                body_hash: row.body_hash,
                reference: BodyRef {
                    block_offset: row.block_offset,
                    offset: row.body_offset,
                    len: row.body_len,
                },
                extent: BlockExtent {
                    disk_len: row.disk_len,
                    raw_len: row.raw_len,
                },
            },
            target_uri: row.target_uri,
            date_text: row.date_text,
            date_unix_ms: row.date_unix_ms,
        })
    }
}

/// A body's target URI and its source row's time, per source table.
///
/// The URI is what the body was a capture of. `net_events` and `model_calls`
/// have a real one; the rest are Capsem's own surfaces and get a `capsem://`
/// URI naming the thing they came from, which is what a reviewer needs to know
/// and is not pretending to be fetchable.
///
/// `tool_responses` is the one table with no timestamp column of its own, so
/// its records are dated by `event_body_blobs.created_at` -- the time the
/// ledger archived the body, written in the same format and never absent. The
/// alternative was to date it from the model call it belongs to, which is a
/// second join that can miss and would skip a body that is perfectly readable.
const WARC_CAPTURE_PAGE_ROWS: i64 = 128;
const MAX_SPOOL_ROW_BYTES: usize = 64 * 1024;
const MAX_WARC_SPOOL_BYTES: usize = 256 * 1024 * 1024;
const MAX_SOURCE_TEXT_BYTES: usize = 64 * 1024;
const MAX_WARC_EXPORTS_PER_PROCESS: usize = 2;
const WARC_CAPTURE_DEADLINE: Duration = Duration::from_secs(30);

fn active_warc_export_process_limit() -> usize {
    #[cfg(test)]
    {
        // The Rust harness runs unrelated session fixtures concurrently in
        // one process. The two-export production limit is proved against a
        // local registry below rather than coupling otherwise independent
        // tests through this process-global counter.
        1_024
    }
    #[cfg(not(test))]
    {
        MAX_WARC_EXPORTS_PER_PROCESS
    }
}
const SOURCE_TABLES: &[&str] = &[
    "net_events",
    "model_calls",
    "tool_calls",
    "tool_responses",
    "exec_events",
    "security_rule_events",
    "security_decision_events",
    "security_ask_events",
];

fn warc_page_sql() -> String {
    format!(
        "SELECT {INDEX_COLUMNS}, b.id, b.created_at FROM {INDEX_FROM}
         WHERE b.block_offset > ?1
            OR (b.block_offset = ?1 AND b.body_offset > ?2)
            OR (b.block_offset = ?1 AND b.body_offset = ?2 AND b.id > ?3)
         ORDER BY b.block_offset, b.body_offset, b.id LIMIT ?4"
    )
}

pub(super) fn capture_warc(conn: &Connection, db_path: &std::path::Path) -> DbResult<CapturedWarc> {
    let deadline = std::time::Instant::now() + WARC_CAPTURE_DEADLINE;
    let archive_lock = lock::acquire_existing_until(
        &crate::writer::archive_lock_path_for_db(db_path),
        LockMode::Shared,
        (std::time::Instant::now() + Duration::from_secs(5)).min(deadline),
    )
    .map_err(|error| format!("acquire WARC archive capture lock: {error}"))?;
    let sql_deadline = ArchiveSqlDeadline::install(conn, deadline);
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("begin WARC capture snapshot: {error}"))?;
    let state = crate::schema::archive_state(&tx).map_err(|error| format!("read archive_state: {error}"))?;
    let directory = ContainedDir::open_root(&crate::writer::archive_path_for_db(db_path))
        .and_then(|directory| {
            directory.validate_private()?;
            Ok(directory)
        })
        .map_err(|error| format!("open WARC generation directory: {error}"))?;
    let reader = BodyLogReader::open_generation(&directory, state.header, state.committed_end)
        .map_err(|error| format!("open WARC generation: {error}"))?;
    drop(archive_lock);
    super::bodies::pause_archive_capture_for_tests(db_path);

    let mut spool = tempfile::tempfile().map_err(|error| format!("create WARC metadata spool: {error}"))?;
    let mut spool_bytes = 0usize;
    let mut last = (-1i64, -1i64, -1i64);
    loop {
        sql_deadline.check("WARC metadata capture")?;
        let sql = warc_page_sql();
        let mut statement = tx.prepare_cached(&sql).map_err(|error| error.to_string())?;
        let mut rows = statement
            .query(rusqlite::params![last.0, last.1, last.2, WARC_CAPTURE_PAGE_ROWS])
            .map_err(|error| error.to_string())?;
        let mut page = Vec::with_capacity(WARC_CAPTURE_PAGE_ROWS as usize);
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let index = index_row_sql(row)?;
            let id: i64 = row.get(12).map_err(|error| error.to_string())?;
            let created_at = bounded_text(row, 13).map_err(|error| error.to_string())?;
            last = (
                index.reference.block_offset as i64,
                i64::from(index.reference.offset),
                id,
            );
            page.push((index, created_at));
        }
        drop(rows);
        drop(statement);
        let count = page.len();
        for (index, created_at) in page {
            let (target_uri, date_text, date_unix_ms) = source_metadata(&tx, &index, &created_at)?;
            write_spool_row(
                &mut spool,
                ExportRow {
                    index,
                    target_uri,
                    date_text,
                    date_unix_ms,
                },
                &mut spool_bytes,
            )?;
        }
        if count < WARC_CAPTURE_PAGE_ROWS as usize {
            break;
        }
    }
    tx.commit()
        .map_err(|error| format!("commit WARC capture snapshot: {error}"))?;
    spool
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind WARC metadata spool: {error}"))?;
    Ok(CapturedWarc { reader, spool })
}

fn source_metadata(
    conn: &Connection,
    index: &IndexRow,
    created_at: &str,
) -> DbResult<(Option<String>, Option<String>, Option<i64>)> {
    if !SOURCE_TABLES.contains(&index.source_table.as_str()) {
        return Err(format!(
            "archive index has unsupported source table {}",
            index.source_table
        ));
    }
    let text = |sql: &str| -> DbResult<Option<(String, String)>> {
        conn.query_row(sql, [&index.event_id], |row| {
            Ok((bounded_text(row, 0)?, bounded_text(row, 1)?))
        })
        .optional()
        .map_err(|error| error.to_string())
    };
    let millis = |sql: &str| -> DbResult<Option<(String, i64)>> {
        conn.query_row(sql, [&index.event_id], |row| Ok((bounded_text(row, 0)?, row.get(1)?)))
            .optional()
            .map_err(|error| error.to_string())
    };
    match index.source_table.as_str() {
        "net_events" => Ok(text(
            "SELECT 'https://' || domain || COALESCE(path, ''), timestamp
             FROM net_events WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), Some(date), None))),
        "model_calls" => Ok(text(
            "SELECT 'https://' || provider || path, timestamp
             FROM model_calls WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), Some(date), None))),
        "tool_calls" => Ok(text(
            "SELECT 'capsem://tool/' || tool_name, timestamp
             FROM tool_calls WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), Some(date), None))),
        "tool_responses" => {
            let uri = conn
                .query_row(
                    "SELECT 'capsem://tool-response/' || call_id
                     FROM tool_responses WHERE event_id = ?1 ORDER BY id LIMIT 1",
                    [&index.event_id],
                    |row| bounded_text(row, 0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            Ok((uri, Some(created_at.to_string()), None))
        }
        "exec_events" => Ok(text(
            "SELECT 'capsem://exec/' || exec_id, timestamp
             FROM exec_events WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), Some(date), None))),
        "security_rule_events" => Ok(millis(
            "SELECT 'capsem://security/' || rule_id, timestamp_unix_ms
             FROM security_rule_events WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), None, Some(date)))),
        "security_decision_events" => Ok(millis(
            "SELECT 'capsem://security-decision/' || COALESCE(event.actor, run.actor), event.timestamp_unix_ms
             FROM security_decision_events AS event
             LEFT JOIN security_decision_runs AS run ON run.id = event.run_id
             WHERE event.event_id = ?1 ORDER BY event.id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), None, Some(date)))),
        "security_ask_events" => Ok(millis(
            "SELECT 'capsem://security-ask/' || ask_id, timestamp_unix_ms
             FROM security_ask_events WHERE event_id = ?1 ORDER BY id LIMIT 1",
        )?
        .map_or((None, None, None), |(uri, date)| (Some(uri), None, Some(date)))),
        table => unreachable!("{table} was checked against SOURCE_TABLES"),
    }
}

fn bounded_text(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<String> {
    let value = row.get_ref(index)?;
    let rusqlite::types::ValueRef::Text(bytes) = value else {
        return Err(rusqlite::Error::InvalidColumnType(
            index,
            "WARC metadata text".into(),
            value.data_type(),
        ));
    };
    if bytes.len() > MAX_SOURCE_TEXT_BYTES {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "WARC source metadata exceeds its per-field bound",
            )),
        ));
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error)))
}

fn write_spool_row(spool: &mut File, row: ExportRow, spool_bytes: &mut usize) -> DbResult<()> {
    let encoded = serde_json::to_vec(&SpoolRow::from(row)).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_SPOOL_ROW_BYTES {
        return Err(format!("WARC metadata row exceeds {MAX_SPOOL_ROW_BYTES} bytes"));
    }
    let next = checked_spool_bytes(*spool_bytes, encoded.len())?;
    let len = u32::try_from(encoded.len()).map_err(|_| "WARC metadata row length overflow".to_string())?;
    spool.write_all(&len.to_le_bytes()).map_err(|error| error.to_string())?;
    spool.write_all(&encoded).map_err(|error| error.to_string())?;
    *spool_bytes = next;
    Ok(())
}

fn checked_spool_bytes(current: usize, encoded_row: usize) -> DbResult<usize> {
    let framed_len = encoded_row
        .checked_add(std::mem::size_of::<u32>())
        .ok_or_else(|| "WARC metadata spool length overflow".to_string())?;
    let next = current
        .checked_add(framed_len)
        .ok_or_else(|| "WARC metadata spool length overflow".to_string())?;
    if next > MAX_WARC_SPOOL_BYTES {
        return Err(format!("WARC metadata spool exceeds {MAX_WARC_SPOOL_BYTES} bytes"));
    }
    Ok(next)
}

fn read_spool_row(spool: &mut File) -> DbResult<Option<ExportRow>> {
    let mut len = [0u8; 4];
    let read = spool.read(&mut len).map_err(|error| error.to_string())?;
    if read == 0 {
        return Ok(None);
    }
    if read != len.len() {
        return Err("truncated WARC metadata spool frame".into());
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_SPOOL_ROW_BYTES {
        return Err("oversized WARC metadata spool frame".into());
    }
    let mut encoded = vec![0u8; len];
    spool.read_exact(&mut encoded).map_err(|error| error.to_string())?;
    let row: SpoolRow = serde_json::from_slice(&encoded).map_err(|error| error.to_string())?;
    row.try_into().map(Some)
}

impl DbHandle {
    /// Write every archived body of this session to `out` as a WARC 1.1 file.
    ///
    /// `out` is taken by value and moved onto the blocking thread, because
    /// that is where the whole export runs: inflating blocks is CPU work on a
    /// file and belongs nowhere near the async runtime.
    ///
    /// The file is bracketed by two `warcinfo` records: a leading one naming
    /// the software, the session and the export date, and a trailing one
    /// carrying the skip counts by reason, so a reader holding only the file
    /// can see that bodies were omitted and why.
    ///
    /// **A `capsem://tool-response/...` record's `WARC-Date` is archive time,
    /// not event time.** `tool_responses` is the one source table with no
    /// timestamp column, so those records are dated from
    /// `event_body_blobs.created_at` -- when the ledger archived the body,
    /// which is within the same exchange but is not the event's own instant.
    /// There is no in-band signal for this, so a reader correlating tool
    /// responses to the millisecond has to know it from here.
    ///
    /// # Errors
    ///
    /// A row the export cannot honestly describe, or whose bytes the archive
    /// cannot produce, is skipped and counted rather than an error: see
    /// [`SkipReason`]. Two things do fail. An archive that cannot be opened,
    /// or a seek on it that fails, is about the file rather than about a row,
    /// and the next row would be read through the same broken handle. A write
    /// failure fails because a truncated export that returned a summary would
    /// be one nobody knew was partial.
    ///
    /// **A hard failure leaves a file with no closing `warcinfo`.** The
    /// opening record is already written and the bytes already streamed, so
    /// the file ends after its last complete record. That absence is the
    /// signal: a reader that finds no closing bracket is holding an export
    /// that did not finish, and must not read its record list as the whole
    /// session.
    pub async fn export_warc<W: Write + Send + 'static>(&self, out: W) -> DbResult<ExportSummary> {
        let permit = WarcExportPermit::acquire(&self.inner.path)?;
        let captured = self.capture_warc().await?;
        let handle = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            handle.export_blocking(captured, out)
        })
        .await
        .map_err(|error| format!("session body WARC export task failed: {error}"))?
    }

    async fn capture_warc(&self) -> DbResult<CapturedWarc> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.inner
            .reader_tx
            .send(super::ReadRequest::CaptureWarc { reply })
            .map_err(|error| format!("db reader worker closed: {error}"))?;
        rx.await
            .map_err(|error| format!("db reader worker dropped WARC capture reply: {error}"))?
            .map(|observed| self.take_observed(observed))
    }

    fn export_blocking<W: Write>(&self, mut captured: CapturedWarc, out: W) -> DbResult<ExportSummary> {
        let mut counting = CountingWriter { inner: out, count: 0 };
        let mut summary = ExportSummary::default();
        let exported_at = warc_date_now();
        let session = self.session_name();
        // One id per export, not per record kind: two exports of the same
        // session must not collide when a reader merges them.
        let export_id = uuid::Uuid::new_v4().simple().to_string();
        write_warcinfo(
            &mut counting,
            &session,
            &export_id,
            "opening",
            &exported_at,
            &opening_fields(&session, &exported_at),
        )?;

        while let Some(row) = read_spool_row(&mut captured.spool)? {
            let identity = RowIdentity::of(&row.index);
            let Some(target_uri) = row.target_uri.clone() else {
                summary.record_skip(identity.because(SkipReason::MissingSourceRow));
                continue;
            };
            // Checked here rather than left to `write_record`, so a URI a
            // counterparty chose is a counted skip instead of a refusal
            // that would end the walk. It is the only header value in a
            // record that a counterparty can reach: the record id is the
            // ledger's own directory name -- already in the opening
            // `warcinfo` id, which would have refused it first -- and an
            // event id SQLite CHECKs to twelve hex digits, the date is
            // generated here, and `content_type` is cut out of a header
            // line and trimmed on the way in -- which
            // `a_stored_content_type_can_never_carry_a_line_break` in
            // writer/tests/headers.rs holds, so this remains the only one.
            if target_uri.contains(['\r', '\n']) {
                summary.record_skip(identity.because(SkipReason::UnrepresentableUri(target_uri)));
                continue;
            }
            let Some(date) = row.warc_date() else {
                let stamp = row.date_text.clone().unwrap_or_else(|| {
                    row.date_unix_ms
                        .map_or_else(|| "<none>".to_string(), |ms| ms.to_string())
                });
                summary.record_skip(identity.because(SkipReason::UnreadableTimestamp(stamp)));
                continue;
            };
            let id = record_id(&session, &row.index);
            let truncated = row.index.truncated;
            // The hash-verified read path, not a shortcut around it.
            // Damage to this row or its block costs this row: a span that
            // falls outside its block and a span that falls inside the
            // wrong part of it are the same edit, one byte apart, and
            // there is no reading on which one should end the export. Only
            // a failed seek is about the file, and the next row would be
            // read through the same handle.
            let mut body = match read_one_checked(&captured.reader, row.index) {
                Ok(body) => body,
                Err(BodyFault::Corrupt(detail)) => {
                    summary.record_skip(identity.because(SkipReason::CorruptBody(detail)));
                    continue;
                }
                Err(BodyFault::Unreadable(detail)) => {
                    summary.record_skip(identity.because(SkipReason::UnreadableBody(detail)));
                    continue;
                }
                Err(fault) => return Err(fault.into_message()),
            };
            if body.content_type.as_deref() == Some("application/vnd.capsem.security+msgpack") {
                let decoded = capsem_proto::forensic::SecurityForensicEvent::decode(&body.bytes)
                    .and_then(|event| event.to_json());
                let json = match decoded {
                    Ok(json) => json,
                    Err(error) => {
                        summary.record_skip(identity.because(SkipReason::CorruptBody(error.to_string())));
                        continue;
                    }
                };
                body.bytes = json.into_bytes();
                body.content_type = Some("application/json".to_string());
            }
            warc::write_record(
                &mut counting,
                &WarcRecord {
                    record_type: warc::WARC_TYPE_RESOURCE,
                    record_id: &id,
                    target_uri: Some(&target_uri),
                    date: &date,
                    content_type: body.content_type.as_deref(),
                    truncated,
                    body: &body.bytes,
                },
            )
            .map_err(|error| format!("session body WARC export could not write record {id}: {error}"))?;
            summary.records += 1;
        }
        self.inner
            .archive_blocks_inflated
            .fetch_add(captured.reader.blocks_inflated(), std::sync::atomic::Ordering::Relaxed);

        write_warcinfo(
            &mut counting,
            &session,
            &export_id,
            "closing",
            &exported_at,
            &closing_fields(&summary),
        )?;
        counting
            .inner
            .flush()
            .map_err(|error| format!("session body WARC export could not flush its output: {error}"))?;
        summary.bytes_written = counting.count;
        Ok(summary)
    }

    /// What the export calls this session in its `warcinfo`.
    ///
    /// The ledger's directory name, which is the session id in
    /// `~/.capsem/run/<session>/session.db` and the VM name under
    /// `run/persistent/`. Derived rather than passed in, so a caller cannot
    /// label one session's bodies with another session's name.
    fn session_name(&self) -> String {
        self.inner
            .path
            .parent()
            .and_then(std::path::Path::file_name)
            .map_or_else(|| "unknown".to_string(), |name| name.to_string_lossy().into_owned())
    }
}

/// `name: value` lines, the `application/warc-fields` block a `warcinfo`
/// record carries.
///
/// Values are sanitized rather than refused, unlike the header block: these
/// are ours, a line break in one would be a bug here rather than a hostile
/// ledger row, and a `warcinfo` that refused to be written would abort an
/// export over its own metadata.
fn warc_fields(fields: &[(String, String)]) -> Vec<u8> {
    use std::fmt::Write as _;
    fields
        .iter()
        .fold(String::new(), |mut block, (name, value)| {
            let _ = writeln!(block, "{name}: {}\r", value.replace(['\r', '\n'], " "));
            block
        })
        .into_bytes()
}

fn opening_fields(session: &str, exported_at: &str) -> Vec<(String, String)> {
    vec![
        ("software".into(), format!("capsem/{}", env!("CARGO_PKG_VERSION"))),
        ("format".into(), "WARC File Format 1.1".into()),
        ("capsem-session".into(), session.to_string()),
        ("capsem-exported-at".into(), exported_at.to_string()),
        // Named because it is a divergence: the convention is base32 sha1 or
        // sha256, and a tool that verifies block digests will not verify ours.
        ("capsem-block-digest-algorithm".into(), "blake3".into()),
    ]
}

/// The closing record: what the walk left out, by reason.
fn closing_fields(summary: &ExportSummary) -> Vec<(String, String)> {
    let mut fields = vec![
        ("software".into(), format!("capsem/{}", env!("CARGO_PKG_VERSION"))),
        ("capsem-records".into(), summary.records.to_string()),
        ("capsem-skipped".into(), summary.skipped_count.to_string()),
    ];
    fields.extend(
        summary
            .counts_by_reason()
            .into_iter()
            .map(|(label, count)| (format!("capsem-skipped-{label}"), count.to_string())),
    );
    fields
}

/// The `warcinfo` record id.
///
/// WARC requires record ids to be globally unique, and these were once
/// `urn:capsem:warcinfo:opening` -- byte-identical in every export ever
/// produced, so two exports merged into one collection collided by
/// construction. The session names which ledger this came from, and the
/// per-export uuid is what actually makes it unique, including across two
/// exports of the same session in the same second.
fn warcinfo_record_id(session: &str, export_id: &str, which: &str) -> String {
    format!("urn:capsem:{session}:warcinfo:{which}:{export_id}")
}

fn write_warcinfo<W: Write>(
    out: &mut W,
    session: &str,
    export_id: &str,
    which: &str,
    date: &str,
    fields: &[(String, String)],
) -> DbResult<()> {
    let block = warc_fields(fields);
    warc::write_record(
        out,
        &WarcRecord {
            record_type: warc::WARC_TYPE_WARCINFO,
            record_id: &warcinfo_record_id(session, export_id, which),
            target_uri: None,
            date,
            content_type: Some(warc::WARCINFO_CONTENT_TYPE),
            truncated: false,
            body: &block,
        },
    )
    .map_err(|error| format!("session body WARC export could not write its {which} warcinfo record: {error}"))
}

/// Now, in the WARC date format.
///
/// Formatted straight to whole seconds rather than formatted to microseconds
/// and parsed back. The round trip needed a fallback for a parse that cannot
/// fail, and this module's whole argument is that it writes no date it did not
/// verify -- a `1970-01-01` that no clock produced would have been the one
/// exception, sitting in the record that describes the file.
fn warc_date_now() -> String {
    humantime::format_rfc3339_seconds(SystemTime::now()).to_string()
}

impl ExportRow {
    /// The record's `WARC-Date`, or `None` when the source row's time cannot
    /// be read as one.
    fn warc_date(&self) -> Option<String> {
        match (&self.date_text, self.date_unix_ms) {
            (Some(text), _) => warc_date_from_ledger(text),
            (None, Some(millis)) => warc_date_from_unix_ms(millis),
            (None, None) => None,
        }
    }
}

/// Which row a skip is about, taken before the read consumes the row.
///
/// A type of its own rather than a half-filled `SkippedBody`: the earlier
/// shape handed back one with a placeholder `reason` that callers overwrote
/// through `..`, so a future spread that forgot the field would have labelled
/// a corrupt body as a missing source row and nothing would have said so.
/// Here there is nothing to forget -- the only way to a `SkippedBody` is
/// [`RowIdentity::because`], and it takes the reason.
#[derive(Debug, Clone)]
struct RowIdentity {
    event_id: String,
    source_table: String,
    direction: String,
}

impl RowIdentity {
    fn of(row: &IndexRow) -> Self {
        Self {
            event_id: row.event_id.clone(),
            source_table: row.source_table.clone(),
            direction: row.direction.as_str().to_string(),
        }
    }

    fn because(self, reason: SkipReason) -> SkippedBody {
        SkippedBody {
            event_id: self.event_id,
            source_table: self.source_table,
            direction: self.direction,
            reason,
        }
    }
}

/// `YYYY-MM-DDTHH:MM:SS` in a WARC date, from the ledger's RFC 3339 UTC.
///
/// Truncating, not converting: a ledger timestamp is UTC to the microsecond
/// and WARC wants whole seconds. Anything that is not this exact shape -- an
/// offset other than `Z` above all -- is refused rather than reinterpreted,
/// because shifting a time by an offset this code did not parse is how an
/// export ends up dated in the wrong hour with nothing to show for it.
fn warc_date_from_ledger(stamp: &str) -> Option<String> {
    let (seconds, rest) = stamp.split_at_checked(19)?;
    let shaped = seconds.bytes().zip(b"0000-00-00T00:00:00".iter()).all(|(got, want)| {
        if *want == b'0' {
            got.is_ascii_digit()
        } else {
            got == *want
        }
    });
    if !shaped {
        return None;
    }
    // Either straight to the zone, or a fractional part and then the zone.
    let zone = rest.strip_prefix('.').map_or(rest, |fraction| {
        let digits = fraction.len() - fraction.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        &fraction[digits..]
    });
    (zone == "Z").then(|| format!("{seconds}Z"))
}

/// The same, from a table that keeps unix milliseconds instead.
fn warc_date_from_unix_ms(millis: i64) -> Option<String> {
    let millis = u64::try_from(millis).ok()?;
    let instant = UNIX_EPOCH.checked_add(Duration::from_millis(millis))?;
    Some(humantime::format_rfc3339_seconds(instant).to_string())
}

/// Counts what reaches the writer, so the summary can report bytes without the
/// export buffering a copy of its own output to measure.
struct CountingWriter<W> {
    inner: W,
    count: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.count += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests;

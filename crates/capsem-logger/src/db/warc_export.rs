//! Exporting a session's archived bodies as a WARC file.
//!
//! The point is to need none of our code to read the result. `capsem-archive`
//! owns the record format; this module owns what a Capsem ledger row means in
//! it -- which URI a body was a capture of, when it happened, and which rows
//! cannot honestly be described at all.
//!
//! One index query, joined to each body's source row, ordered by
//! `(block_offset, body_offset)`, and the whole walk on one blocking thread
//! through one `BodyLogReader`. Archive order is what makes the export cost
//! one inflate per block instead of one per body; anything else would inflate
//! a 256 KiB block again for every body that happens to sit in it.
//!
//! **Nothing is invented.** A body whose source row is gone has no URI, and a
//! row whose timestamp will not parse has no date. Both are skipped and
//! counted with their reason, because a WARC record carrying a plausible
//! guess is worse than one that is not there: a reviewer cannot tell the two
//! apart afterwards, and the summary is the only place that can say so.
//!
//! Every body goes out through the same hash-verified read path a route uses.
//! An export is evidence, and evidence that skipped the check the interactive
//! path performs would be the one copy nobody verified.

use std::fmt;
use std::io::Write;
use std::time::{Duration, UNIX_EPOCH};

use capsem_archive::{warc, WarcRecord};
use serde_json::Value;

use super::bodies::{index_row, read_one, IndexRow, INDEX_COLUMNS};
use super::{DbHandle, DbResult};

/// What an export did, and what it could not describe.
#[derive(Debug, Default, Clone)]
pub struct ExportSummary {
    /// Records written, one per exported body.
    pub records: u64,
    /// Bytes handed to the writer, gzip members included.
    pub bytes_written: u64,
    /// Index rows that were not exported, each with why. An empty export and
    /// an export that refused every row are different statements.
    pub skipped: Vec<SkippedBody>,
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
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSourceRow => write!(f, "its source row is missing, so the body has no target URI"),
            Self::UnreadableTimestamp(stamp) => {
                write!(f, "its source row's timestamp {stamp:?} is not a ledger timestamp")
            }
        }
    }
}

/// How a body's identity is spelled in `WARC-Record-ID`. Unique by the
/// `UNIQUE(event_id, source_table, direction)` index on `event_body_blobs`:
/// one event has at most one body per direction, whichever table it is in.
fn record_id(row: &IndexRow) -> String {
    format!("urn:capsem:{}:{}", row.event_id, row.direction.as_str())
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
const SOURCE_BRANCHES: &[(&str, &str, &str)] = &[
    (
        "net_events",
        "'https://' || s.domain || COALESCE(s.path, '')",
        "s.timestamp, NULL",
    ),
    ("model_calls", "'https://' || s.provider || s.path", "s.timestamp, NULL"),
    ("tool_calls", "'capsem://tool/' || s.tool_name", "s.timestamp, NULL"),
    (
        "tool_responses",
        "'capsem://tool-response/' || s.call_id",
        "b.created_at, NULL",
    ),
    ("exec_events", "'capsem://exec/' || s.exec_id", "s.timestamp, NULL"),
    (
        "security_rule_events",
        "'capsem://security/' || s.rule_id",
        "NULL, s.timestamp_unix_ms",
    ),
];

/// One `SELECT` per source table, unioned and then ordered as a whole.
///
/// `LEFT JOIN`, not `JOIN`: an inner join would drop a body whose source row
/// is gone, and the export would report a count that quietly excluded it. The
/// left join brings it back with a NULL URI, which is what gets counted as a
/// skip with a reason.
///
/// `source_table` is CHECK-constrained to exactly these six, so the union
/// covers every row in the table.
fn export_sql() -> String {
    let branches: Vec<String> = SOURCE_BRANCHES
        .iter()
        .map(|(table, uri, dates)| {
            format!(
                "SELECT {columns}, {uri} AS target_uri, {dates}
                 FROM event_body_blobs AS b
                 LEFT JOIN {table} AS s ON s.event_id = b.event_id
                 WHERE b.source_table = '{table}'",
                columns = INDEX_COLUMNS
                    .split(", ")
                    .map(|column| format!("b.{column}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })
        .collect();
    // Archive order over the whole union, not per branch: one block holds
    // bodies from several tables, and it must inflate once.
    format!(
        "SELECT * FROM ({}) ORDER BY block_offset, body_offset",
        branches.join(" UNION ALL ")
    )
}

impl DbHandle {
    /// Write every archived body of this session to `out` as a WARC 1.1 file.
    ///
    /// `out` is taken by value and moved onto the blocking thread, because
    /// that is where the whole export runs: inflating blocks is CPU work on a
    /// file and belongs nowhere near the async runtime.
    ///
    /// # Errors
    ///
    /// A body the index names and the archive cannot produce, or does not
    /// produce intact, is a broken ledger and fails -- the same rule the
    /// interactive reads follow. A write failure fails too: a truncated
    /// export that returned a summary would be one nobody knew was partial.
    pub async fn export_warc<W: Write + Send + 'static>(&self, out: W) -> DbResult<ExportSummary> {
        let rows = self.export_rows().await?;
        let handle = self.clone();
        tokio::task::spawn_blocking(move || handle.export_blocking(rows, out))
            .await
            .map_err(|error| format!("session body WARC export task failed: {error}"))?
    }

    async fn export_rows(&self) -> DbResult<Vec<ExportRow>> {
        self.body_index_values(&export_sql(), &[])
            .await?
            .iter()
            .map(export_row)
            .collect()
    }

    fn export_blocking<W: Write>(&self, rows: Vec<ExportRow>, out: W) -> DbResult<ExportSummary> {
        let mut counting = CountingWriter { inner: out, count: 0 };
        let mut summary = ExportSummary::default();
        self.with_archive_reader(|reader| {
            for row in rows {
                let Some(target_uri) = row.target_uri.clone() else {
                    summary.skipped.push(skipped(&row.index, SkipReason::MissingSourceRow));
                    continue;
                };
                let Some(date) = row.warc_date() else {
                    let stamp = row.date_text.clone().unwrap_or_else(|| {
                        row.date_unix_ms
                            .map_or_else(|| "<none>".to_string(), |ms| ms.to_string())
                    });
                    summary
                        .skipped
                        .push(skipped(&row.index, SkipReason::UnreadableTimestamp(stamp)));
                    continue;
                };
                let id = record_id(&row.index);
                let truncated = row.index.truncated;
                // The hash-verified read path, not a shortcut around it.
                let body = read_one(reader, row.index)?;
                warc::write_record(
                    &mut counting,
                    &WarcRecord {
                        record_id: &id,
                        target_uri: &target_uri,
                        date: &date,
                        content_type: body.content_type.as_deref(),
                        truncated,
                        body: &body.bytes,
                    },
                )
                .map_err(|error| format!("session body WARC export could not write record {id}: {error}"))?;
                summary.records += 1;
            }
            Ok(())
        })?;
        counting
            .inner
            .flush()
            .map_err(|error| format!("session body WARC export could not flush its output: {error}"))?;
        summary.bytes_written = counting.count;
        Ok(summary)
    }
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

fn skipped(row: &IndexRow, reason: SkipReason) -> SkippedBody {
    SkippedBody {
        event_id: row.event_id.clone(),
        source_table: row.source_table.clone(),
        direction: row.direction.as_str().to_string(),
        reason,
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

fn export_row(row: &Value) -> DbResult<ExportRow> {
    Ok(ExportRow {
        index: index_row(row)?,
        target_uri: row.get(10).and_then(Value::as_str).map(str::to_string),
        date_text: row.get(11).and_then(Value::as_str).map(str::to_string),
        date_unix_ms: row.get(12).and_then(Value::as_i64),
    })
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

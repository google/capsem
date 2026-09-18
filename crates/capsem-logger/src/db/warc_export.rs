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

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use capsem_archive::{warc, WarcRecord};
use serde_json::Value;

use super::bodies::{index_row, read_one_checked, BodyFault, IndexRow, INDEX_COLUMNS};
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

impl ExportSummary {
    /// How many rows each reason accounted for.
    ///
    /// This is what the trailing `warcinfo` carries, and it is deliberately a
    /// count per reason rather than a list of event ids: the file is handed to
    /// a reviewer, and "11 bodies omitted, all of them corrupt" is the thing
    /// they need to know before they conclude anything from what is there.
    #[must_use]
    pub fn counts_by_reason(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for body in &self.skipped {
            *counts.entry(body.reason.label()).or_insert(0) += 1;
        }
        counts
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
/// names the ledger and the event id names the row in it -- the
/// `UNIQUE(event_id, source_table, direction)` index on `event_body_blobs`
/// gives one event at most one body per direction -- so the id is unique
/// across sessions and still points straight back at what it came from.
fn record_id(session: &str, row: &IndexRow) -> String {
    format!("urn:capsem:{session}:{}:{}", row.event_id, row.direction.as_str())
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

        self.with_archive_reader(|reader| {
            for row in rows {
                let identity = RowIdentity::of(&row.index);
                let Some(target_uri) = row.target_uri.clone() else {
                    summary.skipped.push(identity.because(SkipReason::MissingSourceRow));
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
                    summary
                        .skipped
                        .push(identity.because(SkipReason::UnrepresentableUri(target_uri)));
                    continue;
                }
                let Some(date) = row.warc_date() else {
                    let stamp = row.date_text.clone().unwrap_or_else(|| {
                        row.date_unix_ms
                            .map_or_else(|| "<none>".to_string(), |ms| ms.to_string())
                    });
                    summary
                        .skipped
                        .push(identity.because(SkipReason::UnreadableTimestamp(stamp)));
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
                let body = match read_one_checked(reader, row.index) {
                    Ok(body) => body,
                    Err(BodyFault::Corrupt(detail)) => {
                        summary.skipped.push(identity.because(SkipReason::CorruptBody(detail)));
                        continue;
                    }
                    Err(BodyFault::Unreadable(detail)) => {
                        summary
                            .skipped
                            .push(identity.because(SkipReason::UnreadableBody(detail)));
                        continue;
                    }
                    Err(fault) => return Err(fault.into_message()),
                };
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
            Ok(())
        })?;

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
        ("capsem-skipped".into(), summary.skipped.len().to_string()),
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

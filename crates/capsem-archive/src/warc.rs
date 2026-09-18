//! WARC 1.1 resource records, one gzip member each.
//!
//! A reviewer without Capsem still has to be able to read what a session
//! captured. WARC (ISO 28500) is what `warcio`, `pywb` and every other
//! web-archive tool already reads, so an export in it needs no reader of ours.
//!
//! The records are `resource`, not `response`: we archive decoded bodies, not
//! the raw HTTP messages they arrived in, and a `response` record promises a
//! reader a status line and headers it would then not find.
//!
//! **One gzip member per record.** The spec asks for it and tools depend on
//! it: a `.warc.gz` whose records are members can be seeked to and read one
//! record at a time, where a single stream over the whole file has to be
//! inflated from the beginning to reach the last record.
//!
//! Headers are a line-oriented format, so every field this writes is a place
//! where a `\r` or `\n` in a value would forge the rest of the block -- a
//! `content_type` of `text/html\r\nWARC-Type: revisit` is a different record
//! than the one the caller asked for. The values come from ledger rows, which
//! carry bytes an agent's counterparty chose. `write_record` refuses them by
//! name rather than sanitizing: a header quietly stripped of a byte is a
//! record that says something the ledger does not.

use std::io::Write;

use flate2::write::GzEncoder;
use flate2::Compression;

use crate::{ArchiveError, Result};

/// The WARC version line this writer emits.
pub const WARC_VERSION: &str = "WARC/1.1";
/// Every record is a `resource`: a stored body, not a captured HTTP message.
pub const WARC_TYPE: &str = "resource";
/// What a body with no recorded content type is declared as.
pub const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";
/// The spec's own field for a record whose block is not the whole entity, with
/// the reason `length` -- which is the only reason a Capsem body is ever cut.
pub const TRUNCATED_REASON: &str = "length";

/// One record to write: a body and the headers that describe it.
///
/// Borrowed throughout. An export walks index rows and writes each record as
/// it resolves it, so nothing here outlives the row it came from, and the
/// bodies are the one thing that must not be copied a second time.
#[derive(Debug, Clone, Copy)]
pub struct WarcRecord<'a> {
    /// The record's identity, written inside angle brackets. A URI by the
    /// spec; the exporter uses `urn:capsem:{event_id}:{direction}`.
    pub record_id: &'a str,
    /// What the body is a capture of.
    pub target_uri: &'a str,
    /// `YYYY-MM-DDTHH:MM:SSZ`. Written through as given: a date this writer
    /// reformatted would be a date the ledger never recorded.
    pub date: &'a str,
    /// The body's media type, or `None` for [`DEFAULT_CONTENT_TYPE`].
    pub content_type: Option<&'a str>,
    /// Whether `body` is a prefix of a larger entity, which adds
    /// `WARC-Truncated: length`. `Content-Length` stays what is written, so a
    /// reader is never told to expect bytes the record does not carry.
    pub truncated: bool,
    pub body: &'a [u8],
}

/// Append one gzip member carrying one WARC record to `out`.
///
/// The block is length-delimited by `Content-Length`, so a body may contain
/// CRLF, a WARC version line, or any other byte sequence a reader might
/// otherwise mistake for structure.
///
/// # Errors
///
/// - [`ArchiveError::WarcHeaderBreak`] when a header value contains `\r` or
///   `\n`. Nothing is written in that case, so a refused record cannot leave a
///   half-written member behind.
/// - [`ArchiveError::Io`] from the gzip encoder or from `out`.
pub fn write_record<W: Write>(out: &mut W, rec: &WarcRecord<'_>) -> Result<()> {
    // Every field is checked before a byte is compressed: the alternative is
    // discovering the forgery with half a member already in the caller's file.
    refuse_line_breaks("record_id", rec.record_id)?;
    refuse_line_breaks("target_uri", rec.target_uri)?;
    refuse_line_breaks("date", rec.date)?;
    if let Some(content_type) = rec.content_type {
        refuse_line_breaks("content_type", content_type)?;
    }

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(header_block(rec).as_bytes())?;
    encoder.write_all(rec.body)?;
    // Two CRLFs end a record, and they are not part of the block: a reader
    // that has consumed `Content-Length` bytes expects exactly these next.
    encoder.write_all(b"\r\n\r\n")?;
    out.write_all(&encoder.finish()?)?;
    Ok(())
}

/// The named headers and the blank line that ends them, in a fixed order.
///
/// Order is not significant to a WARC reader, but it is significant to a human
/// diffing two exports, and a fixed one costs nothing.
fn header_block(rec: &WarcRecord<'_>) -> String {
    let digest = blake3::hash(rec.body);
    let content_type = rec.content_type.unwrap_or(DEFAULT_CONTENT_TYPE);
    let truncated = if rec.truncated {
        format!("WARC-Truncated: {TRUNCATED_REASON}\r\n")
    } else {
        String::new()
    };
    format!(
        "{WARC_VERSION}\r\n\
         WARC-Type: {WARC_TYPE}\r\n\
         WARC-Record-ID: <{record_id}>\r\n\
         WARC-Target-URI: {target_uri}\r\n\
         WARC-Date: {date}\r\n\
         WARC-Block-Digest: blake3:{digest}\r\n\
         {truncated}\
         Content-Type: {content_type}\r\n\
         Content-Length: {length}\r\n\
         \r\n",
        record_id = rec.record_id,
        target_uri = rec.target_uri,
        date = rec.date,
        length = rec.body.len(),
    )
}

fn refuse_line_breaks(field: &'static str, value: &str) -> Result<()> {
    if value.contains('\r') || value.contains('\n') {
        return Err(ArchiveError::WarcHeaderBreak { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests;

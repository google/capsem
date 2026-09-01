//! Bounded gzip decompression shared by the response-materialization paths.
//!
//! Upstream response bodies are only bounded while *compressed* (the collectors
//! cap them at 100 MB). gzip can expand ~1000x, so a hostile or compromised
//! upstream can send a small compressed body that decompresses to tens of GB
//! and OOM the host. Every full-body decompression must go through
//! [`decompress_gzip_capped`], which refuses a body larger than the cap instead
//! of reading it into memory.

use std::io::Read;

/// Ceiling on a decompressed response body. Chosen well above any legitimate
/// non-streaming model/MCP response (the compressed source is already capped at
/// 100 MB) while keeping a single hostile body from exhausting host RAM.
pub(crate) const MAX_DECOMPRESSED_BODY: usize = 256 * 1024 * 1024;

/// Decompress a complete gzip `body`, allocating at most `cap` (+1) bytes.
///
/// Returns an error if the decompressed output would exceed `cap` -- a likely
/// compression bomb -- rather than truncating (which would corrupt the JSON the
/// callers parse). The `+1` byte lets us distinguish "exactly at the cap" from
/// "over the cap" without unbounded reads.
pub(crate) fn decompress_gzip_capped(body: &[u8], cap: usize) -> anyhow::Result<Vec<u8>> {
    let mut decoder = flate2::read::GzDecoder::new(body).take(cap as u64 + 1);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    if out.len() > cap {
        anyhow::bail!("decompressed response body exceeds {cap} byte cap (possible compression bomb)");
    }
    Ok(out)
}

#[cfg(test)]
mod tests;

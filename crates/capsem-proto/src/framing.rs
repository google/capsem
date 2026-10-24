//! One bounded named-MessagePack encoder for records and length-prefixed streams.
use std::io::{self, Write};

use serde::Serialize;

pub use limited_writer::LimitedWriter;
mod limited_writer;

/// Append directly to final storage, stopping serialization before excess writes.
fn append_record<T: Serialize>(bytes: &mut Vec<u8>, message: &T, limit: usize) -> io::Result<()> {
    let maximum = bytes
        .len()
        .checked_add(limit)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame byte budget overflow"))?;
    let mut writer = LimitedWriter::new(FinalBuffer { bytes, maximum }, limit);
    let encoded = rmp_serde::encode::write_named(&mut writer, message);
    let exceeded = writer.exceeded();
    encoded.map_err(|error| {
        if exceeded {
            io::Error::new(io::ErrorKind::InvalidData, TooLarge(limit))
        } else {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("encode MessagePack record: {error}"),
            )
        }
    })
}

#[derive(Debug)]
struct TooLarge(usize);

impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MessagePack frame too large: {}", self.0)
    }
}
impl std::error::Error for TooLarge {}

pub(crate) fn is_frame_too_large(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|error| error.is::<TooLarge>())
}

// Vec's ordinary amortized growth may double a nearly complete final frame
// beyond its budget for one small trailing field. Keep amortization, capped at
// the selected final ceiling, without eagerly allocating MAX_FRAME_SIZE per ping.
struct FinalBuffer<'a> {
    bytes: &'a mut Vec<u8>,
    maximum: usize,
}

impl Write for FinalBuffer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let needed = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|needed| *needed <= self.maximum)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame byte budget exceeded"))?;
        if needed > self.bytes.capacity() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(8)
                .max(needed)
                .min(self.maximum);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serialize once after a reserved header, without a payload-to-frame copy.
pub fn encode_frame<T: Serialize>(message: &T, limit: u32) -> io::Result<Vec<u8>> {
    let mut frame = vec![0; 4];
    append_record(&mut frame, message, limit as usize)?;
    let len = (frame.len() - 4) as u32;
    frame[..4].copy_from_slice(&len.to_be_bytes());
    Ok(frame)
}

/// Encode a record whose transport already supplies its boundary.
pub fn encode_record<T: Serialize>(message: &T, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    append_record(&mut bytes, message, limit)?;
    Ok(bytes)
}

/// Typed immutable final bytes cross queue/replay boundaries together. Original
/// payload ownership is not retained; clones share only the encoded frame.
pub struct PreparedFrame<T> {
    bytes: std::sync::Arc<Vec<u8>>,
    marker: std::marker::PhantomData<fn() -> T>,
}

impl<T> Clone for PreparedFrame<T> {
    fn clone(&self) -> Self {
        Self {
            bytes: self.bytes.clone(),
            marker: std::marker::PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for PreparedFrame<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedFrame")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl<T: Serialize> PreparedFrame<T> {
    pub fn new(message: &T, limit: u32) -> io::Result<Self> {
        Ok(Self {
            bytes: std::sync::Arc::new(encode_frame(message, limit)?),
            marker: std::marker::PhantomData,
        })
    }
}

impl<T> PreparedFrame<T> {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests;

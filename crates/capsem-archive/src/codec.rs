//! Compression streams used by archive blocks.
//!
//! The archive owns framing, bounds and hashes. A codec owns only one block's
//! streaming compression state. Keeping this interface in the pure-Rust
//! archive crate lets the confined ledger process provide a native codec
//! without linking that native library into the service or session process.

use std::io;
use std::sync::Arc;

use flate2::{Compress, Compression, FlushCompress, Status};
use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZFlush, MZStatus};

use crate::format::{CODEC_DEFLATE, DEFLATE_LEVEL, SYNC_FLUSH_TAIL};
use crate::{ArchiveError, Result};

/// One block's streaming compressor.
pub trait BlockEncoder: Send {
    fn write(&mut self, input: &[u8], output: &mut Vec<u8>) -> Result<()>;
    fn flush(&mut self, output: &mut Vec<u8>, finish: bool) -> Result<()>;
}

/// One block's streaming decompressor.
pub trait BlockDecoder: Send {
    /// Decode exactly one framed segment while retaining the block's history.
    fn decode_segment(&mut self, input: &[u8], raw_len: usize, finish: bool, at: u64) -> Result<Vec<u8>>;
}

/// Factory for the encoder and decoder of one on-disk codec id.
pub trait BlockCodec: Send + Sync {
    fn id(&self) -> u8;
    fn encoder(&self) -> Result<Box<dyn BlockEncoder>>;
    fn decoder(&self) -> Result<Box<dyn BlockDecoder>>;
}

/// The codec used for new blocks and the codecs accepted while reading.
#[derive(Clone)]
pub struct ArchiveCodecs {
    write: Arc<dyn BlockCodec>,
    read: Arc<[Arc<dyn BlockCodec>]>,
}

impl ArchiveCodecs {
    /// The v3 compatibility configuration: write and read raw deflate.
    #[must_use]
    pub fn deflate() -> Self {
        let codec: Arc<dyn BlockCodec> = Arc::new(DeflateCodec);
        Self {
            write: Arc::clone(&codec),
            read: vec![codec].into(),
        }
    }

    /// Write with `codec` while retaining v3 deflate read compatibility.
    pub fn with_write_codec(codec: Arc<dyn BlockCodec>) -> Result<Self> {
        let id = codec.id();
        if id == 0 || id == CODEC_DEFLATE {
            return Err(ArchiveError::InvalidCodecId(id));
        }
        let deflate: Arc<dyn BlockCodec> = Arc::new(DeflateCodec);
        Ok(Self {
            write: Arc::clone(&codec),
            read: vec![deflate, codec].into(),
        })
    }

    pub(crate) fn encoder(&self) -> Result<(u8, Box<dyn BlockEncoder>)> {
        Ok((self.write.id(), self.write.encoder()?))
    }

    pub(crate) fn decoder(&self, id: u8, block_offset: u64) -> Result<Box<dyn BlockDecoder>> {
        self.read
            .iter()
            .find(|codec| codec.id() == id)
            .ok_or(ArchiveError::UnsupportedCodec {
                block_offset,
                codec: id,
            })?
            .decoder()
    }
}

impl Default for ArchiveCodecs {
    fn default() -> Self {
        Self::deflate()
    }
}

struct DeflateCodec;

impl BlockCodec for DeflateCodec {
    fn id(&self) -> u8 {
        CODEC_DEFLATE
    }

    fn encoder(&self) -> Result<Box<dyn BlockEncoder>> {
        Ok(Box::new(DeflateEncoder(Compress::new(
            Compression::new(DEFLATE_LEVEL),
            false,
        ))))
    }

    fn decoder(&self) -> Result<Box<dyn BlockDecoder>> {
        Ok(Box::new(DeflateDecoder(InflateState::new_boxed(DataFormat::Raw))))
    }
}

struct DeflateEncoder(Compress);

impl BlockEncoder for DeflateEncoder {
    fn write(&mut self, input: &[u8], output: &mut Vec<u8>) -> Result<()> {
        deflate_into(&mut self.0, input, output, FlushCompress::None)
    }

    fn flush(&mut self, output: &mut Vec<u8>, finish: bool) -> Result<()> {
        let flush = if finish {
            FlushCompress::Finish
        } else {
            FlushCompress::Sync
        };
        deflate_into(&mut self.0, &[], output, flush)
    }
}

struct DeflateDecoder(Box<InflateState>);

impl BlockDecoder for DeflateDecoder {
    fn decode_segment(&mut self, input: &[u8], raw_len: usize, finish: bool, at: u64) -> Result<Vec<u8>> {
        if !finish && !input.ends_with(&SYNC_FLUSH_TAIL) {
            return Err(ArchiveError::BadSegment(at));
        }
        let mut raw = vec![0u8; raw_len + 1];
        let (mut consumed, mut written) = (0, 0);
        let mut ended = false;
        loop {
            let result = inflate(&mut self.0, &input[consumed..], &mut raw[written..], MZFlush::None);
            consumed += result.bytes_consumed;
            written += result.bytes_written;
            match result.status {
                Ok(MZStatus::StreamEnd) => {
                    ended = true;
                    break;
                }
                Ok(_) if consumed == input.len() || (result.bytes_consumed == 0 && result.bytes_written == 0) => break,
                Ok(_) => {}
                Err(miniz_oxide::MZError::Buf) => break,
                Err(error) => return Err(ArchiveError::Inflate(at, format!("{error:?}"))),
            }
        }
        raw.truncate(written.min(raw_len));
        if consumed != input.len() || written != raw_len || ended != finish {
            return Err(ArchiveError::Integrity(at));
        }
        Ok(raw)
    }
}

fn deflate_into(compressor: &mut Compress, input: &[u8], out: &mut Vec<u8>, flush: FlushCompress) -> Result<()> {
    let mut consumed = 0;
    loop {
        out.reserve((input.len() - consumed) + (input.len() - consumed) / 1000 + 64 * 1024);
        let before = compressor.total_in();
        let status = compressor
            .compress_vec(&input[consumed..], out, flush)
            .map_err(|error| ArchiveError::Io(io::Error::other(error)))?;
        consumed += usize::try_from(compressor.total_in() - before).expect("bounded by the input");
        let done = match flush {
            FlushCompress::Finish => status == Status::StreamEnd,
            _ => consumed == input.len() && out.len() < out.capacity(),
        };
        if done {
            return Ok(());
        }
    }
}

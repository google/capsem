use std::io;
use std::sync::Arc;

use capsem_archive::{
    ArchiveCodecs, ArchiveError, BlockCodec, BlockDecoder, BlockEncoder, Result as ArchiveResult, CODEC_ZSTD,
};
use zstd::stream::raw::{Decoder, Encoder, InBuffer, Operation, OutBuffer};

const ZSTD_LEVEL: i32 = 3;
const OUTPUT_CHUNK: usize = 128 * 1024;

pub(crate) fn archive_codecs() -> ArchiveResult<ArchiveCodecs> {
    ArchiveCodecs::with_write_codec(Arc::new(ZstdCodec))
}

struct ZstdCodec;

impl BlockCodec for ZstdCodec {
    fn id(&self) -> u8 {
        CODEC_ZSTD
    }

    fn encoder(&self) -> ArchiveResult<Box<dyn BlockEncoder>> {
        Encoder::new(ZSTD_LEVEL)
            .map(|encoder| Box::new(ZstdEncoder(encoder)) as Box<dyn BlockEncoder>)
            .map_err(ArchiveError::Io)
    }

    fn decoder(&self) -> ArchiveResult<Box<dyn BlockDecoder>> {
        Decoder::new()
            .map(|decoder| Box::new(ZstdDecoder(decoder)) as Box<dyn BlockDecoder>)
            .map_err(ArchiveError::Io)
    }
}

struct ZstdEncoder(Encoder<'static>);

impl BlockEncoder for ZstdEncoder {
    fn write(&mut self, input: &[u8], output: &mut Vec<u8>) -> ArchiveResult<()> {
        let mut input = InBuffer::around(input);
        while input.pos() < input.src.len() {
            let before = input.pos();
            with_output_capacity(output, |out| self.0.run(&mut input, out)).map_err(ArchiveError::Io)?;
            if input.pos() == before {
                return Err(ArchiveError::Io(io::Error::other(
                    "zstd encoder made no input progress",
                )));
            }
        }
        Ok(())
    }

    fn flush(&mut self, output: &mut Vec<u8>, finish: bool) -> ArchiveResult<()> {
        loop {
            let remaining = with_output_capacity(output, |out| {
                if finish {
                    self.0.finish(out, true)
                } else {
                    self.0.flush(out)
                }
            })
            .map_err(ArchiveError::Io)?;
            if remaining == 0 {
                return Ok(());
            }
        }
    }
}

struct ZstdDecoder(Decoder<'static>);

impl BlockDecoder for ZstdDecoder {
    fn decode_segment(&mut self, input: &[u8], raw_len: usize, finish: bool, at: u64) -> ArchiveResult<Vec<u8>> {
        let mut raw = Vec::with_capacity(raw_len + 1);
        let (consumed, written, ended) = {
            let mut input = InBuffer::around(input);
            let mut output = OutBuffer::around(&mut raw);
            let mut ended = false;
            loop {
                let before = (input.pos(), output.pos());
                let remaining = self
                    .0
                    .run(&mut input, &mut output)
                    .map_err(|error| ArchiveError::Inflate(at, error.to_string()))?;
                if remaining == 0 {
                    ended = true;
                    break;
                }
                if input.pos() == input.src.len() || before == (input.pos(), output.pos()) {
                    break;
                }
            }
            (input.pos(), output.pos(), ended)
        };
        if consumed != input.len() || written != raw_len || ended != finish {
            return Err(ArchiveError::Integrity(at));
        }
        Ok(raw)
    }
}

fn with_output_capacity(
    output: &mut Vec<u8>,
    operation: impl FnOnce(&mut OutBuffer<'_, Vec<u8>>) -> io::Result<usize>,
) -> io::Result<usize> {
    output.reserve(OUTPUT_CHUNK);
    let position = output.len();
    let mut buffer = OutBuffer::around_pos(output, position);
    operation(&mut buffer)
}

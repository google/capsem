use capsem_archive::format::{BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, SEGMENT_HEADER_BYTES};
use capsem_archive::{BodyLogReader, BodyLogWriter, CODEC_ZSTD};

use super::archive_codecs;

#[test]
fn zstd_keeps_history_across_durable_segments_and_deflate_stays_readable() {
    let dir = tempfile::tempdir().unwrap();
    let zstd_path = dir.path().join("zstd.bodies");
    let codecs = archive_codecs().unwrap();
    let mut writer = BodyLogWriter::open_with_codecs(&zstd_path, codecs.clone()).unwrap();
    let body: Vec<u8> = (0_usize..8192)
        .map(|index| (index.wrapping_mul(37) & 0xff) as u8)
        .collect();
    let first = writer.stage(&body).unwrap();
    writer.flush_segment().unwrap();
    let before = writer.end();
    let second = writer.stage(&body).unwrap();
    writer.close_block().unwrap();
    assert!(
        writer.end() - before < 256,
        "the second segment did not reuse zstd history"
    );
    drop(writer);

    let encoded = std::fs::read(&zstd_path).unwrap();
    assert_eq!(encoded[FILE_HEADER_BYTES + 4], CODEC_ZSTD);
    let reader = BodyLogReader::open_with_codecs(&zstd_path, codecs.clone()).unwrap();
    assert_eq!(reader.read(first).unwrap(), body);
    assert_eq!(reader.read(second).unwrap(), body);

    let deflate_path = dir.path().join("deflate.bodies");
    let mut deflate = BodyLogWriter::open(&deflate_path).unwrap();
    let reference = deflate.stage(b"an archive from before zstd").unwrap();
    deflate.close_block().unwrap();
    let compatible = BodyLogReader::open_with_codecs(&deflate_path, codecs).unwrap();
    assert_eq!(compatible.read(reference).unwrap(), b"an archive from before zstd");
}

#[test]
fn zstd_corruption_is_refused_before_bytes_are_returned() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.bodies");
    let codecs = archive_codecs().unwrap();
    let mut writer = BodyLogWriter::open_with_codecs(&path, codecs.clone()).unwrap();
    let reference = writer.stage(b"authenticated zstd bytes").unwrap();
    writer.close_block().unwrap();
    drop(writer);

    let mut encoded = std::fs::read(&path).unwrap();
    let payload = FILE_HEADER_BYTES + BLOCK_HEADER_BYTES + SEGMENT_HEADER_BYTES;
    encoded[payload + 1] ^= 0x80;
    std::fs::write(&path, encoded).unwrap();
    let reader = BodyLogReader::open_with_codecs(&path, codecs).unwrap();
    assert!(reader.read(reference).is_err());
}

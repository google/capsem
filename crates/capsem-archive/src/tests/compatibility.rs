//! Old-format bytes are built independently of the current framing encoder.
use std::io::Write;

use crate::format::{ArchiveId, BodyRef, GenerationId, FILE_HEADER_BYTES};
use crate::{commit_retained, stage_retained_blocks, BodyLogReader, BodyLogWriter};

fn legacy_archive(path: &std::path::Path) -> (BodyRef, u64) {
    let body = b"legacy body, retained without rewriting its compressed bytes";
    let mut file = Vec::new();
    file.extend_from_slice(b"CAPSEMBL\x03\0\0\0\x50\0\0\0");
    file.extend_from_slice(ArchiveId::generate().as_bytes());
    file.extend_from_slice(GenerationId::generate().as_bytes());
    file.extend_from_slice(capsem_crypto::blake3::hash(&file).as_bytes());
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
    encoder.write_all(body).unwrap();
    let compressed = encoder.finish().unwrap();
    file.extend_from_slice(b"BLK2\x01\0\0\0SGMT\x01\0\0\0");
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    file.extend_from_slice(capsem_crypto::blake3::hash(body).as_bytes());
    file.extend_from_slice(&compressed);
    std::fs::write(path, &file).unwrap();
    (
        BodyRef {
            block_offset: 80,
            offset: 0,
            len: body.len() as u32,
        },
        file.len() as u64 - 80,
    )
}

#[test]
fn legacy_generation_reads_appends_and_compacts_into_xxh3_generation() {
    let dir = tempfile::tempdir().unwrap();
    let path = super::archive(&dir);
    let (old, old_disk_len) = legacy_archive(&path);
    let old_bytes = BodyLogReader::open(&path).unwrap().read(old).unwrap();
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let second = writer.stage(b"after reopen").unwrap();
    let second_segment = writer.close_block().unwrap().unwrap();
    drop(writer);
    let before = std::fs::read(&path).unwrap();
    assert_eq!(before[8], 3, "append does not rewrite a legacy header");
    assert_eq!(
        before[second.block_offset as usize + 8 + 5],
        0,
        "v3 append retains BLAKE3 tag"
    );
    let staged = stage_retained_blocks(
        &path,
        &[
            (old.block_offset, old_disk_len),
            (second.block_offset, second_segment.disk_len),
        ],
    )
    .unwrap();
    let moved_old = BodyRef {
        block_offset: staged.map()[&old.block_offset],
        ..old
    };
    let moved_second = BodyRef {
        block_offset: staged.map()[&second.block_offset],
        ..second
    };
    commit_retained(staged).unwrap();
    let compacted = std::fs::read(&path).unwrap();
    assert_eq!(compacted[8], 4);
    assert_eq!(
        &compacted[FILE_HEADER_BYTES..],
        &before[FILE_HEADER_BYTES..],
        "retention copies payloads verbatim"
    );
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(moved_old).unwrap(), old_bytes);
    assert_eq!(reader.read(moved_second).unwrap(), b"after reopen");
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let third = writer.stage(b"new XXH3 segment alongside old BLAKE3 blocks").unwrap();
    writer.close_block().unwrap();
    let after = std::fs::read(&path).unwrap();
    assert_eq!(after[third.block_offset as usize + 8 + 5], 1);
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(moved_old).unwrap(), old_bytes);
    assert_eq!(
        reader.read(third).unwrap(),
        b"new XXH3 segment alongside old BLAKE3 blocks"
    );
}

use std::io::{Seek, SeekFrom};
use std::os::unix::fs::FileExt;

use super::*;
use crate::format::FILE_HEADER_BYTES;
use crate::tests::{archive, XorShift};
use crate::{BodyLogWriter, SegmentWritten};

/// Three segments of one open block, one body each, and where they landed.
fn three_segments(path: &Path) -> (Vec<(BodyRef, Vec<u8>)>, Vec<SegmentWritten>) {
    let mut writer = BodyLogWriter::open(path).unwrap();
    let mut rng = XorShift(0x5eed);
    let mut bodies = Vec::new();
    let mut segments = Vec::new();
    for _ in 0..3 {
        let body = rng.text(3000);
        bodies.push((writer.stage(&body).unwrap(), body));
        segments.push(writer.flush_segment().unwrap().unwrap());
    }
    (bodies, segments)
}

/// File offset where segment `k` of the first block starts.
fn segment_start(segments: &[SegmentWritten], k: usize) -> u64 {
    let block = FILE_HEADER_BYTES as u64;
    if k == 0 {
        block + BLOCK_HEADER_BYTES as u64
    } else {
        block + segments[k - 1].disk_len
    }
}

fn flip(path: &Path, at: u64) {
    let file = std::fs::OpenOptions::new().read(true).write(true).open(path).unwrap();
    let mut byte = [0u8];
    file.read_exact_at(&mut byte, at).unwrap();
    file.write_all_at(&[byte[0] ^ 0x5a], at).unwrap();
}

#[test]
fn reading_forward_continues_the_cursor_one_segment_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (bodies, _) = three_segments(&path);
    let reader = BodyLogReader::open(&path).unwrap();
    for (k, (reference, body)) in bodies.iter().enumerate() {
        assert_eq!(&reader.read(*reference).unwrap(), body);
        assert_eq!(reader.segments_inflated(), k as u64 + 1, "one new segment per read");
    }
    assert_eq!(reader.blocks_inflated(), 1);

    // Anything already inflated is served from the cursor.
    assert_eq!(reader.read(bodies[0].0).unwrap(), bodies[0].1);
    assert_eq!(reader.segments_inflated(), 3);
}

#[test]
fn reading_the_last_body_first_inflates_every_segment_before_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (bodies, _) = three_segments(&path);
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(bodies[2].0).unwrap(), bodies[2].1);
    assert_eq!(reader.segments_inflated(), 3);
    assert_eq!(reader.read(bodies[1].0).unwrap(), bodies[1].1);
    assert_eq!(reader.segments_inflated(), 3);
}

#[test]
fn switching_blocks_restarts_the_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let a = writer.stage(b"block a").unwrap();
    writer.close_block().unwrap();
    let b = writer.stage(b"block b").unwrap();
    writer.close_block().unwrap();
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(a).unwrap(), b"block a");
    assert_eq!(reader.read(b).unwrap(), b"block b");
    assert_eq!(reader.read(a).unwrap(), b"block a");
    assert_eq!(reader.blocks_inflated(), 3);
}

/// A reference into bytes the block never committed is an error, never a
/// guess: past the last written segment of an open block the file simply
/// ends, and past a closed block's FINAL segment there is nothing more.
#[test]
fn a_forged_ref_past_the_committed_extent_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let reference = writer.stage(b"twelve bytes").unwrap();
    writer.flush_segment().unwrap();

    let reader = BodyLogReader::open(&path).unwrap();
    let past = BodyRef {
        offset: 10,
        len: 5,
        ..reference
    };
    assert!(matches!(reader.read(past), Err(ArchiveError::TruncatedBlock(_))));
    assert_eq!(
        reader.read(reference).unwrap(),
        b"twelve bytes",
        "a failed read leaves nothing stale"
    );

    writer.close_block().unwrap();
    let reader = BodyLogReader::open(&path).unwrap();
    assert!(matches!(reader.read(past), Err(ArchiveError::RefOutOfRange)));
    let overflow = BodyRef {
        offset: u32::MAX,
        len: u32::MAX,
        ..reference
    };
    assert!(matches!(reader.read(overflow), Err(ArchiveError::RefOutOfRange)));
}

#[test]
fn a_ref_that_is_not_a_block_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (bodies, _) = three_segments(&path);
    let reader = BodyLogReader::open(&path).unwrap();
    let inside = BodyRef {
        block_offset: bodies[0].0.block_offset + 3,
        ..bodies[0].0
    };
    assert!(matches!(reader.read(inside), Err(ArchiveError::BadBlockHeader(_))));
    let beyond = BodyRef {
        block_offset: 1 << 40,
        ..bodies[0].0
    };
    assert!(matches!(reader.read(beyond), Err(ArchiveError::BadBlockHeader(_))));
}

/// Flip one byte of segment `k` -- in its header or in its payload -- and
/// every body that needs segment `k` fails, while bodies wholly before it
/// still read. Never the wrong bytes, never a panic.
#[test]
fn a_flipped_byte_in_segment_k_fails_that_segment_and_nothing_before_it() {
    for k in 0..3 {
        for (label, offset_in_segment) in [
            ("magic", 0u64),
            ("flags", 4),
            ("raw_start", 8),
            ("raw_len", 12),
            ("comp_len", 16),
            ("hash", 30),
            ("payload", SEGMENT_HEADER_BYTES as u64 + 5),
            ("tail", u64::MAX),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = archive(&dir);
            let (bodies, segments) = three_segments(&path);
            let start = segment_start(&segments, k);
            let at = if offset_in_segment == u64::MAX {
                FILE_HEADER_BYTES as u64 + segments[k].disk_len - 1
            } else {
                start + offset_in_segment
            };
            flip(&path, at);
            let reader = BodyLogReader::open(&path).unwrap();
            for (index, (reference, body)) in bodies.iter().enumerate() {
                match reader.read(*reference) {
                    Ok(bytes) => {
                        assert!(index < k, "segment {k} {label}: body {index} read despite the flip");
                        assert_eq!(&bytes, body, "segment {k} {label}: wrong bytes for body {index}");
                    }
                    Err(_) => assert!(index >= k, "segment {k} {label}: body {index} before the flip failed"),
                }
            }
        }
    }
}

#[test]
fn an_unknown_codec_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (bodies, _) = three_segments(&path);
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.write_all_at(&[2], FILE_HEADER_BYTES as u64 + 4).unwrap();
    let reader = BodyLogReader::open(&path).unwrap();
    assert!(matches!(
        reader.read(bodies[0].0),
        Err(ArchiveError::UnsupportedCodec { codec: 2, .. })
    ));
}

#[test]
fn a_version_one_archive_is_refused_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    crate::tests::write_version_one_archive(&path);
    assert!(matches!(BodyLogReader::open(&path), Err(ArchiveError::BadFileHeader)));
}

#[test]
fn a_replaced_file_is_noticed() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    three_segments(&path);
    let reader = BodyLogReader::open(&path).unwrap();
    assert!(!reader.file_was_replaced());
    let copy = dir.path().join("copy");
    std::fs::copy(&path, &copy).unwrap();
    std::fs::rename(&copy, &path).unwrap();
    assert!(reader.file_was_replaced());
}

#[test]
fn open_refuses_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    three_segments(&path);
    let link = dir.path().join("link.bodies");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(matches!(BodyLogReader::open(&link), Err(ArchiveError::Symlink(_))));
}

#[test]
fn descriptor_reader_pins_identity_and_never_crosses_captured_extents() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let first = writer.stage(b"first body").unwrap();
    let first_extent = writer.flush_segment().unwrap().unwrap();
    let second = writer.stage(b"second body").unwrap();
    writer.flush_segment().unwrap().unwrap();
    writer.sync().unwrap();
    let header = writer.header();

    let mut file = std::fs::File::open(&path).unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    let committed_end = first.block_offset + first_extent.disk_len;
    let reader = BodyLogReader::from_descriptor(file, header, committed_end).unwrap();
    let extent = BlockExtent {
        disk_len: first_extent.disk_len,
        raw_len: first_extent.raw_len,
    };
    assert_eq!(reader.read_bounded(first, extent).unwrap(), b"first body");
    assert!(matches!(
        reader.read_bounded(second, extent),
        Err(ArchiveError::RefOutOfRange)
    ));
    assert!(matches!(
        reader.read_bounded(
            first,
            BlockExtent {
                disk_len: u64::MAX,
                raw_len: first_extent.raw_len,
            }
        ),
        Err(ArchiveError::CommittedExtent)
    ));

    let wrong = FileHeader {
        generation_id: crate::GenerationId::new_v4(),
        ..header
    };
    assert!(matches!(
        BodyLogReader::from_descriptor(std::fs::File::open(&path).unwrap(), wrong, committed_end),
        Err(ArchiveError::ArchiveIdentityMismatch)
    ));
    assert!(matches!(
        BodyLogReader::from_descriptor(
            std::fs::File::open(&path).unwrap(),
            header,
            std::fs::metadata(&path).unwrap().len() + 1
        ),
        Err(ArchiveError::CommittedExtent)
    ));
}

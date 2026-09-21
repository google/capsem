use super::*;
use crate::format::{BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, SEGMENT_HEADER_BYTES};
use crate::tests::{archive, file_len};
use crate::BodyLogReader;

const FIRST_BLOCK: u64 = FILE_HEADER_BYTES as u64;

#[test]
fn stage_names_the_real_block_offset_and_a_flush_makes_it_readable() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let first = writer.stage(b"first body").unwrap();
    let second = writer.stage(b"second body, longer").unwrap();
    assert_eq!(
        first,
        BodyRef {
            block_offset: FIRST_BLOCK,
            offset: 0,
            len: 10
        }
    );
    assert_eq!(second.block_offset, FIRST_BLOCK);
    assert_eq!(second.offset, 10);
    assert_eq!(writer.pending_bytes(), 29);
    assert_eq!(file_len(&path), FIRST_BLOCK, "nothing reaches the file before a flush");

    let written = writer.flush_segment().unwrap().expect("bodies were pending");
    assert_eq!(
        written,
        SegmentWritten {
            block_offset: FIRST_BLOCK,
            raw_len: 29,
            disk_len: file_len(&path) - FIRST_BLOCK,
            closed: false,
        }
    );
    assert_eq!(writer.pending_bytes(), 0);
    assert_eq!(writer.end(), file_len(&path));

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(first).unwrap(), b"first body");
    assert_eq!(reader.read(second).unwrap(), b"second body, longer");
}

#[test]
fn a_flush_with_nothing_staged_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    assert!(writer.flush_segment().unwrap().is_none(), "no block is open");
    writer.stage(b"body").unwrap();
    writer.flush_segment().unwrap().expect("one segment");
    let end = file_len(&path);
    assert!(
        writer.flush_segment().unwrap().is_none(),
        "an empty segment is never written"
    );
    assert_eq!(file_len(&path), end);
}

/// The whole point of the format: one block across many flushes, its raw
/// and on-disk lengths growing under one offset, only the first segment
/// carrying the block header.
#[test]
fn segments_grow_one_block_under_one_offset() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let mut previous = 0u64;
    let mut refs = Vec::new();
    for round in 0..5u32 {
        let body = format!("round {round} body ").repeat(40).into_bytes();
        refs.push((writer.stage(&body).unwrap(), body));
        let before = file_len(&path);
        let written = writer.flush_segment().unwrap().expect("a segment");
        assert_eq!(written.block_offset, FIRST_BLOCK, "the block stays open");
        assert_eq!(written.disk_len, file_len(&path) - FIRST_BLOCK);
        let header = if round == 0 { BLOCK_HEADER_BYTES } else { 0 };
        assert!(file_len(&path) - before > (header + SEGMENT_HEADER_BYTES) as u64);
        assert!(written.disk_len > previous);
        previous = written.disk_len;
    }
    assert_eq!(
        writer.open_block_raw_len(),
        Some(refs.iter().map(|(_, body)| body.len()).sum())
    );

    let reader = BodyLogReader::open(&path).unwrap();
    for (reference, body) in &refs {
        assert_eq!(&reader.read(*reference).unwrap(), body);
    }
    assert_eq!(reader.blocks_inflated(), 1);
    assert_eq!(reader.segments_inflated(), 5);
}

#[test]
fn closing_writes_a_final_segment_and_the_next_body_opens_a_block_at_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let first = writer.stage(b"in the first block").unwrap();
    writer.flush_segment().unwrap();
    let closed = writer.close_block().unwrap().expect("a block was open");
    assert!(closed.closed);
    assert_eq!(closed.raw_len, 18, "an empty final segment adds no raw bytes");
    assert_eq!(writer.open_block_raw_len(), None);
    assert!(writer.close_block().unwrap().is_none(), "nothing left to close");

    let second = writer.stage(b"in the second").unwrap();
    assert_eq!(second.offset, 0, "a fresh block starts at zero");
    assert_eq!(second.block_offset, FIRST_BLOCK + closed.disk_len);
    let last = writer.close_block().unwrap().expect("close carries the pending body");
    assert_eq!(last.raw_len, 13);
    assert_eq!(writer.end(), file_len(&path));

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(first).unwrap(), b"in the first block");
    assert_eq!(reader.read(second).unwrap(), b"in the second");
}

#[test]
fn a_block_wants_closing_at_its_target_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = BodyLogWriter::open(&archive(&dir)).unwrap();
    writer.stage(&vec![b'a'; TARGET_BLOCK_BYTES - 1]).unwrap();
    assert!(!writer.wants_close());
    writer.stage(b"b").unwrap();
    assert!(writer.wants_close());
    writer.close_block().unwrap();
    assert!(!writer.wants_close());
}

#[test]
fn a_body_past_the_ceiling_is_refused_and_one_past_the_open_block_asks_for_a_close() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    assert!(matches!(
        writer.stage(&vec![0; MAX_BLOCK_RAW_BYTES + 1]),
        Err(ArchiveError::BodyTooLarge { .. })
    ));

    writer.stage(&vec![1; MAX_BLOCK_RAW_BYTES - 10]).unwrap();
    let body = vec![2; 11];
    assert!(matches!(writer.stage(&body), Err(ArchiveError::BlockFull)));
    writer.close_block().unwrap();
    let reference = writer.stage(&body).expect("an empty block holds it");
    writer.flush_segment().unwrap();
    assert_eq!(BodyLogReader::open(&path).unwrap().read(reference).unwrap(), body);
}

#[test]
fn an_empty_body_reads_back_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let empty = writer.stage(b"").unwrap();
    let after = writer.stage(b"after").unwrap();
    writer.flush_segment().unwrap();
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(empty).unwrap(), b"");
    assert_eq!(reader.read(after).unwrap(), b"after");
}

#[test]
fn a_partial_write_poisons_the_writer_and_does_not_advance_its_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let kept = writer.stage(b"committed before the failure").unwrap();
    writer.flush_segment().unwrap();
    let end = writer.end();

    writer.stage(b"lost to the torn write").unwrap();
    writer.fail_next_write_after(7);
    assert!(matches!(writer.flush_segment(), Err(ArchiveError::Io(_))));
    assert!(writer.is_poisoned());
    assert_eq!(
        writer.end(),
        end,
        "the end is not advanced past bytes it cannot vouch for"
    );
    assert_eq!(file_len(&path), end + 7, "the torn prefix really reached the file");
    assert!(matches!(writer.stage(b"x"), Err(ArchiveError::Poisoned)));
    assert!(matches!(writer.flush_segment(), Err(ArchiveError::Poisoned)));
    assert!(matches!(writer.close_block(), Err(ArchiveError::Poisoned)));
    drop(writer);

    assert_eq!(
        BodyLogReader::open(&path).unwrap().read(kept).unwrap(),
        b"committed before the failure"
    );
}

/// A reopened writer has no compressor state for the block it finds, and the
/// bytes after that block's last committed segment may be torn. It starts a
/// new block at the end of the file and leaves the old one as it is.
#[test]
fn a_reopened_writer_never_continues_an_old_block() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let old = writer.stage(b"from the first process").unwrap();
    writer.flush_segment().unwrap();
    drop(writer);
    let end = file_len(&path);

    let mut writer = BodyLogWriter::open(&path).unwrap();
    assert_eq!(writer.end(), end);
    let new = writer.stage(b"from the second").unwrap();
    assert_eq!(new.block_offset, end, "a new block, at the end of the file");
    assert_eq!(new.offset, 0);
    writer.flush_segment().unwrap();

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(old).unwrap(), b"from the first process");
    assert_eq!(reader.read(new).unwrap(), b"from the second");
}

#[test]
fn open_refuses_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere");
    std::fs::write(&target, b"not yours").unwrap();
    let link = archive(&dir);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(matches!(BodyLogWriter::open(&link), Err(ArchiveError::Symlink(_))));
    assert_eq!(std::fs::read(&target).unwrap(), b"not yours");
}

#[test]
fn open_creates_mode_0600_and_a_reopen_narrows_a_widened_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    drop(BodyLogWriter::open(&path).unwrap());
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    drop(BodyLogWriter::open(&path).unwrap());
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn open_refuses_a_version_one_archive_and_a_foreign_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    crate::tests::write_version_one_archive(&path);
    assert!(matches!(BodyLogWriter::open(&path), Err(ArchiveError::BadFileHeader)));
    std::fs::write(&path, b"some other file entirely").unwrap();
    assert!(matches!(BodyLogWriter::open(&path), Err(ArchiveError::BadFileHeader)));
}

#[test]
fn open_on_a_directory_is_an_io_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    assert!(BodyLogWriter::open(dir.path()).is_err());
}

#[test]
fn generation_creation_uses_its_typed_unique_name_and_private_descriptor() {
    use capsem_foundation::unix::contained::ContainedDir;
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let generations = root.path().join("session.bodies");
    std::fs::create_dir(&generations).unwrap();
    std::fs::set_permissions(&generations, std::fs::Permissions::from_mode(0o700)).unwrap();
    let archive_id = crate::ArchiveId::new_v4();
    let generation_id = crate::GenerationId::new_v4();
    let directory = ContainedDir::open_root(&generations).unwrap();
    let mut writer = BodyLogWriter::create_generation(&directory, archive_id, generation_id).unwrap();
    assert_eq!(
        writer.header(),
        FileHeader {
            archive_id,
            generation_id
        }
    );
    writer.sync().unwrap();
    directory.sync().unwrap();

    let path = generations.join(generation_id.file_name());
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(BodyLogWriter::create_generation(&directory, archive_id, generation_id).is_err());
    let reader = BodyLogReader::open_generation(
        &directory,
        FileHeader {
            archive_id,
            generation_id,
        },
        FILE_HEADER_BYTES as u64,
    )
    .unwrap();
    assert_eq!(
        reader.header(),
        FileHeader {
            archive_id,
            generation_id
        }
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "unflushed bodies")]
fn dropping_a_writer_with_unflushed_bodies_trips_the_debug_assert() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = BodyLogWriter::open(&archive(&dir)).unwrap();
    writer.stage(b"never flushed").unwrap();
    drop(writer);
}

#[test]
fn abandoning_a_writer_discards_its_unflushed_bodies_and_leaves_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let kept = writer.stage(b"flushed before").unwrap();
    writer.flush_segment().unwrap();
    let end = file_len(&path);
    writer.stage(b"never flushed").unwrap();
    writer.abandon();
    assert_eq!(file_len(&path), end, "nothing more is written");
    assert_eq!(
        BodyLogReader::open(&path).unwrap().read(kept).unwrap(),
        b"flushed before"
    );
}

#[test]
fn dropping_a_writer_whose_open_block_is_flushed_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let reference = writer.stage(b"flushed, block still open").unwrap();
    writer.flush_segment().unwrap();
    drop(writer);
    assert_eq!(
        BodyLogReader::open(&path).unwrap().read(reference).unwrap(),
        b"flushed, block still open"
    );
}

use std::collections::BTreeMap;

use super::*;
use crate::format::BodyRef;
use crate::tests::{archive, file_len, XorShift};
use crate::{BodyLogReader, BodyLogWriter, SegmentWritten};

/// Three blocks: two closed, the last still open after two segments. Returns
/// every body with the extent its block had committed.
fn three_blocks(path: &Path) -> (Vec<(BodyRef, Vec<u8>)>, Vec<SegmentWritten>) {
    let mut writer = BodyLogWriter::open(path).unwrap();
    let mut rng = XorShift(77);
    let mut bodies = Vec::new();
    let mut extents = Vec::new();
    for block in 0..3 {
        for _ in 0..2 {
            let body = rng.text(900);
            bodies.push((writer.stage(&body).unwrap(), body));
            let written = writer.flush_segment().unwrap().unwrap();
            if block == 2 {
                extents.retain(|e: &SegmentWritten| e.block_offset != written.block_offset);
                extents.push(written);
            }
        }
        if block < 2 {
            extents.push(writer.close_block().unwrap().unwrap());
        }
    }
    writer.sync().unwrap();
    (bodies, extents)
}

fn keep_of(extents: &[SegmentWritten]) -> Vec<(u64, u64)> {
    extents.iter().map(|e| (e.block_offset, e.disk_len)).collect()
}

fn retain(path: &Path, keep: &[(u64, u64)]) -> Result<BTreeMap<u64, u64>> {
    let staging = stage_retained_blocks(path, keep)?;
    let moved = staging.map().clone();
    commit_retained(staging)?;
    Ok(moved)
}

fn moved_ref(moved: &BTreeMap<u64, u64>, reference: BodyRef) -> BodyRef {
    BodyRef {
        block_offset: moved[&reference.block_offset],
        ..reference
    }
}

#[test]
fn kept_extents_move_up_verbatim_and_still_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (bodies, extents) = three_blocks(&path);
    let before = file_len(&path);

    let keep = keep_of(&extents[1..]);
    let moved = retain(&path, &keep).unwrap();
    assert_eq!(moved.len(), 2);
    assert_eq!(moved[&extents[1].block_offset], FILE_HEADER_BYTES as u64);
    assert_eq!(
        file_len(&path),
        FILE_HEADER_BYTES as u64 + extents[1].disk_len + extents[2].disk_len,
        "exactly the committed extents, nothing else"
    );
    assert_eq!(before - file_len(&path), extents[0].disk_len);

    let reader = BodyLogReader::open(&path).unwrap();
    for (reference, body) in &bodies[2..] {
        assert_eq!(&reader.read(moved_ref(&moved, *reference)).unwrap(), body);
    }
}

/// A block stranded open by a crash, with a torn segment after its last
/// committed one, is copied to its committed extent: the torn bytes go.
#[test]
fn a_stranded_block_is_copied_to_its_committed_extent_and_its_torn_tail_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let kept = writer.stage(b"committed").unwrap();
    let extent = writer.flush_segment().unwrap().unwrap();
    writer.stage(b"torn away").unwrap();
    writer.fail_next_write_after(30);
    assert!(writer.flush_segment().is_err());
    drop(writer);

    let moved = retain(&path, &[(extent.block_offset, extent.disk_len)]).unwrap();
    assert_eq!(file_len(&path), FILE_HEADER_BYTES as u64 + extent.disk_len);
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(moved_ref(&moved, kept)).unwrap(), b"committed");

    // And the compacted file takes new blocks after it.
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let next = writer.stage(b"after retention").unwrap();
    writer.close_block().unwrap();
    assert_eq!(
        BodyLogReader::open(&path).unwrap().read(next).unwrap(),
        b"after retention"
    );
}

/// An extent that does not end exactly on a segment boundary is refused, and
/// the original is left exactly as it was.
#[test]
fn an_extent_off_a_segment_boundary_is_refused_and_the_original_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (_, extents) = three_blocks(&path);
    let original = std::fs::read(&path).unwrap();
    let open = extents[2];
    let closed = extents[0];
    let first_segment_end = {
        // The open block's first segment: its extent before the second flush.
        let reader_len = BLOCK_HEADER_BYTES as u64 + SEGMENT_HEADER_BYTES as u64;
        (open.block_offset, reader_len)
    };
    for (offset, disk_len) in [
        (open.block_offset, open.disk_len - 1),
        (open.block_offset, open.disk_len + 1),
        (open.block_offset, 3),
        first_segment_end,
        (closed.block_offset, closed.disk_len + 60),
    ] {
        let error = stage_retained_blocks(&path, &[(offset, disk_len)]).expect_err("refused");
        assert!(
            matches!(error, ArchiveError::BadSegment(_) | ArchiveError::TruncatedBlock(_)),
            "({offset}, {disk_len}): {error:?}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "a refused staging removes its temporary");
}

#[test]
fn an_offset_that_is_not_a_block_or_an_extent_past_the_end_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (_, extents) = three_blocks(&path);
    assert!(matches!(
        stage_retained_blocks(&path, &[(extents[1].block_offset + 1, 10)]),
        Err(ArchiveError::BadBlockHeader(_))
    ));
    let open = extents[2];
    assert!(matches!(
        stage_retained_blocks(&path, &[(open.block_offset, open.disk_len + 500)]),
        Err(ArchiveError::TruncatedBlock(_) | ArchiveError::BadSegment(_))
    ));
}

#[test]
fn keeping_nothing_leaves_a_header_and_keeping_everything_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (_, extents) = three_blocks(&path);
    let original = std::fs::read(&path).unwrap();
    let moved = retain(&path, &keep_of(&extents)).unwrap();
    assert!(moved.iter().all(|(old, new)| old == new));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(retain(&path, &[]).unwrap().is_empty());
    assert_eq!(file_len(&path), FILE_HEADER_BYTES as u64);
}

#[test]
fn a_staging_that_is_dropped_leaves_the_original_and_no_temporary() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (_, extents) = three_blocks(&path);
    let original = std::fs::read(&path).unwrap();
    let staging = stage_retained_blocks(&path, &keep_of(&extents[2..])).unwrap();
    let temporary = staging.temporary_path().to_path_buf();
    assert!(temporary.exists());
    assert!(staging.bytes_freed() > 0);
    drop(staging);
    assert!(!temporary.exists());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn a_symlink_or_a_version_one_archive_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    three_blocks(&path);
    let link = dir.path().join("link.bodies");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(matches!(
        stage_retained_blocks(&link, &[]),
        Err(ArchiveError::Symlink(_))
    ));
    let old = dir.path().join("old.bodies");
    crate::tests::write_version_one_archive(&old);
    assert!(matches!(
        stage_retained_blocks(&old, &[]),
        Err(ArchiveError::BadFileHeader)
    ));
}

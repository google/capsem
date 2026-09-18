use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::*;
use crate::format::BodyRef;
use crate::{BodyLogReader, BodyLogWriter};

fn archive(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("session.bodies")
}

/// Write one body per block and return the placed reference for each, in the
/// order the bodies were given.
fn write_one_body_per_block(path: &Path, bodies: &[&[u8]]) -> Vec<BodyRef> {
    let mut writer = BodyLogWriter::open(path).unwrap();
    let mut placed = Vec::new();
    for body in bodies {
        let reference = writer.stage(body).unwrap();
        let sealed = writer.seal().unwrap().expect("a block was pending");
        placed.push(BodyRef {
            block_offset: sealed.block_offset,
            ..reference
        });
    }
    writer.sync().unwrap();
    placed
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap().len()
}

/// Stage and commit in one step, for the tests whose subject is what
/// retention keeps rather than when it becomes visible.
fn retain_blocks(path: &Path, keep: &[u64]) -> Result<BTreeMap<u64, u64>> {
    let staging = stage_retained_blocks(path, keep)?;
    let moved = staging.map().clone();
    commit_retained(staging)?;
    Ok(moved)
}

#[test]
fn keeping_a_later_block_moves_it_to_the_front_and_shrinks_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let placed = write_one_body_per_block(&path, &[b"first body", b"second body", b"third body"]);
    let before = file_len(&path);

    let moved = retain_blocks(&path, &[placed[2].block_offset]).unwrap();

    assert_eq!(moved.len(), 1, "only the kept block is remapped");
    let new_offset = moved[&placed[2].block_offset];
    assert_eq!(
        new_offset, FILE_HEADER_BYTES as u64,
        "the surviving block moves up behind the file header"
    );
    assert!(file_len(&path) < before, "the dropped blocks' bytes are reclaimed");

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: new_offset,
                ..placed[2]
            })
            .unwrap(),
        b"third body",
        "the kept body survives the move byte for byte"
    );
}

#[test]
fn keeping_nothing_leaves_a_header_only_archive() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    write_one_body_per_block(&path, &[b"first body", b"second body"]);

    let moved = retain_blocks(&path, &[]).unwrap();

    assert!(moved.is_empty());
    assert_eq!(
        file_len(&path),
        FILE_HEADER_BYTES as u64,
        "an archive with no kept blocks is its header and nothing else"
    );
    BodyLogReader::open(&path).expect("a header-only archive is still an archive");
}

#[test]
fn keeping_every_block_is_a_faithful_copy() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let placed = write_one_body_per_block(&path, &[b"alpha", b"beta", b"gamma"]);
    let before = std::fs::read(&path).unwrap();

    let offsets: Vec<u64> = placed.iter().map(|reference| reference.block_offset).collect();
    let moved = retain_blocks(&path, &offsets).unwrap();

    for offset in offsets {
        assert_eq!(moved[&offset], offset, "nothing dropped means nothing moves");
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a retain that keeps everything rewrites the same bytes, uncompressed anew"
    );
}

#[test]
fn an_unparseable_block_refuses_and_leaves_the_original_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let placed = write_one_body_per_block(&path, &[b"first body", b"second body"]);
    let corrupted = placed[1].block_offset;
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(corrupted)).unwrap();
    file.write_all(b"NOPE").unwrap();
    file.sync_all().unwrap();
    drop(file);
    let before = std::fs::read(&path).unwrap();

    let error = retain_blocks(&path, &[placed[0].block_offset, corrupted]).unwrap_err();

    assert!(
        matches!(error, ArchiveError::BadBlockHeader(offset) if offset == corrupted),
        "a block this code cannot parse is refused, not copied: {error}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a failed retain leaves the archive exactly as it was"
    );
    assert!(
        std::fs::read_dir(dir.path())
            .unwrap()
            .all(|entry| entry.unwrap().file_name() == "session.bodies"),
        "the temporary is removed on failure"
    );
}

#[test]
fn a_truncated_block_is_refused_rather_than_copied_short() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let placed = write_one_body_per_block(&path, &[b"first body", b"second body"]);
    let truncated = placed[1].block_offset;
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(file_len(&path) - 4).unwrap();
    file.sync_all().unwrap();
    drop(file);

    let error = retain_blocks(&path, &[truncated]).unwrap_err();

    assert!(
        matches!(error, ArchiveError::TruncatedBlock(offset) if offset == truncated),
        "a block whose payload the file does not hold is refused: {error}"
    );
}

#[test]
fn a_symlink_at_the_archive_path_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.bodies");
    write_one_body_per_block(&real, &[b"first body"]);
    let link = archive(&dir);
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let error = retain_blocks(&link, &[FILE_HEADER_BYTES as u64]).unwrap_err();

    assert!(
        matches!(error, ArchiveError::Symlink(ref path) if path == &link),
        "retention never writes through a link planted at the archive path: {error}"
    );
}

#[test]
fn a_file_that_is_not_an_archive_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    std::fs::write(&path, b"not an archive at all").unwrap();

    let error = retain_blocks(&path, &[]).unwrap_err();

    assert!(matches!(error, ArchiveError::BadFileHeader), "got {error}");
    assert_eq!(std::fs::read(&path).unwrap(), b"not an archive at all");
}

#[test]
fn a_retained_archive_reopens_and_accepts_new_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let placed = write_one_body_per_block(&path, &[b"first body", b"second body", b"third body"]);
    let moved = retain_blocks(&path, &[placed[1].block_offset, placed[2].block_offset]).unwrap();
    let kept = BodyRef {
        block_offset: moved[&placed[1].block_offset],
        ..placed[1]
    };

    let mut writer = BodyLogWriter::open(&path).expect("a retained archive is still appendable");
    assert_eq!(
        writer.end(),
        file_len(&path),
        "the writer appends after the retained end, not after the old one"
    );
    let staged = writer.stage(b"after retention").unwrap();
    let sealed = writer.seal().unwrap().expect("a block was pending");
    writer.sync().unwrap();
    assert!(
        sealed.block_offset >= moved[&placed[2].block_offset],
        "the new block lands past every kept one"
    );

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(kept).unwrap(), b"second body", "kept bodies still read");
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: sealed.block_offset,
                ..staged
            })
            .unwrap(),
        b"after retention",
        "and so do the ones written afterwards"
    );
}

#[test]
fn staging_leaves_the_original_in_place_until_it_is_committed() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let placed = write_one_body_per_block(&path, &[b"first body", b"second body"]);
    let before = std::fs::read(&path).unwrap();

    let staging = stage_retained_blocks(&path, &[placed[1].block_offset]).unwrap();

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the archive is still the old one while the replacement waits"
    );
    assert_eq!(
        staging.map()[&placed[1].block_offset],
        FILE_HEADER_BYTES as u64,
        "the map is known before anything is replaced, so the index can be written first"
    );
    assert!(staging.bytes_freed() > 0, "and so is what the commit will reclaim");
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader.read(placed[1]).unwrap(),
        b"second body",
        "every body still reads at its old offset"
    );

    commit_retained(staging).unwrap();

    assert!(file_len(&path) < before.len() as u64);
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: FILE_HEADER_BYTES as u64,
                ..placed[1]
            })
            .unwrap(),
        b"second body"
    );
}

#[test]
fn an_abandoned_staging_removes_itself_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let placed = write_one_body_per_block(&path, &[b"first body", b"second body"]);
    let before = std::fs::read(&path).unwrap();

    let temporary = {
        let staging = stage_retained_blocks(&path, &[placed[1].block_offset]).unwrap();
        staging.temporary_path().to_path_buf()
        // Dropped here: the caller decided not to go through with it.
    };

    assert!(!temporary.exists(), "the replacement is removed with the staging");
    assert_eq!(std::fs::read(&path).unwrap(), before, "and the archive is untouched");
}

#[test]
fn a_reader_open_before_a_commit_knows_its_file_was_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let placed = write_one_body_per_block(&path, &[b"first body", b"second body"]);
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(placed[0]).unwrap(), b"first body");
    assert!(!reader.file_was_replaced(), "nothing has happened yet");

    let staging = stage_retained_blocks(&path, &[placed[1].block_offset]).unwrap();
    assert!(
        !reader.file_was_replaced(),
        "staging alone does not replace the archive"
    );
    commit_retained(staging).unwrap();

    assert!(
        reader.file_was_replaced(),
        "after the rename this handle is on an inode that is no longer the archive"
    );
}

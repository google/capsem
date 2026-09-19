use super::*;
use crate::format::BLOCK_MAGIC;
use crate::writer::{BodyLogWriter, SealedBlock};
use crate::ArchiveError;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// One sealed block holding `bodies`, with the references already pointing at
/// it. Every reader test starts from a valid archive and then damages it.
fn write_block(path: &Path, bodies: &[&[u8]]) -> (SealedBlock, Vec<BodyRef>) {
    let mut writer = BodyLogWriter::open(path).unwrap();
    let staged: Vec<BodyRef> = bodies.iter().map(|body| writer.stage(body).unwrap()).collect();
    let sealed = writer.seal().unwrap().expect("bodies were staged");
    writer.sync().unwrap();
    let refs = staged
        .into_iter()
        .map(|reference| BodyRef {
            block_offset: sealed.block_offset,
            ..reference
        })
        .collect();
    (sealed, refs)
}

fn archive(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("session.bodies")
}

fn patch(path: &Path, at: u64, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(at)).unwrap();
    file.write_all(bytes).unwrap();
}

#[test]
fn reader_rejects_a_ref_past_the_block_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (sealed, refs) = write_block(&path, &[b"a short body"]);

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(refs[0]).unwrap(), b"a short body");
    let past = BodyRef {
        block_offset: sealed.block_offset,
        offset: 0,
        len: sealed.raw_len + 1,
    };
    assert!(matches!(reader.read(past), Err(ArchiveError::RefOutOfRange)));
}

#[test]
fn reader_rejects_a_ref_into_the_torn_tail() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (sealed, refs) = write_block(&path, &[b"the body that survived the crash"]);

    // A block header that the process died in the middle of writing. No index
    // row points at it, so it is unreachable bytes rather than corruption.
    let torn_at = {
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        let at = file.metadata().unwrap().len();
        file.write_all(BLOCK_MAGIC).unwrap();
        file.write_all(&[0u8; 8]).unwrap();
        file.sync_all().unwrap();
        at
    };

    let reader = BodyLogReader::open(&path).unwrap();
    let stray = BodyRef {
        block_offset: sealed.block_offset + 100,
        offset: 0,
        len: 1,
    };
    assert!(matches!(reader.read(stray), Err(ArchiveError::BadBlockHeader(_))));

    let into_torn = BodyRef {
        block_offset: torn_at,
        offset: 0,
        len: 1,
    };
    assert!(matches!(
        reader.read(into_torn),
        Err(ArchiveError::BadBlockHeader(_) | ArchiveError::Integrity(_))
    ));

    assert_eq!(
        reader.read(refs[0]).unwrap(),
        b"the body that survived the crash",
        "the torn tail does not cost the sealed blocks"
    );
}

#[test]
fn reader_caches_the_last_block() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (_, refs) = write_block(&path, &[b"first", b"second"]);

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(refs[0]).unwrap(), b"first");
    assert_eq!(reader.read(refs[1]).unwrap(), b"second");
    assert_eq!(reader.blocks_inflated(), 1, "one block, inflated once");
}

#[test]
fn reader_rejects_a_block_header_claiming_an_absurd_raw_len() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (sealed, refs) = write_block(&path, &[b"a body whose header gets forged"]);

    patch(&path, sealed.block_offset + 4, &u32::MAX.to_le_bytes());
    let reader = BodyLogReader::open(&path).unwrap();
    assert!(matches!(reader.read(refs[0]), Err(ArchiveError::BadBlockHeader(_))));

    // Forging the compressed length too proves the refusal happens in the
    // header, before a 4 GiB payload buffer could be allocated.
    patch(&path, sealed.block_offset + 8, &u32::MAX.to_le_bytes());
    let reader = BodyLogReader::open(&path).unwrap();
    assert!(matches!(reader.read(refs[0]), Err(ArchiveError::BadBlockHeader(_))));
}

/// The reader verifies before it returns: a block whose stored hash no longer
/// describes its payload yields an error, never the bytes.
#[test]
fn reader_rejects_a_block_whose_hash_no_longer_matches() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (sealed, refs) = write_block(&path, &[b"a body someone edited in place"]);

    // An all-zero digest is one no input produces, so this is a mismatch
    // without depending on what the real digest happens to be.
    patch(&path, sealed.block_offset + 12, &[0u8; 32]);
    let reader = BodyLogReader::open(&path).unwrap();
    assert!(matches!(reader.read(refs[0]), Err(ArchiveError::Integrity(_))));
}

#[test]
fn reader_rejects_a_truncated_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (_, refs) = write_block(&path, &[b"a body long enough to lose its tail".repeat(40).as_slice()]);

    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let len = file.metadata().unwrap().len();
    file.set_len(len - 10).unwrap();
    drop(file);

    let reader = BodyLogReader::open(&path).unwrap();
    assert!(reader.read(refs[0]).is_err(), "never bytes from a short file");
}

#[test]
fn reader_rejects_a_ref_whose_len_overflows() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (sealed, _) = write_block(&path, &[b"a body of an ordinary size"]);

    let reader = BodyLogReader::open(&path).unwrap();
    let overflowing = BodyRef {
        block_offset: sealed.block_offset,
        offset: u32::MAX,
        len: u32::MAX,
    };
    assert!(matches!(reader.read(overflowing), Err(ArchiveError::RefOutOfRange)));
}

/// Two reads of *different* blocks from one reader: the second replaces the
/// cached block while the first read is long finished. `read` returns owned
/// bytes precisely so this cannot become a borrow panic.
#[test]
fn two_reads_of_different_blocks_do_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let first = writer.stage(b"a body in the first block").unwrap();
    let first_block = writer.seal().unwrap().unwrap();
    let second = writer.stage(b"a body in the second block").unwrap();
    let second_block = writer.seal().unwrap().unwrap();
    writer.sync().unwrap();

    let reader = BodyLogReader::open(&path).unwrap();
    let first = BodyRef {
        block_offset: first_block.block_offset,
        ..first
    };
    let second = BodyRef {
        block_offset: second_block.block_offset,
        ..second
    };
    assert_eq!(reader.read(first).unwrap(), b"a body in the first block");
    assert_eq!(reader.read(second).unwrap(), b"a body in the second block");
    assert_eq!(reader.read(first).unwrap(), b"a body in the first block");
    assert_eq!(reader.blocks_inflated(), 3, "alternating reads miss the cache");
}

/// Integrity is per block, not per body. A reference that names a valid but
/// wrong span of a valid block returns those bytes, with no error -- the
/// index row is the claim about which bytes are which, and SQLite is what
/// protects it. This is documented behavior, locked in so a later change
/// cannot quietly promise more than the format delivers.
#[test]
fn reader_returns_the_wrong_body_for_a_wrong_but_in_range_ref() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (sealed, refs) = write_block(&path, &[b"first body", b"second body"]);

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(refs[0]).unwrap(), b"first body");

    let shifted = BodyRef {
        block_offset: sealed.block_offset,
        offset: refs[0].offset + 1,
        len: refs[0].len,
    };
    assert_eq!(
        reader.read(shifted).unwrap(),
        b"irst bodys",
        "a valid span of a valid block, and not the body the row named"
    );
}

#[test]
fn reader_rejects_a_block_whose_payload_is_cut_short() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let (sealed, refs) = write_block(&path, &[b"a body long enough to have a tail".repeat(40).as_slice()]);

    // The header survives, the payload does not: that is TruncatedBlock, and
    // not the BadBlockHeader a short header would give.
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let len = file.metadata().unwrap().len();
    file.set_len(len - 5).unwrap();
    drop(file);

    let reader = BodyLogReader::open(&path).unwrap();
    assert!(matches!(
        reader.read(refs[0]),
        Err(ArchiveError::TruncatedBlock(offset)) if offset == sealed.block_offset
    ));
}

#[test]
fn reader_open_refuses_an_empty_file_a_missing_path_and_a_directory() {
    let dir = tempfile::tempdir().unwrap();

    let empty = dir.path().join("empty.bodies");
    std::fs::write(&empty, b"").unwrap();
    assert!(matches!(BodyLogReader::open(&empty), Err(ArchiveError::BadFileHeader)));

    let missing = dir.path().join("not-here.bodies");
    assert!(matches!(BodyLogReader::open(&missing), Err(ArchiveError::Io(_))));

    let Err(error) = BodyLogReader::open(dir.path()) else {
        panic!("a directory is not an archive");
    };
    assert!(matches!(error, ArchiveError::Io(_)), "unexpected error: {error:?}");
}

#[cfg(unix)]
#[test]
fn reader_open_refuses_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("real.bodies");
    write_block(&target, &[b"bytes behind the link"]);
    let path = archive(&dir);
    std::os::unix::fs::symlink(&target, &path).unwrap();

    assert!(matches!(BodyLogReader::open(&path), Err(ArchiveError::Symlink(_))));
}

#[test]
fn reader_refuses_a_file_that_is_not_an_archive() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    std::fs::write(&path, b"some other file entirely, sixteen+").unwrap();
    assert!(matches!(BodyLogReader::open(&path), Err(ArchiveError::BadFileHeader)));
}

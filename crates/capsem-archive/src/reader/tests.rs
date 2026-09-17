use super::*;
use crate::format::BLOCK_MAGIC;
use crate::{ArchiveError, BodyLogWriter, SealedBlock};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// One sealed block holding `bodies`, with the references already pointing at
/// it. Every reader test starts from a valid archive and then damages it.
fn write_block(path: &Path, bodies: &[&[u8]]) -> (SealedBlock, Vec<BodyRef>) {
    let mut writer = BodyLogWriter::open(path).unwrap();
    let staged: Vec<BodyRef> = bodies.iter().map(|body| writer.stage(body)).collect();
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

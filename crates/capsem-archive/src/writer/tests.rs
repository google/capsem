use super::*;
use crate::format::TARGET_BLOCK_BYTES;
use crate::{ArchiveError, BodyLogReader};
use std::path::PathBuf;

fn archive(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("session.bodies")
}

/// Deterministic pseudo-random bytes: the same corpus on every machine, so a
/// failure is reproducible from the test name alone.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next() & 0xff) as u8).collect()
    }
}

#[test]
fn stage_then_seal_returns_refs_the_reader_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let first = writer.stage(b"first body");
    let second = writer.stage(b"second body, longer");
    assert_eq!(first.offset, 0);
    assert_eq!(first.len, 10);
    assert_eq!(second.offset, 10);
    assert_eq!(second.len, 19);
    assert_eq!(writer.pending_bytes(), 29);

    let sealed = writer.seal().unwrap().expect("a block was pending");
    assert_eq!(sealed.block_offset, FILE_HEADER_BYTES as u64);
    assert_eq!(sealed.raw_len, 29);
    assert_eq!(writer.pending_bytes(), 0);
    assert!(writer.seal().unwrap().is_none(), "nothing left to seal");

    let reader = BodyLogReader::open(&path).unwrap();
    let at = |r: BodyRef| BodyRef {
        block_offset: sealed.block_offset,
        ..r
    };
    assert_eq!(reader.read(at(first)).unwrap(), b"first body");
    assert_eq!(reader.read(at(second)).unwrap(), b"second body, longer");
}

#[test]
fn a_full_block_seals_itself_and_the_next_stage_starts_a_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let big = vec![b'z'; TARGET_BLOCK_BYTES];
    let first = writer.stage(&big);
    assert!(writer.wants_seal());
    let first_block = writer.seal().unwrap().expect("full block seals");
    assert!(!writer.wants_seal());

    let second = writer.stage(b"after the boundary");
    assert_eq!(second.offset, 0, "a fresh block starts at zero");
    let second_block = writer.seal().unwrap().expect("second block seals");
    assert!(second_block.block_offset > first_block.block_offset);

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: first_block.block_offset,
                ..first
            })
            .unwrap(),
        big
    );
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: second_block.block_offset,
                ..second
            })
            .unwrap(),
        b"after the boundary"
    );
}

#[test]
fn reopening_appends_after_existing_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);

    let (first_block, first) = {
        let mut writer = BodyLogWriter::open(&path).unwrap();
        let body = writer.stage(b"written before the restart");
        let sealed = writer.seal().unwrap().unwrap();
        writer.sync().unwrap();
        (sealed, body)
    };

    let mut writer = BodyLogWriter::open(&path).unwrap();
    let second = writer.stage(b"written after the restart");
    let second_block = writer.seal().unwrap().unwrap();
    assert!(second_block.block_offset > first_block.block_offset);

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: first_block.block_offset,
                ..first
            })
            .unwrap(),
        b"written before the restart"
    );
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: second_block.block_offset,
                ..second
            })
            .unwrap(),
        b"written after the restart"
    );
}

#[test]
fn two_phase_seal_matches_seal() {
    let dir = tempfile::tempdir().unwrap();
    let split_path = dir.path().join("split.bodies");
    let sealed_path = dir.path().join("sealed.bodies");

    let mut split = BodyLogWriter::open(&split_path).unwrap();
    let first = split.stage(b"one body in the first block");
    let pending = split.take_pending().expect("a block was pending");
    assert_eq!(pending.raw_len(), 27);
    let second = split.stage(b"staged while the first block was in flight");
    assert_eq!(second.offset, 0, "take_pending starts a fresh block");
    let first_block = split.append(pending.encode()).unwrap();
    let second_block = split.seal().unwrap().unwrap();
    split.sync().unwrap();

    let mut plain = BodyLogWriter::open(&sealed_path).unwrap();
    plain.stage(b"one body in the first block");
    plain.seal().unwrap().unwrap();
    plain.stage(b"staged while the first block was in flight");
    plain.seal().unwrap().unwrap();
    plain.sync().unwrap();

    assert_eq!(
        std::fs::read(&split_path).unwrap(),
        std::fs::read(&sealed_path).unwrap(),
        "two-phase seal writes the same bytes as seal"
    );

    let reader = BodyLogReader::open(&split_path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: first_block.block_offset,
                ..first
            })
            .unwrap(),
        b"one body in the first block"
    );
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: second_block.block_offset,
                ..second
            })
            .unwrap(),
        b"staged while the first block was in flight"
    );
}

#[test]
fn many_random_bodies_round_trip_across_block_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let mut rng = XorShift(0x5eed_1234_9876_abcd);

    let mut pending: Vec<(BodyRef, Vec<u8>)> = Vec::new();
    let mut resolved: Vec<(BodyRef, Vec<u8>)> = Vec::new();

    for _ in 0..2000 {
        let len = rng.below(40 * 1024);
        let body = rng.bytes(len);
        let reference = writer.stage(&body);
        pending.push((reference, body));

        if writer.wants_seal() || rng.below(50) == 0 {
            let sealed = writer.seal().unwrap().expect("bodies were pending");
            resolved.extend(pending.drain(..).map(|(reference, body)| {
                (
                    BodyRef {
                        block_offset: sealed.block_offset,
                        ..reference
                    },
                    body,
                )
            }));
        }
    }
    if let Some(sealed) = writer.seal().unwrap() {
        resolved.extend(pending.drain(..).map(|(reference, body)| {
            (
                BodyRef {
                    block_offset: sealed.block_offset,
                    ..reference
                },
                body,
            )
        }));
    }
    assert!(pending.is_empty());
    writer.sync().unwrap();

    let reader = BodyLogReader::open(&path).unwrap();
    for (reference, body) in &resolved {
        assert_eq!(&reader.read(*reference).unwrap(), body);
    }
    assert_eq!(resolved.len(), 2000);
}

#[test]
fn a_body_larger_than_the_target_block_is_its_own_block() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let huge = vec![b'q'; 3 * TARGET_BLOCK_BYTES];
    let reference = writer.stage(&huge);
    assert!(writer.wants_seal());
    let sealed = writer.seal().unwrap().unwrap();
    assert_eq!(sealed.raw_len as usize, huge.len());

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: sealed.block_offset,
                ..reference
            })
            .unwrap(),
        huge
    );
}

#[test]
fn reader_sees_a_block_the_writer_sealed_after_the_reader_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let reader = BodyLogReader::open(&path).unwrap();

    let reference = writer.stage(b"sealed after the reader opened");
    let sealed = writer.seal().unwrap().unwrap();
    writer.sync().unwrap();

    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: sealed.block_offset,
                ..reference
            })
            .unwrap(),
        b"sealed after the reader opened"
    );
}

#[test]
fn open_refuses_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere.bodies");
    std::fs::write(&target, b"not ours").unwrap();
    let path = archive(&dir);
    std::os::unix::fs::symlink(&target, &path).unwrap();

    assert!(matches!(BodyLogWriter::open(&path), Err(ArchiveError::Symlink(_))));
}

#[test]
fn open_creates_the_file_mode_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    BodyLogWriter::open(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "bodies are readable by their owner only");
}

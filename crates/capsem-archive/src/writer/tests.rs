use super::*;
use crate::format::{MAX_BLOCK_RAW_BYTES, TARGET_BLOCK_BYTES};
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

    let first = writer.stage(b"first body").unwrap();
    let second = writer.stage(b"second body, longer").unwrap();
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
    let first = writer.stage(&big).unwrap();
    assert!(writer.wants_seal());
    let first_block = writer.seal().unwrap().expect("full block seals");
    assert!(!writer.wants_seal());

    let second = writer.stage(b"after the boundary").unwrap();
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
        let body = writer.stage(b"written before the restart").unwrap();
        let sealed = writer.seal().unwrap().unwrap();
        writer.sync().unwrap();
        (sealed, body)
    };

    let mut writer = BodyLogWriter::open(&path).unwrap();
    let second = writer.stage(b"written after the restart").unwrap();
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
    let first = split.stage(b"one body in the first block").unwrap();
    let pending = split.take_pending().expect("a block was pending");
    assert_eq!(pending.raw_len(), 27);
    let second = split.stage(b"staged while the first block was in flight").unwrap();
    assert_eq!(second.offset, 0, "take_pending starts a fresh block");
    let first_block = split.append(pending.encode()).unwrap();
    let second_block = split.seal().unwrap().unwrap();
    split.sync().unwrap();

    let mut plain = BodyLogWriter::open(&sealed_path).unwrap();
    plain.stage(b"one body in the first block").unwrap();
    plain.seal().unwrap().unwrap();
    plain.stage(b"staged while the first block was in flight").unwrap();
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
        let reference = writer.stage(&body).unwrap();
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
    let reference = writer.stage(&huge).unwrap();
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

    let reference = writer.stage(b"sealed after the reader opened").unwrap();
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

#[cfg(unix)]
#[test]
fn open_refuses_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere.bodies");
    std::fs::write(&target, b"not ours").unwrap();
    let path = archive(&dir);
    std::os::unix::fs::symlink(&target, &path).unwrap();

    assert!(matches!(BodyLogWriter::open(&path), Err(ArchiveError::Symlink(_))));
}

#[cfg(unix)]
#[test]
fn open_creates_the_file_mode_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    BodyLogWriter::open(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "bodies are readable by their owner only");
}

/// The mode is maintained, not just chosen once: an archive whose mode was
/// widened -- by an older build, or by anything else -- is narrowed again the
/// next time a session opens it.
#[cfg(unix)]
#[test]
fn reopen_narrows_a_widened_mode_back_to_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    writer.stage(b"a body from the first session").unwrap();
    writer.seal().unwrap().unwrap();
    drop(writer);

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let _reopened = BodyLogWriter::open(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_partial_write_poisons_the_writer_and_does_not_advance_its_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let good = writer.stage(b"a body whose block lands whole").unwrap();
    let sealed = writer.seal().unwrap().unwrap();
    let end_before = writer.end();
    assert!(!writer.is_poisoned());

    // The next block reaches the file half-written, exactly as it would if
    // the device filled up part-way through the append.
    writer.stage(b"a body whose block dies in flight").unwrap();
    writer.fail_next_write_after(20);
    let error = writer.seal().expect_err("a failed write is reported");
    assert!(matches!(error, ArchiveError::Io(_)), "unexpected error: {error:?}");
    assert!(writer.is_poisoned());
    assert_eq!(
        writer.end(),
        end_before,
        "the end is not advanced past a block that only partly landed"
    );

    // Everything afterwards refuses, rather than handing out offsets that
    // are no longer provable.
    assert!(matches!(writer.stage(b"after"), Err(ArchiveError::Poisoned)));
    assert!(matches!(writer.seal(), Err(ArchiveError::Poisoned)));
    assert!(writer.take_pending().is_none());

    // The block that did land is still readable; the torn tail is not.
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: sealed.block_offset,
                ..good
            })
            .unwrap(),
        b"a body whose block lands whole"
    );
    assert!(reader
        .read(BodyRef {
            block_offset: end_before,
            offset: 0,
            len: 1,
        })
        .is_err());
}

#[test]
fn a_body_larger_than_the_block_ceiling_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let huge = vec![b'x'; 20 * 1024 * 1024];
    let error = writer.stage(&huge).expect_err("20 MiB exceeds the ceiling");
    assert!(
        matches!(
            error,
            ArchiveError::BodyTooLarge { len, max } if len == huge.len() && max == MAX_BLOCK_RAW_BYTES
        ),
        "unexpected error: {error:?}"
    );
    assert_eq!(writer.pending_bytes(), 0, "a refused body stages nothing");
}

#[test]
fn a_body_that_would_cross_the_ceiling_asks_for_a_seal_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let ten_mib = vec![b'a'; 10 * 1024 * 1024];
    writer.stage(&ten_mib).unwrap();
    let second = vec![b'b'; 10 * 1024 * 1024];
    assert!(
        matches!(writer.stage(&second), Err(ArchiveError::BlockFull)),
        "two 10 MiB bodies cannot share a 16 MiB block"
    );
    assert_eq!(
        writer.pending_bytes(),
        ten_mib.len(),
        "the refused body did not join the pending block"
    );

    let first_block = writer.seal().unwrap().unwrap();
    let reference = writer.stage(&second).expect("it fits an empty block");
    let second_block = writer.seal().unwrap().unwrap();
    assert!(second_block.block_offset > first_block.block_offset);

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: second_block.block_offset,
                ..reference
            })
            .unwrap(),
        second
    );
}

#[test]
fn append_refuses_a_block_that_is_not_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    writer.stage(b"the first block").unwrap();
    let first = writer.take_pending().unwrap();
    writer.stage(b"the second block").unwrap();
    let second = writer.take_pending().unwrap();
    assert_eq!((first.seq(), second.seq()), (0, 1));

    let encoded = second.encode();
    assert_eq!(encoded.seq(), 1, "encoding carries the order with it");
    let error = writer.append(encoded).expect_err("block 1 cannot land before block 0");
    assert!(
        matches!(error, ArchiveError::OutOfOrderBlock { expected: 0, got: 1 }),
        "unexpected error: {error:?}"
    );
    assert!(!writer.is_poisoned(), "a rejected append is not a failed write");

    // In order, both land.
    writer.append(first.encode()).unwrap();
    writer.stage(b"the second block").unwrap();
    let again = writer.take_pending().unwrap();
    assert_eq!(again.seq(), 2, "the refused block consumed its number");
    assert!(matches!(
        writer.append(again.encode()),
        Err(ArchiveError::OutOfOrderBlock { expected: 1, got: 2 })
    ));
}

#[test]
fn append_refuses_a_block_encoded_by_a_different_writer() {
    let dir = tempfile::tempdir().unwrap();
    let mut mine = BodyLogWriter::open(&dir.path().join("mine.bodies")).unwrap();
    let mut theirs = BodyLogWriter::open(&dir.path().join("theirs.bodies")).unwrap();

    mine.stage(b"my first block").unwrap();
    mine.seal().unwrap().unwrap();

    theirs.stage(b"their first block").unwrap();
    let foreign = theirs.take_pending().unwrap();
    assert!(
        matches!(
            mine.append(foreign.encode()),
            Err(ArchiveError::OutOfOrderBlock { expected: 1, got: 0 })
        ),
        "another writer's sequence space is not this one's"
    );
    theirs.seal().unwrap();
}

#[test]
fn an_empty_body_reads_back_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let empty = writer.stage(b"").unwrap();
    let after = writer.stage(b"something").unwrap();
    assert_eq!((empty.offset, empty.len), (0, 0));
    assert_eq!(after.offset, 0, "an empty body takes no space");
    let sealed = writer.seal().unwrap().unwrap();

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: sealed.block_offset,
                ..empty
            })
            .unwrap(),
        Vec::<u8>::new()
    );
}

#[test]
fn two_bodies_straddle_the_target_boundary_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();

    let first = vec![b'1'; TARGET_BLOCK_BYTES - 1];
    let first_ref = writer.stage(&first).unwrap();
    assert!(!writer.wants_seal(), "one byte short of the target");

    let second_ref = writer.stage(b"2").unwrap();
    assert!(writer.wants_seal(), "the target is reached exactly");
    assert_eq!(writer.pending_bytes(), TARGET_BLOCK_BYTES);

    let sealed = writer.seal().unwrap().unwrap();
    assert_eq!(sealed.raw_len as usize, TARGET_BLOCK_BYTES);

    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: sealed.block_offset,
                ..first_ref
            })
            .unwrap(),
        first
    );
    assert_eq!(
        reader
            .read(BodyRef {
                block_offset: sealed.block_offset,
                ..second_ref
            })
            .unwrap(),
        b"2"
    );
}

/// Dropping unsealed bodies loses them silently, which is exactly the kind of
/// mistake a debug build should shout about. Release builds compile the
/// assertion out, so this test only exists where it does.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "seal before dropping")]
fn dropping_a_writer_with_unsealed_bodies_trips_the_debug_assert() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = BodyLogWriter::open(&archive(&dir)).unwrap();
    writer.stage(b"a body nobody sealed").unwrap();
    drop(writer);
}

#[test]
fn open_on_a_directory_is_an_io_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let Err(error) = BodyLogWriter::open(dir.path()) else {
        panic!("a directory is not an archive");
    };
    assert!(matches!(error, ArchiveError::Io(_)), "unexpected error: {error:?}");
}

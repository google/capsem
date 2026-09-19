//! A crash can land at any byte. Whatever it leaves, three things hold: a
//! body whose segment was completely written reads back exactly; a body whose
//! segment was not errors -- never wrong bytes, never a panic; and a writer
//! reopened on the torn file starts after the tear and works.

use super::{archive, file_len, XorShift};
use crate::format::BodyRef;
use crate::{BodyLogReader, BodyLogWriter};

/// Bodies of a small session with a flush after each, and the file length
/// after each flush: the committed extent at that moment.
struct Session {
    bodies: Vec<(BodyRef, Vec<u8>)>,
    /// File length once body `i`'s segment was written.
    committed_at: Vec<u64>,
}

fn write_session(path: &std::path::Path, close_last: bool) -> Session {
    let mut writer = BodyLogWriter::open(path).unwrap();
    let mut rng = XorShift(0xc0ffee);
    let mut session = Session {
        bodies: Vec::new(),
        committed_at: Vec::new(),
    };
    for index in 0..4usize {
        let body = if index.is_multiple_of(2) {
            rng.text(300)
        } else {
            rng.bytes(120)
        };
        session.bodies.push((writer.stage(&body).unwrap(), body));
        if close_last && index == 3 {
            writer.close_block().unwrap();
        } else {
            writer.flush_segment().unwrap();
        }
        session.committed_at.push(file_len(path));
    }
    session
}

/// Check a torn copy of length `cut`: every body committed at or before `cut`
/// reads exactly; every other one errors.
fn check_cut(original: &[u8], session: &Session, cut: usize) {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    std::fs::write(&path, &original[..cut]).unwrap();
    let Ok(reader) = BodyLogReader::open(&path) else {
        assert!(cut < 16, "only a cut inside the file header may refuse the file");
        return;
    };
    for (index, (reference, body)) in session.bodies.iter().enumerate() {
        let committed = session.committed_at[index] <= cut as u64;
        // A fresh reader per body as well as the shared one: the shared
        // cursor exercises continuing after a failed read.
        for reader in [&reader, &BodyLogReader::open(&path).unwrap()] {
            match reader.read(*reference) {
                Ok(bytes) => {
                    assert!(committed, "cut {cut}: body {index} read from an uncommitted segment");
                    assert_eq!(&bytes, body, "cut {cut}: wrong bytes for body {index}");
                }
                Err(error) => assert!(!committed, "cut {cut}: committed body {index} failed: {error}"),
            }
        }
    }

    // The reopened writer appends after the tear and its bodies read.
    let mut writer = BodyLogWriter::open(&path).unwrap();
    assert_eq!(writer.end(), cut as u64);
    let reference = writer.stage(b"after the crash").unwrap();
    assert_eq!(reference.block_offset, cut as u64);
    writer.flush_segment().unwrap();
    assert_eq!(
        BodyLogReader::open(&path).unwrap().read(reference).unwrap(),
        b"after the crash"
    );
}

/// Every byte of the file is a place a crash could have stopped it.
#[test]
fn a_crash_at_every_byte_leaves_committed_bodies_readable_and_nothing_else() {
    for close_last in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = archive(&dir);
        let session = write_session(&path, close_last);
        let original = std::fs::read(&path).unwrap();
        for cut in 16..=original.len() {
            check_cut(&original, &session, cut);
        }
    }
}

/// The torn write the writer itself produces, at every byte of the segment in
/// flight: poisoned writer, committed bodies intact, reopened writer fine.
#[test]
fn a_torn_segment_write_at_every_byte_poisons_and_strands_nothing_committed() {
    let mut rng = XorShift(0xdead);
    let first = rng.text(500);
    let second = rng.text(700);
    let segment_len = {
        let dir = tempfile::tempdir().unwrap();
        let path = archive(&dir);
        let mut writer = BodyLogWriter::open(&path).unwrap();
        writer.stage(&first).unwrap();
        writer.flush_segment().unwrap();
        let before = file_len(&path);
        writer.stage(&second).unwrap();
        writer.flush_segment().unwrap();
        file_len(&path) - before
    };
    for torn_after in 0..segment_len as usize {
        let dir = tempfile::tempdir().unwrap();
        let path = archive(&dir);
        let mut writer = BodyLogWriter::open(&path).unwrap();
        let kept = writer.stage(&first).unwrap();
        writer.flush_segment().unwrap();
        let lost = writer.stage(&second).unwrap();
        writer.fail_next_write_after(torn_after);
        assert!(writer.flush_segment().is_err());
        assert!(writer.is_poisoned());
        drop(writer);

        let reader = BodyLogReader::open(&path).unwrap();
        assert_eq!(reader.read(kept).unwrap(), first, "torn after {torn_after}");
        assert!(
            reader.read(lost).is_err(),
            "torn after {torn_after}: a torn segment read"
        );

        let mut writer = BodyLogWriter::open(&path).unwrap();
        let next = writer.stage(b"next process").unwrap();
        writer.close_block().unwrap();
        let reader = BodyLogReader::open(&path).unwrap();
        assert_eq!(reader.read(next).unwrap(), b"next process");
        assert_eq!(reader.read(kept).unwrap(), first);
    }
}

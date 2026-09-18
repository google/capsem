//! A reader on another thread, with its own file handle, while the writer
//! keeps appending segments to the same open block -- the service reading a
//! ledger `capsem-process` is still writing.
//!
//! The writer hands a reference over only after its segment is written and
//! synced, which is what the logger's commit ordering guarantees, and then
//! races on to the next segments without waiting. The reader must read each
//! reference, continuing its cursor rather than starting the block again, and
//! stop at the segment it was told about however far the file has grown.

use std::sync::mpsc;

use super::{archive, XorShift};
use crate::v2::format::BodyRef;
use crate::v2::{BodyLogReader, BodyLogWriter};

#[test]
fn a_reader_beside_the_writer_continues_its_cursor_one_segment_per_commit() {
    const ROUNDS: usize = 200;
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let (committed, commits) = mpsc::channel::<(BodyRef, Vec<u8>)>();

    let reader = std::thread::spawn(move || {
        // Opened before any block exists: it must see what arrives later.
        let reader = BodyLogReader::open(&path).unwrap();
        let mut seen = 0u64;
        for (reference, body) in commits {
            assert_eq!(reader.read(reference).unwrap(), body, "body {seen}");
            seen += 1;
            assert_eq!(
                reader.segments_inflated(),
                seen,
                "each commit costs the reader exactly one new segment"
            );
            assert_eq!(reader.blocks_inflated(), 1, "the open block is never restarted");
        }
        seen
    });

    let mut rng = XorShift(0x0b5e55ed);
    for round in 0..ROUNDS {
        let body = rng.text(1000 + round * 7);
        let reference = writer.stage(&body).unwrap();
        writer.flush_segment().unwrap();
        writer.sync().unwrap();
        committed.send((reference, body)).unwrap();
    }
    assert!(!writer.wants_close(), "the test must stay inside one open block");
    drop(committed);
    assert_eq!(reader.join().unwrap(), ROUNDS as u64);
    writer.close_block().unwrap();
}

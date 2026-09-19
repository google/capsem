//! Properties of the stream the format rests on.
//!
//! **Self-sufficiency.** A sync flush must leave everything staged so far
//! decodable from the bytes written so far. If it did not, a body would read
//! only once some later segment arrived -- or never, on a session that ended
//! first -- and the index would have committed a row the file cannot answer.
//!
//! **Ratio.** Keeping the block open is only worth it if flushing it every
//! few seconds costs little against compressing it in one go.

use flate2::{Compress, Compression, FlushCompress};

use super::{archive, file_len, XorShift};
use crate::format::{BodyRef, FILE_HEADER_BYTES, MAX_BLOCK_RAW_BYTES};
use crate::{BodyLogReader, BodyLogWriter};

/// Random bodies -- text, incompressible, empty -- with random flushes and
/// closes. After every flush, a copy of the file cut at the writer's end must
/// answer every reference handed out so far: nothing may depend on bytes not
/// yet written.
#[test]
fn every_flush_leaves_everything_so_far_readable_from_the_bytes_so_far() {
    for seed in [1u64, 2, 3, 0xfeed] {
        let dir = tempfile::tempdir().unwrap();
        let path = archive(&dir);
        let mut writer = BodyLogWriter::open(&path).unwrap();
        let mut rng = XorShift(seed);
        let mut flushed: Vec<(BodyRef, Vec<u8>)> = Vec::new();
        let mut staged: Vec<(BodyRef, Vec<u8>)> = Vec::new();
        for _ in 0..300 {
            let body = match rng.below(6) {
                0 => {
                    let len = rng.below(20_000);
                    rng.bytes(len)
                }
                1 => Vec::new(),
                _ => {
                    let len = rng.below(8_000);
                    rng.text(len)
                }
            };
            staged.push((writer.stage(&body).unwrap(), body));
            let written = match rng.below(10) {
                0 => writer.close_block().unwrap(),
                1..=3 => writer.flush_segment().unwrap(),
                _ if writer.wants_close() => writer.close_block().unwrap(),
                _ => None,
            };
            if written.is_some() {
                flushed.append(&mut staged);
                let snapshot = dir.path().join("snapshot.bodies");
                std::fs::copy(&path, &snapshot).unwrap();
                assert_eq!(file_len(&snapshot), writer.end());
                let reader = BodyLogReader::open(&snapshot).unwrap();
                for (reference, body) in &flushed {
                    assert_eq!(&reader.read(*reference).unwrap(), body, "seed {seed}");
                }
            }
        }
        writer.close_block().unwrap();
        flushed.append(&mut staged);
        // And in a scrambled order, which restarts blocks.
        let reader = BodyLogReader::open(&path).unwrap();
        for _ in 0..flushed.len() {
            let (reference, body) = &flushed[rng.below(flushed.len())];
            assert_eq!(&reader.read(*reference).unwrap(), body, "seed {seed}");
        }
    }
}

#[test]
fn a_ten_mebibyte_incompressible_body_round_trips_in_one_segment() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let body = XorShift(10).bytes(10 * 1024 * 1024);
    assert!(body.len() < MAX_BLOCK_RAW_BYTES);
    let reference = writer.stage(&body).unwrap();
    writer.flush_segment().unwrap();
    let tail = writer.stage(b"and a small one after it").unwrap();
    writer.close_block().unwrap();
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(reference).unwrap(), body);
    assert_eq!(reader.read(tail).unwrap(), b"and a small one after it");
}

/// A segment compresses against every segment before it in its block: that
/// is the whole reason to keep the block open. Incompressible bytes repeated
/// after a flush must come out as back-references, not as a second copy --
/// which a full flush, resetting the dictionary, would produce.
#[test]
fn the_dictionary_survives_a_flush() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let body = XorShift(99).bytes(8 * 1024);
    let first = writer.stage(&body).unwrap();
    writer.flush_segment().unwrap();
    let before = file_len(&path);
    let second = writer.stage(&body).unwrap();
    writer.flush_segment().unwrap();
    let second_segment = file_len(&path) - before;
    assert!(
        second_segment < 200,
        "a repeat after a flush took {second_segment} bytes; the dictionary was reset"
    );
    let reader = BodyLogReader::open(&path).unwrap();
    assert_eq!(reader.read(first).unwrap(), body);
    assert_eq!(reader.read(second).unwrap(), body);
    writer.close_block().unwrap();
}

/// One mebibyte of JSON-shaped text flushed twelve times -- a block's life
/// at one flush every five seconds on a busy session -- lands within 3% of
/// the same bytes deflated in one go.
#[test]
fn twelve_flushes_cost_under_three_percent_against_one_shot_deflate() {
    let dir = tempfile::tempdir().unwrap();
    let path = archive(&dir);
    let mut writer = BodyLogWriter::open(&path).unwrap();
    let mut rng = XorShift(12);
    let mut raw = Vec::new();
    let bodies: Vec<Vec<u8>> = (0..240).map(|_| rng.text(1024 * 1024 / 240)).collect();
    for (index, body) in bodies.iter().enumerate() {
        writer.stage(body).unwrap();
        raw.extend_from_slice(body);
        if index % 20 == 19 {
            writer.flush_segment().unwrap();
        }
    }
    writer.close_block().unwrap();
    let archived = file_len(&path) - FILE_HEADER_BYTES as u64;

    let mut one_shot = Compress::new(Compression::new(6), false);
    let mut out = Vec::with_capacity(raw.len());
    one_shot.compress_vec(&raw, &mut out, FlushCompress::Finish).unwrap();
    let ideal = out.len() as u64;
    assert!(
        archived * 100 <= ideal * 103,
        "open block with 12 flushes took {archived} bytes against {ideal} one-shot"
    );
}

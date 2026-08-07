use super::*;

#[test]
fn record_and_read_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pty.log");

    let log = PtyLog::open(&path).unwrap();
    log.record_output(b"hello from guest\r\n");
    log.record_input(b"ls -la\n");
    log.record_output(b"total 42\r\n");
    drop(log);

    let entries = read_pty_log(&path).unwrap();
    assert_eq!(entries.len(), 3);

    assert_eq!(entries[0].0, DIR_OUTPUT);
    assert_eq!(entries[0].2, b"hello from guest\r\n");

    assert_eq!(entries[1].0, DIR_INPUT);
    assert_eq!(entries[1].2, b"ls -la\n");

    assert_eq!(entries[2].0, DIR_OUTPUT);
    assert_eq!(entries[2].2, b"total 42\r\n");
}

#[test]
fn read_output_bytes_filters_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pty.log");

    let log = PtyLog::open(&path).unwrap();
    log.record_output(b"output1");
    log.record_input(b"input1");
    log.record_output(b"output2");
    drop(log);

    let output = read_output_bytes(&path).unwrap();
    assert_eq!(output, b"output1output2");
}

#[test]
fn rotation_at_max_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pty.log");

    // Set tiny max to trigger rotation
    let log = PtyLog::open_with_max(&path, 100).unwrap();
    log.record_output(&[0x41; 80]); // 80 + 13 header = 93 bytes
    assert!(log.bytes_written() > 0);

    log.record_output(&[0x42; 80]); // triggers rotation
    drop(log);

    // Rotated file should exist
    let rotated = dir.path().join("pty.log.1");
    assert!(rotated.exists(), "rotated file should exist");
    // New file should have only the post-rotation data
    assert!(path.exists(), "current file should exist");
}

#[test]
fn empty_data_not_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pty.log");

    let log = PtyLog::open(&path).unwrap();
    log.record_output(b"");
    log.record_input(b"");
    drop(log);

    let entries = read_pty_log(&path).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn timestamps_are_monotonic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pty.log");

    let log = PtyLog::open(&path).unwrap();
    for _ in 0..10 {
        log.record_output(b"x");
    }
    drop(log);

    let entries = read_pty_log(&path).unwrap();
    assert_eq!(entries.len(), 10);
    for i in 1..entries.len() {
        assert!(
            entries[i].1 >= entries[i - 1].1,
            "timestamps must be monotonic"
        );
    }
}

#[test]
fn binary_data_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pty.log");

    let binary: Vec<u8> = (0..=255).collect();
    let log = PtyLog::open(&path).unwrap();
    log.record_output(&binary);
    drop(log);

    let entries = read_pty_log(&path).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].2, binary);
}

// ── Malformed record buffers ───────────────────────────────────────
//
// parse_pty_log walks a length-prefixed binary format off disk. The existing
// tests all round-trip through the writer, so nothing exercised a file that
// was truncated by a crash, rotated mid-record, or corrupted. Every case here
// must return what it could parse rather than panicking or over-reading:
// a session recording is replayed into a terminal view, and a panic there
// takes the reader down with it.

/// `[1 byte direction][8 bytes LE timestamp][4 bytes LE length][payload]`
fn record_bytes(direction: u8, ts_us: u64, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![direction];
    v.extend_from_slice(&ts_us.to_le_bytes());
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

#[test]
fn a_buffer_too_short_for_one_header_yields_nothing() {
    for len in 0..13 {
        let buf = vec![0xffu8; len];
        assert_eq!(
            parse_pty_log(&buf).unwrap().len(),
            0,
            "{len} bytes is less than one 13-byte header"
        );
    }
}

#[test]
fn a_record_truncated_mid_payload_is_dropped_not_partially_returned() {
    let mut buf = record_bytes(DIR_OUTPUT, 1, b"complete");
    let mut cut = record_bytes(DIR_OUTPUT, 2, b"this payload never finished");
    cut.truncate(13 + 4); // header plus four of the declared bytes
    buf.extend_from_slice(&cut);

    let entries = parse_pty_log(&buf).unwrap();

    assert_eq!(entries.len(), 1, "only the complete record survives");
    assert_eq!(entries[0].2, b"complete");
}

#[test]
fn an_absurd_declared_length_stops_the_scan_without_reading_past_the_buffer() {
    // A corrupt length field is the dangerous case: the parser must compare
    // against the real buffer length rather than trusting the header.
    let mut buf = record_bytes(DIR_OUTPUT, 1, b"ok");
    buf.push(DIR_OUTPUT);
    buf.extend_from_slice(&7u64.to_le_bytes());
    buf.extend_from_slice(&u32::MAX.to_le_bytes()); // claims 4 GiB of payload

    let entries = parse_pty_log(&buf).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].2, b"ok");
}

#[test]
fn zero_length_payloads_are_preserved_as_entries() {
    let mut buf = record_bytes(DIR_INPUT, 5, b"");
    buf.extend_from_slice(&record_bytes(DIR_OUTPUT, 6, b"after"));

    let entries = parse_pty_log(&buf).unwrap();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, DIR_INPUT);
    assert!(entries[0].2.is_empty());
    assert_eq!(entries[1].2, b"after");
}

#[test]
fn an_unknown_direction_byte_is_carried_through_not_rejected() {
    // The parser is a framing layer; classifying direction is the caller's
    // job. A future direction value must not silently drop the record.
    let buf = record_bytes(0x7f, 9, b"payload");
    let entries = parse_pty_log(&buf).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, 0x7f);
}

#[test]
fn leading_garbage_is_framed_as_a_record_rather_than_resynchronised() {
    // Documents that there is no resync: the format has no magic number, so a
    // corrupt prefix consumes following bytes as a header. Anything relying on
    // recovery mid-stream has to add framing, not assume the parser skips.
    let mut buf = vec![0u8; 13];
    buf[9..13].copy_from_slice(&4u32.to_le_bytes()); // declares a 4-byte payload
    buf.extend_from_slice(b"junk");
    buf.extend_from_slice(&record_bytes(DIR_OUTPUT, 1, b"real"));

    let entries = parse_pty_log(&buf).unwrap();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].2, b"junk");
    assert_eq!(entries[1].2, b"real");
}

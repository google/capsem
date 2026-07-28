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

use super::*;
use std::io::Write;
use std::os::unix::io::FromRawFd;
use std::time::Duration;

fn make_pipe() -> (RawFd, RawFd) {
    let mut fds = [0 as RawFd; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    (fds[0], fds[1])
}

/// Collect all broadcast chunks into a single byte vector.
fn collect_all(rx: &mut broadcast::Receiver<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        match rx.blocking_recv() {
            Ok(chunk) => out.extend_from_slice(&chunk),
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
        }
    }
    out
}

#[test]
fn reader_broadcasts_written_data() {
    let (read_fd, write_fd) = make_pipe();
    let console = create_console_from_fd(read_fd, -1);
    let mut rx = console.subscribe();
    console.spawn_reader();
    drop(console); // drop tx so collect_all gets Closed after EOF

    let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
    writer.write_all(b"hello world\n").unwrap();
    writer.write_all(b"second line\n").unwrap();
    drop(writer);

    let all = collect_all(&mut rx);
    assert_eq!(all, b"hello world\nsecond line\n");
}

#[test]
fn reader_broadcasts_partial_writes() {
    let (read_fd, write_fd) = make_pipe();
    let console = create_console_from_fd(read_fd, -1);
    let mut rx = console.subscribe();
    console.spawn_reader();
    drop(console);

    let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
    writer.write_all(b"partial").unwrap();
    writer.write_all(b" complete\n").unwrap();
    drop(writer);

    let all = collect_all(&mut rx);
    assert_eq!(all, b"partial complete\n");
}

#[test]
fn reader_broadcasts_data_without_trailing_newline() {
    let (read_fd, write_fd) = make_pipe();
    let console = create_console_from_fd(read_fd, -1);
    let mut rx = console.subscribe();
    console.spawn_reader();
    drop(console);

    let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
    writer.write_all(b"first\nno newline at end").unwrap();
    drop(writer);

    let all = collect_all(&mut rx);
    assert_eq!(all, b"first\nno newline at end");
}

#[test]
fn reader_broadcasts_empty_lines() {
    let (read_fd, write_fd) = make_pipe();
    let console = create_console_from_fd(read_fd, -1);
    let mut rx = console.subscribe();
    console.spawn_reader();
    drop(console);

    let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
    writer.write_all(b"a\n\nb\n").unwrap();
    drop(writer);

    let all = collect_all(&mut rx);
    assert_eq!(all, b"a\n\nb\n");
}

#[test]
fn reader_handles_immediate_eof() {
    let (read_fd, write_fd) = make_pipe();
    let console = create_console_from_fd(read_fd, -1);
    let mut rx = console.subscribe();

    // Close write end immediately
    unsafe {
        libc::close(write_fd);
    }

    console.spawn_reader();

    // Should get Closed with no lines
    std::thread::sleep(Duration::from_millis(50));
    match rx.try_recv() {
        Err(broadcast::error::TryRecvError::Closed) => {}
        Err(broadcast::error::TryRecvError::Empty) => {}
        other => panic!("expected closed or empty, got {other:?}"),
    }
}

#[test]
fn subscribe_returns_receiver() {
    let (read_fd, _write_fd) = make_pipe();
    let console = create_console_from_fd(read_fd, -1);

    let _rx1 = console.subscribe();
    let _rx2 = console.subscribe();
    // Multiple subscribers should work without panic
}

#[test]
fn create_serial_port_returns_valid_config() {
    let (config, console) = create_serial_port().unwrap();
    // The config should have an attachment set
    let attachment = unsafe { config.attachment() };
    assert!(attachment.is_some());
    // input_fd should be a valid file descriptor
    assert!(console.input_fd >= 0);
    unsafe {
        libc::close(console.input_fd);
    }
}

#[test]
fn serial_console_trait_input_fd() {
    let (read_fd, write_fd) = make_pipe();
    let console = create_console_from_fd(read_fd, write_fd);
    let trait_ref: &dyn crate::hypervisor::SerialConsole = &console;
    assert_eq!(trait_ref.input_fd(), write_fd);
}

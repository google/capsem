use super::*;
use std::io::Write;

fn make_pipe() -> (RawFd, RawFd) {
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    (fds[0], fds[1])
}

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
fn reader_broadcasts_data() {
    let (read_fd, write_fd) = make_pipe();
    let console = KvmSerialConsole::new(read_fd, -1);
    let mut rx = console.subscribe();
    console.spawn_reader();
    drop(console); // drop sender so collect_all gets Closed

    let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
    writer.write_all(b"hello world\n").unwrap();
    writer.write_all(b"second line\n").unwrap();
    drop(writer);

    let all = collect_all(&mut rx);
    assert_eq!(all, b"hello world\nsecond line\n");
}

#[test]
fn reader_mirrors_bytes_to_serial_log() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("serial.log");
    let (read_fd, write_fd) = make_pipe();
    let console = KvmSerialConsole::new(read_fd, -1);
    let mut rx = console.subscribe();
    console.spawn_reader_with_log(Some(log_path.clone()));
    drop(console);

    let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
    writer.write_all(b"boot line\n").unwrap();
    drop(writer);

    let all = collect_all(&mut rx);
    assert_eq!(all, b"boot line\n");
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&log_path, usize::MAX)
            .unwrap()
            .into_bytes(),
        b"boot line\n"
    );
}

#[test]
fn reader_handles_partial_writes() {
    let (read_fd, write_fd) = make_pipe();
    let console = KvmSerialConsole::new(read_fd, -1);
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
fn reader_handles_immediate_eof() {
    let (read_fd, write_fd) = make_pipe();
    let console = KvmSerialConsole::new(read_fd, -1);
    let mut rx = console.subscribe();

    unsafe {
        libc::close(write_fd);
    }
    console.spawn_reader();

    std::thread::sleep(std::time::Duration::from_millis(50));
    match rx.try_recv() {
        Err(broadcast::error::TryRecvError::Closed) => {}
        Err(broadcast::error::TryRecvError::Empty) => {}
        other => panic!("expected closed or empty, got {other:?}"),
    }
}

#[test]
fn multiple_subscribers() {
    let (read_fd, write_fd) = make_pipe();
    let console = KvmSerialConsole::new(read_fd, -1);
    let _rx1 = console.subscribe();
    let _rx2 = console.subscribe();
    // Should not panic
    unsafe {
        libc::close(write_fd);
    }
}

#[test]
fn input_fd_returns_stored_value() {
    let (read_fd, write_fd) = make_pipe();
    let console = KvmSerialConsole::new(read_fd, write_fd);
    let trait_ref: &dyn crate::hypervisor::SerialConsole = &console;
    assert_eq!(trait_ref.input_fd(), write_fd);
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn serial_console_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<KvmSerialConsole>();
}

#[test]
fn negative_input_fd() {
    let (read_fd, _write_fd) = make_pipe();
    let console = KvmSerialConsole::new(read_fd, -1);
    let trait_ref: &dyn crate::hypervisor::SerialConsole = &console;
    assert_eq!(trait_ref.input_fd(), -1);
}

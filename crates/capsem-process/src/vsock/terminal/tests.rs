use super::*;
use std::os::fd::{AsFd, AsRawFd};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod checkpoint;
mod drain;
mod rollback;

fn connection() -> (Arc<VsockConnection>, tokio::net::UnixStream) {
    let (owner, guest) = std::os::unix::net::UnixStream::pair().unwrap();
    guest.set_nonblocking(true).unwrap();
    let fd = owner.as_raw_fd();
    (
        Arc::new(VsockConnection::new(
            fd,
            capsem_proto::VSOCK_PORT_TERMINAL,
            Box::new(owner),
        )),
        tokio::net::UnixStream::from_std(guest).unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiet_guest_sync_timeout_flushes_display_effects_and_query_reply_once() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&home.path().join("serial.log"), 1024 * 1024)
        .unwrap()
        .shared();
    let bridge = spawn(Arc::clone(&terminal), rekey_rx, input_rx, None, log);
    rekey.send(Arc::clone(&owner)).await.unwrap();
    guest
        .write_all(b"before\x1b[?2026hheld\x07\x1b]2;Ready\x07\x1b[6n")
        .await
        .unwrap();
    // No more PTY bytes arrive. The bridge must drive the owner's deadline.
    let changed = tokio::time::timeout(Duration::from_secs(2), summaries.changed()).await;
    let summary = summaries.borrow().clone();
    let mut response = [0; 7];
    let reply = if changed.is_ok() {
        Some(tokio::time::timeout(Duration::from_secs(2), guest.read_exact(&mut response)).await)
    } else {
        None
    };
    owner.shutdown_both().unwrap();
    drop(rekey);
    drop(input);
    tokio::time::timeout(Duration::from_secs(2), bridge.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(changed.is_ok(), "quiet terminal never expired synchronized output");
    assert_eq!(summary.title, "Ready");
    assert_eq!(summary.bell_count, 1);
    assert!(
        reply
            .as_ref()
            .is_some_and(|result| result.as_ref().is_ok_and(Result::is_ok)),
        "missing query reply: {reply:?}"
    );
    assert_eq!(&response, b"\x1b[1;11R");
    let (_, snapshot, _) = terminal.subscribe();
    assert!(snapshot
        .render
        .windows(b"beforeheld".len())
        .any(|bytes| bytes == b"beforeheld"));
    assert!(snapshot.state.events.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_guest_input_does_not_stall_sync_expiry_or_rekey() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let fd = owner.try_clone_fd().unwrap();
    capsem_foundation::unix::fd::set_stream_buffers(fd.as_fd(), 4096).unwrap();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&home.path().join("serial.log"), 1024 * 1024)
        .unwrap()
        .shared();
    let bridge = spawn(Arc::clone(&terminal), rekey_rx, input_rx, None, log);
    rekey.send(Arc::clone(&owner)).await.unwrap();
    input
        .send(vec![b'x'; capsem_proto::stream::MAX_DATA_BYTES])
        .await
        .unwrap();
    // Observe actual socket data without consuming it. The complete input
    // frame cannot fit in the deliberately small queue, so its write stalls.
    tokio::time::timeout(Duration::from_secs(2), guest.readable())
        .await
        .unwrap()
        .unwrap();
    guest
        .write_all(b"\x1b[?2026hheld\x07\x1b]2;While blocked\x07")
        .await
        .unwrap();
    let expired = tokio::time::timeout(Duration::from_secs(2), summaries.changed()).await;
    let first = summaries.borrow_and_update().clone();
    let (replacement, mut fresh_guest) = connection();
    rekey.send(Arc::clone(&replacement)).await.unwrap();
    fresh_guest.write_all(b"\x1b]2;Rekeyed\x07").await.unwrap();
    let rekeyed = tokio::time::timeout(Duration::from_secs(2), summaries.changed()).await;
    let second = summaries.borrow().clone();
    // Rekey actively shuts the old socket down; cleanup may find it closed.
    if let Err(error) = owner.shutdown_both() {
        assert_eq!(error.kind(), std::io::ErrorKind::NotConnected);
    }
    input.send(b"fresh input".to_vec()).await.unwrap();
    let mut received = [0; 11];
    tokio::time::timeout(Duration::from_secs(2), fresh_guest.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&received, b"fresh input", "rekey replayed input from the old socket");
    replacement.shutdown_both().unwrap();
    drop(input);
    drop(rekey);
    tokio::time::timeout(Duration::from_secs(2), bridge.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(expired.is_ok(), "blocked input stopped synchronized-output expiry");
    assert_eq!(first.title, "While blocked");
    assert_eq!(first.bell_count, 1);
    assert!(rekeyed.is_ok(), "blocked input stopped terminal rekey");
    assert_eq!(second.title, "Rekeyed");
    assert_eq!(second.bell_count, 1);
}

#[tokio::test]
async fn timer_cancellation_preserves_the_partial_write_cursor() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let fd = owner.try_clone_fd().unwrap();
    capsem_foundation::unix::fd::set_stream_buffers(fd.as_fd(), 4096).unwrap();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&home.path().join("serial.log"), 1024 * 1024)
        .unwrap()
        .shared();
    let bridge = spawn(terminal, rekey_rx, input_rx, None, log);
    rekey.send(owner).await.unwrap();
    let payload: Vec<u8> = (0..capsem_proto::stream::MAX_DATA_BYTES)
        .map(|index| ((index * 31 + index / 251) % 251) as u8)
        .collect();
    input.send(payload.clone()).await.unwrap();
    guest.readable().await.unwrap();
    guest.write_all(b"\x1b[?2026h\x1b]2;Timer fired\x07").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summaries.borrow().title, "Timer fired");
    let mut received = vec![0; payload.len()];
    tokio::time::timeout(Duration::from_secs(2), guest.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, payload);
    input.send(b"after".to_vec()).await.unwrap();
    let mut marker = [0; 5];
    tokio::time::timeout(Duration::from_secs(2), guest.read_exact(&mut marker))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&marker, b"after", "partial write was replayed or truncated");
    drop(rekey);
    drop(input);
    tokio::time::timeout(Duration::from_secs(2), bridge.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn disconnected_guest_still_expires_the_session_sync_deadline() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&home.path().join("serial.log"), 1024 * 1024)
        .unwrap()
        .shared();
    let bridge = spawn(terminal, rekey_rx, input_rx, None, log);
    rekey.send(owner).await.unwrap();
    guest
        .write_all(b"\x1b[?2026hheld\x07\x1b]2;Disconnected\x07\x1b[6n")
        .await
        .unwrap();
    guest.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summaries.borrow().title, "Disconnected");
    assert_eq!(summaries.borrow().bell_count, 1);
    let (fresh, mut new_guest) = connection();
    rekey.send(fresh).await.unwrap();
    let mut response = [0; 6];
    tokio::time::timeout(Duration::from_secs(2), new_guest.read_exact(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&response, b"\x1b[1;5R", "query expired between connections was lost");
    drop(rekey);
    drop(input);
    tokio::time::timeout(Duration::from_secs(2), bridge.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn full_serial_log_queue_does_not_stall_sync_expiry_or_socket_rekey() {
    struct Release(Option<std::sync::mpsc::SyncSender<()>>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    struct SlowLog {
        entered: Option<tokio::sync::oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
        log: capsem_foundation::telemetry::SharedLogWriter,
    }
    impl std::io::Write for SlowLog {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
                self.release.recv().unwrap();
            }
            self.log.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.log.flush()
        }
    }
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("serial.log");
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&path, 1024 * 1024)
        .unwrap()
        .shared();
    let (entered, writing) = tokio::sync::oneshot::channel();
    let (unblock, resume) = std::sync::mpsc::sync_channel(1);
    let release = Release(Some(unblock));
    let bridge = spawn(
        terminal,
        rekey_rx,
        input_rx,
        None,
        SlowLog {
            entered: Some(entered),
            release: resume,
            log,
        },
    );
    rekey.send(owner).await.unwrap();
    let mut expected = Vec::new();
    for index in 0..=SERIAL_LOG_QUEUE_CAPACITY {
        let bytes = format!("\x1b]2;Batch{index}\x07");
        expected.extend_from_slice(bytes.as_bytes());
        guest.write_all(bytes.as_bytes()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), summaries.changed())
            .await
            .unwrap()
            .unwrap();
        summaries.borrow_and_update();
    }
    tokio::time::timeout(Duration::from_secs(2), writing)
        .await
        .unwrap()
        .unwrap();
    let held = b"\x1b[?2026hheld\x07\x1b]2;While log full\x07";
    expected.extend_from_slice(held);
    guest.write_all(held).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summaries.borrow_and_update().title, "While log full");
    let (fresh, mut new_guest) = connection();
    rekey.send(fresh).await.unwrap();
    let mut eof = [0; 1];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), guest.read(&mut eof))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    drop(release);
    let next = b"\x1b]2;After log unblocked\x07";
    expected.extend_from_slice(next);
    new_guest.write_all(next).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summaries.borrow().title, "After log unblocked");
    drop(rekey);
    drop(input);
    tokio::time::timeout(Duration::from_secs(2), bridge.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail_bytes(&path, 1024 * 1024).unwrap(),
        expected,
        "serial output was reordered or lost"
    );
}

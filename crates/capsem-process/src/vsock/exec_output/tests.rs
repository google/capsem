use super::*;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[test]
fn forwards_binary_output_before_guest_closes_stream() {
    let (mut guest, mut host) = UnixStream::pair().unwrap();
    host.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        read_output(
            &mut host,
            |data| tx.send(data.to_vec()).map_err(std::io::Error::other),
            true,
        )
    });
    guest.write_all(b"ready\0\xff").unwrap();
    let first = rx.recv_timeout(Duration::from_secs(1));
    drop(guest);
    let captured = reader.join().unwrap().unwrap();
    assert_eq!(first.unwrap(), b"ready\0\xff");
    assert_eq!(captured, (b"ready\0\xff".to_vec(), 7));
}

#[test]
fn closed_consumer_stops_reading_instead_of_discarding_output() {
    let mut reader = std::io::Cursor::new(vec![42; 100_000]);
    let result = read_output(&mut reader, |_| Err(std::io::ErrorKind::BrokenPipe.into()), true);
    assert_eq!(result.err().unwrap().kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(reader.position(), 8192);
}

#[test]
fn stream_read_failure_is_not_successful_eof() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::ConnectionReset.into())
        }
    }
    assert_eq!(
        read_output(&mut Broken, |_| Ok(()), true).unwrap_err().kind(),
        std::io::ErrorKind::ConnectionReset
    );
}

#[test]
fn bounded_consumer_applies_backpressure_without_holding_job_store_locks() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Chunks {
        reads: std::sync::Arc<AtomicUsize>,
        observed: std::sync::mpsc::Sender<usize>,
    }
    impl Read for Chunks {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let count = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
            self.observed.send(count).unwrap();
            if count > 4 {
                return Ok(0);
            }
            buffer.fill(count as u8);
            Ok(buffer.len())
        }
    }
    let reads = std::sync::Arc::new(AtomicUsize::new(0));
    let (observed, events) = std::sync::mpsc::channel();
    let (sender, mut consumer) = tokio::sync::mpsc::channel(1);
    let mut source = Chunks {
        reads: reads.clone(),
        observed,
    };
    let worker = std::thread::spawn(move || stream_exec_output(&mut source, 91, &sender));
    assert_eq!(events.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
    assert_eq!(events.recv_timeout(Duration::from_secs(1)).unwrap(), 2);
    // One queued chunk and one pending send: reading cannot race ahead.
    assert!(events.recv_timeout(Duration::from_millis(30)).is_err());
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    let mut bytes = Vec::new();
    while let Some(message) = consumer.blocking_recv() {
        match message {
            capsem_proto::ipc::ProcessToService::ExecOutput { id, data } => {
                assert_eq!(id, 91);
                assert!(data.len() <= 8192);
                bytes.extend(data);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }
    let (captured, total) = worker.join().unwrap().unwrap();
    assert_eq!(captured, bytes);
    assert_eq!(total, 4 * 8192);
}

#[test]
fn detached_stream_keeps_draining_without_unbounded_capture() {
    let mut source = std::io::Cursor::new(vec![42; MAX_EXEC_OUTPUT_BYTES + 100_000]);
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);
    let (captured, total) = stream_exec_output(&mut source, 91, &sender).unwrap();
    assert_eq!(total, MAX_EXEC_OUTPUT_BYTES as u64 + 100_000);
    assert_eq!(captured.len(), MAX_EXEC_OUTPUT_BYTES);
    assert_eq!(source.position(), total);
}

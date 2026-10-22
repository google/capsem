use super::*;
use crate::pty_log::{parse_pty_log, read_output_bytes, read_pty_log, timestamp_us, DIR_INPUT, DIR_OUTPUT};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Default)]
struct SlowDisk {
    released: Mutex<bool>,
    wake: Condvar,
    bytes: Mutex<Vec<u8>>,
}

impl SlowDisk {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

// Release the worker even if an assertion unwinds, so failures cannot leak it.
struct Release(Arc<SlowDisk>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn slow_recorder() -> (PtyRecorder, Arc<SlowDisk>, Release, tokio::sync::oneshot::Receiver<()>) {
    let disk = Arc::new(SlowDisk::default());
    let worker_disk = disk.clone();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let mut entered = Some(entered);
    let recorder = PtyRecorder::spawn_writer(move |frame| {
        if let Some(entered) = entered.take() {
            let _ = entered.send(());
        }
        let released = worker_disk.released.lock().unwrap();
        drop(worker_disk.wake.wait_while(released, |released| !*released).unwrap());
        worker_disk.bytes.lock().unwrap().extend_from_slice(frame);
        Ok(())
    })
    .unwrap();
    (recorder, disk.clone(), Release(disk), ready)
}

#[tokio::test(flavor = "current_thread")]
async fn slow_storage_preserves_producer_time_and_direction_without_blocking_runtime() {
    let (recorder, disk, _release, ready) = slow_recorder();
    recorder.record_output(b"first").unwrap();
    ready.await.unwrap();
    recorder.record_input(b"command").unwrap();
    recorder.record_output(b"response").unwrap();
    let before_disk_release = timestamp_us();
    // A storage-blocked runtime cannot complete this current-thread timer.
    tokio::time::timeout(std::time::Duration::from_secs(1), tokio::task::yield_now())
        .await
        .unwrap();
    let barrier = recorder.flush();
    tokio::pin!(barrier);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(10), &mut barrier)
        .await
        .is_err());
    disk.release();
    barrier.await.unwrap();
    let entries = parse_pty_log(&disk.bytes.lock().unwrap()).unwrap();
    assert_eq!(
        entries.iter().map(|entry| entry.0).collect::<Vec<_>>(),
        [DIR_OUTPUT, DIR_INPUT, DIR_OUTPUT]
    );
    assert_eq!(entries[1].2, b"command");
    assert_eq!(entries[2].2, b"response");
    assert!(entries.iter().all(|entry| entry.1 <= before_disk_release));
    recorder.shutdown().await.unwrap();
}

#[tokio::test]
async fn overflow_stops_recording_but_drains_the_accepted_prefix() {
    let (recorder, disk, _release, ready) = slow_recorder();
    recorder.record_output(b"in flight").unwrap();
    ready.await.unwrap();
    let mut accepted = 1;
    loop {
        match recorder.record_input(b"queued") {
            Ok(()) => accepted += 1,
            Err(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
                break;
            }
        }
        assert!(accepted <= QUEUE_RECORDS + 1);
    }
    assert!(recorder.record_output(b"must never resume").is_err());
    disk.release();
    assert_eq!(
        recorder.shutdown().await.unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let entries = parse_pty_log(&disk.bytes.lock().unwrap()).unwrap();
    assert_eq!(entries.len(), accepted);
    assert!(entries.iter().skip(1).all(|entry| entry.2 == b"queued"));
}

#[tokio::test]
async fn queued_and_in_flight_bytes_share_one_bound_before_allocation() {
    let (recorder, disk, _release, ready) = slow_recorder();
    recorder.record_output(&vec![b'x'; QUEUE_BYTES - 13]).unwrap();
    ready.await.unwrap();
    assert_eq!(
        recorder.record_input(b"over budget").unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    disk.release();
    assert!(recorder.shutdown().await.is_err());
    assert_eq!(disk.bytes.lock().unwrap().len(), QUEUE_BYTES);
}

#[tokio::test]
async fn disk_failure_is_returned_by_read_and_shutdown_barriers() {
    let recorder = PtyRecorder::spawn_writer(|_| Err(std::io::Error::from(std::io::ErrorKind::StorageFull))).unwrap();
    recorder.record_output(b"cannot persist").unwrap();
    assert_eq!(
        recorder.flush().await.unwrap_err().kind(),
        std::io::ErrorKind::StorageFull
    );
    assert_eq!(
        recorder.record_input(b"later").unwrap_err().kind(),
        std::io::ErrorKind::StorageFull
    );
    assert_eq!(
        recorder.shutdown().await.unwrap_err().kind(),
        std::io::ErrorKind::StorageFull
    );
    drop(recorder);
}

#[tokio::test]
async fn shared_recorders_drain_once_and_reject_writes_after_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pty.log");
    let recorder = PtyRecorder::new(PtyLog::open(&path).unwrap()).unwrap();
    let input = recorder.clone();
    recorder.record_output(b"hello").unwrap();
    input.record_input(b"private input").unwrap();
    let (first, second) = tokio::join!(recorder.shutdown(), input.shutdown());
    drop(recorder);
    first.unwrap();
    second.unwrap();
    assert_eq!(read_output_bytes(&path).unwrap(), b"hello");
    assert_eq!(read_pty_log(&path).unwrap().len(), 2);
    assert_eq!(
        input.record_output(b"after close").unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    drop(input);
}

#[tokio::test]
async fn rotation_failure_is_observable_instead_of_growing_the_active_log_forever() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pty.log");
    let recorder = PtyRecorder::new(PtyLog::open_with_max(&path, 30).unwrap()).unwrap();
    recorder.record_output(b"a").unwrap();
    recorder.flush().await.unwrap();
    // Make rename fail, rather than letting optional recording grow forever.
    std::fs::create_dir(directory.path().join("pty.log.1")).unwrap();
    recorder.record_output(b"rotation failure").unwrap();
    assert!(recorder.flush().await.is_err());
    assert!(recorder.shutdown().await.is_err());
    drop(recorder);
}

#[tokio::test]
async fn worker_rotation_stays_private_and_anchored_to_the_opened_directory() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let logs = root.path().join("logs");
    let moved = root.path().join("moved");
    let outside = root.path().join("outside");
    std::fs::create_dir(&logs).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("pty.log"), b"host-only").unwrap();
    let recorder = PtyRecorder::new(PtyLog::open_with_max(&logs.join("pty.log"), 100).unwrap()).unwrap();
    recorder.record_output(&[b'a'; 80]).unwrap();
    recorder.flush().await.unwrap();
    std::fs::rename(&logs, &moved).unwrap();
    std::os::unix::fs::symlink(&outside, &logs).unwrap();
    recorder.record_output(&[b'b'; 80]).unwrap();
    recorder.record_input(b"secret").unwrap();
    recorder.record_output(b"final").unwrap();
    recorder.shutdown().await.unwrap();
    drop(recorder);
    assert_eq!(std::fs::read(outside.join("pty.log")).unwrap(), b"host-only");
    assert!(!outside.join("pty.log.1").exists());
    assert_eq!(
        read_output_bytes(&moved.join("pty.log.1")).unwrap(),
        [vec![b'a'; 80], vec![b'b'; 80]].concat()
    );
    assert_eq!(read_output_bytes(&moved.join("pty.log")).unwrap(), b"final");
    for name in ["pty.log", "pty.log.1"] {
        assert_eq!(
            std::fs::metadata(moved.join(name)).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn dropping_producers_or_cancelling_a_barrier_still_releases_the_worker() {
    let (recorder, disk, _release, ready) = slow_recorder();
    recorder.record_output(b"accepted").unwrap();
    ready.await.unwrap();
    let mut completed = recorder.completed.clone();
    {
        let barrier = recorder.flush();
        tokio::pin!(barrier);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(10), &mut barrier)
            .await
            .is_err());
    }
    drop(recorder);
    disk.release();
    tokio::time::timeout(std::time::Duration::from_secs(2), completed.wait_for(|done| *done))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parse_pty_log(&disk.bytes.lock().unwrap()).unwrap()[0].2, b"accepted");
}

#[tokio::test]
async fn concurrent_producers_keep_complete_frames_and_each_producers_order() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pty.log");
    let recorder = PtyRecorder::new(PtyLog::open(&path).unwrap()).unwrap();
    let mut producers = Vec::new();
    for direction in [DIR_INPUT, DIR_OUTPUT] {
        let producer = recorder.clone();
        producers.push(std::thread::spawn(move || {
            for sequence in 0..50u8 {
                producer.record(direction, &[direction, sequence]).unwrap();
            }
        }));
    }
    for producer in producers {
        producer.join().unwrap();
    }
    recorder.shutdown().await.unwrap();
    drop(recorder);
    let entries = read_pty_log(&path).unwrap();
    assert_eq!(entries.len(), 100);
    assert!(entries.windows(2).all(|pair| pair[0].1 <= pair[1].1));
    for direction in [DIR_INPUT, DIR_OUTPUT] {
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.0 == direction)
                .map(|entry| entry.2[1])
                .collect::<Vec<_>>(),
            (0..50u8).collect::<Vec<_>>()
        );
    }
}

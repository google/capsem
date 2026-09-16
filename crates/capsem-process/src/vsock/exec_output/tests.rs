use super::*;
use capsem_proto::{ExecOutputChannel, ExecOutputFrame};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn write_frame(writer: &mut impl Write, channel: ExecOutputChannel, data: &[u8]) {
    capsem_proto::write_exec_output(
        writer,
        &ExecOutputFrame {
            channel,
            data: data.to_vec(),
        },
    )
    .unwrap();
}

fn encoded(channel: ExecOutputChannel, total: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut remaining = total;
    while remaining > 0 {
        let size = remaining.min(capsem_proto::MAX_EXEC_DATA_BYTES);
        write_frame(&mut bytes, channel, &vec![42; size]);
        remaining -= size;
    }
    bytes
}

#[test]
fn forwards_binary_output_before_guest_closes_stream() {
    let (mut guest, mut host) = UnixStream::pair().unwrap();
    host.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        read_output(
            &mut host,
            |channel, data| tx.send((channel, data.to_vec())).map_err(std::io::Error::other),
            true,
            MAX_EXEC_OUTPUT_BYTES,
        )
    });
    write_frame(&mut guest, ExecOutputChannel::Stderr, b"ready\0\xff");
    let first = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    drop(guest);
    let captured = reader.join().unwrap().unwrap();
    assert_eq!(first, (ExecOutputChannel::Stderr, b"ready\0\xff".to_vec()));
    assert_eq!(captured.stderr, b"ready\0\xff");
    assert_eq!(captured.stderr_bytes, 7);
}

#[test]
fn closed_consumer_stops_before_the_next_frame() {
    let mut reader = std::io::Cursor::new(encoded(ExecOutputChannel::Stdout, 100_000));
    let result = read_output(
        &mut reader,
        |_, _| Err(std::io::ErrorKind::BrokenPipe.into()),
        true,
        MAX_EXEC_OUTPUT_BYTES,
    );
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::BrokenPipe);
    assert!(reader.position() > 4);
}

#[test]
fn stream_read_failure_is_not_successful_eof() {
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::ConnectionReset.into())
        }
    }
    assert_eq!(
        read_output(&mut Broken, |_, _| Ok(()), true, MAX_EXEC_OUTPUT_BYTES)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::ConnectionReset
    );
}

#[test]
fn truncated_frame_is_not_clean_socket_eof() {
    let mut truncated = std::io::Cursor::new(vec![0, 0, 0, 12, 1, 2]);
    assert_eq!(
        read_output(&mut truncated, |_, _| Ok(()), true, MAX_EXEC_OUTPUT_BYTES)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn stream_preserves_lanes_and_retains_only_ledger_previews() {
    let mut source = Vec::new();
    write_frame(&mut source, ExecOutputChannel::Stdout, &vec![7; 64_000]);
    write_frame(&mut source, ExecOutputChannel::Stderr, &vec![8; 32_000]);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
    let worker = std::thread::spawn(move || stream_exec_output(&mut std::io::Cursor::new(source), 5, &sender));
    let mut lanes = Vec::new();
    while let Some(message) = receiver.blocking_recv() {
        if let capsem_proto::ipc::ProcessToService::ExecOutput { channel, data, .. } = message {
            lanes.push((channel, data.len()));
        }
    }
    let capture = worker.join().unwrap().unwrap();
    assert_eq!(
        lanes,
        vec![(ExecOutputChannel::Stdout, 64_000), (ExecOutputChannel::Stderr, 32_000)]
    );
    assert_eq!(capture.stdout.len(), EXEC_LEDGER_PREVIEW_BYTES);
    assert_eq!(capture.stderr.len(), EXEC_LEDGER_PREVIEW_BYTES);
    assert_eq!((capture.stdout_bytes, capture.stderr_bytes), (64_000, 32_000));
}

#[test]
fn detached_stream_drains_with_bounded_capture() {
    let total = MAX_EXEC_OUTPUT_BYTES + 100_000;
    let source = encoded(ExecOutputChannel::Stdout, total);
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);
    let capture = stream_exec_output(&mut std::io::Cursor::new(source), 91, &sender).unwrap();
    assert_eq!(capture.stdout_bytes, total as u64);
    assert_eq!(capture.stdout.len(), EXEC_LEDGER_PREVIEW_BYTES);
}

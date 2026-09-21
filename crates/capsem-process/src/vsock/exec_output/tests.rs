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

#[test]
fn legacy_raw_output_is_captured_as_stdout_without_frame_decoding() {
    let capture = read_exec_output_protocol(&mut &b"ready\n\0\xff"[..], capsem_proto::ExecOutputProtocol::RawMerged);
    assert_eq!(capture.stdout, b"ready\n\0\xff");
    assert_eq!(capture.stdout_bytes, 8);
    assert!(capture.stderr.is_empty());
    assert!(capture.error.is_none(), "{:?}", capture.error);
}

#[test]
fn legacy_raw_stream_forwards_binary_chunks_as_stdout() {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
    let worker = std::thread::spawn(move || {
        stream_exec_output_protocol(
            &mut &b"legacy\0\xff"[..],
            17,
            &sender,
            capsem_proto::ExecOutputProtocol::RawMerged,
        )
    });
    let message = receiver.blocking_recv().unwrap();
    let capsem_proto::ipc::ProcessToService::ExecOutput { id, channel, data } = message else {
        panic!("expected streamed exec output");
    };
    assert_eq!(id, 17);
    assert_eq!(channel, ExecOutputChannel::Stdout);
    assert_eq!(data, b"legacy\0\xff");
    drop(receiver);
    let capture = worker.join().unwrap().unwrap();
    assert_eq!(capture.stdout_bytes, 8);
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

/// A non-streaming exec swallowed malformed and truncated frames as a clean
/// EOF, so it returned partial output with the guest's exit code and nothing
/// recorded that the output was incomplete.
#[test]
fn a_truncated_capture_records_why_the_output_stopped() {
    let mut source = Vec::new();
    write_frame(&mut source, ExecOutputChannel::Stdout, b"partial");
    source.extend_from_slice(&[0, 0, 0, 12, 1, 2]);
    let capture = read_output(
        &mut std::io::Cursor::new(source),
        |_, _| Ok(()),
        false,
        MAX_EXEC_OUTPUT_BYTES,
    )
    .expect("a non-streaming capture still returns what it read");

    assert_eq!(capture.stdout, b"partial");
    let error = capture.error.expect("the truncated frame is reported");
    assert!(error.contains("exec output transport failed"), "{error}");
}

#[test]
fn a_complete_capture_reports_no_error() {
    let mut source = Vec::new();
    write_frame(&mut source, ExecOutputChannel::Stdout, b"done");
    let capture = read_output(
        &mut std::io::Cursor::new(source),
        |_, _| Ok(()),
        false,
        MAX_EXEC_OUTPUT_BYTES,
    )
    .unwrap();
    assert_eq!(capture.error, None);
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

/// Each `read` on the vsock fd is a syscall.
struct CountedReads<'a> {
    inner: &'a [u8],
    calls: usize,
}

impl std::io::Read for CountedReads<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.calls += 1;
        std::io::Read::read(&mut self.inner, buf)
    }
}

/// Finding 31: every frame cost a one-byte EOF probe, a header read and a
/// payload read on an unbuffered fd.
#[test]
fn a_run_of_frames_is_read_in_bulk_not_per_frame() {
    let mut wire = Vec::new();
    for index in 0..100_u8 {
        capsem_proto::write_exec_output(
            &mut wire,
            &capsem_proto::ExecOutputFrame {
                channel: capsem_proto::ExecOutputChannel::Stdout,
                data: vec![index],
            },
        )
        .unwrap();
    }
    let mut reader = CountedReads { inner: &wire, calls: 0 };
    let capture = read_exec_output(&mut reader);
    assert_eq!(capture.stdout, (0..100).collect::<Vec<u8>>());
    assert!(capture.error.is_none(), "{:?}", capture.error);
    assert!(reader.calls <= 3, "{} reads for 100 frames", reader.calls);
}

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
    let capture = worker.join().unwrap();
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
        read_output(&mut host, |channel, data| {
            let _ = tx.send((channel, data.to_vec()));
        })
    });
    write_frame(&mut guest, ExecOutputChannel::Stderr, b"ready\0\xff");
    let first = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    drop(guest);
    let captured = reader.join().unwrap();
    assert_eq!(first, (ExecOutputChannel::Stderr, b"ready\0\xff".to_vec()));
    assert_eq!(captured.stderr, b"ready\0\xff");
    assert_eq!(captured.stderr_bytes, 7);
}

#[test]
fn a_read_failure_is_recorded_not_mistaken_for_eof() {
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::ConnectionReset.into())
        }
    }
    let error = read_exec_output(&mut Broken).error.expect("the failure is recorded");
    assert!(error.contains("exec output transport failed"), "{error}");
}

/// A stream that broke mid-output used to hand back only its error, and the
/// EXEC-port reader then deposited an empty capture: the ledger lost exactly
/// the output of the commands whose streams failed.
#[test]
fn a_broken_stream_keeps_what_it_captured_for_the_ledger() {
    let mut source = Vec::new();
    write_frame(&mut source, ExecOutputChannel::Stdout, b"before the break\n");
    write_frame(&mut source, ExecOutputChannel::Stderr, b"\xff\xfe warn\n");
    source.extend_from_slice(&[0, 0, 0, 12, 1, 2]);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    let worker = std::thread::spawn(move || stream_exec_output(&mut std::io::Cursor::new(source), 3, &sender));
    let mut forwarded = 0;
    while receiver.blocking_recv().is_some() {
        forwarded += 1;
    }
    let capture = worker.join().unwrap();

    assert_eq!(forwarded, 2, "the client still got every whole frame");
    assert_eq!(capture.stdout, b"before the break\n");
    assert_eq!(capture.stderr, b"\xff\xfe warn\n");
    let error = capture.error.expect("the broken frame is reported");
    assert!(error.contains("exec output transport failed"), "{error}");
}

#[test]
fn truncated_frame_is_not_clean_socket_eof() {
    let mut truncated = std::io::Cursor::new(vec![0, 0, 0, 12, 1, 2]);
    assert!(read_exec_output(&mut truncated).error.is_some());
}

/// A non-streaming exec swallowed malformed and truncated frames as a clean
/// EOF, so it returned partial output with the guest's exit code and nothing
/// recorded that the output was incomplete.
#[test]
fn a_truncated_capture_records_why_the_output_stopped() {
    let mut source = Vec::new();
    write_frame(&mut source, ExecOutputChannel::Stdout, b"partial");
    source.extend_from_slice(&[0, 0, 0, 12, 1, 2]);
    let capture = read_exec_output(&mut std::io::Cursor::new(source));

    assert_eq!(capture.stdout, b"partial");
    let error = capture.error.expect("the truncated frame is reported");
    assert!(error.contains("exec output transport failed"), "{error}");
}

#[test]
fn a_complete_capture_reports_no_error() {
    let mut source = Vec::new();
    write_frame(&mut source, ExecOutputChannel::Stdout, b"done");
    assert_eq!(read_exec_output(&mut std::io::Cursor::new(source)).error, None);
}

/// A streamed exec's output reaching its client is not the ledger's copy. It
/// used to keep 1 KiB per lane; it keeps the lanes, up to the body cap.
#[test]
fn a_stream_retains_whole_lanes_for_the_ledger() {
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
    let capture = worker.join().unwrap();
    assert_eq!(
        lanes,
        vec![(ExecOutputChannel::Stdout, 64_000), (ExecOutputChannel::Stderr, 32_000)]
    );
    assert_eq!(capture.stdout, vec![7; 64_000]);
    assert_eq!(capture.stderr, vec![8; 32_000]);
    assert_eq!((capture.stdout_bytes, capture.stderr_bytes), (64_000, 32_000));
}

/// A client that detaches -- a cancelled or dropped stream -- stops receiving,
/// but the guest is still drained and the ledger still gets its copy.
#[test]
fn detached_stream_drains_and_retains_up_to_the_ledger_cap() {
    let total = EXEC_LEDGER_BODY_BYTES + 100_000;
    let source = encoded(ExecOutputChannel::Stdout, total);
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);
    let capture = stream_exec_output(&mut std::io::Cursor::new(source), 91, &sender);
    assert_eq!(capture.stdout_bytes, total as u64);
    assert_eq!(capture.stdout.len(), EXEC_LEDGER_BODY_BYTES);
    assert!(capture.stdout.iter().all(|byte| *byte == 42));
}

#[test]
fn non_utf8_output_is_retained_byte_for_byte_on_both_lanes() {
    let stdout: Vec<u8> = (0..=255).cycle().take(3000).collect();
    let stderr: Vec<u8> = (0..=255).rev().cycle().take(2500).collect();
    let mut source = Vec::new();
    write_frame(&mut source, ExecOutputChannel::Stdout, &stdout[..1000]);
    write_frame(&mut source, ExecOutputChannel::Stderr, &stderr);
    write_frame(&mut source, ExecOutputChannel::Stdout, &stdout[1000..]);
    let capture = read_exec_output(&mut std::io::Cursor::new(source));
    assert_eq!(capture.stdout, stdout);
    assert_eq!(capture.stderr, stderr);
    assert_eq!(capture.response_cut, None, "all of it fits one result");
}

/// The buffered result's budget is shared by both lanes in the order they
/// arrived, as before; the ledger's is per lane and is not.
#[test]
fn the_result_budget_runs_out_in_arrival_order_while_the_lanes_keep_going() {
    let six_mib = 6 * 1024 * 1024;
    let mut source = encoded(ExecOutputChannel::Stdout, six_mib);
    source.extend(encoded(ExecOutputChannel::Stderr, six_mib));
    let capture = read_exec_output(&mut std::io::Cursor::new(source));

    assert_eq!(
        capture.response_cut,
        Some((six_mib, MAX_EXEC_OUTPUT_BYTES - six_mib)),
        "stdout came first and is whole; stderr gets what is left"
    );
    assert_eq!((capture.stdout.len(), capture.stderr.len()), (six_mib, six_mib));
}

#[test]
fn output_exactly_filling_the_result_is_cut_at_its_own_end() {
    let capture = read_exec_output(&mut std::io::Cursor::new(encoded(
        ExecOutputChannel::Stderr,
        MAX_EXEC_OUTPUT_BYTES,
    )));
    assert_eq!(capture.response_cut, Some((0, MAX_EXEC_OUTPUT_BYTES)));
    assert_eq!(capture.stderr_bytes, capture.stderr.len() as u64, "nothing was cut");
}

#[test]
fn a_lane_one_byte_over_the_ledger_cap_loses_exactly_that_byte() {
    let capture = read_exec_output(&mut std::io::Cursor::new(encoded(
        ExecOutputChannel::Stdout,
        EXEC_LEDGER_BODY_BYTES + 1,
    )));
    assert_eq!(capture.stdout.len(), EXEC_LEDGER_BODY_BYTES);
    assert_eq!(capture.stdout_bytes, EXEC_LEDGER_BODY_BYTES as u64 + 1);
}

/// Legacy guests send one merged stream; it is budgeted the same way.
#[test]
fn raw_merged_output_has_the_same_budgets() {
    let total = EXEC_LEDGER_BODY_BYTES + 5;
    let capture = read_exec_output_protocol(
        &mut std::io::Cursor::new(vec![b'r'; total]),
        capsem_proto::ExecOutputProtocol::RawMerged,
    );
    assert_eq!(capture.stdout.len(), EXEC_LEDGER_BODY_BYTES);
    assert_eq!(capture.stdout_bytes, total as u64);
    assert_eq!(capture.response_cut, Some((MAX_EXEC_OUTPUT_BYTES, 0)));
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

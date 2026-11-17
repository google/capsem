use super::*;
use std::io::BufRead;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;

#[test]
fn local_input_refusal_omits_unavailable_correlation_identity() {
    let mut bytes = Vec::new();
    write_input_error(&mut bytes, "Request was not forwarded").unwrap();
    let message: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(message.get("id").is_none());
    assert!(message.get("result").is_none());
    assert_eq!(message["error"]["code"], -32600);
    assert_eq!(message["error"]["message"], "Request was not forwarded");
}

#[test]
fn input_content_bound_is_independent_of_identity_and_recovers_after_refusal() {
    let empty = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"text":""}}"#;
    let overhead = empty.len();
    let prefix = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"text":""#;
    let suffix = r#""},"id":""#;
    let exact = format!(
        "{prefix}{}{suffix}{}\"}}",
        "x".repeat(capsem_proto::MCP_PAYLOAD_MAX_BYTES - overhead),
        "\\u0000".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES)
    );
    let result = classify_input(exact.as_bytes()).unwrap();
    assert_eq!(
        result,
        JsonRpcLineKind::Request {
            json_id: Some(serde_json::json!("\0".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES)))
        }
    );
    let too_large = format!(
        r#"{{"jsonrpc":"2.0","method":"tools/call","params":{{"text":"{}"}},"id":1}}"#,
        "x".repeat(capsem_proto::MCP_PAYLOAD_MAX_BYTES - overhead + 1)
    );
    let input = format!("{too_large}\n{exact}\n");
    let mut lines = lines::McpLines::new(
        std::io::Cursor::new(input.as_bytes()),
        capsem_proto::MCP_JSON_RPC_MAX_BYTES,
    );
    let lines::LineRead::Message(first) = lines.next_line().unwrap() else {
        panic!("physical envelope fits")
    };
    assert!(
        classify_input(&first).is_err(),
        "unused identity space must not admit oversized content"
    );
    let lines::LineRead::Message(next) = lines.next_line().unwrap() else {
        panic!("next line survives")
    };
    assert_eq!(classify_input(&next).unwrap(), result);
    assert!(classify_input(b"\xff").is_err());
}

#[test]
fn correlation_identity_is_bounded_before_tracking() {
    for id in ["null", "[]", "{}", "true", "1.5"] {
        assert_eq!(
            classify_jsonrpc_line(&format!(r#"{{"id":{id}}}"#)),
            JsonRpcLineKind::InvalidCorrelationId
        );
    }
    let oversized_id = "x".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES + 1);
    assert_eq!(
        classify_jsonrpc_line(&format!(r#"{{"id":"{oversized_id}"}}"#)),
        JsonRpcLineKind::InvalidCorrelationId
    );
}

#[test]
fn escaped_boundary_identity_and_large_parameters_preserve_the_id() {
    let escaped = "\\u0000".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES);
    let line = format!(r#"{{"id":"{escaped}","params":{{"input":"{}"}}}}"#, "x".repeat(100_000));
    assert_eq!(
        classify_jsonrpc_line(&line),
        JsonRpcLineKind::Request {
            json_id: Some(Value::String("\0".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES)))
        }
    );
}

#[test]
fn forwarding_borrows_the_original_payload_and_stops_after_header_failure() {
    let payload = b"{\"id\":1}";
    let header = capsem_proto::encode_mcp_frame_header(1, 0, "test", payload.len()).unwrap();
    let mut calls = 0;
    write_frame(&header, payload, |bytes| {
        if calls == 0 {
            assert_eq!(bytes, header.as_bytes());
        } else {
            assert_eq!(bytes.as_ptr(), payload.as_ptr());
            assert_eq!(bytes, payload);
        }
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 2);
    calls = 0;
    assert!(write_frame(&header, payload, |_| {
        calls += 1;
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "test"))
    })
    .is_err());
    assert_eq!(calls, 1);
}

#[test]
fn mcp_transport_uses_mitm_vsock_port() {
    assert_eq!(MCP_TRANSPORT_PORT, VSOCK_PORT_SNI_PROXY);
    assert_eq!(MCP_TRANSPORT_PORT, 5002);
}

#[test]
fn classify_valid_request_tracks_id() {
    let line = r#"{"jsonrpc":"2.0","id":"abc","method":"tools/call"}"#;
    assert_eq!(
        classify_jsonrpc_line(line),
        JsonRpcLineKind::Request {
            json_id: Some(Value::String("abc".to_string())),
        }
    );
}

#[test]
fn classify_notification_uses_reserved_stream_zero() {
    let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    assert_eq!(classify_jsonrpc_line(line), JsonRpcLineKind::Notification);
}

#[test]
fn classify_invalid_json_as_request_so_host_can_return_parse_error() {
    assert_eq!(
        classify_jsonrpc_line("{not json"),
        JsonRpcLineKind::Request { json_id: None }
    );
}

#[test]
fn pending_disconnect_errors_are_emitted_once_with_original_ids() {
    let pending = PendingRequests::new();
    pending.insert(
        1,
        PendingRequest {
            json_id: Value::from(7),
        },
    );
    pending.insert(
        2,
        PendingRequest {
            json_id: Value::String("abc".to_string()),
        },
    );

    let mut out = Vec::new();
    for request in pending.take_all() {
        write_disconnect_error(&mut out, request, "unit test").unwrap();
    }
    assert!(pending.take_all().is_empty());

    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert!(text.contains(r#""id":7"#));
    assert!(text.contains(r#""id":"abc""#));
    assert!(text.contains("MCP transport disconnected"));
}

#[test]
fn write_then_read_binary_data() {
    let (writer, reader) = UnixStream::pair().unwrap();
    let writer_fd = writer.into_raw_fd();

    let binary_line = b"{\"data\":\"\\x00\\xff\"}\n";
    write_all_fd(writer_fd, binary_line).expect("write binary");
    unsafe {
        nix::libc::close(writer_fd);
    }

    let file = unsafe { std::fs::File::from_raw_fd(reader.into_raw_fd()) };
    let buf = io::BufReader::new(file);
    let mut lines = buf.lines();
    assert!(lines.next().unwrap().is_ok());
    assert!(lines.next().is_none());
}

#[test]
fn large_json_line_preserved() {
    let (writer, reader) = UnixStream::pair().unwrap();
    let writer_fd = writer.into_raw_fd();

    let large_content = "x".repeat(100_000);
    let line = format!("{{\"content\":\"{}\"}}\n", large_content);

    std::thread::spawn(move || {
        write_all_fd(writer_fd, line.as_bytes()).expect("write large");
        unsafe {
            nix::libc::close(writer_fd);
        }
    });

    let file = unsafe { std::fs::File::from_raw_fd(reader.into_raw_fd()) };
    let buf = std::io::BufReader::new(file);
    let lines: Vec<String> = buf.lines().map(|l| l.unwrap()).collect();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].len() > 100_000);
}

fn run_framed_reader(input: &[u8], pending: PendingRequests) -> bool {
    let (mut host, guest) = UnixStream::pair().unwrap();
    host.write_all(input).unwrap();
    drop(host);

    let guest_fd = guest.into_raw_fd();
    let alive = Arc::new(AtomicBool::new(true));
    framed_vsock_to_stdout(
        guest_fd,
        pending,
        Arc::new(Mutex::new(io::stdout())),
        Arc::clone(&alive),
    );
    unsafe {
        nix::libc::close(guest_fd);
    }
    alive.load(Ordering::SeqCst)
}

#[test]
fn framed_reader_rejects_invalid_and_truncated_host_frames() {
    assert!(!run_framed_reader(&[], PendingRequests::new()));

    let too_short = u32::from(MCP_FRAME_HEADER_LEN - 1).to_be_bytes();
    assert!(!run_framed_reader(&too_short, PendingRequests::new()));

    let too_large = ((MCP_FRAME_MAX_SIZE + 1) as u32).to_be_bytes();
    assert!(!run_framed_reader(&too_large, PendingRequests::new()));

    let mut truncated = u32::from(MCP_FRAME_HEADER_LEN).to_be_bytes().to_vec();
    truncated.extend_from_slice(&[0; 3]);
    assert!(!run_framed_reader(&truncated, PendingRequests::new()));

    let mut invalid = u32::from(MCP_FRAME_HEADER_LEN).to_be_bytes().to_vec();
    invalid.extend_from_slice(&vec![0; MCP_FRAME_HEADER_LEN as usize]);
    assert!(!run_framed_reader(&invalid, PendingRequests::new()));
}

#[test]
fn framed_reader_consumes_empty_responses_and_pending_ids() {
    let pending = PendingRequests::new();
    pending.insert(
        9,
        PendingRequest {
            json_id: Value::from(9),
        },
    );
    let frame = capsem_proto::encode_mcp_frame(9, 0, "host", &[]).unwrap();

    assert!(!run_framed_reader(&frame, pending.clone()));
    assert!(pending.remove(9).is_none());
}

#[test]
fn jsonrpc_classification_fails_closed_on_malformed_shapes() {
    assert_eq!(classify_jsonrpc_line("[]"), JsonRpcLineKind::Request { json_id: None });
    assert_eq!(
        classify_jsonrpc_line(r#"{"id":1,"method":7}"#),
        JsonRpcLineKind::Request {
            json_id: Some(Value::from(1)),
        }
    );
}

/// The relay says it is done with a zero-length frame and keeps the socket
/// open for the answers still owed: a vsock shutdown can reach the host ahead
/// of the last request's bytes on Apple VZ and was lost behind them.
#[test]
fn ending_a_session_sends_the_end_frame_without_shutting_the_socket() {
    use std::io::Read;
    let (relay, mut host) = UnixStream::pair().unwrap();
    let relay_fd = relay.into_raw_fd();
    end_session(relay_fd).expect("end the session");
    let mut end = [0xffu8; 4];
    host.read_exact(&mut end).unwrap();
    assert_eq!(end, capsem_proto::MCP_SESSION_END);
    host.set_nonblocking(true).unwrap();
    assert_eq!(
        host.read(&mut [0u8; 1]).unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "the relay must not half-close; the host ends the session"
    );
    unsafe {
        nix::libc::close(relay_fd);
    }
}

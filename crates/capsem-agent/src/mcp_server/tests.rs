use super::*;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;

#[test]
fn mcp_transport_uses_mitm_vsock_port() {
    assert_eq!(MCP_TRANSPORT_PORT, VSOCK_PORT_SNI_PROXY);
    assert_eq!(MCP_TRANSPORT_PORT, 5002);
}

#[test]
fn classify_valid_request_tracks_id_and_method() {
    let line = r#"{"jsonrpc":"2.0","id":"abc","method":"tools/call"}"#;
    assert_eq!(
        classify_jsonrpc_line(line),
        JsonRpcLineKind::Request {
            json_id: Some(Value::String("abc".to_string())),
            method: Some("tools/call".to_string()),
            snapshot_revert_path: None,
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
        JsonRpcLineKind::Request {
            json_id: None,
            method: None,
            snapshot_revert_path: None,
        }
    );
}

#[test]
fn pending_disconnect_errors_are_emitted_once_with_original_ids() {
    let pending = PendingRequests::new();
    pending.insert(
        1,
        PendingRequest {
            json_id: Value::from(7),
            method: Some("tools/call".to_string()),
            snapshot_revert_path: None,
        },
    );
    pending.insert(
        2,
        PendingRequest {
            json_id: Value::String("abc".to_string()),
            method: Some("resources/list".to_string()),
            snapshot_revert_path: None,
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

/// The classification carries the revert path; one parse per line.
fn snapshot_revert_path_of(line: &str) -> Option<String> {
    match classify_jsonrpc_line(line) {
        JsonRpcLineKind::Request {
            snapshot_revert_path, ..
        } => snapshot_revert_path,
        JsonRpcLineKind::Notification => None,
    }
}

#[test]
fn extracts_snapshot_revert_path_from_tool_call() {
    let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"snapshots_revert","arguments":{"path":"/root/poem.md","checkpoint":"cp-0"}}}"#;

    assert_eq!(snapshot_revert_path_of(line).as_deref(), Some("/root/poem.md"));
}

#[test]
fn extracts_namespaced_snapshot_revert_path_from_tool_call() {
    let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"local__snapshots_revert","arguments":{"path":"poem.md","checkpoint":"cp-0"}}}"#;

    assert_eq!(snapshot_revert_path_of(line).as_deref(), Some("poem.md"));
}

#[test]
fn ignores_non_snapshot_tool_calls_for_guest_side_effects() {
    let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fetch_http","arguments":{"url":"https://example.com"}}}"#;

    assert!(snapshot_revert_path_of(line).is_none());
}

#[test]
fn snapshot_delete_response_must_be_successful_deleted_action() {
    let ok = br#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"reverted\":true,\"action\":\"deleted\"}"}]}}"#;
    let restored = br#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"reverted\":true,\"action\":\"restored\"}"}]}}"#;
    let error = br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"nope"}}"#;

    assert!(response_reports_snapshot_delete(ok));
    assert!(!response_reports_snapshot_delete(restored));
    assert!(!response_reports_snapshot_delete(error));
}

#[test]
fn normalizes_guest_snapshot_paths_under_root_only() {
    assert_eq!(
        normalize_guest_snapshot_path("nested/file.txt").unwrap(),
        std::path::PathBuf::from("/root/nested/file.txt")
    );
    assert_eq!(
        normalize_guest_snapshot_path("/root/poem.md").unwrap(),
        std::path::PathBuf::from("/root/poem.md")
    );
    assert!(normalize_guest_snapshot_path("../escape").is_none());
    assert!(normalize_guest_snapshot_path("/etc/passwd").is_none());
    assert!(normalize_guest_snapshot_path("bad\0path").is_none());
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
            method: Some("tools/list".to_string()),
            snapshot_revert_path: None,
        },
    );
    let frame = capsem_proto::encode_mcp_frame(9, 0, "host", &[]).unwrap();

    assert!(!run_framed_reader(&frame, pending.clone()));
    assert!(pending.remove(9).is_none());
}

#[test]
fn jsonrpc_and_snapshot_helpers_fail_closed_on_malformed_shapes() {
    assert_eq!(
        classify_jsonrpc_line("[]"),
        JsonRpcLineKind::Request {
            json_id: None,
            method: None,
            snapshot_revert_path: None,
        }
    );
    assert_eq!(
        classify_jsonrpc_line(r#"{"id":1,"method":7}"#),
        JsonRpcLineKind::Request {
            json_id: Some(Value::from(1)),
            method: None,
            snapshot_revert_path: None,
        }
    );

    for line in [
        "not json",
        "[]",
        r#"{"method":"tools/list"}"#,
        r#"{"method":"tools/call","params":[]}"#,
        r#"{"method":"tools/call","params":{"name":7}}"#,
        r#"{"method":"tools/call","params":{"name":"snapshots_revert"}}"#,
        r#"{"method":"tools/call","params":{"name":"snapshots_revert","arguments":[]}}"#,
    ] {
        assert!(snapshot_revert_path_of(line).is_none(), "accepted {line}");
    }

    for payload in [
        &b"not json"[..],
        &br#"{"result":{}}"#[..],
        &br#"{"result":{"content":{}}}"#[..],
        &br#"{"result":{"content":[{"text":"not json"}]}}"#[..],
    ] {
        assert!(!response_reports_snapshot_delete(payload));
    }
}

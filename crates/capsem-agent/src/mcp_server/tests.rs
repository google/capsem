use super::*;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;

#[test]
fn mcp_transport_uses_mitm_vsock_port() {
    assert_eq!(MCP_TRANSPORT_PORT, VSOCK_PORT_SNI_PROXY);
    assert_eq!(MCP_TRANSPORT_PORT, 5002);
}

#[test]
fn meta_line_format() {
    let name = "claude";
    let meta = format!("\0CAPSEM_META:{}\n", name);
    assert!(meta.starts_with('\0'));
    assert!(meta.contains("CAPSEM_META:claude"));
    assert!(meta.ends_with('\n'));
}

#[test]
fn meta_line_nul_prefix_required() {
    let meta = "\0CAPSEM_META:gemini\n".to_string();
    assert_eq!(meta.as_bytes()[0], 0x00);
    let json = r#"{"jsonrpc":"2.0","method":"tools/call"}"#;
    assert_ne!(json.as_bytes()[0], 0x00);
}

#[test]
fn classify_valid_request_tracks_id_and_method() {
    let line = r#"{"jsonrpc":"2.0","id":"abc","method":"tools/call"}"#;
    assert_eq!(
        classify_jsonrpc_line(line),
        JsonRpcLineKind::Request {
            json_id: Some(Value::String("abc".to_string())),
            method: Some("tools/call".to_string()),
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
        },
    );
    pending.insert(
        2,
        PendingRequest {
            json_id: Value::String("abc".to_string()),
            method: Some("resources/list".to_string()),
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
fn sanitize_strips_control_chars() {
    assert_eq!(sanitize_process_name("clean"), "clean");
    assert_eq!(sanitize_process_name("has space"), "has_space");
    assert_eq!(sanitize_process_name("has\nnewline"), "has_newline");
    assert_eq!(sanitize_process_name("has\rcarriage"), "has_carriage");
    assert_eq!(sanitize_process_name("has\0nul"), "has_nul");
    assert_eq!(sanitize_process_name("has\ttab"), "has_tab");
}

#[test]
fn sanitize_truncates_long_names() {
    let long = "x".repeat(200);
    let result = sanitize_process_name(&long);
    assert_eq!(result.len(), 128);
}

#[test]
fn sanitize_preserves_slashes_and_dashes() {
    assert_eq!(
        sanitize_process_name("claude/code-v4.0"),
        "claude/code-v4.0"
    );
}

#[test]
fn sanitize_meta_line_injection_blocked() {
    let evil = "evil\nCAPS_META:spoof";
    let sanitized = sanitize_process_name(evil);
    assert!(!sanitized.contains('\n'), "newline must be stripped");
    let meta = format!("\0CAPSEM_META:{}\n", sanitized);
    assert_eq!(meta.matches('\n').count(), 1);
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
    let lines: Vec<String> = buf.lines().map(|l| l.unwrap()).collect();
    assert_eq!(lines.len(), 1);
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

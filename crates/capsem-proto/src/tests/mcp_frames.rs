use super::*;

// -------------------------------------------------------------------
// MCP frame envelope
// -------------------------------------------------------------------

#[test]
fn mcp_frame_header_preserves_exact_wire_without_owning_the_payload() {
    let header = encode_mcp_frame_header(42, 0, "codex", 2).unwrap();
    let golden = b"\0\0\0\x17MC\x01\x10\0\0\0\x2a\0\0\0\x05\0\0\0\x02codex";
    assert_eq!(header.as_bytes(), golden);
    let full = encode_mcp_frame(42, 0, "codex", b"{}").unwrap();
    assert_eq!(&full[..golden.len()], header.as_bytes());
    assert_eq!(&full[golden.len()..], b"{}");
}

#[test]
fn mcp_frame_header_checks_all_bounds_without_allocating_a_payload() {
    let name = "🧪".repeat(MCP_FRAME_MAX_PROCESS_NAME_LEN / 4);
    let limit = MCP_FRAME_MAX_SIZE - MCP_FRAME_HEADER_LEN as usize - name.len();
    let header = encode_mcp_frame_header(1, 0, &name, limit).unwrap();
    assert_eq!(header.as_bytes().len(), 4 + MCP_FRAME_HEADER_LEN as usize + name.len());
    assert!(encode_mcp_frame_header(1, 0, &name, limit + 1).is_err());
    assert!(encode_mcp_frame_header(1, 0, "codex", usize::MAX).is_err());
    assert!(encode_mcp_frame_header(1, 0, &"x".repeat(MCP_FRAME_MAX_PROCESS_NAME_LEN + 1), 0).is_err());
    for (stream, flags) in [(0, 0), (7, MCP_FRAME_FLAG_NOTIFICATION), (1, 0x8000)] {
        assert!(encode_mcp_frame_header(stream, flags, "codex", 2).is_err());
    }
    assert!(encode_mcp_frame_header(0, MCP_FRAME_FLAG_NOTIFICATION, "codex", 2).is_ok());
}

#[test]
fn mcp_correlation_metadata_leaves_room_for_an_escaped_error_reply() {
    const {
        assert!(MCP_REQUEST_ID_MAX_BYTES > 0);
        assert!(
            6 * MCP_REQUEST_ID_MAX_BYTES + 512
                < MCP_FRAME_MAX_SIZE - MCP_FRAME_HEADER_LEN as usize - MCP_FRAME_MAX_PROCESS_NAME_LEN
        );
    }
}

#[test]
fn mcp_request_identity_admission_bounds_metadata_without_sizing_json() {
    use crate::mcp_contracts::is_valid_mcp_request_id;
    let mut id = "🧪".repeat(MCP_REQUEST_ID_MAX_BYTES / 4);
    id.push_str(&"x".repeat(MCP_REQUEST_ID_MAX_BYTES % 4));
    assert!(is_valid_mcp_request_id(&serde_json::Value::String(id.clone())));
    id.push('x');
    assert!(!is_valid_mcp_request_id(&serde_json::Value::String(id)));
    for id in [
        serde_json::json!(i64::MIN),
        serde_json::json!(u64::MAX),
        serde_json::json!(""),
    ] {
        assert!(is_valid_mcp_request_id(&id));
    }
    for id in [
        serde_json::json!(null),
        serde_json::json!(true),
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!(1.25),
    ] {
        assert!(!is_valid_mcp_request_id(&id));
    }
}

#[test]
fn mcp_frame_roundtrip() {
    let payload = br#"{"jsonrpc":"2.0","id":7,"method":"tools/call"}"#;
    let encoded = encode_mcp_frame(42, 0, "claude", payload).unwrap();

    assert!(looks_like_mcp_frame_prefix(&encoded));
    let total_len = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]) as usize;
    assert_eq!(total_len, encoded.len() - 4);

    let decoded = decode_mcp_frame_body(&encoded[4..]).unwrap();
    assert_eq!(decoded.stream_id, 42);
    assert_eq!(decoded.flags, 0);
    assert_eq!(decoded.process_name, "claude");
    assert_eq!(decoded.payload, payload);
    assert!(!decoded.is_notification());
}

#[test]
fn mcp_frame_notification_reserved_stream() {
    let encoded = encode_mcp_frame(
        0,
        MCP_FRAME_FLAG_NOTIFICATION,
        "codex",
        br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .unwrap();
    let decoded = decode_mcp_frame_body(&encoded[4..]).unwrap();
    assert!(decoded.is_notification());
}

#[test]
fn mcp_frame_rejects_stream_zero_without_notification_flag() {
    assert!(encode_mcp_frame(0, 0, "codex", b"{}").is_err());
}

#[test]
fn mcp_frame_rejects_notification_flag_on_nonzero_stream() {
    assert!(encode_mcp_frame(7, MCP_FRAME_FLAG_NOTIFICATION, "codex", b"{}").is_err());
}

#[test]
fn mcp_frame_rejects_reserved_flags() {
    assert!(encode_mcp_frame(1, 0x8000, "codex", b"{}").is_err());
}

#[test]
fn mcp_frame_prefix_rejects_bad_magic() {
    let mut encoded = encode_mcp_frame(1, 0, "codex", b"{}").unwrap();
    encoded[4] = b'X';
    assert!(!looks_like_mcp_frame_prefix(&encoded));
    assert!(decode_mcp_frame_body(&encoded[4..]).is_err());
}

#[test]
fn mcp_frame_rejects_oversized_process_name() {
    let name = "x".repeat(MCP_FRAME_MAX_PROCESS_NAME_LEN + 1);
    assert!(encode_mcp_frame(1, 0, &name, b"{}").is_err());
}

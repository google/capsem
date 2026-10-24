use super::*;

// -------------------------------------------------------------------
// MCP frame envelope
// -------------------------------------------------------------------

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

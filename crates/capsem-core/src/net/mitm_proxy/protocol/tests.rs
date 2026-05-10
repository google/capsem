use super::{detect, Protocol};

#[test]
fn detects_tls_handshake_byte() {
    // TLS Handshake record type. ClientHello is the only valid first
    // record from the client side.
    assert_eq!(detect(&[0x16]), Some(Protocol::Tls));
    assert_eq!(detect(&[0x16, 0x03, 0x01, 0x00]), Some(Protocol::Tls));
}

#[test]
fn detects_uppercase_ascii_as_http() {
    // Real HTTP method first letters.
    for method in [
        "GET", "POST", "PUT", "HEAD", "DELETE", "OPTIONS", "PATCH", "TRACE", "CONNECT",
    ] {
        assert_eq!(
            detect(method.as_bytes()),
            Some(Protocol::Http),
            "method {method:?} should classify as Http"
        );
    }
}

#[test]
fn rejects_lowercase_method() {
    // HTTP/1.1 methods are case-sensitive uppercase. A lowercase
    // first byte is not a valid request line and should not classify
    // as Http -- a permissive detector would let junk in.
    assert_eq!(detect(b"get / HTTP/1.1\r\n"), None);
}

#[test]
fn rejects_other_tls_record_types() {
    // Non-Handshake TLS record types (ChangeCipherSpec=0x14,
    // Alert=0x15, AppData=0x17) are never the FIRST record from a
    // client. The listener must not accept them as TLS.
    assert_eq!(detect(&[0x14]), None);
    assert_eq!(detect(&[0x15]), None);
    assert_eq!(detect(&[0x17]), None);
}

#[test]
fn rejects_empty_buffer() {
    assert_eq!(detect(&[]), None);
}

#[test]
fn rejects_high_bit_junk() {
    assert_eq!(detect(&[0xff]), None);
    assert_eq!(detect(&[0x00]), None);
}

#[test]
fn detects_mcp_frame_prefix() {
    let frame = capsem_proto::encode_mcp_frame(
        1,
        0,
        "codex",
        br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    )
    .unwrap();
    assert_eq!(detect(&frame), Some(Protocol::McpFrame));
}

#[test]
fn label_round_trip() {
    assert_eq!(Protocol::Tls.label(), "tls");
    assert_eq!(Protocol::Http.label(), "http");
    assert_eq!(Protocol::McpFrame.label(), "mcp-frame");
    assert_eq!(Protocol::Unknown.label(), "unknown");
}

#[test]
fn default_is_unknown() {
    assert_eq!(Protocol::default(), Protocol::Unknown);
}

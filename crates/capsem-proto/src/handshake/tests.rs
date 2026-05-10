//! Tests for handshake::verify against version + schema mismatches.

use super::*;

#[test]
fn hello_serializes_compactly() {
    let h = Hello::ours("capsem-service-test", "");
    let bytes = rmp_serde::to_vec_named(&h).unwrap();
    // Sanity: small, fits well under MAX_FRAME_SIZE.
    assert!(
        bytes.len() < 256,
        "hello unexpectedly large: {}",
        bytes.len()
    );
    let decoded: Hello = rmp_serde::from_slice(&bytes).unwrap();
    assert_eq!(decoded, h);
}

#[test]
fn verify_matches_when_ours() {
    let h = Hello::ours("capsem-service-test", "");
    assert!(verify(&h).is_ok());
}

#[test]
fn verify_detects_version_mismatch() {
    let mut h = Hello::ours("capsem-process-old", "");
    h.version = h.version.wrapping_add(1);
    let err = verify(&h).unwrap_err();
    assert!(matches!(err, HandshakeError::Version { .. }), "{err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("capsem-process-old"),
        "msg should name peer: {msg}"
    );
}

#[test]
fn verify_detects_schema_mismatch() {
    let mut h = Hello::ours("capsem-process-rebuilt", "");
    h.schema_hash = h.schema_hash.wrapping_add(1);
    let err = verify(&h).unwrap_err();
    assert!(matches!(err, HandshakeError::Schema { .. }), "{err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("capsem-process-rebuilt"),
        "msg should name peer: {msg}"
    );
}

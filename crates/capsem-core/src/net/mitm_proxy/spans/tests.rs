use super::*;

#[test]
fn span_names_match_capsem_mitm_contract() {
    for name in [
        MITM_CONNECTION,
        MITM_REQUEST,
        MITM_VSOCK_CLASSIFY,
        MITM_TLS_GUEST_HANDSHAKE,
        MITM_POLICY_REQUEST,
        MITM_SECURITY_ACTIONS,
        MITM_MODEL_REQUEST_POLICY,
        MITM_UPSTREAM_PREPARE,
        MITM_UPSTREAM_SEND,
        MITM_POLICY_RESPONSE,
        MITM_MODEL_RESPONSE_POLICY,
        MITM_BODY_CHUNK_HOOKS,
        MITM_WEBSOCKET,
        MITM_TELEMETRY_EMIT,
    ] {
        assert!(name.starts_with("capsem.mitm."));
        assert!(!name.contains("host"));
        assert!(!name.contains("path"));
        assert!(!name.contains("url"));
    }
}

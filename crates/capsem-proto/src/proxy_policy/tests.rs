use super::*;

#[test]
fn request_is_path_free_and_bounded() {
    let request = ProxyPolicyRequest::apply(1, b"network = {}".to_vec());
    request.validate().unwrap();
    let encoded = rmp_serde::to_vec_named(&request).unwrap();
    let visible = String::from_utf8_lossy(&encoded);
    assert!(!visible.contains("path"));
    assert!(!visible.contains("session"));
    assert!(ProxyPolicyRequest::Apply {
        schema_hash: PROXY_SCHEMA_HASH,
        request_id: 2,
        active_policy: vec![0; MAX_ACTIVE_POLICY_BYTES + 1],
    }
    .validate()
    .is_err());
    assert!(ProxyPolicyRequest::Apply {
        schema_hash: PROXY_SCHEMA_HASH.wrapping_add(1),
        request_id: 3,
        active_policy: b"network = {}".to_vec(),
    }
    .validate()
    .is_err());
}

#[test]
fn rejection_diagnostic_is_utf8_bounded() {
    let response = ProxyPolicyResponse::rejected(3, "é".repeat(MAX_POLICY_ERROR_BYTES));
    let ProxyPolicyResponse::Rejected { request_id, error } = response else {
        panic!("expected rejection")
    };
    assert_eq!(request_id, 3);
    assert!(error.len() <= MAX_POLICY_ERROR_BYTES);
    assert!(std::str::from_utf8(error.as_bytes()).is_ok());
}

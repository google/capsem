use super::*;

fn request() -> ApprovalRequest {
    ApprovalRequest {
        ask_id: "0123456789ab".into(),
        event_id: "fedcba987654".into(),
        event_type: "http.request".into(),
        rule_id: "ask.http".into(),
        rule_name: "Approve HTTP".into(),
        summary: "HTTP POST example.test".into(),
        held_bytes: 12,
    }
}

#[test]
fn bounded_safe_request_round_trips_without_extra_payload_fields() {
    let request = request();
    assert_eq!(request.validate(), Ok(()));
    let bytes = rmp_serde::to_vec_named(&request).unwrap();
    assert_eq!(rmp_serde::from_slice::<ApprovalRequest>(&bytes).unwrap(), request);
}

#[test]
fn malformed_identity_context_and_held_memory_are_refused() {
    let mut value = request();
    value.ask_id = "not-an-id".into();
    assert!(value.validate().is_err());
    let mut value = request();
    value.summary = "bad\nnotification".into();
    assert!(value.validate().is_err());
    let mut value = request();
    value.summary = "x".repeat(crate::APPROVAL_CONTEXT_BYTES + 1);
    assert!(value.validate().is_err());
    let mut value = request();
    value.held_bytes = crate::APPROVAL_HELD_BYTES_PER_REQUEST as u64 + 1;
    assert!(value.validate().is_err());
}

#[test]
fn only_positive_durable_approval_can_allow() {
    for outcome in [
        ApprovalOutcome::Denied,
        ApprovalOutcome::Expired,
        ApprovalOutcome::Cancelled,
        ApprovalOutcome::Unavailable,
    ] {
        assert!(!outcome.is_allowed());
    }
    assert!(ApprovalOutcome::Approved.is_allowed());
}

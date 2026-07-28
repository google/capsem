use super::*;
use std::time::Duration;

#[test]
fn decision_roundtrip() {
    for decision in [
        Decision::Allowed,
        Decision::Denied,
        Decision::Error,
        Decision::Redirected,
    ] {
        assert_eq!(Decision::parse_str(decision.as_str()), decision);
    }
}

#[test]
fn decision_redirected_string() {
    assert_eq!(Decision::Redirected.as_str(), "redirected");
    assert_eq!(Decision::parse_str("redirected"), Decision::Redirected);
}

#[test]
fn decision_json_roundtrip() {
    let event = NetEvent {
        event_id: None,
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
        domain: "elie.net".to_string(),
        port: 443,
        decision: Decision::Allowed,
        process_name: None,
        pid: None,
        method: None,
        path: None,
        query: None,
        status_code: None,
        bytes_sent: 0,
        bytes_received: 0,
        duration_ms: 0,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        request_body_full: None,
        response_body_full: None,
        conn_type: None,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: None,
        credential_ref: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    let decoded: NetEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.decision, Decision::Allowed);
    assert_eq!(decoded.domain, "elie.net");
}

#[test]
fn decision_unknown_str() {
    assert_eq!(Decision::parse_str("bogus"), Decision::Error);
    assert_eq!(Decision::parse_str(""), Decision::Error);
}

#[test]
fn file_action_roundtrip() {
    for action in [
        FileAction::Created,
        FileAction::Modified,
        FileAction::Deleted,
        FileAction::Restored,
        FileAction::Read,
        FileAction::Imported,
        FileAction::Exported,
    ] {
        assert_eq!(FileAction::parse_str(action.as_str()), action);
    }
}

#[test]
fn file_action_unknown_str() {
    assert_eq!(FileAction::parse_str("bogus"), FileAction::Modified);
    assert_eq!(FileAction::parse_str(""), FileAction::Modified);
}

/// "error" must be an explicit match arm, not caught by the _ wildcard.
/// This ensures adding future variants (e.g. Timeout) won't silently
/// map their as_str() to Decision::Error via the catchall.
#[test]
fn decision_from_str_explicitly_matches_error() {
    // "error" should match explicitly, not via _ => Error.
    assert_eq!(Decision::parse_str("error"), Decision::Error);
    // Verify the roundtrip: as_str -> from_str for all variants.
    assert_eq!(Decision::parse_str("allowed"), Decision::Allowed);
    assert_eq!(Decision::parse_str("denied"), Decision::Denied);
    assert_eq!(Decision::parse_str("error"), Decision::Error);
}

#[test]
fn credential_reference_is_domain_separated_and_stable() {
    let raw = "sk-test-credential";
    let openai = credential_reference("openai", raw);
    let openai_again = credential_reference("openai", raw);
    let github = credential_reference("github", raw);

    assert_eq!(openai, openai_again);
    assert_ne!(openai, github);
    assert!(is_credential_reference(&openai));
    assert!(!is_credential_reference(raw));
    assert!(openai.starts_with(CREDENTIAL_REF_PREFIX));
}

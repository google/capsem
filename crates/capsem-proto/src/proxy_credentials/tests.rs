use super::*;

#[test]
fn bounded_capture_and_substitution_requests_validate() {
    let capture = ProxyCredentialRequest::Capture {
        request_id: 1,
        observation: ProxyCredentialObservation {
            provider: ProxyCredentialProvider::OpenAi,
            raw_value: "secret".to_string(),
            source: "http.header.authorization".to_string(),
            event_type: Some("http.request".to_string()),
            trace_id: None,
            context_json: None,
        },
    };
    capture.validate().unwrap();

    let substitute = ProxyCredentialRequest::Substitute {
        request_id: 2,
        domain: "api.openai.com".to_string(),
        ai_provider: Some(ProxyModelProvider::OpenAi),
        headers: vec![ProxyHeader::new(
            "authorization",
            b"Bearer credential:blake3:abc".to_vec(),
        )],
        query: Some("key=credential%3Ablake3%3Aabc".to_string()),
    };
    substitute.validate().unwrap();
    assert_eq!(capture.request_id(), 1);
    assert_eq!(substitute.request_id(), 2);
}

#[test]
fn optional_fields_are_omitted_from_named_frames() {
    let observation = ProxyCredentialObservation {
        provider: ProxyCredentialProvider::OpenAi,
        raw_value: "secret".into(),
        source: "header".into(),
        event_type: None,
        trace_id: None,
        context_json: None,
    };
    let encoded = rmp_serde::to_vec_named(&observation).unwrap();
    let fields: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(
        fields,
        serde_json::json!({"provider":"open_ai", "raw_value":"secret", "source":"header"})
    );
}

#[test]
fn oversized_or_uncorrelated_values_fail_closed() {
    let request = ProxyCredentialRequest::Capture {
        request_id: 0,
        observation: ProxyCredentialObservation {
            provider: ProxyCredentialProvider::Google,
            raw_value: "x".repeat(MAX_CREDENTIAL_VALUE_BYTES + 1),
            source: "test".to_string(),
            event_type: None,
            trace_id: None,
            context_json: None,
        },
    };
    assert!(request.validate().is_err());
    assert!(ProxyCredentialResponse::rejected(0, "no").is_err());
    assert!(ProxyCredentialResponse::rejected(1, "x".repeat(MAX_CREDENTIAL_ERROR_BYTES + 1)).is_err());
}

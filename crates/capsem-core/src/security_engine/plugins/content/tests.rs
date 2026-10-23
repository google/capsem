use super::*;

#[test]
fn reviewed_detectors_validate_and_mask_without_exposing_matches() {
    let options = ContentFilterConfig::default();
    for value in ["user@example.com", "212-555-1234", "4111 1111 1111 1111", "219-09-9999"] {
        let masked = inspect_bytes(value.as_bytes(), options, false, true).expect(value);
        assert!(!masked.windows(value.len()).any(|bytes| bytes == value.as_bytes()));
    }
    for value in [
        "4111 1111 1111 1112",
        "000-12-3456",
        "219-00-9999",
        "123-45-6789",
        "plain words",
    ] {
        assert!(
            inspect_bytes(value.as_bytes(), options, false, true).is_none(),
            "{value}"
        );
    }
    let options = ContentFilterConfig {
        email: false,
        ..options
    };
    assert!(inspect_bytes(b"user@example.com", options, false, true).is_none());
}

#[test]
fn pii_rewrite_keeps_unquoted_json_integer_valid() {
    let masked = inspect_bytes(
        br#"{"card":4111111111111111,"email":"user@example.com"}"#,
        ContentFilterConfig::default(),
        false,
        true,
    )
    .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&masked).unwrap();
    assert_eq!(json["card"], 1111111111111111_u64);
}

#[test]
fn provider_tokens_and_private_keys_are_detected_separately_from_pii() {
    for value in [
        "ghp_0123456789abcdefghijklmnopqrstuvwxyz",
        "credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ] {
        assert!(inspect_bytes(value.as_bytes(), ContentFilterConfig::default(), true, false).is_some());
        assert!(inspect_bytes(value.as_bytes(), ContentFilterConfig::default(), false, false).is_none());
    }
    assert!(
        inspect_bytes(
            b"ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ContentFilterConfig::default(),
            true,
            false
        )
        .is_none(),
        "upstream entropy floor is retained"
    );
}

#[test]
fn direction_selection_does_not_filter_unselected_direction() {
    let event = SecurityEvent::new(crate::security_engine::RuntimeSecurityEventType::HttpRequest).with_http(
        crate::security_engine::HttpSecurityEvent {
            status: Some("200".into()),
            body: Some("user@example.com".into()),
            ..Default::default()
        },
    );
    let config = SecurityPluginConfig {
        mode: SecurityPluginMode::Block,
        content: ContentFilterConfig {
            direction: capsem_config::ContentDirection::Outbound,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(!PiiFilterPlugin.apply(event, config).unwrap().applied);
}

#[test]
fn provider_credential_slot_is_allowed_but_custom_header_copy_is_exfiltration() {
    let token = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
    let config = SecurityPluginConfig {
        mode: SecurityPluginMode::Block,
        ..Default::default()
    };
    let make = |domain, name: &str| {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        SecurityEvent::new(crate::security_engine::RuntimeSecurityEventType::HttpRequest)
            .with_http(crate::security_engine::HttpSecurityEvent::default())
            .with_http_request(crate::security_engine::HttpRequestSecurityEvent::new(
                domain, None, headers, None,
            ))
    };
    assert!(
        !SecretExfiltrationPlugin
            .apply(make("api.github.com", "authorization"), config)
            .unwrap()
            .applied
    );
    assert!(
        SecretExfiltrationPlugin
            .apply(make("api.github.com", "x-copy"), config)
            .unwrap()
            .applied
    );
    assert!(
        SecretExfiltrationPlugin
            .apply(make("api.github.com.evil.example", "authorization"), config)
            .unwrap()
            .applied
    );
}

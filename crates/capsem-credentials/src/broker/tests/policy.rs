use super::*;

#[test]
fn bundled_limits_are_finite_and_closed_to_unknown_fields() {
    let limits = BrokerLimits::bundled().unwrap();
    assert!(limits.max_profiles > 0 && limits.max_capabilities > 0 && limits.max_scope_bytes > 0);
    assert_eq!(limits.retention_lifetime, Duration::from_secs(86400));
    let source = include_str!("../../../../../config/data/credential-broker-policy.json");
    let mut value: serde_json::Value = serde_json::from_str(source).unwrap();
    value["surprise"] = serde_json::json!(true);
    assert!(BrokerLimits::parse(&value.to_string()).is_err());
    value.as_object_mut().unwrap().remove("surprise");
    value["max_profiles"] = serde_json::json!(0);
    assert!(BrokerLimits::parse(&value.to_string()).is_err());
}

#[test]
fn broker_storage_and_maintenance_limits_are_config_owned_and_finite() {
    let source = include_str!("../../../../../config/data/credential-broker-policy.json");
    let mut value: serde_json::Value = serde_json::from_str(source).unwrap();
    value["max_metadata_bytes"] = serde_json::json!(8192);
    value["maintenance_interval"] = serde_json::json!(7);
    let limits = BrokerLimits::parse(&value.to_string()).unwrap();
    assert_eq!(limits.max_metadata_bytes, 8192);
    assert_eq!(limits.maintenance_interval, Duration::from_secs(7));
    for field in ["max_metadata_bytes", "maintenance_interval"] {
        let mut invalid = value.clone();
        invalid[field] = serde_json::json!(0);
        assert!(BrokerLimits::parse(&invalid.to_string()).is_err());
    }
}

#[test]
fn grant_payload_normalization_preserves_instants_and_rejects_duplicates() {
    let mut request = api::CreateSessionGrantRequest {
        request_id: capsem_crypto::uuid_v4().to_string(),
        expected_owner_generation: capsem_crypto::uuid_v4().to_string(),
        connection_id: capsem_crypto::uuid_v4().to_string(),
        capability_ids: vec!["b".into(), "a".into()],
        expires_at: Some("2026-10-14T01:00:00.000001+01:00".into()),
    };
    let original = humantime::parse_rfc3339("2026-10-14T00:00:00.000001Z").unwrap();
    normalize_grant_request(&mut request, BrokerLimits::bundled().unwrap()).unwrap();
    assert_eq!(request.capability_ids, vec!["a", "b"]);
    assert_eq!(
        humantime::parse_rfc3339(request.expires_at.as_ref().unwrap()).unwrap(),
        original
    );
    let mut equivalent = request.clone();
    equivalent.capability_ids.reverse();
    equivalent.expires_at = Some("2026-10-14T00:00:00.000001Z".into());
    normalize_grant_request(&mut equivalent, BrokerLimits::bundled().unwrap()).unwrap();
    assert_eq!(request, equivalent);
    request.capability_ids = vec!["a".into(), "a".into()];
    assert_eq!(
        normalize_grant_request(&mut request, BrokerLimits::bundled().unwrap()),
        Err(api::CredentialErrorCode::UnsupportedCapability)
    );
}

#[test]
fn shared_grant_scope_admission_uses_documented_google_aliases() {
    let capability = GrantCapability::new(
        "identity".into(),
        BTreeSet::from([
            "email".into(),
            "https://www.googleapis.com/auth/userinfo.email".into(),
            "openid".into(),
        ]),
        vec![crate::GrantTarget::new("GET", "https://example.test/identity").unwrap()],
    )
    .unwrap();
    assert_eq!(
        capability.required_scopes(),
        &BTreeSet::from(["https://www.googleapis.com/auth/userinfo.email".into(), "openid".into()])
    );
}

#[test]
fn profiles_apply_budgets_before_deduplication_and_match_canonical_scopes() {
    let mut summary = api::ScopeProfileSummary {
        id: "unconfigured".into(),
        enrollment_kind: api::EnrollmentKind::NativeConsumerSignIn,
        label: "Native".into(),
        application_label: "Official component".into(),
        requested_scopes: vec!["email".into()],
        capabilities: vec![api::CapabilitySummary {
            id: "identity".into(),
            label: "Identity".into(),
            consumer: "fixture".into(),
            required_scopes: vec!["https://www.googleapis.com/auth/userinfo.email".into()],
            supported: false,
        }],
        enabled: false,
    };
    let profile = BrokerProfile::unconfigured(summary.clone()).unwrap();
    assert_eq!(
        profile.summary.requested_scopes,
        vec!["https://www.googleapis.com/auth/userinfo.email"]
    );
    let mut small = BrokerLimits::bundled().unwrap();
    small.max_identifier_bytes = 4;
    assert_eq!(
        BrokerProfile::validate(&profile.summary, small),
        Err(api::CredentialErrorCode::ConfigurationRequired)
    );
    summary.requested_scopes = vec!["email".into(); BrokerLimits::bundled().unwrap().max_scopes + 1];
    assert!(
        BrokerProfile::unconfigured(summary.clone()).is_err(),
        "duplicate aliases cannot evade input budgets"
    );
    summary.requested_scopes = vec!["email".into()];
    summary.label = "bad\nlabel".into();
    assert!(BrokerProfile::unconfigured(summary).is_err());
}

use super::*;
use serde_json::json;

#[test]
fn authorization_contract_rejects_secrets_and_noncanonical_request_ids() {
    let valid = json!({"request_id":"12345678-1234-4234-8234-123456789abc", "scope_profile_id":"agy", "reconnect_connection_id":null});
    let parsed: AuthorizeGoogleRequest = serde_json::from_value(valid.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), valid);
    for field in ["access_token", "provider_url", "approval_token"] {
        let mut invalid = valid.clone();
        invalid[field] = json!("secret");
        assert!(serde_json::from_value::<AuthorizeGoogleRequest>(invalid).is_err());
    }
    for id in [
        "",
        "12345678-1234-4234-8234-123456789ABC",
        "../other",
        "00000000-0000-0000-0000-000000000000",
    ] {
        let mut invalid = valid.clone();
        invalid["request_id"] = json!(id);
        assert!(serde_json::from_value::<AuthorizeGoogleRequest>(invalid).is_err());
    }
}

#[test]
fn connection_contract_rejects_unsafe_revisions_and_raw_tokens() {
    let valid = json!({"id":"connection-1", "provider":"google", "scope_profile_id":"agy", "account_label":null,
        "state":"connected", "revision":1, "granted_scopes":["openid"], "available_capabilities":[],
        "created_at":"2026-10-14T01:00:00Z", "updated_at":"2026-10-14T01:00:00Z", "last_refresh_at":null,
        "provider_revocation":"not_started", "error_code":null});
    let mut parsed: ConnectionSummary = serde_json::from_value(valid.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), valid);
    parsed.revision = 9007199254740992_u64;
    assert!(
        serde_json::to_value(parsed).is_err(),
        "A server-created revision must not lose precision on the wire"
    );
    for revision in [json!(-1), json!(9007199254740992_u64), json!("1")] {
        let mut invalid = valid.clone();
        invalid["revision"] = revision;
        assert!(serde_json::from_value::<ConnectionSummary>(invalid).is_err());
    }
    let mut invalid = valid;
    invalid["refresh_token"] = json!("secret");
    assert!(serde_json::from_value::<ConnectionSummary>(invalid).is_err());
}

#[test]
fn explicit_grant_contract_requires_no_separate_approval_token() {
    let valid = json!({"request_id":"12345678-1234-5234-8234-123456789abc",
        "expected_owner_generation":"12345678-1234-4234-8234-123456789abc", "connection_id":"connection-1",
        "capability_ids":["agy"], "expires_at":null});
    let parsed: CreateSessionGrantRequest = serde_json::from_value(valid.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), valid);
    for field in ["approval_token", "approved", "credential_ref", "access_token"] {
        let mut invalid = valid.clone();
        invalid[field] = json!("unexpected");
        assert!(serde_json::from_value::<CreateSessionGrantRequest>(invalid).is_err());
    }
    let mut invalid = valid;
    invalid["expected_owner_generation"] = json!("../unrelated-owner");
    assert!(serde_json::from_value::<CreateSessionGrantRequest>(invalid).is_err());
}

#[test]
fn generated_connection_schema_is_closed_and_keeps_nullable_fields_required() {
    let schema = serde_json::to_value(<ConnectionSummary as utoipa::PartialSchema>::schema()).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["revision"]["maximum"].as_f64(),
        Some(9007199254740991_f64)
    );
    let required = schema["required"].as_array().unwrap();
    for field in ["account_label", "last_refresh_at", "error_code"] {
        assert!(
            required.contains(&json!(field)),
            "Nullable {field} remains an explicit response field"
        );
    }
}

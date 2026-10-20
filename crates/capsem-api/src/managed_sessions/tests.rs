use super::*;
use serde_json::json;

const REQUEST: &str = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61";

#[test]
fn canonical_managed_identity_validation_is_shared_without_echoing_input() {
    assert!(validate_managed_request_id(REQUEST).is_ok());
    for value in [
        "",
        "private-invalid-identity",
        "FC120782-2272-4B3D-A2E6-A26EE2F8AB61",
        "fc120782-2272-1b3d-a2e6-a26ee2f8ab61",
        "fc120782-2272-4b3d-02e6-a26ee2f8ab61",
        "00000000-0000-0000-0000-000000000000",
    ] {
        let error = validate_managed_request_id(value).unwrap_err();
        assert_eq!(error, "managed request id must be a canonical UUIDv4");
    }
}

#[test]
fn optional_managed_create_is_validated_redacted_and_preserves_ordinary_wire_defaults() {
    let secret = "af".repeat(32);
    let wire = json!({"managed": {"request_id": REQUEST, "ownership_token": secret}, "persistent": false});
    let create: crate::ProvisionRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&create).unwrap()["managed"], wire["managed"]);
    assert!(!format!("{create:?}").contains(&secret));
    assert!(serde_json::from_value::<crate::ProvisionRequest>(json!({
        "managed": {"request_id": "invalid", "ownership_token": secret}
    }))
    .is_err());
    let ordinary: crate::ProvisionRequest = serde_json::from_value(json!({})).unwrap();
    assert_eq!(
        serde_json::to_value(ordinary).unwrap(),
        json!({"name": null, "persistent": false})
    );
}

#[test]
fn provision_response_roundtrips_optional_nonsecret_ownership_metadata() {
    let wire = json!({
        "id": "vm-1", "name": "vm-1", "status": "Running", "persistent": false,
        "can_resume": false, "available_actions": ["stop"],
        "ownership": {"request_id": REQUEST, "generation": REQUEST,
            "expires_at": "2026-10-13T20:00:00Z", "lease_seconds": 300, "renew_after_seconds": 60}
    });
    let response: crate::ProvisionResponse = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), wire);
    let ordinary = crate::ProvisionResponse::new(
        "vm-1".into(),
        "vm-1".into(),
        crate::VmLifecycleState::Running,
        false,
        false,
        vec![crate::VmAction::Stop],
    );
    assert!(serde_json::to_value(ordinary).unwrap().get("ownership").is_none());
}

#[test]
fn managed_requests_redact_secrets_but_keep_the_explicit_wire_body() {
    let secret = "af".repeat(32);
    let create: ManagedSessionCreate =
        serde_json::from_value(json!({"request_id": REQUEST, "ownership_token": secret})).unwrap();
    assert!(!format!("{create:?}").contains(&secret));
    assert_eq!(serde_json::to_value(&create).unwrap()["ownership_token"], secret);
    assert_eq!(create.ownership_token.as_str(), secret);
    assert_eq!(ManagedOwnershipToken::new(secret.clone()).unwrap().as_str(), secret);

    let claim: ManagedSessionClaimRequest = serde_json::from_value(json!({"ownership_token": secret})).unwrap();
    assert!(!format!("{claim:?}").contains(&secret));
    let close: ManagedSessionCloseRequest =
        serde_json::from_value(json!({"ownership_token": secret, "outcome": "cancelled"})).unwrap();
    assert!(!format!("{close:?}").contains(&secret));
    assert_eq!(close.outcome, ManagedSessionOutcome::Cancelled);
}

#[test]
fn managed_request_validation_never_echoes_invalid_secret_material() {
    for secret in ["private-token".into(), "A".repeat(64), "a".repeat(63), "a".repeat(65)] {
        let error =
            serde_json::from_value::<ManagedSessionClaimRequest>(json!({"ownership_token": secret})).unwrap_err();
        assert!(!error.to_string().contains(&secret));
    }
    for request in [
        "bad",
        "FC120782-2272-4B3D-A2E6-A26EE2F8AB61",
        "00000000-0000-0000-0000-000000000000",
    ] {
        assert!(serde_json::from_value::<ManagedSessionCreate>(
            json!({"request_id": request, "ownership_token": "af".repeat(32)})
        )
        .is_err());
    }
    assert!(serde_json::from_value::<ManagedSessionCloseRequest>(
        json!({"ownership_token": "af".repeat(32), "outcome": "unknown"})
    )
    .is_err());
    assert!(serde_json::from_value::<ManagedSessionClaimRequest>(
        json!({"ownership_token": "af".repeat(32), "unexpected": true})
    )
    .is_err());
}

#[test]
fn managed_schemas_keep_secrets_write_only_and_close_acknowledgements_terminal() {
    let create = serde_json::to_value(<ManagedSessionCreate as utoipa::PartialSchema>::schema()).unwrap();
    assert_eq!(create["properties"]["ownership_token"]["writeOnly"], true);
    assert_eq!(create["properties"]["ownership_token"]["type"], "string");
    for schema in [
        serde_json::to_value(<ManagedSessionOwnership as utoipa::PartialSchema>::schema()).unwrap(),
        serde_json::to_value(<ManagedSessionCloseResponse as utoipa::PartialSchema>::schema()).unwrap(),
    ] {
        assert!(schema["properties"].get("ownership_token").is_none());
    }
    let closed = json!({
        "request_id": REQUEST, "generation": REQUEST, "state": "closed", "evidence_retained": false,
    });
    assert!(serde_json::from_value::<ManagedSessionCloseResponse>(closed.clone()).is_ok());
    let mut pending = closed;
    pending["state"] = json!("closing");
    assert!(serde_json::from_value::<ManagedSessionCloseResponse>(pending).is_err());
    assert_eq!(
        serde_json::to_value(crate::ProvisionRequest::default()).unwrap(),
        json!({"name": null, "persistent": false})
    );
}

use super::*;

#[test]
fn ask_routes_export_typed_generation_bound_resolutions() {
    let schema = serde_json::to_value(crate::openapi()).unwrap();
    assert_eq!(
        schema["paths"]["/vms/{id}/security/asks"]["get"]["operationId"],
        "listSecurityAsks"
    );
    assert_eq!(
        schema["paths"]["/vms/{id}/security/asks/{ask_id}/resolve"]["post"]["operationId"],
        "resolveSecurityAsk"
    );
}

#[test]
fn resolution_rejects_unbounded_extra_context_and_unknown_decisions() {
    let valid = serde_json::json!({"generation":"00000000-0000-4000-8000-000000000001","decision":"approve"});
    assert_eq!(
        serde_json::from_value::<SecurityAskResolveRequest>(valid.clone())
            .unwrap()
            .decision,
        SecurityAskDecision::Approve
    );
    let mut extra = valid.clone();
    extra["reason"] = "secret request contents".into();
    assert!(serde_json::from_value::<SecurityAskResolveRequest>(extra).is_err());
    let mut unknown = valid;
    unknown["decision"] = "allow_always".into();
    assert!(serde_json::from_value::<SecurityAskResolveRequest>(unknown).is_err());
}

#[test]
fn resolution_requires_exact_canonical_generation() {
    assert!(SecurityAskResolveRequest {
        generation: "display-name".into(),
        decision: SecurityAskDecision::Approve
    }
    .validate()
    .is_err());
    assert!(SecurityAskResolveRequest {
        generation: "00000000-0000-4000-8000-000000000001".into(),
        decision: SecurityAskDecision::Deny
    }
    .validate()
    .is_ok());
}

use super::*;
use serde_json::json;

#[test]
fn request_resolution_requires_observed_identity_without_accepting_actor_claims() {
    for observed in ["abcdef012345", "00000000-0000-4000-8000-000000000001"] {
        for decision in [RequestDecision::Approve, RequestDecision::Deny, RequestDecision::Cancel] {
            let request = RequestResolveRequest {
                observed_revision: observed.into(),
                decision,
                reason: "private-reason-canary".into(),
            };
            request.validate().unwrap();
            let wire = serde_json::to_value(&request).unwrap();
            assert_eq!(
                serde_json::from_value::<RequestResolveRequest>(wire.clone()).unwrap(),
                request
            );
            assert!(!format!("{request:?}").contains("private-reason-canary"));
            let mut forged = wire;
            forged["actor"] = json!("operator");
            assert!(serde_json::from_value::<RequestResolveRequest>(forged).is_err());
        }
    }
    for observed in [
        "",
        "server",
        "ABCDEF012345",
        "abcdef0123456",
        "00000000-0000-4000-0000-000000000001",
    ] {
        assert!(RequestResolveRequest {
            observed_revision: observed.into(),
            decision: RequestDecision::Approve,
            reason: "reviewed".into()
        }
        .validate()
        .is_err());
    }
    for reason in [
        String::new(),
        "x\0y".into(),
        "x".repeat(capsem_proto::APPROVAL_CONTEXT_BYTES + 1),
    ] {
        assert!(RequestResolveRequest {
            observed_revision: "abcdef012345".into(),
            decision: RequestDecision::Cancel,
            reason
        }
        .validate()
        .is_err());
    }
}

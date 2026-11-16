use super::*;
use serde_json::json;

const ACCESS: &str = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61";
const ENV: &str = "bd220782-2272-4b3d-a2e6-a26ee2f8ab61";
const VM: &str = "ab320782-2272-4b3d-a2e6-a26ee2f8ab61";
const GENERATION: &str = "cd420782-2272-4b3d-a2e6-a26ee2f8ab61";

fn scope() -> serde_json::Value {
    json!({"operations": ["discover", "inspect", "execute", "launch"],
        "environment_ids": [ENV], "sessions": [{"session_id": VM, "owner_generation": GENERATION}]})
}

#[test]
fn scope_names_exact_operations_environments_and_vm_generations() {
    let wire = scope();
    let scope: AgentAccessScope = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&scope).unwrap(), wire);
    assert!(scope.permits(AgentOperation::Execute));
    assert!(!scope.permits(AgentOperation::Propose));
    assert!(scope.permits_launch(ENV));
    assert!(!scope.permits_launch(ACCESS));
    assert!(!scope.permits_launch("*"));
    assert!(scope.permits_session(VM, GENERATION));
    assert!(!scope.permits_session(VM, ACCESS));
    assert!(!scope.permits_session(ACCESS, GENERATION));
}

#[test]
fn scopes_reject_privilege_fields_wildcards_duplicates_and_unbounded_lists() {
    let mut cases = vec![
        json!({"operations": []}),
        json!({"operations": ["approve"]}),
        json!({"operations": ["inspect"], "actor": {"kind": "operator"}}),
        json!({"operations": ["inspect", "inspect"]}),
        json!({"operations": ["launch"]}),
        json!({"operations": ["inspect"], "environment_ids": ["*"]}),
        json!({"operations": ["inspect"], "environment_ids": [ENV, ENV]}),
        json!({"operations": ["inspect"], "sessions": [{"session_id": VM, "owner_generation": "*"}]}),
        json!({"operations": ["inspect"], "sessions": [{"session_id": VM, "owner_generation": GENERATION, "operator": true}]}),
    ];
    let binding = json!({"session_id": VM, "owner_generation": GENERATION});
    cases.push(json!({"operations": ["inspect"], "sessions": [binding, binding]}));
    cases.push(json!({"operations": ["inspect"], "environment_ids": vec![ENV; capsem_proto::AGENT_ACCESS_MAX_ENVIRONMENT_IDS + 1]}));
    cases.push(json!({"operations": ["inspect"], "sessions": vec![binding; capsem_proto::AGENT_ACCESS_MAX_SESSION_BINDINGS + 1]}));
    for wire in cases {
        assert!(
            serde_json::from_value::<AgentAccessScope>(wire.clone()).is_err(),
            "{wire}"
        );
    }
}

#[test]
fn absent_targets_never_grant_implicit_access() {
    let scope: AgentAccessScope = serde_json::from_value(json!({"operations": ["discover"]})).unwrap();
    assert!(!scope.permits_launch(ENV));
    assert!(!scope.permits_session(VM, GENERATION));
    assert!(scope.environment_ids().is_empty());
    assert!(scope.sessions().is_empty());
}

#[test]
fn operator_issuance_reuses_redacted_write_only_managed_capabilities_and_bounded_lease() {
    let secret = "ab".repeat(32);
    let wire = json!({"request_id": ACCESS, "ownership_token": secret, "scope": scope(), "lease_seconds": 300});
    let request: CreateAgentAccessRequest = serde_json::from_value(wire.clone()).unwrap();
    assert!(!format!("{request:?}").contains(&secret));
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    let response = CreateAgentAccessResponse {
        access: AgentAccessSummary {
            id: ACCESS.into(),
            scope: request.scope,
            expires_at: "2026-11-16T20:00:00Z".into(),
        },
    };
    let response = serde_json::to_value(response).unwrap().to_string();
    assert!(!response.contains(&secret));
    assert!(!response.contains("ownership_token"));
    for (field, bad) in [
        ("lease_seconds", json!(0)),
        ("lease_seconds", json!(capsem_proto::AGENT_ACCESS_MAX_LEASE_SECONDS + 1)),
        ("lease_seconds", json!(-1)),
        ("request_id", json!("*")),
        ("ownership_token", json!("private-invalid-token")),
        ("actor", json!({"kind": "operator"})),
    ] {
        let mut bad_wire = wire.clone();
        bad_wire[field] = bad;
        let error = serde_json::from_value::<CreateAgentAccessRequest>(bad_wire)
            .unwrap_err()
            .to_string();
        assert!(!error.contains(&secret));
        assert!(!error.contains("private-invalid-token"));
    }
}

#[test]
fn trusted_actor_evidence_distinguishes_agents_and_owner_generations() {
    assert_eq!(CallerActor::Operator.ledger_id(), "operator");
    assert_eq!(CallerActor::Corporate.ledger_id(), "corporate");
    assert_eq!(
        CallerActor::DelegatedAgent {
            access_id: ACCESS.into()
        }
        .ledger_id(),
        format!("agent:{ACCESS}")
    );
    assert_eq!(
        CallerActor::VmOwner {
            session_id: VM.into(),
            owner_generation: GENERATION.into()
        }
        .ledger_id(),
        format!("vm:{VM}/{GENERATION}")
    );
}

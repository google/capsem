use super::*;
use serde_json::json;
use utoipa::OpenApi;

fn id(value: u8) -> String {
    format!("00000000-0000-4000-8000-{value:012x}")
}
fn submission() -> serde_json::Value {
    json!({
        "request_id": id(1), "reason":"Set up development",
        "specification": {"name":"development", "image":{"reference":"development:stable"},
            "startup":{"script":"printf '%s\\n' 'exact bytes'\n", "timeout_seconds":30},
            "variables":{"WORKFLOW":"private-variable-canary"}}
    })
}

#[test]
fn environment_submissions_roundtrip_selector_intent_without_sensitive_debug() {
    let create: EnvironmentCreateRequest = serde_json::from_value(submission()).unwrap();
    create.validate().unwrap();
    let encoded = serde_json::to_value(&create).unwrap();
    assert_eq!(
        encoded["specification"]["startup"]["script"],
        submission()["specification"]["startup"]["script"]
    );
    assert!(!format!("{create:?}").contains("private-variable-canary"));
    let mut invalid = create;
    invalid.request_id = "private-mistyped-id-canary".into();
    assert!(!format!("{invalid:?}").contains("private-mistyped-id-canary"));
    let mut change = submission();
    change["base_revision_id"] = json!(id(2));
    change["requesting_sandbox"] = json!({"session_id":id(3),"generation":id(4)});
    let change: EnvironmentChangeRequest = serde_json::from_value(change).unwrap();
    change.validate().unwrap();
    assert_eq!(change.requesting_sandbox.unwrap().generation, id(4));
}

#[test]
fn environment_submissions_refuse_forged_authority_resolution_and_invalid_intent() {
    for (path, key, value) in [
        ("", "actor", json!("operator")),
        ("", "environment_id", json!(id(10))),
        ("", "revision_id", json!(id(11))),
        ("specification", "identity", json!({"id":id(12)})),
        (
            "specification.image",
            "resolved_digest",
            json!(format!("sha256:{}", "a".repeat(64))),
        ),
    ] {
        let mut input = submission();
        let node = match path {
            "" => &mut input,
            "specification" => &mut input["specification"],
            _ => &mut input["specification"]["image"],
        };
        node[key] = value;
        assert!(
            serde_json::from_value::<EnvironmentCreateRequest>(input).is_err(),
            "{key}"
        );
    }
    for (field, value) in [
        ("request_id", json!("not-an-id")),
        ("reason", json!(" ")),
        ("reason", json!("x".repeat(capsem_proto::APPROVAL_CONTEXT_BYTES + 1))),
    ] {
        let mut input = submission();
        input[field] = value;
        assert!(serde_json::from_value::<EnvironmentCreateRequest>(input)
            .unwrap()
            .validate()
            .is_err());
    }
    let mut input = submission();
    input["specification"]["plugins"] = json!({"credential_broker":{"mode":"disable"}});
    assert!(serde_json::from_value::<EnvironmentCreateRequest>(input)
        .unwrap()
        .validate()
        .is_err());
    let target = EnvironmentSandboxTarget {
        session_id: id(3),
        generation: "stale-name".into(),
    };
    assert!(target.validate().is_err());
}

#[derive(OpenApi)]
#[openapi(components(schemas(
    EnvironmentCreateRequest,
    EnvironmentChangeRequest,
    EnvironmentMutationResponse,
    EnvironmentRevisionReference,
    capsem_config::environment::EnvironmentDocument,
    crate::requests::RequestResolveRequest
)))]
struct Contract;

fn check_refs(value: &serde_json::Value, schemas: &serde_json::Value) {
    match value {
        serde_json::Value::Object(values) => {
            if let Some(reference) = values.get("$ref").and_then(serde_json::Value::as_str) {
                let name = reference.strip_prefix("#/components/schemas/").unwrap();
                assert!(schemas.get(name).is_some(), "missing DTO schema {name}");
            }
            for value in values.values() {
                check_refs(value, schemas);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                check_refs(value, schemas);
            }
        }
        _ => {}
    }
}
#[test]
fn environment_dto_schemas_collect_complete_selector_and_canonical_graphs() {
    let document = serde_json::to_value(Contract::openapi()).unwrap();
    let schemas = &document["components"]["schemas"];
    check_refs(schemas, schemas);
    assert_eq!(schemas["EnvironmentCreateRequest"]["additionalProperties"], false);
    assert!(schemas
        .as_object()
        .unwrap()
        .keys()
        .any(|name| name.starts_with("EnvironmentSpecification_EnvironmentImageSelection")));
    assert!(schemas
        .as_object()
        .unwrap()
        .keys()
        .any(|name| name.starts_with("EnvironmentSpecification_EnvironmentImage_")));
}

#[test]
fn mutation_response_separates_approved_decision_from_unapplied_effect() {
    let wire = json!({"request_id":id(1), "environment_id":id(2),
        "decision":"approved", "application":"pending", "result":null,
        "application_error":"private-application-diagnostic"});
    let response: EnvironmentMutationResponse = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(response.decision, crate::requests::RequestState::Approved);
    assert_eq!(response.application, crate::requests::RequestApplicationState::Pending);
    assert!(response.result.is_none());
    assert_eq!(serde_json::to_value(&response).unwrap(), wire);
    assert!(!format!("{response:?}").contains("private-application-diagnostic"));
    for (key, value) in [("decision", "applied"), ("application", "approved")] {
        let mut invalid = wire.clone();
        invalid[key] = json!(value);
        assert!(serde_json::from_value::<EnvironmentMutationResponse>(invalid).is_err());
    }
    let mut applied = response;
    applied.application = crate::requests::RequestApplicationState::Applied;
    applied.result = Some(EnvironmentRevisionReference {
        environment_id: id(2),
        revision_id: id(3),
    });
    applied.application_error = None;
    assert_eq!(
        serde_json::from_value::<EnvironmentMutationResponse>(serde_json::to_value(&applied).unwrap()).unwrap(),
        applied
    );
}

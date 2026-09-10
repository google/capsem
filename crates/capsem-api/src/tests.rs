use super::*;

#[test]
fn stats_detail_schema_names_every_event_and_uses_booleans() {
    let doc = serde_json::to_value(openapi()).unwrap();
    assert_eq!(
        doc["paths"]["/vms/{id}/stats/detail"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/VmStatsDetailResponse"
    );
    let schemas = &doc["components"]["schemas"];
    for (field, model) in [
        ("model_stats", "ModelUsage"),
        ("model_events", "ModelEvent"),
        ("tool_events", "ToolEvent"),
        ("http_events", "HttpEvent"),
        ("dns_events", "DnsEvent"),
        ("file_events", "FileEvent"),
        ("process_events", "ProcessEvent"),
        ("audit_events", "AuditEvent"),
        ("credential_events", "CredentialEvent"),
    ] {
        assert_eq!(
            schemas["VmStatsDetailResponse"]["properties"][field]["items"]["$ref"],
            format!("#/components/schemas/{model}")
        );
    }
    assert_eq!(
        schemas["ToolEvent"]["properties"]["model_parent_missing"]["type"],
        "boolean"
    );
    assert_eq!(schemas["EventBody"]["properties"]["truncated"]["type"], "boolean");
    assert_eq!(
        schemas["VmStatsDetailResponse"]["properties"]["body_blobs"]["additionalProperties"]["items"]["$ref"],
        "#/components/schemas/EventBody"
    );
    for invalid in ["invented", ""] {
        let value = serde_json::json!(invalid);
        assert!(serde_json::from_value::<NetworkDecision>(value.clone()).is_err());
        assert!(serde_json::from_value::<NetworkProtocol>(value.clone()).is_err());
        assert!(serde_json::from_value::<ToolOrigin>(value.clone()).is_err());
        assert!(serde_json::from_value::<CredentialOutcome>(value.clone()).is_err());
        assert!(serde_json::from_value::<CredentialEventType>(value.clone()).is_err());
        assert!(serde_json::from_value::<BodyDirection>(value).is_err());
    }
}

#[test]
fn inspection_types_reject_unknown_categories_and_preserve_union_values() {
    assert!(serde_json::from_str::<HistoryLayerFilter>("\"net\"").is_err());
    assert!(serde_json::from_str::<TimelineLayer>("\"tools\"").is_err());
    assert!(serde_json::from_str::<ToolDecision>("\"magic\"").is_err());
    assert_eq!(
        serde_json::from_str::<TimelineStatus>("200").unwrap(),
        TimelineStatus::Code(200)
    );
    assert_eq!(
        serde_json::from_str::<TimelineStatus>("\"allowed\"").unwrap(),
        TimelineStatus::Decision(ToolDecision::Allowed)
    );
    let query: TimelineQuery = serde_json::from_value(serde_json::json!({"layers":"exec,tool"})).unwrap();
    assert_eq!(query.layers.unwrap(), vec![TimelineLayer::Exec, TimelineLayer::Tool]);
    assert!(serde_json::from_value::<TimelineQuery>(serde_json::json!({"layers":"exec,invented"})).is_err());
    let schema = serde_json::to_value(openapi()).unwrap();
    let parameters = schema["paths"]["/vms/{id}/timeline"]["get"]["parameters"]
        .as_array()
        .unwrap();
    let layers = parameters
        .iter()
        .find(|parameter| parameter["name"] == "layers")
        .unwrap();
    assert_eq!(layers["explode"], false);
    assert_eq!(layers["schema"]["type"], "array");
}
use serde_json::json;
use utoipa::PartialSchema;

#[test]
fn management_categories_are_closed_enums() {
    assert!(serde_json::from_value::<ServiceAvailability>(json!("maybe")).is_err());
    assert!(serde_json::from_value::<UpdateActionStatus>(json!("maybe")).is_err());
    assert!(serde_json::from_value::<ValidationStatus>(json!("maybe")).is_err());
    assert!(serde_json::from_value::<ProfileCatalogSource>(json!("directory")).is_err());
    assert_eq!(
        serde_json::to_value(UpdateActionStatus::Succeeded).unwrap(),
        "succeeded"
    );
    assert_eq!(
        serde_json::to_value(ValidationStatus::FetchError).unwrap(),
        "fetch_error"
    );
}

#[test]
fn checked_in_openapi_matches_the_rust_contract() {
    let exported: serde_json::Value =
        serde_json::from_str(include_str!("../../../sdk/specification/openapi.json")).unwrap();
    assert_eq!(
        exported,
        serde_json::to_value(crate::openapi()).unwrap(),
        "Regenerate sdk/specification/openapi.json using the capsem-api export_openapi example"
    );
}

#[test]
fn every_schema_reference_resolves_including_recursive_file_entries() {
    fn check(value: &serde_json::Value, document: &serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                if let Some(reference) = fields.get("$ref").and_then(serde_json::Value::as_str) {
                    let pointer = reference
                        .strip_prefix('#')
                        .expect("contract uses local schema references");
                    assert!(
                        document.pointer(pointer).is_some(),
                        "unresolved schema reference: {reference}"
                    );
                }
                for child in fields.values() {
                    check(child, document);
                }
            }
            serde_json::Value::Array(values) => {
                for child in values {
                    check(child, document);
                }
            }
            _ => {}
        }
    }
    let document = serde_json::to_value(crate::openapi()).unwrap();
    check(&document, &document);
}

#[test]
fn openapi_uses_named_schemas_and_bearer_authentication() {
    let document = serde_json::to_value(crate::openapi()).unwrap();
    let create = &document["paths"]["/vms/create"]["post"];
    assert_eq!(create["operationId"], "createVm");
    assert_eq!(
        create["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/ProvisionRequest"
    );
    assert_eq!(
        create["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/ProvisionResponse"
    );
    assert_eq!(
        document["components"]["securitySchemes"]["bearerAuth"]["scheme"],
        "bearer"
    );
    assert_eq!(document["security"], json!([{"bearerAuth": []}]));
    for (action, method, schema) in [
        ("stop", "post", "StopResponse"),
        ("pause", "post", "VmActionResponse"),
        ("delete", "delete", "VmActionResponse"),
    ] {
        let operation = &document["paths"][format!("/vms/{{id}}/{action}")][method];
        assert_eq!(
            operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            format!("#/components/schemas/{schema}")
        );
        assert!(operation.get("requestBody").is_none());
    }
}

#[test]
fn openapi_describes_binary_copy_and_required_vm_identity() {
    let document = serde_json::to_value(crate::openapi()).unwrap();
    let copy = &document["paths"]["/vms/{id}/files/content"];
    assert_eq!(
        copy["get"]["responses"]["200"]["content"]["application/octet-stream"]["schema"]["format"],
        "binary"
    );
    assert_eq!(
        copy["post"]["requestBody"]["content"]["application/octet-stream"]["schema"]["format"],
        "binary"
    );
    assert!(copy["get"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "id" && p["required"] == true));
    assert!(copy["get"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "path" && p["required"] == true));
}

#[test]
fn lifecycle_values_preserve_the_existing_wire_contract() {
    assert_eq!(
        serde_json::to_value(VmLifecycleState::Running).unwrap(),
        json!("Running")
    );
    assert_eq!(serde_json::to_value(VmAction::Pause).unwrap(), json!("pause"));
    assert!(serde_json::from_value::<VmLifecycleState>(json!("running")).is_err());
    assert!(serde_json::from_value::<VmAction>(json!("invented")).is_err());
}

#[test]
fn action_availability_distinguishes_resume_from_start() {
    assert_eq!(VmLifecycleState::Suspended.available_actions(true)[0], VmAction::Resume);
    assert_eq!(VmLifecycleState::Stopped.available_actions(true)[0], VmAction::Start);
    assert_eq!(
        VmLifecycleState::Defunct.available_actions(true),
        vec![VmAction::Delete]
    );
}

#[test]
fn omitted_resources_remain_profile_owned() {
    let request: ProvisionRequest = serde_json::from_value(json!({"profile_id": "code"})).unwrap();
    let wire = serde_json::to_value(request).unwrap();
    assert!(wire.get("ram_mb").is_none());
    assert!(wire.get("cpus").is_none());
    assert_eq!(wire["persistent"], false);
}

#[test]
fn schema_exposes_closed_lifecycle_and_action_values() {
    let state = serde_json::to_value(VmLifecycleState::schema()).unwrap();
    let action = serde_json::to_value(VmAction::schema()).unwrap();
    assert_eq!(
        state["enum"],
        json!(["Running", "Stopped", "Suspended", "Defunct", "Incompatible"])
    );
    assert_eq!(
        action["enum"],
        json!(["pause", "stop", "start", "resume", "fork", "delete"])
    );
}

use super::*;

#[test]
fn existing_update_check_contract_uses_original_typed_request_and_response() {
    let doc = serde_json::to_value(openapi()).unwrap();
    let operation = &doc["paths"]["/update/check"]["post"];
    assert_eq!(operation["operationId"], "checkHypervisorUpdate");
    assert_eq!(
        operation["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/UpdateCheckRequest"
    );
    assert_eq!(
        operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/UpdateActionResponse"
    );
    assert_eq!(
        doc["components"]["schemas"]["UpdateCheckRequest"]["additionalProperties"],
        false
    );
    assert_eq!(
        serde_json::from_value::<UpdateCheckRequest>(serde_json::json!({})).unwrap(),
        UpdateCheckRequest { dry_run: false }
    );
    assert!(serde_json::from_value::<UpdateCheckRequest>(serde_json::json!({"confirmed": true})).is_err());
    assert!(
        operation["responses"]["202"].is_null(),
        "registration must not invent asynchronous update semantics"
    );
}

#[test]
fn provision_response_constructor_preserves_the_supplied_lifecycle_and_actions() {
    let response = ProvisionResponse::new(
        "vm-id".into(),
        "named".into(),
        VmLifecycleState::Suspended,
        true,
        true,
        vec![VmAction::Delete],
    );
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::json!({
            "id": "vm-id", "name": "named", "status": "Suspended",
            "persistent": true, "can_resume": true, "available_actions": ["delete"],
        })
    );
}

#[test]
fn restart_contract_acknowledges_202_and_requires_fresh_authentication() {
    let doc = serde_json::to_value(openapi()).unwrap();
    let operation = &doc["paths"]["/restart"]["post"];
    assert_eq!(operation["operationId"], "restartHypervisor");
    assert!(operation["responses"]["200"].is_null());
    assert_eq!(
        operation["responses"]["202"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/RestartResponse"
    );
    let response = RestartResponse {
        status: RestartStatus::Accepted,
        manager: ServiceManager::Systemd,
        authentication: RestartAuthentication::NewTokenRequired,
    };
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["authentication"], "new_token_required");
    assert_eq!(serde_json::from_value::<RestartResponse>(wire).unwrap(), response);
}

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
    assert!(schemas["EventBody"]["properties"].get("body").is_none());
    assert_eq!(
        doc["paths"]["/vms/{id}/bodies/{event_id}"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/EventBodiesResponse"
    );
    assert_eq!(
        doc["paths"]["/vms/{id}/bodies/export.warc.gz"]["get"]["responses"]["200"]["content"]["application/gzip"]
            ["schema"]["format"],
        "binary"
    );
    for invalid in ["invented", ""] {
        let value = serde_json::json!(invalid);
        assert!(serde_json::from_value::<NetworkDecision>(value.clone()).is_err());
        assert!(serde_json::from_value::<NetworkProtocol>(value.clone()).is_err());
        assert!(serde_json::from_value::<ToolOrigin>(value.clone()).is_err());
        assert!(serde_json::from_value::<CredentialOutcome>(value.clone()).is_err());
        assert!(serde_json::from_value::<CredentialEventType>(value.clone()).is_err());
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
    assert!(serde_json::from_value::<AssetFileState>(json!("maybe")).is_err());
    assert_eq!(
        serde_json::to_value(UpdateActionStatus::Succeeded).unwrap(),
        "succeeded"
    );
    assert_eq!(
        serde_json::to_value(ValidationStatus::FetchError).unwrap(),
        "fetch_error"
    );
}

/// The contract version is the SDK's, not the binary's. While it was
/// `CARGO_PKG_VERSION` every version bump changed the checked-in
/// specification, so the next release turned the fast phase red for a
/// contract that had not changed at all.
#[test]
fn the_contract_version_is_independent_of_the_binary_version() {
    assert_eq!(crate::openapi().info.version, crate::CONTRACT_VERSION);
    assert!(
        !include_str!("document.rs").contains("CARGO_PKG_VERSION"),
        "the published contract version must not track the crate version"
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
fn image_pull_contract_allows_download_and_verification_before_client_timeout() {
    let document = serde_json::to_value(crate::openapi()).unwrap();
    assert_eq!(
        document["paths"]["/images/pull"]["post"]["x-capsem-request-timeout-ms"],
        1_020_000
    );
    assert_eq!(
        document["paths"]["/environments/create"]["post"]["x-capsem-request-timeout-ms"],
        1_020_000
    );
    assert!(document["paths"]["/images"]["get"]
        .get("x-capsem-request-timeout-ms")
        .is_none());
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
fn openapi_describes_standalone_proxy_leases() {
    let document = serde_json::to_value(crate::openapi()).unwrap();
    let paths = &document["paths"];

    assert_eq!(paths["/proxies"]["post"]["operationId"], "createProxy");
    assert_eq!(
        paths["/proxies/{id}/heartbeat"]["post"]["operationId"],
        "heartbeatProxy"
    );
    assert_eq!(paths["/proxies/{id}/stop"]["post"]["operationId"], "stopProxy");
    assert_eq!(crate::CONTRACT_VERSION, "3.6.0");
}

#[test]
fn openapi_exec_output_is_byte_safe_and_retires_ambiguous_file_json() {
    let document = serde_json::to_value(crate::openapi()).unwrap();
    let paths = &document["paths"];
    assert!(paths.get("/vms/{id}/files/read").is_none());
    assert!(paths.get("/vms/{id}/files/write").is_none());

    let schemas = &document["components"]["schemas"];
    for stream in ["stdout", "stderr"] {
        assert_eq!(
            schemas["ExecResponse"]["properties"][stream]["$ref"],
            "#/components/schemas/ExecOutput"
        );
    }
    assert_eq!(schemas["ExecOutput"]["properties"]["data"]["type"], "string");
    assert_eq!(
        schemas["ExecOutput"]["properties"]["encoding"]["$ref"],
        "#/components/schemas/ExecOutputEncoding"
    );
    assert_eq!(
        schemas["ExecOutputEncoding"]["enum"],
        serde_json::json!(["utf8", "base64"])
    );
}

#[test]
fn exec_output_roundtrips_utf8_and_arbitrary_bytes() {
    let utf8 = ExecOutput::from_bytes(b"hello\n".to_vec());
    assert_eq!(utf8.encoding, ExecOutputEncoding::Utf8);
    assert_eq!(utf8.data, "hello\n");
    assert_eq!(utf8.decode().unwrap(), b"hello\n");

    let binary = ExecOutput::from_bytes(vec![0, 0xff, b'\n']);
    assert_eq!(binary.encoding, ExecOutputEncoding::Base64);
    assert_eq!(binary.data, "AP8K");
    let wire = serde_json::to_value(&binary).unwrap();
    assert_eq!(wire, serde_json::json!({"encoding":"base64", "data":"AP8K"}));
    assert_eq!(
        serde_json::from_value::<ExecOutput>(wire).unwrap().decode().unwrap(),
        vec![0, 0xff, b'\n']
    );
}

#[test]
fn openapi_describes_network_mutations_and_both_member_path_parameters() {
    let document = serde_json::to_value(crate::openapi()).unwrap();
    let responses = &document["paths"]["/networks"]["post"]["responses"];
    assert!(responses.get("200").is_none(), "network creation returns 201 Created");
    assert_eq!(
        responses["201"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/NetworkInfo"
    );
    let paths = &document["paths"];
    assert_eq!(paths["/networks"]["post"]["operationId"], "createNetwork");
    assert_eq!(paths["/networks"]["get"]["operationId"], "listNetworks");
    assert_eq!(paths["/networks/{id}"]["delete"]["operationId"], "deleteNetwork");

    let member = &paths["/networks/{id}/members/{vm_id}"];
    assert_eq!(member["put"]["operationId"], "attachNetworkMember");
    assert_eq!(member["delete"]["operationId"], "detachNetworkMember");
    for method in ["put", "delete"] {
        let parameters = member[method]["parameters"].as_array().unwrap();
        assert_eq!(
            parameters
                .iter()
                .map(|parameter| parameter["name"].as_str().unwrap())
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["id", "vm_id"])
        );
        assert!(parameters.iter().all(|parameter| parameter["required"] == true));
    }

    let logs = &paths["/networks/{id}/logs"]["get"];
    assert!(logs["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|parameter| parameter["name"] == "cursor"));
}

#[test]
fn openapi_exposes_existing_diagnostics_persistence_and_mcp_routes() {
    let document = serde_json::to_value(crate::openapi()).unwrap();
    let paths = &document["paths"];
    for (path, method, operation_id) in [
        ("/run", "post", "runVm"),
        ("/purge", "post", "purgeVms"),
        ("/panics", "get", "getPanics"),
        ("/triage", "get", "getTriage"),
        ("/vms/{id}/save", "post", "persistVm"),
        ("/assets/status", "get", "getAssetStatus"),
        ("/mcp/info", "get", "getMcpInfo"),
        ("/mcp/servers/list", "get", "listMcpServers"),
        ("/mcp/default/info", "get", "getMcpDefault"),
        ("/mcp/servers/{server_id}/tools/list", "get", "listMcpTools"),
        ("/mcp/servers/{server_id}/refresh", "post", "refreshMcpServer"),
        ("/mcp/servers/{server_id}/tools/{tool_id}/call", "post", "callMcpTool"),
    ] {
        assert_eq!(paths[path][method]["operationId"], operation_id, "{method} {path}");
    }
    let call = &paths["/mcp/servers/{server_id}/tools/{tool_id}/call"]["post"];
    let parameters = call["parameters"].as_array().unwrap();
    assert_eq!(
        parameters
            .iter()
            .map(|parameter| parameter["name"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["server_id", "tool_id"])
    );
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
    assert!(!VmLifecycleState::Running
        .available_actions(false)
        .contains(&VmAction::Pause));
    assert!(!VmLifecycleState::Suspended
        .available_actions(true)
        .contains(&VmAction::Resume));
    assert_eq!(VmLifecycleState::Stopped.available_actions(true)[0], VmAction::Start);
    assert_eq!(
        VmLifecycleState::Defunct.available_actions(true),
        vec![VmAction::Delete]
    );
}

#[test]
fn omitted_resources_stay_off_the_wire() {
    let request: ProvisionRequest = serde_json::from_value(json!({})).unwrap();
    let wire = serde_json::to_value(request).unwrap();
    assert!(wire.get("ram_mb").is_none());
    assert!(wire.get("cpus").is_none());
    assert_eq!(wire["persistent"], false);
}

#[test]
fn image_is_not_a_clone_source_alias() {
    let error = serde_json::from_value::<ProvisionRequest>(json!({"image": "old-img"})).unwrap_err();
    assert!(error.to_string().contains("unknown field `image`"), "{error}");
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

/// Exec used to wait forever when `timeout_secs` was absent, while the gateway
/// cut every request at 120s: long commands answered 502 through HTTP and
/// never ended through the local socket. The contract now bounds both.
#[test]
fn exec_timeout_is_bounded_and_validated() {
    assert_eq!(exec_timeout_secs(None), Ok(DEFAULT_EXEC_TIMEOUT_SECS));
    assert_eq!(exec_timeout_secs(Some(30)), Ok(30));
    assert_eq!(
        exec_timeout_secs(Some(MAX_EXEC_TIMEOUT_SECS)),
        Ok(MAX_EXEC_TIMEOUT_SECS)
    );
    assert!(exec_timeout_secs(Some(0)).is_err());
    assert!(exec_timeout_secs(Some(MAX_EXEC_TIMEOUT_SECS + 1)).is_err());
}

/// An image session's exec enters its workload unless the caller names the
/// VM; a session without one runs in the VM, and asking it for a workload is
/// refused rather than silently answered by the VM.
#[test]
fn exec_target_defaults_to_the_workload_only_where_there_is_one() {
    let absent: ExecRequest = serde_json::from_value(json!({"command": "id -u"})).unwrap();
    assert_eq!(absent.target, None);
    assert_eq!(exec_target(absent.target, true), Ok(ExecTarget::Workload));
    assert_eq!(exec_target(absent.target, false), Ok(ExecTarget::Vm));
    assert_eq!(exec_target(Some(ExecTarget::Vm), true), Ok(ExecTarget::Vm));
    assert_eq!(exec_target(Some(ExecTarget::Vm), false), Ok(ExecTarget::Vm));
    assert_eq!(exec_target(Some(ExecTarget::Workload), true), Ok(ExecTarget::Workload));
    let refused = exec_target(Some(ExecTarget::Workload), false).unwrap_err();
    assert!(refused.contains("no container workload"), "{refused}");

    let named: ExecRequest = serde_json::from_value(json!({"command": "true", "target": "vm"})).unwrap();
    assert_eq!(named.target, Some(ExecTarget::Vm));
    assert!(serde_json::from_value::<ExecRequest>(json!({"command": "true", "target": "container"})).is_err());
    // An absent target stays absent on the wire, so older services decode it.
    assert_eq!(serde_json::to_value(&absent).unwrap(), json!({"command": "id -u"}));
}

#[test]
fn validate_name_enforces_shared_vm_identifier_rule() {
    for valid in [
        "a",
        "9lives",
        "my-vm",
        "project_alpha",
        "vm-1712678400",
        &"a".repeat(64),
    ] {
        assert_eq!(validate_name("VM name", valid), Ok(()), "{valid}");
        assert_eq!(validate_name("VM id", valid), Ok(()), "{valid}");
    }

    assert_eq!(validate_name("VM name", ""), Err("VM name cannot be empty".to_string()));
    assert_eq!(validate_name("VM id", ""), Err("VM id cannot be empty".to_string()));
    assert_eq!(
        validate_name("VM name", &"a".repeat(65)),
        Err("VM name too long (max 64 characters)".to_string())
    );
    for bad_start in ["-bad", "_bad", ".bad"] {
        assert_eq!(
            validate_name("VM id", bad_start),
            Err("VM id must start with a letter or digit".to_string()),
            "{bad_start}"
        );
    }
    for bad_body in ["my/vm", "../escape", "my vm", "my.vm", "vm\0id", "nai\u{00ef}ve"] {
        assert!(
            validate_name("VM name", bad_body).is_err(),
            "{bad_body} should be rejected"
        );
    }
    assert_eq!(
        validate_name("VM name", "my.vm"),
        Err("VM name must contain only letters, digits, hyphens, and underscores".to_string())
    );
}

#[test]
fn environment_changes_contract_carries_observed_base_and_resolution_budget() {
    let document = serde_json::to_value(openapi()).unwrap();
    let change = &document["paths"]["/environments/{id}/changes"]["post"];
    assert_eq!(change["operationId"], "changeEnvironment");
    assert_eq!(change["x-capsem-request-timeout-ms"], IMAGE_PULL_REQUEST_TIMEOUT_MS);
    assert_eq!(
        change["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/EnvironmentChangeRequest"
    );
    assert!(
        document["components"]["schemas"]["EnvironmentChangeRequest"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "base_revision_id")
    );
}

#[test]
fn provision_environment_selection_is_an_exact_revision_without_inline_overrides() {
    let request: ProvisionRequest = serde_json::from_value(json!({"environment": {
        "environment_id": "00000000-0000-4000-8000-000000000001",
        "revision_id": "00000000-0000-4000-8000-000000000002"
    }}))
    .unwrap();
    assert_eq!(
        request.environment.as_ref().unwrap().revision_id,
        "00000000-0000-4000-8000-000000000002"
    );
    request.validate_environment().unwrap();
    for extra in [
        json!({"env": {"UNAPPROVED": "value"}}),
        json!({"container": {"image": "other"}}),
    ] {
        let mut wire = serde_json::to_value(&request).unwrap();
        wire.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let invalid: ProvisionRequest = serde_json::from_value(wire).unwrap();
        assert!(invalid.validate_environment().is_err());
    }
}

#[test]
fn attached_environment_invocation_keeps_transport_separate_from_workload_content() {
    let reference = json!({
        "environment_id": "00000000-0000-4000-8000-000000000001",
        "revision_id": "00000000-0000-4000-8000-000000000002"
    });
    let detached: ProvisionRequest = serde_json::from_value(json!({"environment": reference})).unwrap();
    let attached: ProvisionRequest = serde_json::from_value(json!({"environment": reference, "attach": true})).unwrap();
    attached.validate_environment().unwrap();
    assert!(!detached.attach && attached.attach);
    assert!(serde_json::to_value(&detached).unwrap().get("attach").is_none());
    assert_eq!(detached.environment, attached.environment);
    for extra in [
        json!({"env": {"UNAPPROVED": "value"}}),
        json!({"container": {"image": "other"}}),
    ] {
        let mut wire = serde_json::to_value(&attached).unwrap();
        wire.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let invalid: ProvisionRequest = serde_json::from_value(wire).unwrap();
        assert!(invalid.validate_environment().is_err());
    }
    let missing: ProvisionRequest = serde_json::from_value(json!({"attach": true})).unwrap();
    assert!(missing.validate_environment().is_err());
}

#[test]
fn environment_readiness_contract_uses_an_exact_revision_and_only_wait_metadata() {
    let doc = serde_json::to_value(openapi()).unwrap();
    let operation = &doc["paths"]["/environments/{id}/revisions/{revision_id}/readiness"]["get"];
    assert_eq!(operation["operationId"], "getEnvironmentReadiness");
    let params = operation["parameters"].as_array().unwrap();
    assert!(params.iter().any(|p| p["name"] == "id" && p["required"] == true));
    assert!(params
        .iter()
        .any(|p| p["name"] == "revision_id" && p["required"] == true));
    let schema = &doc["components"]["schemas"]["EnvironmentReadinessResponse"];
    let properties = schema["properties"].as_object().unwrap();
    assert_eq!(properties.len(), 2, "no executable or secret-bearing content");
    assert!(properties.contains_key("environment"));
    assert!(properties.contains_key("startup_timeout_seconds"));
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn environment_readiness_roundtrips_the_full_approved_timeout_without_executable_content() {
    use environments::{EnvironmentReadinessResponse, EnvironmentRevisionReference};
    let environment = EnvironmentRevisionReference {
        environment_id: "00000000-0000-4000-8000-000000000001".into(),
        revision_id: "00000000-0000-4000-8000-000000000002".into(),
    };
    for seconds in [0, 1, 600, u32::MAX] {
        let response = EnvironmentReadinessResponse {
            environment: environment.clone(),
            startup_timeout_seconds: seconds,
        };
        let mut wire = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serde_json::from_value::<EnvironmentReadinessResponse>(wire.clone()).unwrap(),
            response
        );
        assert_eq!(wire.as_object().unwrap().len(), 2);
        wire["startup_timeout_seconds"] = serde_json::json!(u64::from(u32::MAX) + 1);
        assert!(serde_json::from_value::<EnvironmentReadinessResponse>(wire).is_err());
    }
    let mut wire = serde_json::json!({"environment": environment, "startup_timeout_seconds": 600});
    wire["script"] = serde_json::json!("hidden");
    assert!(serde_json::from_value::<EnvironmentReadinessResponse>(wire).is_err());
}

#[test]
fn lifecycle_readiness_contract_preserves_optional_original_selection_preconditions() {
    let doc = serde_json::to_value(openapi()).unwrap();
    assert_eq!(
        doc["paths"]["/vms/{id}/readiness"]["get"]["operationId"],
        "getVmReadiness"
    );
    for path in ["/vms/{id}/start", "/vms/{id}/resume"] {
        let body = &doc["paths"][path]["post"]["requestBody"];
        assert_eq!(body["required"], false, "empty legacy requests remain valid");
        assert_eq!(
            body["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/VmStartRequest"
        );
    }
    let selection = &doc["components"]["schemas"]["VmLifecycleSelection"];
    assert_eq!(selection["additionalProperties"], false);
    assert_eq!(
        selection["properties"].as_object().unwrap().len(),
        4,
        "only identity and launch timing selection"
    );
    assert_eq!(
        doc["components"]["schemas"]["VmReadinessResponse"]["properties"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn lifecycle_selection_is_strict_matching_data_with_legacy_absence_only_when_stopped() {
    let selection = VmLifecycleSelection {
        id: "vm-1".into(),
        generation: Some("00000000-0000-4000-8000-000000000001".into()),
        running: true,
        environment: Some(environments::EnvironmentRevisionReference {
            environment_id: "00000000-0000-4000-8000-000000000002".into(),
            revision_id: "00000000-0000-4000-8000-000000000003".into(),
        }),
    };
    assert_eq!(selection.validate_target("vm-1"), Ok(()));
    assert!(selection.validate_target("vm-2").is_err());
    let mut invalid = selection.clone();
    invalid.generation = None;
    assert!(invalid.validate().is_err());
    invalid.running = false;
    assert_eq!(invalid.validate(), Ok(()));
    for generation in [
        "not-a-generation",
        "00000000-0000-0000-0000-000000000000",
        "00000000-0000-4000-8000-00000000000A",
    ] {
        invalid.generation = Some(generation.into());
        assert!(invalid.validate().is_err());
    }
    for field in ["actor", "approval", "script", "capability"] {
        let mut value = serde_json::to_value(&selection).unwrap();
        value[field] = serde_json::json!("forged");
        assert!(serde_json::from_value::<VmLifecycleSelection>(value).is_err());
    }
    assert_eq!(
        serde_json::to_value(VmStartRequest::default()).unwrap(),
        serde_json::json!({})
    );
    let start: VmStartRequest = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(start.validate("vm-1"), Ok(()));
    assert!(serde_json::from_value::<VmStartRequest>(serde_json::json!({"actor":"operator"})).is_err());
    let mut request = ProvisionRequest {
        expected_source: Some(selection.clone()),
        ..Default::default()
    };
    assert!(request.validate_environment().is_err());
    request.from = Some("source-name".into());
    assert_eq!(request.validate_environment(), Ok(()));
    for seconds in [0, u32::MAX] {
        let response = VmReadinessResponse {
            selection: selection.clone(),
            startup_timeout_seconds: seconds,
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serde_json::from_value::<VmReadinessResponse>(encoded).unwrap(),
            response
        );
    }
}

#[test]
fn lifecycle_optional_expectations_preserve_legacy_fork_and_start_bytes() {
    let value = serde_json::json!({"name":"child"});
    let mut fork: ForkRequest = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&fork).unwrap(), value);
    assert_eq!(fork.validate_expected("parent"), Ok(()));
    assert!(serde_json::from_value::<ForkRequest>(serde_json::json!({"name":"child", "actor":"operator"})).is_err());
    let selection = VmLifecycleSelection {
        id: "parent".into(),
        generation: None,
        running: false,
        environment: None,
    };
    fork.expected = Some(selection.clone());
    assert_eq!(fork.validate_expected("parent"), Ok(()));
    assert!(fork.validate_expected("other").is_err());
    let start = VmStartRequest {
        expected: Some(selection),
    };
    assert_eq!(start.validate("parent"), Ok(()));
    assert!(start.validate("other").is_err());
    let wire = serde_json::to_value(start).unwrap();
    assert_eq!(wire["expected"]["generation"], serde_json::Value::Null);
    assert_eq!(wire["expected"]["running"], false);
}

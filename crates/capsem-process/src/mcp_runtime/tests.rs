use super::*;
use capsem_core::net::mitm_proxy::ScopedMcpTools;
use capsem_proto::ipc::ServiceToProcess;
use tokio::sync::mpsc;

fn fixture() -> GuestExposureTools {
    let (control, _requests) = mpsc::channel::<ServiceToProcess>(4);
    GuestExposureTools::new(Arc::new(capsem_core::container::publish::Publisher::default()), control)
}

#[test]
fn guest_exposure_schema_requires_an_explicit_namespace_and_carries_no_control_credential() {
    let definitions = fixture().definitions();
    assert_eq!(definitions.len(), 1);
    let tool = &definitions[0];
    assert_eq!(tool.namespaced_name, "capsem__expose_port");
    assert_eq!(
        tool.input_schema["required"],
        serde_json::json!(["guest_port", "target"])
    );
    assert_eq!(tool.input_schema["additionalProperties"], false);
    let schema = tool.input_schema.to_string();
    for forbidden in ["token", "gateway", "socket", "vm_id", "vm_name"] {
        assert!(!schema.contains(forbidden), "guest tool schema exposed {forbidden}");
    }
}

#[tokio::test]
async fn guest_exposure_rejects_untyped_or_forbidden_arguments_before_owner_dispatch() {
    let tool = fixture();
    for (arguments, expected) in [
        (
            serde_json::json!({"guest_port": 8080, "target": "container", "token": "secret"}),
            "unknown field",
        ),
        (
            serde_json::json!({"guest_port": "8080", "target": "container"}),
            "invalid type",
        ),
        (serde_json::json!({"guest_port": 8080}), "missing field"),
    ] {
        let error = tool.call_tool("capsem__expose_port", arguments).await.unwrap_err();
        assert!(error.contains(expected), "expected {expected:?} in {error:?}");
    }
}

#[tokio::test]
async fn guest_exposure_dispatches_against_the_injected_owner_publisher() {
    let error = fixture()
        .call_tool(
            "capsem__expose_port",
            serde_json::json!({"guest_port": 8080, "host_port": 0, "target": "vm"}),
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("publication security context missing"),
        "the fixture's injected publisher should own dispatch: {error:?}"
    );
}

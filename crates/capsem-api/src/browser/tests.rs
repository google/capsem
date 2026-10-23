use super::*;

#[test]
fn browser_ack_is_bound_to_one_owner_and_request_without_auth_url() {
    let body = serde_json::json!({"owner_generation":"11111111-1111-4111-8111-111111111111","request_id":4});
    let request: BrowserAcknowledgeRequest = serde_json::from_value(body.clone()).unwrap();
    assert!(request.validate().is_ok());
    let mut with_url = body;
    with_url["url"] = "https://example.org/?state=private".into();
    assert!(serde_json::from_value::<BrowserAcknowledgeRequest>(with_url).is_err());
    for invalid in [
        serde_json::json!({"owner_generation":"session-name","request_id":4}),
        serde_json::json!({"owner_generation":"11111111-1111-4111-8111-111111111111","request_id":0}),
    ] {
        assert!(serde_json::from_value::<BrowserAcknowledgeRequest>(invalid)
            .unwrap()
            .validate()
            .is_err());
    }
}

#[test]
fn browser_ack_exports_the_single_metadata_only_mutation() {
    let schema = serde_json::to_value(crate::openapi()).unwrap();
    assert_eq!(
        schema["paths"]["/vms/{id}/browser/acknowledge"]["post"]["operationId"],
        "acknowledgeBrowserRequest"
    );
}

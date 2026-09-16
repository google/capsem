use super::*;
use serde_json::json;

#[test]
fn exposure_request_defaults_to_a_picked_port_in_the_container() {
    let request: ExposureRequest = serde_json::from_value(json!({"guest_port": 6379})).unwrap();
    assert_eq!(
        request,
        ExposureRequest {
            guest_port: 6379,
            host_port: 0,
            target: ExposureTarget::Container,
            access: ExposureAccess::LoopbackTcp,
        }
    );
    let vm: ExposureRequest = serde_json::from_value(json!({"guest_port": 8080, "target": "vm"})).unwrap();
    assert_eq!(vm.target, ExposureTarget::Vm);
    assert!(serde_json::from_value::<ExposureRequest>(json!({"guest_port": 70000})).is_err());
}

#[test]
fn exposure_identity_is_the_host_port_and_generation_is_a_string() {
    let info = ExposureInfo::loopback(16379, 6379, ExposureTarget::Container);
    assert_eq!(info.id, "16379");
    assert_eq!(info.host_port, Some(16379));
    let list = ExposureListResponse {
        owner_generation: u64::MAX.to_string(),
        exposures: vec![info],
    };
    let wire = serde_json::to_value(&list).unwrap();
    assert_eq!(wire["owner_generation"], "18446744073709551615");
    assert_eq!(wire["exposures"][0]["target"], "container");
    assert_eq!(wire["exposures"][0]["access"], "loopback_tcp");
}

#[test]
fn browser_previews_have_no_unauthenticated_loopback_listener() {
    let request: ExposureRequest = serde_json::from_value(json!({
        "guest_port": 8080,
        "target": "container",
        "access": "http_preview"
    }))
    .unwrap();
    assert_eq!(request.access, ExposureAccess::HttpPreview);
    let info = ExposureInfo::preview(
        "0199df26-d0f2-74f2-a304-ef67b79d1217".into(),
        8080,
        ExposureTarget::Container,
    );
    assert_eq!(info.host_port, None);
    assert_eq!(info.access, ExposureAccess::HttpPreview);

    let session = PreviewSessionResponse {
        url: "http://0199df26-d0f2-74f2-a304-ef67b79d1217.localhost:19444/_capsem/bootstrap".into(),
        bootstrap_token: "secret-post-body-token".into(),
        expires_in_seconds: 30,
    };
    assert!(!session.url.contains(&session.bootstrap_token));
    assert!(!session.url.contains('?'));
}

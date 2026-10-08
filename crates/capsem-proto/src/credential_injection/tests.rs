use super::*;

#[test]
fn host_only_material_roundtrips_without_debug_disclosure() {
    let material = CredentialMaterial {
        provider: "openai".into(),
        credential_ref: crate::credential_reference::credential_reference("openai", "test-private-token"),
        value: "test-private-token".into(),
    };
    let request = crate::ipc::ServiceToProcess::InjectCredentials {
        id: 71,
        credentials: vec![material],
    };
    assert!(!format!("{request:?}").contains("test-private-token"));
    let bytes = rmp_serde::to_vec_named(&request).unwrap();
    let decoded: crate::ipc::ServiceToProcess = rmp_serde::from_slice(&bytes).unwrap();
    assert_eq!(decoded.request_id(), Some(71));
    let crate::ipc::ServiceToProcess::InjectCredentials { credentials, .. } = decoded else {
        panic!("wrong request");
    };
    assert_eq!(credentials[0].value, "test-private-token");
    assert_eq!(credentials[0].provider, "openai");
    let reply = crate::ipc::ProcessToService::CredentialsInjected { id: 71, error: None };
    assert_eq!(reply.reply_id(), Some(71));
}

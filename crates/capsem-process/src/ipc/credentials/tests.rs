use super::*;
use capsem_core::credential_broker::CredentialProvider;

#[test]
fn owner_acknowledges_valid_host_material_and_redacts_refusal() {
    let store = CredentialStore::default();
    let reference = capsem_proto::credential_reference::credential_reference("anthropic", "private-test-secret");
    let material = CredentialMaterial {
        provider: "anthropic".into(),
        credential_ref: reference.clone(),
        value: "private-test-secret".into(),
    };
    assert!(matches!(
        apply(&store, 6, vec![material.clone()]),
        ProcessToService::CredentialsInjected { id: 6, error: None }
    ));
    assert_eq!(
        store
            .resolve(CredentialProvider::Anthropic, &reference)
            .unwrap()
            .as_deref(),
        Some("private-test-secret")
    );
    let mut invalid = material;
    invalid.credential_ref = "private-invalid-reference".into();
    let reply = apply(&store, 7, vec![invalid]);
    assert!(
        matches!(&reply, ProcessToService::CredentialsInjected { id: 7, error: Some(error) } if error == "credential reference mismatch")
    );
    assert!(!format!("{reply:?}").contains("private"));
}

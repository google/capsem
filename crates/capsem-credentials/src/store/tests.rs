use super::*;

#[test]
fn memory_injection_resolves_without_creating_disk_state_and_loses_on_restart() {
    let store = CredentialStore::default();
    let reference = store
        .inject(
            CredentialProvider::OpenAi,
            "injected-secret",
            CredentialPersistence::Memory,
        )
        .unwrap();
    assert_eq!(
        reference,
        capsem_proto::credential_reference::credential_reference("openai", "injected-secret")
    );
    assert_eq!(
        store
            .resolve(CredentialProvider::OpenAi, &reference)
            .unwrap()
            .as_deref(),
        Some("injected-secret")
    );
    assert_eq!(store.memory_credentials().unwrap().len(), 1);
    assert!(!format!("{:?}", store.memory_credentials().unwrap()).contains("injected-secret"));
    assert!(!CredentialStore::default().replay_available_in_memory(CredentialProvider::OpenAi, &reference));
}

#[test]
fn host_handoff_validates_entire_batch_before_mutating_memory() {
    let source = CredentialStore::default();
    let reference = source
        .inject(
            CredentialProvider::Github,
            "injected-secret",
            CredentialPersistence::Memory,
        )
        .unwrap();
    let target = CredentialStore::default();
    let mut entries = source.memory_credentials().unwrap();
    let mut invalid = entries[0].clone();
    invalid.provider = "openai".into();
    entries.push(invalid);
    assert!(target.import_memory_credentials(entries).is_err());
    assert_eq!(target.status().cached_count, 0);
    target
        .import_memory_credentials(source.memory_credentials().unwrap())
        .unwrap();
    assert_eq!(
        target
            .resolve(CredentialProvider::Github, &reference)
            .unwrap()
            .as_deref(),
        Some("injected-secret")
    );
    assert!(target.memory_credentials().unwrap().len() == 1);
}

#[test]
fn invalid_injection_is_redacted_and_does_not_enter_cache() {
    let store = CredentialStore::default();
    for secret in ["", "private\nsecret", "private\0secret"] {
        let error = store
            .inject(CredentialProvider::OpenAi, secret, CredentialPersistence::Memory)
            .unwrap_err();
        assert!(!error.contains("private"));
    }
    assert_eq!(store.status().cached_count, 0);
}

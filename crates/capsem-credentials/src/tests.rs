use std::sync::Mutex;

use super::*;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct StorePathGuard(Option<std::ffi::OsString>);

impl StorePathGuard {
    fn redirect(path: &std::path::Path) -> Self {
        let previous = std::env::var_os(STORE_PATH_ENV);
        std::env::set_var(STORE_PATH_ENV, path);
        CredentialStore::global().clear_for_test();
        Self(previous)
    }
}

impl Drop for StorePathGuard {
    fn drop(&mut self) {
        CredentialStore::global().clear_for_test();
        match self.0.take() {
            Some(previous) => std::env::set_var(STORE_PATH_ENV, previous),
            None => std::env::remove_var(STORE_PATH_ENV),
        }
    }
}

fn credential_ref(byte: char) -> String {
    format!("credential:blake3:{}", byte.to_string().repeat(64))
}

#[test]
fn reference_validation_rejects_raw_and_malformed_secrets() {
    assert!(is_broker_reference(&credential_ref('a')));
    assert!(!is_broker_reference("raw-token"));
    assert!(!is_broker_reference("credential:blake3:xyz"));
}

#[test]
fn capture_is_idempotent_but_rejects_reference_collisions() {
    let _lock = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _guard = StorePathGuard::redirect(&dir.path().join("credentials.json"));
    let reference = credential_ref('b');
    let store = CredentialStore::global();

    assert!(store
        .capture(CredentialProvider::Mcp, &reference, "secret-one")
        .unwrap());
    assert!(!store
        .capture(CredentialProvider::Mcp, &reference, "secret-one")
        .unwrap());
    assert!(store
        .capture(CredentialProvider::Mcp, &reference, "secret-two")
        .is_err());
    assert_eq!(
        store.resolve(CredentialProvider::Mcp, &reference).unwrap(),
        Some("secret-one".to_string())
    );
}

#[test]
fn hydration_restores_the_runtime_cache_from_disk() {
    let _lock = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _guard = StorePathGuard::redirect(&dir.path().join("credentials.json"));
    let reference = credential_ref('c');
    let store = CredentialStore::global();
    store
        .capture(CredentialProvider::Github, &reference, "gh-secret")
        .unwrap();
    store.clear_for_test();

    assert_eq!(store.hydrate_from_durable_store().unwrap(), 1);
    assert_eq!(
        store.resolve(CredentialProvider::Github, &reference).unwrap(),
        Some("gh-secret".to_string())
    );
}

#[test]
fn injected_file_credentials_survive_restart_and_memory_credentials_never_touch_disk() {
    let _lock = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credentials.json");
    let _guard = StorePathGuard::redirect(&path);
    let store = CredentialStore::default();
    let memory_ref = store
        .inject(
            CredentialProvider::Google,
            "memory-private-token",
            CredentialPersistence::Memory,
        )
        .unwrap();
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let file_ref = store
        .inject(
            CredentialProvider::Anthropic,
            "file-private-token",
            CredentialPersistence::File,
        )
        .unwrap();
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("file-private-token"));
    assert!(!contents.contains("memory-private-token"));
    assert_eq!(store.memory_credentials().unwrap().len(), 1);
    store.clear_for_test();
    assert_eq!(store.hydrate_from_durable_store().unwrap(), 1);
    assert_eq!(
        store
            .resolve(CredentialProvider::Anthropic, &file_ref)
            .unwrap()
            .as_deref(),
        Some("file-private-token")
    );
    assert!(!store.replay_available_in_memory(CredentialProvider::Google, &memory_ref));
}

#[test]
fn failed_injection_persistence_does_not_publish_a_runtime_credential() {
    let _lock = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _guard = StorePathGuard::redirect(dir.path());
    let store = CredentialStore::default();
    let error = store
        .inject(
            CredentialProvider::Mcp,
            "private-failed-token",
            CredentialPersistence::File,
        )
        .unwrap_err();
    assert_eq!(error, "credential file persistence failed");
    assert_eq!(store.status().cached_count, 0);
}

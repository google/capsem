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

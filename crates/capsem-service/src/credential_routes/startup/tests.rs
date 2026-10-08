use super::*;
use crate::tests::{EnvVarGuard, SETTINGS_ENV_LOCK};

#[tokio::test]
async fn startup_does_not_capture_unselected_host_credentials() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let _store = EnvVarGuard::set("CAPSEM_CREDENTIAL_STORE_PATH", dir.path().join("store.json"));
    let _file = EnvVarGuard::set(INPUT_FILE_ENV, "");
    let _selection = EnvVarGuard::set("CAPSEM_CREDENTIAL_INJECTION_ENV", "");
    let _mode = EnvVarGuard::set(INPUT_STORAGE_ENV, "memory");
    let _keys: Vec<_> = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GITHUB_TOKEN",
        "GH_TOKEN",
    ]
    .iter()
    .map(|name| EnvVarGuard::set(name, ""))
    .collect();
    let _key = EnvVarGuard::set("OPENAI_API_KEY", "private-unselected-key");
    CredentialStore::global().clear_for_test();
    assert_eq!(import_host_inputs().unwrap(), 0);
    assert_eq!(CredentialStore::global().status().cached_count, 0);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    CredentialStore::global().clear_for_test();
}

#[tokio::test]
async fn startup_import_accepts_private_file_and_host_environment_without_persisting_memory() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.json");
    let store_path = dir.path().join("store.json");
    let _store = EnvVarGuard::set("CAPSEM_CREDENTIAL_STORE_PATH", &store_path);
    let _source = EnvVarGuard::set(INPUT_FILE_ENV, &input);
    let _selection = EnvVarGuard::set(INPUT_SELECTION_ENV, "OPENAI_API_KEY");
    let _mode = EnvVarGuard::set(INPUT_STORAGE_ENV, "memory");
    let _keys: Vec<_> = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GITHUB_TOKEN",
        "GH_TOKEN",
    ]
    .iter()
    .map(|name| EnvVarGuard::set(name, ""))
    .collect();
    let _key = EnvVarGuard::set("OPENAI_API_KEY", "private-host-key");
    CredentialStore::global().clear_for_test();
    capsem_foundation::unix::fs::atomic_write_private(
        &input,
        br#"{"credentials":[{"provider":"google","value":"private-file-source","storage":"memory"}]}"#,
    )
    .unwrap();
    assert_eq!(import_host_inputs().unwrap(), 2);
    for (provider, value) in [
        (CredentialProvider::OpenAi, "private-host-key"),
        (CredentialProvider::Google, "private-file-source"),
    ] {
        let reference = capsem_proto::credential_reference::credential_reference(provider.as_str(), value);
        assert_eq!(
            CredentialStore::global()
                .resolve(provider, &reference)
                .unwrap()
                .as_deref(),
            Some(value)
        );
    }
    assert!(!store_path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    capsem_foundation::unix::fs::atomic_write_private(&input, b"private-invalid-input").unwrap();
    assert_eq!(import_host_inputs().unwrap_err(), "invalid credential input file");
    let _invalid = EnvVarGuard::set(INPUT_STORAGE_ENV, "private-invalid-mode");
    assert_eq!(
        import_host_inputs().unwrap_err(),
        "invalid host credential storage mode"
    );
    CredentialStore::global().clear_for_test();
}

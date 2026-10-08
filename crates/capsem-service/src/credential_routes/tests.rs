use super::*;
use crate::tests::{make_test_state, EnvVarGuard, SETTINGS_ENV_LOCK};
use axum::body::Body;
use tower::ServiceExt;

#[tokio::test]
async fn injection_route_file_and_memory_return_only_references_and_invalid_input_is_redacted() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let _env = EnvVarGuard::set("CAPSEM_CREDENTIAL_STORE_PATH", &path);
    CredentialStore::global().clear_for_test();
    let app = build_service_router(make_test_state());
    for (storage, value) in [("memory", "private-memory-token"), ("file", "private-file-token")] {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/credentials/inject")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"provider":"openai","value":"{value}","storage":"{storage}"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["storage"], storage);
        assert_eq!(
            json["credential_ref"],
            capsem_proto::credential_reference::credential_reference("openai", value)
        );
        assert_eq!(json.as_object().unwrap().len(), 2);
        assert!(!String::from_utf8_lossy(&body).contains(value));
        if storage == "memory" {
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("private-file-token"));
    assert!(!contents.contains("private-memory-token"));
    let bad = r#"{"provider":"private-secret-invalid-provider","value":"private-secret-value"}"#;
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/credentials/inject")
                .header("content-type", "application/json")
                .body(Body::from(bad))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(!String::from_utf8_lossy(&body).contains("private-secret"));
    CredentialStore::global().clear_for_test();
}

#[tokio::test]
async fn memory_handoff_uses_correlated_real_host_ipc_and_acknowledgement() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    CredentialStore::global().clear_for_test();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("owner.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let reference = CredentialStore::global()
        .inject(
            CredentialProvider::Google,
            "private-injected-token",
            CredentialPersistence::Memory,
        )
        .unwrap();
    let owner = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = stream.into_std().unwrap();
        let stream = tokio::task::spawn_blocking(move || {
            capsem_foundation::ipc_handshake::negotiate_responder(&mut stream, "capsem-process-test", "").unwrap();
            stream
        })
        .await
        .unwrap();
        let (tx, rx) =
            capsem_foundation::ipc_channel::channel_from_std::<ProcessToService, ServiceToProcess>(stream).unwrap();
        let ServiceToProcess::InjectCredentials { id, credentials } = rx.recv().await.unwrap() else {
            panic!("expected injection");
        };
        let store = CredentialStore::default();
        store.import_memory_credentials(credentials).unwrap();
        assert_eq!(
            store
                .resolve(CredentialProvider::Google, &reference)
                .unwrap()
                .as_deref(),
            Some("private-injected-token")
        );
        tx.send(ProcessToService::StateChanged {
            id: "vm".into(),
            state: "Running".into(),
            trigger: "test".into(),
        })
        .await
        .unwrap();
        tx.send(ProcessToService::CredentialsInjected { id, error: None })
            .await
            .unwrap();
    });
    sync_memory(&make_test_state(), &socket).await.unwrap();
    owner.await.unwrap();
    assert!(sync_memory(&make_test_state(), &dir.path().join("absent.sock"))
        .await
        .is_err());
    CredentialStore::global().clear_for_test();
}

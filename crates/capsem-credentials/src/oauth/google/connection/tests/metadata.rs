use super::*;

#[tokio::test]
async fn invalid_history_cannot_replace_a_valid_connection_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private").join("connection.json");
    let (_fixture, client, tokens) = setup(vec![]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap())
        .await
        .unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut invalid = connection.snapshot().unwrap();
    invalid.updated_at_ms = u64::MAX;
    assert_eq!(
        connection.0.storage.write(&invalid),
        Err(GoogleConnectionError::InvalidRecord)
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "A rejected metadata write preserves the last committed authority"
    );
}

#[tokio::test]
async fn failed_refresh_commit_does_not_claim_committed_refresh_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private").join("connection.json");
    let (_fixture, client, tokens) = setup(vec![Reply::json(
        r#"{"access_token":"private-next","expires_in":60,"token_type":"Bearer"}"#,
    )])
    .await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap())
        .await
        .unwrap();
    let initial = connection.metadata().unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        connection
            .access(Instant::now() + Duration::from_secs(2))
            .await
            .unwrap_err(),
        GoogleConnectionError::StorageUnavailable
    );
    let failed = connection.metadata().unwrap();
    assert_eq!(failed.created_at_ms, initial.created_at_ms);
    assert_eq!(failed.last_refresh_at_ms, None);
    assert_eq!(failed.status.state, GoogleConnectionState::Degraded);
}

#[tokio::test]
async fn redacted_metadata_is_stable_and_follows_committed_refresh() {
    let (_fixture, client, tokens) = setup(vec![
        Reply::json(r#"{"access_token":"private-next","expires_in":60,"token_type":"Bearer"}"#),
        Reply::json(""),
        Reply::json(JWKS),
    ])
    .await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let initial = connection.metadata().unwrap();
    assert!(initial.created_at_ms > 0);
    assert_eq!(initial.updated_at_ms, initial.created_at_ms);
    assert_eq!(initial.last_refresh_at_ms, None);
    assert_eq!(initial.granted_scopes, BTreeSet::from(["openid".into()]));
    assert_eq!(
        connection.metadata().unwrap(),
        initial,
        "Polling cannot invent a new lifecycle timestamp"
    );
    let debug = format!("{initial:?}");
    for secret in [SUBJECT, "private-access", "private-refresh", "private-next"] {
        assert!(!debug.contains(secret));
    }
    connection
        .access(Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    let refreshed = connection.metadata().unwrap();
    assert_eq!(refreshed.created_at_ms, initial.created_at_ms);
    assert!(refreshed.updated_at_ms >= initial.updated_at_ms);
    assert_eq!(refreshed.last_refresh_at_ms, Some(refreshed.updated_at_ms));
    assert!(refreshed.status.revision > initial.status.revision);
    assert_eq!(connection.metadata().unwrap(), refreshed);
    connection.disconnect().await.unwrap();
    let disconnected = connection.metadata().unwrap();
    assert_eq!(disconnected.status.state, GoogleConnectionState::Disconnected);
    assert!(disconnected.granted_scopes.is_empty());
    assert_eq!(disconnected.created_at_ms, initial.created_at_ms);
    assert_eq!(disconnected.last_refresh_at_ms, refreshed.last_refresh_at_ms);
    let (_other_fixture, _, tokens) = setup(vec![]).await;
    connection.reconnect(tokens).await.unwrap();
    let reconnected = connection.metadata().unwrap();
    assert_eq!(reconnected.status.state, GoogleConnectionState::Connected);
    assert_eq!(reconnected.granted_scopes, initial.granted_scopes);
    assert_eq!(reconnected.created_at_ms, initial.created_at_ms);
    assert!(reconnected.updated_at_ms >= disconnected.updated_at_ms);
}

#[tokio::test]
async fn file_restore_preserves_metadata_and_rejects_invalid_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private").join("connection.json");
    let (_fixture, client, tokens) = setup(vec![]).await;
    let storage = OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap();
    let connection = client.open_connection(tokens, storage).await.unwrap();
    let initial = connection.metadata().unwrap();
    drop(connection);
    let connection = GoogleConnection::restore(
        client.clone(),
        OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(connection.metadata().unwrap(), initial);
    drop(connection);
    let original: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for invalid in [
        {
            let mut value = original.clone();
            value["created_at_ms"] = json!(0);
            value
        },
        {
            let mut value = original.clone();
            value["updated_at_ms"] = json!(1);
            value
        },
        {
            let mut value = original.clone();
            value["last_refresh_at_ms"] = json!(u64::MAX);
            value
        },
        {
            let mut value = original.clone();
            value["version"] = json!(3);
            value
        },
    ] {
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let storage = OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap();
        assert_eq!(
            GoogleConnection::restore(client.clone(), storage).await.unwrap_err(),
            GoogleConnectionError::InvalidRecord
        );
    }
}

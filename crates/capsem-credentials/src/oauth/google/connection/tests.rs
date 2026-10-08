use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::rand::SystemRandom;
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use serde_json::json;

use super::super::tests::fixture::{Fixture, Reply};
use super::*;
use crate::{LoopbackRedirect, OAuthAttempt, OAuthHttpPolicy, OAuthPolicy};

const CLIENT: &str = "fixture.apps.googleusercontent.com";
const SUBJECT: &str = "private-connection-subject";
const JWKS: &str = include_str!("../../../../../../tests/fixtures/oauth/identity-jwks-test-only.json");
const KEY: &[u8] = include_bytes!("../../../../../../tests/fixtures/oauth/identity-signing-test-only.der");

pub(crate) async fn setup(
    extra: Vec<Reply>,
) -> (Fixture, Arc<super::super::GoogleOAuthClient>, super::super::OAuthTokens) {
    setup_subject(SUBJECT, extra).await
}

async fn setup_subject(
    subject: &str,
    extra: Vec<Reply>,
) -> (Fixture, Arc<super::super::GoogleOAuthClient>, super::super::OAuthTokens) {
    let now = Instant::now();
    let mut attempt = OAuthAttempt::new(
        LoopbackRedirect::new("127.0.0.1:4123".parse().unwrap(), "/callback").unwrap(),
        OAuthPolicy {
            lifetime: Duration::from_secs(5),
            max_callback_bytes: 8192,
        },
        now,
    )
    .unwrap();
    let params = attempt.authorization(now).unwrap();
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let claims = json!({"iss":"https://accounts.google.com","aud":CLIENT,"sub":subject,"iat":epoch-1,"exp":epoch+60,"nonce":params.nonce});
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","kid":"test-only"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let pair = RsaKeyPair::from_der(KEY).unwrap();
    let mut signature = vec![0; pair.public().modulus_len()];
    pair.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        input.as_bytes(),
        &mut signature,
    )
    .unwrap();
    let jwt = format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature));
    let callback = format!(
        "http://127.0.0.1:4123/callback?state={}&code=private-code",
        params.state
    );
    let exchange = attempt.accept(&callback, now).unwrap();
    let body = json!({"access_token":"private-access","refresh_token":"private-refresh","id_token":jwt,"expires_in":1,"scope":"openid","token_type":"Bearer"}).to_string();
    let mut replies = vec![Reply::json(&body), Reply::json(JWKS)];
    replies.extend(extra);
    let fixture = Fixture::new(replies).await;
    let policy = OAuthHttpPolicy {
        request_timeout: Duration::from_secs(1),
        max_request_bytes: 4096,
        max_response_bytes: 8192,
    };
    let mut client = super::super::GoogleOAuthClient::new(
        super::super::GoogleRegistration::new(CLIENT.into(), None).unwrap(),
        BTreeSet::from(["openid".into()]),
        policy,
    )
    .unwrap();
    client.http = super::super::build_http(policy, false).unwrap();
    client.token_endpoint = format!("{}/token", fixture.base).parse().unwrap();
    client.identity_keys_endpoint = format!("{}/certs", fixture.base).parse().unwrap();
    client.revoke_endpoint = format!("{}/revoke", fixture.base).parse().unwrap();
    let client = Arc::new(client);
    let tokens = client.exchange(exchange).await.unwrap();
    (fixture, client, tokens)
}

#[tokio::test]
async fn connection_refresh_coalesces_and_disconnect_invalidates_old_leases() {
    let (fixture, client, tokens) = setup(vec![Reply::json(r#"{"access_token":"private-next","refresh_token":"private-rotated","expires_in":60,"token_type":"Bearer"}"#), Reply::json("")]).await;
    let storage = OAuthConnectionStorage::memory(65536).unwrap();
    let connection = client.open_connection(tokens, storage).await.unwrap();
    let consent = connection.authorization().unwrap();
    let old = connection.access(Instant::now()).await.unwrap();
    assert_eq!(old.with_token(Instant::now(), str::to_owned).unwrap(), "private-access");
    let now = Instant::now() + Duration::from_secs(2);
    let mut callers = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let connection = connection.clone();
        callers.spawn(async move {
            connection
                .access(now)
                .await
                .unwrap()
                .with_token(now, str::to_owned)
                .unwrap()
        });
    }
    while let Some(result) = callers.join_next().await {
        assert_eq!(result.unwrap(), "private-next");
    }
    assert_eq!(connection.status().revision, 2);
    assert_eq!(connection.authorization().unwrap().generation, consent.generation);
    let lease = connection.access(now).await.unwrap();
    let mut called = false;
    assert!(lease
        .with_scopes(now, &BTreeSet::from(["scope.missing".into()]), |_| {
            called = true;
        })
        .is_err());
    assert!(!called);
    assert_eq!(
        lease
            .with_scopes(now, &BTreeSet::from(["openid".into()]), str::to_owned)
            .unwrap(),
        "private-next"
    );
    assert_eq!(
        connection.disconnect().await.unwrap(),
        GoogleRevocationOutcome::Succeeded
    );
    assert_eq!(connection.status().state, GoogleConnectionState::Disconnected);
    assert!(connection.authorization().is_err());
    assert!(old.with_token(Instant::now(), str::to_owned).is_err());
    assert!(connection.access(Instant::now()).await.is_err());
    let debug = format!("{connection:?} {old:?}");
    for secret in [SUBJECT, "private-access", "private-refresh", "private-rotated"] {
        assert!(!debug.contains(secret));
    }
    let records = fixture.records.lock().unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!(records[2].fields["refresh_token"], "private-refresh");
    assert_eq!(records[3].fields["token"], "private-rotated");
    drop(records);
}

#[tokio::test]
async fn real_file_restart_requires_refresh_and_disconnection_survives_restart() {
    let (fixture, client, tokens) = setup(vec![
        Reply::json(r#"{"access_token":"private-restored","expires_in":60,"token_type":"Bearer"}"#),
        Reply::json(""),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private").join("connection.json");
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap())
        .await
        .unwrap();
    let bytes = std::fs::read_to_string(&path).unwrap();
    let consent = connection.authorization().unwrap();
    assert!(bytes.contains("private-refresh"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!bytes.contains("id_token"));
    assert!(!bytes.contains("private-code"));
    assert!(
        OAuthConnectionStorage::file(path.clone(), 65536).await.is_err(),
        "only one broker owns the file"
    );
    drop(connection);
    let restored = GoogleConnection::restore(
        client.clone(),
        OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(restored.authorization().unwrap(), consent);
    let lease = restored.access(Instant::now()).await.unwrap();
    assert_eq!(
        lease.with_token(Instant::now(), str::to_owned).unwrap(),
        "private-restored"
    );
    restored.disconnect().await.unwrap();
    drop(lease);
    drop(restored);
    let disconnected =
        GoogleConnection::restore(client, OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap())
            .await
            .unwrap();
    assert_eq!(disconnected.status().state, GoogleConnectionState::Disconnected);
    assert!(disconnected.access(Instant::now()).await.is_err());
    let bytes = std::fs::read_to_string(&path).unwrap();
    assert!(!bytes.contains("private-refresh"));
    assert!(!bytes.contains("private-restored"));
    drop(disconnected);
    let other_client = Arc::new(
        super::super::GoogleOAuthClient::new(
            super::super::GoogleRegistration::new("other.apps.googleusercontent.com".into(), None).unwrap(),
            BTreeSet::from(["openid".into()]),
            OAuthHttpPolicy {
                request_timeout: Duration::from_secs(1),
                max_request_bytes: 4096,
                max_response_bytes: 8192,
            },
        )
        .unwrap(),
    );
    let storage = OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap();
    assert!(
        matches!(
            GoogleConnection::restore(other_client, storage).await,
            Err(GoogleConnectionError::InvalidRecord)
        ),
        "disconnected records retain their registration binding"
    );
    assert_eq!(fixture.records.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn a_late_refresh_cannot_restore_a_disconnected_connection() {
    let mut refresh = Reply::json(r#"{"access_token":"private-late","expires_in":60,"token_type":"Bearer"}"#);
    refresh.delay = Duration::from_millis(100);
    let (fixture, client, tokens) = setup(vec![refresh, Reply::json("")]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    // Drain prior request notifications before observing the refresh admission.
    fixture.received.notified().await;
    let refresh_connection = connection.clone();
    let task = tokio::spawn(async move { refresh_connection.access(Instant::now() + Duration::from_secs(2)).await });
    fixture.received.notified().await;
    assert_eq!(
        connection.disconnect().await.unwrap(),
        GoogleRevocationOutcome::Succeeded
    );
    assert!(task.await.unwrap().is_err());
    assert_eq!(connection.status().state, GoogleConnectionState::Disconnected);
    assert!(connection.access(Instant::now()).await.is_err());
    assert_eq!(fixture.records.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn reauthorization_keeps_the_verified_account_and_invalidates_prior_leases() {
    let (fixture, client, tokens) = setup(vec![Reply::json(JWKS), Reply::json(JWKS)]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let old = connection.access(Instant::now()).await.unwrap();
    let consent = connection.authorization().unwrap();
    let (_other, _, wrong_tokens) = setup_subject("private-other-subject", vec![]).await;
    assert_eq!(
        connection.reconnect(wrong_tokens).await.unwrap_err(),
        GoogleConnectionError::Identity(OAuthIdentityError::AccountMismatch)
    );
    assert_eq!(connection.status().revision, 1);
    let (_same, _, right_tokens) = setup(vec![]).await;
    connection.reconnect(right_tokens).await.unwrap();
    assert_eq!(connection.status().revision, 2);
    assert_ne!(connection.authorization().unwrap().generation, consent.generation);
    assert!(old.with_token(Instant::now(), str::to_owned).is_err());
    assert_eq!(fixture.records.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn failed_rotation_commit_denies_material_instead_of_publishing_unstored_tokens() {
    let (fixture, client, tokens) = setup(vec![Reply::json(r#"{"access_token":"private-uncommitted","refresh_token":"private-uncommitted-refresh","expires_in":60,"token_type":"Bearer"}"#)]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private").join("connection.json");
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap())
        .await
        .unwrap();
    let lease = connection.access(Instant::now()).await.unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(matches!(
        connection.access(Instant::now() + Duration::from_secs(2)).await,
        Err(GoogleConnectionError::StorageUnavailable)
    ));
    assert_eq!(connection.status().state, GoogleConnectionState::Degraded);
    assert!(lease.with_token(Instant::now(), str::to_owned).is_err());
    assert!(connection.access(Instant::now()).await.is_err());
    assert_eq!(fixture.records.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn concurrent_transient_failures_share_one_refresh_and_a_new_request_can_retry() {
    let mut failed = Reply::json(r#"{"error":"temporarily_unavailable","description":"private-provider-echo"}"#);
    failed.status = 503;
    failed.delay = Duration::from_millis(100);
    let (fixture, client, tokens) = setup(vec![
        failed,
        Reply::json(r#"{"access_token":"private-recovered","expires_in":60,"token_type":"Bearer"}"#),
    ])
    .await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let now = Instant::now() + Duration::from_secs(2);
    let mut callers = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let connection = connection.clone();
        callers.spawn(async move { connection.access(now).await.unwrap_err() });
    }
    while let Some(result) = callers.join_next().await {
        let error = result.unwrap();
        assert!(matches!(error, GoogleConnectionError::Token(_)));
        assert!(!format!("{error:?} {error}").contains("private-provider-echo"));
    }
    assert_eq!(fixture.records.lock().unwrap().len(), 3);
    let lease = connection.access(now).await.unwrap();
    assert_eq!(lease.with_token(now, str::to_owned).unwrap(), "private-recovered");
    assert_eq!(fixture.records.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn invalid_grant_requires_reauthorization_and_corrupt_records_fail_closed() {
    let mut rejected = Reply::json(r#"{"error":"invalid_grant"}"#);
    rejected.status = 400;
    let (fixture, client, tokens) = setup(vec![rejected]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private").join("connection.json");
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::file(path.clone(), 65536).await.unwrap())
        .await
        .unwrap();
    assert!(connection
        .access(Instant::now() + Duration::from_secs(2))
        .await
        .is_err());
    assert_eq!(connection.status().state, GoogleConnectionState::ReauthRequired);
    assert!(connection.access(Instant::now()).await.is_err());
    drop(connection);
    capsem_foundation::unix::fs::atomic_write_private(&path, b"private-corrupt-record").unwrap();
    let error = GoogleConnection::restore(client, OAuthConnectionStorage::file(path, 65536).await.unwrap())
        .await
        .unwrap_err();
    assert_eq!(error, GoogleConnectionError::InvalidRecord);
    assert!(!format!("{error:?} {error}").contains("private-corrupt-record"));
    assert_eq!(fixture.records.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn disconnect_during_account_verification_prevents_reconnect_publication() {
    let mut keys = Reply::json(JWKS);
    keys.delay = Duration::from_millis(100);
    let (fixture, client, tokens) = setup(vec![keys, Reply::json("")]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let lease = connection.access(Instant::now()).await.unwrap();
    let (_fresh, _, fresh_tokens) = setup(vec![]).await;
    fixture.received.notified().await;
    let reauth_connection = connection.clone();
    let reauth = tokio::spawn(async move { reauth_connection.reconnect(fresh_tokens).await });
    fixture.received.notified().await;
    connection.disconnect().await.unwrap();
    assert_eq!(
        reauth.await.unwrap().unwrap_err(),
        GoogleConnectionError::RevisionChanged
    );
    assert_eq!(connection.status().state, GoogleConnectionState::Disconnected);
    assert!(lease.with_token(Instant::now(), str::to_owned).is_err());
    assert!(connection.access(Instant::now()).await.is_err());
    assert_eq!(fixture.records.lock().unwrap().len(), 4);
}

use super::*;
use crate::oauth::{test_connection, TestReply};
use crate::OAuthConnectionStorage;
use futures::FutureExt;

fn session() -> GrantSession {
    GrantSession::new([1; 16], 1, 1).unwrap()
}
fn target() -> GrantTarget {
    GrantTarget::new("POST", "https://api.example.test/v1/run").unwrap()
}
fn capability() -> GrantCapability {
    GrantCapability::new("fixture.run".into(), BTreeSet::from(["openid".into()]), vec![target()]).unwrap()
}
fn authority() -> GrantAuthority {
    GrantAuthority::new(GrantLimits {
        max_grants: 8,
        max_lifetime: Duration::from_secs(60),
    })
    .unwrap()
}
fn allow(_: &GrantSession, _: &GrantCapability) -> bool {
    true
}

#[tokio::test]
async fn denied_requests_never_resolve_or_refresh_credentials() {
    let (fixture, client, tokens) = test_connection(vec![]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let grants = authority();
    let now = Instant::now();
    let handle = grants
        .issue(connection, session(), capability(), Duration::from_secs(10), now, allow)
        .unwrap();
    let wrong = [
        GrantSession::new([2; 16], 1, 1).unwrap(),
        GrantSession::new([1; 16], 2, 1).unwrap(),
        GrantSession::new([1; 16], 1, 2).unwrap(),
    ];
    let later = now + Duration::from_secs(2);
    for binding in wrong {
        assert_eq!(
            grants
                .with_token(handle, binding, &target(), later, allow, |_| panic!("denied callback"))
                .await,
            Err(GrantError::Denied)
        );
    }
    for url in [
        "https://api.example.test.evil/v1/run",
        "https://api.example.test/v1/other",
        "https://api.example.test:8443/v1/run",
        "https://api.example.test/v1/run?extra=1",
    ] {
        let target = GrantTarget::new("POST", url).unwrap();
        assert_eq!(
            grants
                .with_token(handle, session(), &target, later, allow, |_| ())
                .await,
            Err(GrantError::Denied)
        );
    }
    let wrong_method = GrantTarget::new("GET", "https://api.example.test/v1/run").unwrap();
    assert_eq!(
        grants
            .with_token(handle, session(), &wrong_method, later, allow, |_| ())
            .await,
        Err(GrantError::Denied)
    );
    assert_eq!(
        grants
            .with_token(handle, session(), &target(), later, |_, _| false, |_| ())
            .await,
        Err(GrantError::Denied)
    );
    assert_eq!(
        grants
            .with_token(GrantHandle([0; 32]), session(), &target(), later, allow, |_| ())
            .await,
        Err(GrantError::Denied)
    );
    assert_eq!(
        grants
            .with_token(
                handle,
                session(),
                &target(),
                now + Duration::from_secs(10),
                allow,
                |_| ()
            )
            .await,
        Err(GrantError::Denied)
    );
    assert_eq!(fixture.records.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn valid_grants_survive_refresh_but_reauthorization_and_revoke_deny() {
    let (fixture, client, tokens) = test_connection(vec![
        TestReply::json(r#"{"access_token":"private-next","expires_in":60,"token_type":"Bearer"}"#),
        TestReply::json(include_str!(
            "../../../../tests/fixtures/oauth/identity-jwks-test-only.json"
        )),
    ])
    .await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let grants = authority();
    let now = Instant::now();
    let handle = grants
        .issue(
            connection.clone(),
            session(),
            capability(),
            Duration::from_secs(30),
            now,
            allow,
        )
        .unwrap();
    let later = now + Duration::from_secs(2);
    assert_eq!(
        grants
            .with_token(handle, session(), &target(), later, allow, str::to_owned)
            .await
            .unwrap(),
        "private-next"
    );
    assert_eq!(fixture.records.lock().unwrap().len(), 3);
    let (_other, _, tokens) = test_connection(vec![]).await;
    connection.reconnect(tokens).await.unwrap();
    let baseline = fixture.records.lock().unwrap().len();
    assert_eq!(
        grants
            .with_token(handle, session(), &target(), later, allow, |_| ())
            .await,
        Err(GrantError::Denied)
    );
    assert_eq!(fixture.records.lock().unwrap().len(), baseline);
    grants.revoke(handle).unwrap();
    assert_eq!(
        grants
            .with_token(handle, session(), &target(), later, allow, |_| ())
            .await,
        Err(GrantError::Denied)
    );
}

#[tokio::test]
async fn preservation_requires_explicit_same_owner_rebind_with_fresh_handle() {
    let (_fixture, client, tokens) = test_connection(vec![]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let grants = authority();
    let now = Instant::now();
    let old = grants
        .issue(connection, session(), capability(), Duration::from_secs(30), now, allow)
        .unwrap();
    grants.detach(session(), GrantStopPolicy::Preserve).unwrap();
    assert_eq!(
        grants.with_token(old, session(), &target(), now, allow, |_| ()).await,
        Err(GrantError::Denied)
    );
    assert!(grants
        .rebind(old, GrantSession::new([2; 16], 1, 2).unwrap(), now, allow)
        .is_err());
    assert!(grants
        .rebind(old, GrantSession::new([1; 16], 2, 2).unwrap(), now, allow)
        .is_err());
    assert!(grants.rebind(old, session(), now, allow).is_err());
    let resumed = GrantSession::new([1; 16], 1, 2).unwrap();
    let fresh = grants.rebind(old, resumed, now, allow).unwrap();
    assert_ne!(fresh, old);
    assert!(grants
        .with_token(old, resumed, &target(), now, allow, |_| ())
        .await
        .is_err());
    assert_eq!(
        grants
            .with_token(fresh, resumed, &target(), now, allow, str::to_owned)
            .await
            .unwrap(),
        "private-access"
    );
    grants.detach(resumed, GrantStopPolicy::Revoke).unwrap();
    assert!(grants
        .rebind(fresh, GrantSession::new([1; 16], 1, 3).unwrap(), now, allow)
        .is_err());
}

#[tokio::test]
async fn revoke_or_policy_change_while_refresh_is_in_flight_prevents_materialization() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for revoke in [true, false] {
        let mut reply = TestReply::json(r#"{"access_token":"private-next","expires_in":60,"token_type":"Bearer"}"#);
        reply.delay = Duration::from_millis(100);
        let (fixture, client, tokens) = test_connection(vec![reply]).await;
        let connection = client
            .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
            .await
            .unwrap();
        let grants = Arc::new(authority());
        let now = Instant::now();
        let handle = grants
            .issue(connection, session(), capability(), Duration::from_secs(30), now, allow)
            .unwrap();
        let _ = fixture.received.notified().now_or_never();
        let worker = grants.clone();
        let policy = Arc::new(AtomicBool::new(true));
        let worker_policy = policy.clone();
        let task = tokio::spawn(async move {
            worker
                .with_token(
                    handle,
                    session(),
                    &target(),
                    now + Duration::from_secs(2),
                    move |_, _| worker_policy.load(Ordering::Acquire),
                    |_| panic!("revoked callback"),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), fixture.received.notified())
            .await
            .unwrap();
        if revoke {
            grants.revoke(handle).unwrap();
        } else {
            policy.store(false, Ordering::Release);
        }
        assert_eq!(task.await.unwrap(), Err(GrantError::Denied));
        assert_eq!(fixture.records.lock().unwrap().len(), 3);
    }
}

#[test]
fn authority_inputs_are_finite_explicit_and_destination_constrained() {
    assert!(GrantSession::new([0; 16], 1, 1).is_err());
    assert!(GrantSession::new([1; 16], 0, 1).is_err());
    assert!(GrantAuthority::new(GrantLimits {
        max_grants: 0,
        max_lifetime: Duration::from_secs(60)
    })
    .is_err());
    for url in [
        "http://api.example.test/v1/run",
        "https://user:password@api.example.test/v1/run",
        "https://api.example.test/v1/run#fragment",
    ] {
        assert!(GrantTarget::new("POST", url).is_err());
    }
    assert!(GrantTarget::new("POST\r\nX:bad", "https://api.example.test/v1/run").is_err());
    assert!(GrantCapability::new("fixture".into(), BTreeSet::new(), vec![target()]).is_err());
}

#[tokio::test]
async fn grant_issuance_requires_scopes_policy_and_bounded_capacity() {
    let (fixture, client, tokens) = test_connection(vec![]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let grants = GrantAuthority::new(GrantLimits {
        max_grants: 1,
        max_lifetime: Duration::from_secs(10),
    })
    .unwrap();
    let now = Instant::now();
    let missing = GrantCapability::new(
        "fixture.extra".into(),
        BTreeSet::from(["scope.missing".into()]),
        vec![target()],
    )
    .unwrap();
    assert_eq!(
        grants.issue(
            connection.clone(),
            session(),
            missing,
            Duration::from_secs(10),
            now,
            allow
        ),
        Err(GrantError::Denied)
    );
    assert_eq!(
        grants.issue(
            connection.clone(),
            session(),
            capability(),
            Duration::from_secs(10),
            now,
            |_, _| false
        ),
        Err(GrantError::Denied)
    );
    assert_eq!(
        grants.issue(
            connection.clone(),
            session(),
            capability(),
            Duration::from_secs(11),
            now,
            allow
        ),
        Err(GrantError::Invalid)
    );
    let first = grants
        .issue(
            connection.clone(),
            session(),
            capability(),
            Duration::from_secs(10),
            now,
            allow,
        )
        .unwrap();
    assert_eq!(
        grants.issue(
            connection.clone(),
            session(),
            capability(),
            Duration::from_secs(10),
            now,
            allow
        ),
        Err(GrantError::Capacity)
    );
    let later = now + Duration::from_secs(10);
    let second = grants
        .issue(
            connection,
            session(),
            capability(),
            Duration::from_secs(10),
            later,
            allow,
        )
        .unwrap();
    assert_ne!(first, second);
    assert!(grants
        .with_token(first, session(), &target(), later, allow, |_| ())
        .await
        .is_err());
    assert_eq!(fixture.records.lock().unwrap().len(), 2);
}

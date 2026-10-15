use super::*;

#[tokio::test]
async fn lifecycle_runtime_fences_all_original_owners_and_leaves_replacements_and_neighbours() {
    let (_fixture, client, tokens) = test_connection(vec![]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let authority = authority();
    let old = session();
    let replacement = GrantSession::new([1; 16], [2; 16], [2; 16]).unwrap();
    let neighbour = GrantSession::new([2; 16], [1; 16], [1; 16]).unwrap();
    let issue = |binding| {
        authority
            .issue(
                connection.clone(),
                binding,
                capability(),
                Duration::from_secs(30),
                Instant::now(),
                allow,
            )
            .unwrap()
    };
    let original_handle = issue(old);
    let fresh = issue(replacement);
    let other = issue(neighbour);
    assert_eq!(
        authority
            .detach_runtime([1; 16], [1; 16], GrantStopPolicy::Preserve)
            .unwrap(),
        1
    );
    assert_eq!(
        authority.visibility(original_handle, old, Instant::now()),
        Some(GrantVisibility::Preserved)
    );
    assert_eq!(
        authority.visibility(fresh, replacement, Instant::now()),
        Some(GrantVisibility::Active)
    );
    assert_eq!(authority.revoke_session([1; 16]).unwrap(), 2);
    assert_eq!(authority.visibility(original_handle, old, Instant::now()), None);
    assert_eq!(authority.visibility(fresh, replacement, Instant::now()), None);
    assert_eq!(
        authority.visibility(other, neighbour, Instant::now()),
        Some(GrantVisibility::Active)
    );
}

#[tokio::test]
async fn issuance_checks_current_policy_inside_the_retirement_lock() {
    let (_fixture, client, tokens) = test_connection(vec![]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let authority = Arc::new(one_slot());
    let (observed, received) = std::sync::mpsc::channel();
    let held = authority.grants.lock().unwrap();
    let issuer = Arc::clone(&authority);
    let worker = std::thread::spawn(move || {
        issuer.issue(
            connection,
            session(),
            capability(),
            Duration::from_secs(10),
            Instant::now(),
            |_, _| {
                observed.send(()).unwrap();
                true
            },
        )
    });
    let checked_before_retirement = received.recv_timeout(Duration::from_millis(30)).is_ok();
    drop(held);
    worker.join().unwrap().unwrap();
    assert!(
        !checked_before_retirement,
        "policy/owner observations must follow admission to the same lock as terminal retirement"
    );
}

fn other_target() -> GrantTarget {
    GrantTarget::new("POST", "https://api.example.test/v1/other").unwrap()
}

fn other_capability() -> GrantCapability {
    GrantCapability::new(
        "fixture.other".into(),
        BTreeSet::from(["openid".into()]),
        vec![other_target()],
    )
    .unwrap()
}

fn one_slot() -> GrantAuthority {
    GrantAuthority::new(GrantLimits {
        max_grants: 1,
        max_lifetime: Duration::from_secs(60),
    })
    .unwrap()
}

#[tokio::test]
async fn one_grant_atomically_owns_all_selected_capabilities() {
    let (_fixture, client, tokens) = test_connection(vec![]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let grants = one_slot();
    let now = Instant::now();
    let handle = grants
        .issue_set(
            connection.clone(),
            session(),
            vec![capability(), other_capability()],
            Duration::from_secs(10),
            now,
            allow,
        )
        .unwrap();
    assert_eq!(
        grants.issue(connection, session(), capability(), Duration::from_secs(10), now, allow),
        Err(GrantError::Capacity)
    );
    for target in [target(), other_target()] {
        assert_eq!(
            grants
                .with_token(handle, session(), &target, now, allow, str::to_owned)
                .await
                .unwrap(),
            "private-access"
        );
    }
    assert_eq!(
        grants
            .with_token(
                handle,
                session(),
                &other_target(),
                now,
                |_, cap| cap.id() != "fixture.other",
                |_| panic!("denied capability callback")
            )
            .await,
        Err(GrantError::Denied)
    );
    grants.revoke(handle).unwrap();
    for target in [target(), other_target()] {
        assert_eq!(
            grants
                .with_token(handle, session(), &target, now, allow, |_| panic!(
                    "revoked set callback"
                ))
                .await,
            Err(GrantError::Denied)
        );
    }
}

#[tokio::test]
async fn denied_invalid_or_partial_consent_sets_leave_no_authority() {
    let (_fixture, client, tokens) = test_connection(vec![]).await;
    let connection = client
        .open_connection(tokens, OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let grants = one_slot();
    let now = Instant::now();
    let lifetime = Duration::from_secs(10);
    assert_eq!(
        grants.issue_set(
            connection.clone(),
            session(),
            vec![capability(), other_capability()],
            lifetime,
            now,
            |_, cap| cap.id() != "fixture.other"
        ),
        Err(GrantError::Denied)
    );
    for set in [vec![], vec![capability(), capability()]] {
        assert_eq!(
            grants.issue_set(connection.clone(), session(), set, lifetime, now, allow),
            Err(GrantError::Invalid)
        );
    }
    let unavailable = GrantCapability::new(
        "fixture.unconsented".into(),
        BTreeSet::from(["email".into()]),
        vec![other_target()],
    )
    .unwrap();
    assert_eq!(
        grants.issue_set(
            connection.clone(),
            session(),
            vec![capability(), unavailable],
            lifetime,
            now,
            allow
        ),
        Err(GrantError::Denied)
    );
    let single = grants
        .issue(connection.clone(), session(), capability(), lifetime, now, allow)
        .unwrap();
    assert_eq!(
        grants.issue_set(
            connection,
            session(),
            vec![capability(), other_capability()],
            lifetime,
            now,
            allow
        ),
        Err(GrantError::Capacity)
    );
    assert_eq!(
        grants
            .with_token(single, session(), &other_target(), now, allow, |_| panic!(
                "partial set leaked authority"
            ))
            .await,
        Err(GrantError::Denied)
    );
    assert_eq!(
        grants
            .with_token(single, session(), &target(), now, allow, str::to_owned)
            .await
            .unwrap(),
        "private-access"
    );
}

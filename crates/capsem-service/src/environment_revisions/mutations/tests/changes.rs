use super::*;

fn change(base: &EnvironmentMutationResponse, n: u64) -> EnvironmentChangeRequest {
    let mut specification = request(n).specification;
    specification.variables.insert("EDITOR".into(), "emacs".into());
    EnvironmentChangeRequest {
        request_id: id(n),
        base_revision_id: base.result.as_ref().unwrap().revision_id.clone(),
        specification,
        reason: "update development environment".into(),
        requesting_sandbox: None,
    }
}
async fn seed(f: &Fixture) -> EnvironmentMutationResponse {
    f.creates
        .submit(request(1), Source::default())
        .unwrap()
        .wait()
        .await
        .unwrap()
}

#[tokio::test]
async fn operator_change_retains_pins_and_reconciles_exact_frozen_intent() {
    let f = Fixture::new();
    let base = seed(&f).await;
    let intent = change(&base, 2);
    let source = Source {
        fail: true,
        ..Default::default()
    };
    let changed = f
        .creates
        .submit_change(base.environment_id.clone(), intent.clone(), source.clone())
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(changed.environment_id, base.environment_id);
    assert_eq!(changed.application, RequestApplicationState::Applied);
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    let current = f.revisions.load_current(&base.environment_id).await.unwrap();
    assert_eq!(
        current.revision.previous_revision_id,
        Some(intent.base_revision_id.clone())
    );
    assert_ne!(current.document.identity.revision_id, intent.base_revision_id);
    assert_eq!(current.document.specification.variables["EDITOR"], "emacs");
    let durable = f
        .revisions
        .ledger
        .environment_request_snapshot(&id(2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(durable.record.base_revision_id, Some(intent.base_revision_id.clone()));
    assert_eq!(durable.record.actor, "operator");
    assert_eq!(
        durable.record.result.as_ref().unwrap().revision_id,
        current.document.identity.revision_id
    );
    assert_eq!(
        f.creates
            .submit_change(base.environment_id.clone(), intent.clone(), source.clone())
            .unwrap()
            .wait()
            .await
            .unwrap(),
        changed
    );
    let mut reason_change = intent.clone();
    reason_change.reason.push_str(" changed");
    let mut content_change = intent.clone();
    content_change.specification.name = "different-intent".into();
    for altered in [reason_change, content_change] {
        assert_eq!(
            f.creates
                .submit_change(base.environment_id.clone(), altered, source.clone())
                .unwrap()
                .wait()
                .await
                .unwrap_err(),
            EnvironmentMutationError::Conflict
        );
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stale_change_refuses_resolution_and_does_not_create_a_request() {
    let f = Fixture::new();
    let base = seed(&f).await;
    f.creates
        .submit_change(base.environment_id.clone(), change(&base, 2), Source::default())
        .unwrap()
        .wait()
        .await
        .unwrap();
    let source = Source::default();
    assert_eq!(
        f.creates
            .submit_change(base.environment_id.clone(), change(&base, 3), source.clone())
            .unwrap()
            .wait()
            .await
            .unwrap_err(),
        EnvironmentMutationError::Conflict
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    assert!(f
        .revisions
        .ledger
        .environment_request_snapshot(&id(3))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn base_change_during_resolution_never_overwrites_or_persists_an_approvable_request() {
    let f = Fixture::new();
    let base = seed(&f).await;
    let mut slow = change(&base, 2);
    slow.specification.image.as_mut().unwrap().reference = "different:stable".into();
    let pause = Arc::new((tokio::sync::Notify::new(), tokio::sync::Notify::new()));
    let source = Source {
        pause: Some(pause.clone()),
        ..Default::default()
    };
    let pending = f
        .creates
        .submit_change(base.environment_id.clone(), slow, source)
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), pause.0.notified())
        .await
        .unwrap();
    let fast = f
        .creates
        .submit_change(base.environment_id.clone(), change(&base, 3), Source::default())
        .unwrap()
        .wait()
        .await
        .unwrap();
    pause.1.notify_one();
    assert_eq!(pending.wait().await.unwrap_err(), EnvironmentMutationError::Conflict);
    assert_eq!(
        f.revisions
            .load_current(&base.environment_id)
            .await
            .unwrap()
            .document
            .identity
            .revision_id,
        fast.result.unwrap().revision_id
    );
    assert!(f
        .revisions
        .ledger
        .environment_request_snapshot(&id(2))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn change_retry_after_later_edits_and_restart_never_rolls_back_current() {
    let f = Fixture::new();
    let base = seed(&f).await;
    let intent = change(&base, 2);
    let first = f
        .creates
        .submit_change(base.environment_id.clone(), intent.clone(), Source::default())
        .unwrap()
        .wait()
        .await
        .unwrap();
    let mut next = change(&first, 3);
    next.specification.variables.insert("EDITOR".into(), "nano".into());
    let latest = f
        .creates
        .submit_change(base.environment_id.clone(), next, Source::default())
        .unwrap()
        .wait()
        .await
        .unwrap();
    let root = f._root.path().to_path_buf();
    drop(f.creates);
    drop(f.revisions);
    let files = Arc::new(EnvironmentFiles::open(&root.join("environments")).unwrap());
    let ledger = Arc::new(DbHandle::open(&root.join("host.db")).unwrap());
    let revisions = Arc::new(EnvironmentRevisions::new(files, ledger));
    let coordinator = Arc::new(EnvironmentMutationCoordinator::new(revisions.clone()));
    let source = Source {
        fail: true,
        ..Default::default()
    };
    assert_eq!(
        coordinator
            .submit_change(base.environment_id.clone(), intent.clone(), source.clone())
            .unwrap()
            .wait()
            .await
            .unwrap(),
        first
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    let current = revisions.load_current(&base.environment_id).await.unwrap();
    assert_eq!(
        current.document.identity.revision_id,
        latest.result.unwrap().revision_id
    );
    assert_eq!(current.document.specification.variables["EDITOR"], "nano");
    assert_eq!(
        coordinator
            .submit_change(id(99), intent, source)
            .unwrap()
            .wait()
            .await
            .unwrap_err(),
        EnvironmentMutationError::Conflict
    );
}

#[tokio::test]
async fn change_refuses_caller_supplied_requester_before_resolution() {
    let f = Fixture::new();
    let base = seed(&f).await;
    let mut intent = change(&base, 2);
    intent.requesting_sandbox = Some(EnvironmentSandboxTarget {
        session_id: id(10),
        generation: id(11),
    });
    let source = Source::default();
    assert!(matches!(
        f.creates.submit_change(base.environment_id, intent, source.clone()),
        Err(EnvironmentMutationError::InvalidIntent)
    ));
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    assert!(f
        .revisions
        .ledger
        .environment_request_snapshot(&id(2))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn missing_environment_is_distinct_from_broken_known_current_state() {
    let f = Fixture::new();
    let base = seed(&f).await;
    let source = Source::default();
    assert_eq!(
        f.creates
            .submit_change(id(99), change(&base, 2), source.clone())
            .unwrap()
            .wait()
            .await
            .unwrap_err(),
        EnvironmentMutationError::NotFound
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    assert!(f
        .revisions
        .ledger
        .environment_request_snapshot(&id(2))
        .await
        .unwrap()
        .is_none());
}

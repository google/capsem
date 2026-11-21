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
async fn exact_operator_decision_publishes_one_fork_of_the_reviewed_base() {
    use capsem_api::requests::{RequestDecision, RequestResolveRequest};
    use capsem_api::CallerActor;

    let f = Fixture::new();
    let parent = seed(&f).await;
    let intent = change(&parent, 2);
    let base = parent.result.as_ref().unwrap().clone();
    let access = uuid::Uuid::parse_str(&id(10)).unwrap();
    let origin = EnvironmentProposalOrigin::delegated(access, id(11), uuid::Uuid::parse_str(&id(12)).unwrap());
    let proposed = f
        .creates
        .submit_proposal(
            base.environment_id.clone(),
            intent.clone(),
            origin,
            Source::default(),
            |_| Ok(()),
        )
        .unwrap()
        .wait()
        .await
        .unwrap();
    let observed = f
        .revisions
        .review_request(&proposed.request_id)
        .await
        .unwrap()
        .observed_revision;
    let approve = RequestResolveRequest {
        observed_revision: observed,
        decision: RequestDecision::Approve,
        reason: "approve the editor change for the original owner".into(),
    };
    let wrong = RequestResolveRequest {
        observed_revision: "abcdef000000".into(),
        ..approve.clone()
    };
    assert!(matches!(
        f.creates
            .resolve_proposal(id(2), wrong, &CallerActor::Operator, |_| false, |_| async { Ok(()) })
            .await,
        Err(EnvironmentMutationError::Conflict)
    ));
    let agent = CallerActor::DelegatedAgent {
        access_id: access.to_string(),
    };
    assert!(matches!(
        f.creates
            .resolve_proposal(id(2), approve.clone(), &agent, |_| true, |_| async { Ok(()) })
            .await,
        Err(EnvironmentMutationError::Refused)
    ));
    let first = f
        .creates
        .resolve_proposal(
            id(2),
            approve.clone(),
            &CallerActor::Operator,
            |_| false,
            |review| {
                let base = base.clone();
                async move {
                    assert_eq!(review.target.session_id, id(11));
                    assert_eq!(review.target.generation, id(12));
                    assert_eq!(review.base.environment_id, base.environment_id);
                    assert_eq!(review.base.revision_id, base.revision_id);
                    assert_eq!(review.specification.variables["EDITOR"], "emacs");
                    Ok(())
                }
            },
        )
        .await
        .unwrap();
    assert!(first.approved.is_some());
    let fork = first.response.result.as_ref().unwrap();
    assert_eq!(first.response.decision, RequestState::Approved);
    assert_eq!(first.response.application, RequestApplicationState::Pending);
    assert_ne!(fork.environment_id, base.environment_id);
    let saved = f.revisions.load_current(&fork.environment_id).await.unwrap();
    assert_eq!(saved.document.specification.variables["EDITOR"], "emacs");
    let fork_parent = saved.revision.fork_parent.unwrap();
    assert_eq!(fork_parent.revision_id, base.revision_id);
    assert_eq!(fork_parent.environment_id, base.environment_id);
    let shared = f.revisions.load_current(&base.environment_id).await.unwrap();
    assert_eq!(shared.document.specification.variables["EDITOR"], "vim");
    let retry = f
        .creates
        .resolve_proposal(
            id(2),
            approve,
            &CallerActor::Operator,
            |_| false,
            |_| async {
                panic!("an exact retry cannot revalidate or publish a second fork");
                #[allow(unreachable_code)]
                Ok(())
            },
        )
        .await
        .unwrap();
    assert!(retry.approved.is_none());
    assert_eq!(retry.response, first.response);
}

#[tokio::test]
async fn denied_cancelled_stale_and_interrupted_requests_never_publish_a_fork() {
    use capsem_api::requests::{RequestDecision, RequestResolveRequest};
    use capsem_api::CallerActor;

    let f = Fixture::new();
    let parent = seed(&f).await;
    let access = uuid::Uuid::parse_str(&id(20)).unwrap();
    let agent = CallerActor::DelegatedAgent {
        access_id: access.to_string(),
    };
    for (number, decision, caller, expected) in [
        (2, RequestDecision::Deny, &CallerActor::Operator, RequestState::Denied),
        (3, RequestDecision::Cancel, &agent, RequestState::Cancelled),
        (4, RequestDecision::Approve, &CallerActor::Operator, RequestState::Stale),
        (
            5,
            RequestDecision::Approve,
            &CallerActor::Operator,
            RequestState::Approved,
        ),
    ] {
        let origin = EnvironmentProposalOrigin::delegated(access, id(21), uuid::Uuid::parse_str(&id(22)).unwrap());
        f.creates
            .submit_proposal(
                parent.environment_id.clone(),
                change(&parent, number),
                origin,
                Source::default(),
                |_| Ok(()),
            )
            .unwrap()
            .wait()
            .await
            .unwrap();
        let request_id = id(number);
        let observed = f.revisions.review_request(&request_id).await.unwrap().observed_revision;
        let intent = RequestResolveRequest {
            observed_revision: observed,
            decision,
            reason: "resolve the original review".into(),
        };
        if decision == RequestDecision::Cancel {
            assert!(matches!(
                f.creates
                    .resolve_proposal(
                        request_id.clone(),
                        intent.clone(),
                        caller,
                        |_| false,
                        |_| async { Ok(()) }
                    )
                    .await,
                Err(EnvironmentMutationError::Refused)
            ));
        }
        if number == 5 {
            // A process may stop between the durable approval and fork preparation.
            f.creates
                .write(WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
                    request_id: request_id.clone(),
                    timestamp_unix_ms: now().unwrap(),
                    actor: "operator".into(),
                    decision: EnvironmentRequestDecisionKind::Approve,
                    reason: intent.reason.clone(),
                }))
                .await
                .unwrap();
        }
        let resolved = f
            .creates
            .resolve_proposal(
                request_id.clone(),
                intent,
                caller,
                |target| target.session_id == id(21) && target.generation == id(22),
                |_| async move {
                    assert_eq!(number, 4, "only the fresh approval checks live authority");
                    Err::<(), _>(EnvironmentMutationError::Conflict)
                },
            )
            .await
            .unwrap();
        assert!(resolved.approved.is_none());
        assert_eq!(resolved.response.decision, expected);
        assert_eq!(resolved.response.application, RequestApplicationState::Pending);
        assert!(resolved.response.result.is_none());
        assert!(f
            .revisions
            .ledger
            .environment_revisions(EnvironmentRevisionQuery::ForRequest { request_id })
            .await
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn delegated_proposal_rechecks_authority_after_source_resolution() {
    use std::sync::atomic::AtomicBool;

    let f = Fixture::new();
    let base = seed(&f).await;
    let mut intent = change(&base, 2);
    intent.specification.image.as_mut().unwrap().reference = "development:next".into();
    let pause = Arc::new((tokio::sync::Notify::new(), tokio::sync::Notify::new()));
    let live = Arc::new(AtomicBool::new(true));
    let authorization = live.clone();
    let job = f
        .creates
        .submit_proposal(
            base.environment_id,
            intent,
            EnvironmentProposalOrigin::delegated(
                uuid::Uuid::parse_str(&id(10)).unwrap(),
                id(11),
                uuid::Uuid::parse_str(&id(12)).unwrap(),
            ),
            Source {
                pause: Some(pause.clone()),
                ..Source::default()
            },
            move |_| {
                authorization
                    .load(Ordering::SeqCst)
                    .then_some(())
                    .ok_or(EnvironmentMutationError::Refused)
            },
        )
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), pause.0.notified())
        .await
        .unwrap();
    live.store(false, Ordering::SeqCst);
    pause.1.notify_one();
    assert_eq!(job.wait().await.unwrap_err(), EnvironmentMutationError::Refused);
    assert!(f
        .revisions
        .ledger
        .environment_request_history(&id(2))
        .await
        .unwrap()
        .is_none());
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

use super::*;

fn binding(id: &str) -> ApprovalBinding {
    ApprovalBinding {
        session_id: id.into(),
        generation: capsem_crypto::uuid_v4(),
    }
}

fn request(id: u64, bytes: u64) -> ApprovalRequest {
    ApprovalRequest {
        ask_id: format!("{id:012x}"),
        event_id: "abcdef012345".into(),
        event_type: "http.request".into(),
        rule_id: "test.ask".into(),
        rule_name: "Ask".into(),
        summary: "HTTP POST example.test".into(),
        held_bytes: bytes,
    }
}

#[tokio::test]
async fn original_continuation_resumes_only_after_one_audited_decision() {
    let broker = ApprovalBroker::default();
    let owner = binding("session");
    let pending = broker.register(owner.clone(), request(1, 7)).unwrap();
    assert_eq!(broker.pending_count(&owner), 1);
    let ticket = broker.claim(&owner, "000000000001").unwrap();
    assert!(matches!(
        broker.claim(&owner, "000000000001"),
        Err(ApprovalError::AlreadyResolving)
    ));
    assert!(ticket.complete_after_audit(ApprovalOutcome::Approved));
    assert_eq!(pending.wait().await, ApprovalOutcome::Approved);
    assert!(matches!(
        broker.claim(&owner, "000000000001"),
        Err(ApprovalError::Missing)
    ));
    assert_eq!(broker.pending_count(&owner), 0);
}

#[tokio::test]
async fn wrong_binding_and_runtime_retirement_cannot_approve() {
    let broker = ApprovalBroker::default();
    let owner = binding("session");
    let pending = broker.register(owner.clone(), request(1, 0)).unwrap();
    assert!(matches!(
        broker.claim(&binding("other"), "000000000001"),
        Err(ApprovalError::Missing)
    ));
    assert!(matches!(
        broker.claim(&binding("session"), "000000000001"),
        Err(ApprovalError::Missing)
    ));
    let ticket = broker.claim(&owner, "000000000001").unwrap();
    broker.cancel_generation(&owner);
    assert!(!ticket.complete_after_audit(ApprovalOutcome::Approved));
    assert_eq!(pending.wait().await, ApprovalOutcome::Cancelled);
}

#[tokio::test]
async fn timeout_wins_over_an_in_flight_resolution_and_releases_capacity() {
    let broker = ApprovalBroker::new(ApprovalLimits {
        timeout: Duration::from_millis(15),
        ..Default::default()
    })
    .unwrap();
    let owner = binding("session");
    let pending = broker.register(owner.clone(), request(1, 7)).unwrap();
    let ticket = broker.claim(&owner, "000000000001").unwrap();
    assert_eq!(pending.wait().await, ApprovalOutcome::Expired);
    assert!(!ticket.complete_after_audit(ApprovalOutcome::Approved));
    assert_eq!(broker.state.lock().unwrap().held_bytes, 0);
}

#[tokio::test]
async fn dropped_operation_and_failed_audit_never_resume_work() {
    let broker = ApprovalBroker::default();
    let owner = binding("session");
    let pending = broker.register(owner.clone(), request(1, 7)).unwrap();
    let ticket = broker.claim(&owner, "000000000001").unwrap();
    drop(pending);
    assert!(!ticket.complete_after_audit(ApprovalOutcome::Approved));
    let pending = broker.register(owner.clone(), request(2, 7)).unwrap();
    drop(broker.claim(&owner, "000000000002").unwrap());
    assert_eq!(pending.wait().await, ApprovalOutcome::Cancelled);
}

#[tokio::test]
async fn per_session_total_and_held_bytes_include_resolving_requests() {
    let broker = ApprovalBroker::new(ApprovalLimits {
        per_session: 1,
        total: 2,
        held_bytes: 10,
        ..Default::default()
    })
    .unwrap();
    let owner = binding("session");
    let pending = broker.register(owner.clone(), request(1, 7)).unwrap();
    let ticket = broker.claim(&owner, "000000000001").unwrap();
    assert!(matches!(
        broker.register(owner.clone(), request(2, 1)),
        Err(ApprovalError::Capacity)
    ));
    let other = binding("other");
    assert!(matches!(
        broker.register(other.clone(), request(2, 4)),
        Err(ApprovalError::Capacity)
    ));
    let second = broker.register(other, request(2, 3)).unwrap();
    assert!(matches!(
        broker.register(binding("third"), request(3, 0)),
        Err(ApprovalError::Capacity)
    ));
    assert!(ticket.complete_after_audit(ApprovalOutcome::Denied));
    assert_eq!(pending.wait().await, ApprovalOutcome::Denied);
    drop(second);
    assert_eq!(broker.state.lock().unwrap().held_bytes, 0);
}

#[tokio::test]
async fn retirement_is_generation_scoped_and_old_drop_cannot_cancel_reused_identity() {
    let broker = ApprovalBroker::default();
    let owner = binding("session");
    let old = broker.register(owner.clone(), request(1, 0)).unwrap();
    let newer = binding("session");
    let pending = broker.register(newer.clone(), request(1, 0)).unwrap();
    broker.cancel_generation(&owner);
    assert_eq!(old.wait().await, ApprovalOutcome::Cancelled);
    assert_eq!(broker.pending_count(&newer), 1);
    assert!(broker
        .claim(&newer, "000000000001")
        .unwrap()
        .complete_after_audit(ApprovalOutcome::Denied));
    // The old waiter survives until a replacement registration has the same key.
    let replacement = broker.register(newer.clone(), request(1, 0)).unwrap();
    drop(pending);
    assert_eq!(broker.pending_count(&newer), 1);
    drop(replacement);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_resolvers_and_stop_have_one_linearized_outcome() {
    for stop in [false, true] {
        for _ in 0..32 {
            let broker = ApprovalBroker::default();
            let owner = binding("session");
            let pending = broker.register(owner.clone(), request(1, 0)).unwrap();
            let barrier = Arc::new(tokio::sync::Barrier::new(3));
            let mut tasks = Vec::new();
            for index in 0..2 {
                let (broker, owner, barrier) = (broker.clone(), owner.clone(), Arc::clone(&barrier));
                tasks.push(tokio::spawn(async move {
                    barrier.wait().await;
                    if stop && index == 1 {
                        broker.cancel_generation(&owner);
                        return false;
                    }
                    broker
                        .claim(&owner, "000000000001")
                        .is_ok_and(|ticket| ticket.complete_after_audit(ApprovalOutcome::Approved))
                }));
            }
            barrier.wait().await;
            let mut delivered = 0;
            for task in tasks {
                delivered += usize::from(task.await.unwrap());
            }
            let outcome = pending.wait().await;
            assert!(delivered <= 1);
            assert_eq!(outcome.is_allowed(), delivered == 1);
            assert_eq!(broker.pending_count(&owner), 0);
        }
    }
}

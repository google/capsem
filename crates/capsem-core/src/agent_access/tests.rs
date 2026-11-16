use super::*;
use capsem_api::{AgentOperation, AgentSessionScope};
use std::time::{Duration, Instant};

fn scope() -> AgentAccessScope {
    AgentAccessScope::new(
        vec![AgentOperation::Inspect, AgentOperation::Execute],
        vec![],
        vec![AgentSessionScope::new(
            "11111111-1111-4111-8111-111111111111".into(),
            "22222222-2222-4222-8222-222222222222".into(),
        )
        .unwrap()],
    )
    .unwrap()
}
fn clock() -> LeaseClock {
    LeaseClock::new(1000, Instant::now())
}
fn pending(registry: &Registry, id: Uuid, cap: &Capability, at: LeaseClock) -> Admission {
    match registry
        .reserve(id, cap, scope(), Duration::from_secs(10), || Some(at))
        .unwrap()
    {
        Reservation::New(admission) => admission,
        _ => panic!("expected new admission"),
    }
}

#[test]
fn delegated_registry_requires_audit_commit_and_exact_retry_intent() {
    let registry = Registry::new(2, 4).unwrap();
    let id = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([19; 32]);
    let at = clock();
    let admission = pending(&registry, id, &cap, at);
    assert!(registry.authenticate(id, &cap, || Some(at)).is_err());
    assert!(matches!(
        registry.reserve(id, &cap, scope(), Duration::from_secs(10), || Some(at)),
        Err(Failure::Pending)
    ));
    let grant = admission.commit(|| Some(at)).unwrap();
    assert_eq!(grant.id(), id);
    assert!(grant.authorizes_session(
        AgentOperation::Execute,
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
        || Some(at)
    ));
    assert!(!grant.authorizes_session(
        AgentOperation::Execute,
        "11111111-1111-4111-8111-111111111111",
        "33333333-3333-4333-8333-333333333333",
        || Some(at)
    ));
    assert!(!grant.authorizes_operation(AgentOperation::Launch, || Some(at)));
    let retry = registry
        .reserve(id, &cap, scope(), Duration::from_secs(10), || Some(at))
        .unwrap();
    let Reservation::Existing(retry) = retry else {
        panic!("retry must reuse authority")
    };
    assert!(Arc::ptr_eq(&grant, &retry));
    assert!(matches!(
        registry.reserve(id, &cap, scope(), Duration::from_secs(11), || Some(at)),
        Err(Failure::Conflict)
    ));
    let changed = AgentAccessScope::new(vec![AgentOperation::Discover], vec![], vec![]).unwrap();
    assert!(matches!(
        registry.reserve(id, &cap, changed, Duration::from_secs(10), || Some(at)),
        Err(Failure::Conflict)
    ));
    let wrong = Capability::from_bytes([20; 32]);
    assert!(matches!(
        registry.reserve(id, &wrong, scope(), Duration::from_secs(10), || Some(at)),
        Err(Failure::Forbidden)
    ));
    assert!(registry.authenticate(id, &wrong, || Some(at)).is_err());
    assert!(registry
        .authenticate(capsem_crypto::uuid_v4(), &cap, || Some(at))
        .is_err());
}

#[test]
fn delegated_registry_revokes_pending_and_live_grants_without_losing_attribution() {
    let registry = Registry::new(2, 4).unwrap();
    let cap = Capability::from_bytes([21; 32]);
    let at = clock();
    let id = capsem_crypto::uuid_v4();
    let admission = pending(&registry, id, &cap, at);
    assert!(registry.revoke(id).unwrap());
    assert!(admission.commit(|| Some(at)).is_err());
    assert!(registry.recognizes(id, &cap).unwrap());
    assert!(!registry.revoke(id).unwrap());
    let live_id = capsem_crypto::uuid_v4();
    let grant = pending(&registry, live_id, &cap, at).commit(|| Some(at)).unwrap();
    assert!(registry.revoke(live_id).unwrap());
    assert!(!grant.is_live(|| Some(at)));
    assert!(registry.authenticate(live_id, &cap, || Some(at)).is_err());
    assert!(registry.recognizes(live_id, &cap).unwrap());
    assert!(!registry.recognizes(live_id, &Capability::from_bytes([22; 32])).unwrap());
}

#[test]
fn delegated_registry_aborts_dropped_admission_and_expires_during_audit() {
    let registry = Registry::new(1, 4).unwrap();
    let cap = Capability::from_bytes([23; 32]);
    let at = clock();
    let id = capsem_crypto::uuid_v4();
    drop(pending(&registry, id, &cap, at));
    assert!(registry.authenticate(id, &cap, || Some(at)).is_err());
    assert!(matches!(
        registry.reserve(id, &cap, scope(), Duration::from_secs(10), || Some(at)),
        Err(Failure::Closed)
    ));
    let id = capsem_crypto::uuid_v4();
    let admission = pending(&registry, id, &cap, at);
    assert!(admission
        .commit(|| Some(LeaseClock::new(11000, Instant::now())))
        .is_err());
    assert!(registry.authenticate(id, &cap, || Some(at)).is_err());
}

#[test]
fn delegated_registry_bounds_pending_active_and_tombstone_capacity() {
    assert!(Registry::new(0, 2).is_err());
    assert!(Registry::new(3, 2).is_err());
    let registry = Registry::new(1, 2).unwrap();
    let cap = Capability::from_bytes([24; 32]);
    let at = clock();
    let first = capsem_crypto::uuid_v4();
    let admission = pending(&registry, first, &cap, at);
    assert!(matches!(
        registry.reserve(capsem_crypto::uuid_v4(), &cap, scope(), Duration::from_secs(10), || {
            Some(at)
        }),
        Err(Failure::Capacity)
    ));
    drop(admission);
    let second = capsem_crypto::uuid_v4();
    let grant = pending(&registry, second, &cap, at).commit(|| Some(at)).unwrap();
    assert!(registry.revoke(second).unwrap());
    assert!(!grant.is_live(|| Some(at)));
    assert!(matches!(
        registry.reserve(capsem_crypto::uuid_v4(), &cap, scope(), Duration::from_secs(10), || {
            Some(at)
        }),
        Err(Failure::Capacity)
    ));
}

#[test]
fn delegated_registry_restart_revokes_outstanding_proofs_and_expiry_never_revives() {
    let cap = Capability::from_bytes([25; 32]);
    let id = capsem_crypto::uuid_v4();
    let at = clock();
    let registry = Registry::new(1, 2).unwrap();
    let grant = pending(&registry, id, &cap, at).commit(|| Some(at)).unwrap();
    drop(registry);
    assert!(!grant.is_live(|| Some(at)));
    let fresh = Registry::new(1, 2).unwrap();
    assert!(fresh.authenticate(id, &cap, || Some(at)).is_err());
    let new_id = capsem_crypto::uuid_v4();
    let grant = pending(&fresh, new_id, &cap, at).commit(|| Some(at)).unwrap();
    assert!(fresh
        .authenticate(new_id, &cap, || Some(LeaseClock::new(11000, Instant::now())))
        .is_err());
    assert!(!grant.is_live(|| Some(at)));
    assert!(fresh.recognizes(new_id, &cap).unwrap());
}

#[test]
fn delegated_registry_revocation_during_clock_check_cannot_return_a_live_proof() {
    let registry = Registry::new(1, 2).unwrap();
    let cap = Capability::from_bytes([26; 32]);
    let at = clock();
    let grant = pending(&registry, capsem_crypto::uuid_v4(), &cap, at)
        .commit(|| Some(at))
        .unwrap();
    // A concurrent revoke can arrive while the lease's clock is sampled.
    assert!(!grant.is_live(|| {
        grant.cancel.cancel();
        Some(at)
    }));
}

#[test]
fn delegated_registry_expires_capacity_and_fails_closed_on_clock_unavailability() {
    let registry = Registry::new(1, 4).unwrap();
    let cap = Capability::from_bytes([27; 32]);
    let at = clock();
    assert!(matches!(
        registry.reserve(Uuid::nil(), &cap, scope(), Duration::from_secs(10), || Some(at)),
        Err(Failure::Invalid)
    ));
    assert!(matches!(
        registry.reserve(
            capsem_crypto::uuid_v4(),
            &cap,
            scope(),
            Duration::from_secs(86401),
            || Some(at)
        ),
        Err(Failure::Invalid)
    ));
    assert!(matches!(
        registry.reserve(capsem_crypto::uuid_v4(), &cap, scope(), Duration::from_secs(10), || {
            None
        }),
        Err(Failure::Unavailable)
    ));
    let old = pending(&registry, capsem_crypto::uuid_v4(), &cap, at)
        .commit(|| Some(at))
        .unwrap();
    registry
        .expire(|| Some(LeaseClock::new(11000, Instant::now())))
        .unwrap();
    assert!(!old.is_live(|| Some(at)));
    let fresh_clock = LeaseClock::new(12000, Instant::now());
    let fresh = pending(&registry, capsem_crypto::uuid_v4(), &cap, fresh_clock)
        .commit(|| Some(fresh_clock))
        .unwrap();
    assert!(!fresh.is_live(|| None));
    assert!(!fresh.is_live(|| Some(fresh_clock)));
}

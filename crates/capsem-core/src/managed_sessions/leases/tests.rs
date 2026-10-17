use super::*;

#[test]
fn policy_narrowing_uses_original_renewal_and_never_extends_existing_leases() {
    let directory = tempfile::tempdir().unwrap();
    let registry = Registry::open(&directory.path().join("ownership")).unwrap();
    let cap = Capability::from_bytes([91; 32]);
    let request = capsem_crypto::uuid_v4();
    let Reservation::New(ticket) = registry.reserve(request, &cap).unwrap() else {
        panic!()
    };
    let now = Instant::now();
    let policy = |seconds| LeasePolicy::new(Duration::from_secs(seconds)).unwrap();
    let clock = |seconds| LeaseClock::new(1000 + seconds * 1000, now + Duration::from_secs(seconds));
    registry.start_lease(&ticket, policy(300), clock(0)).unwrap();
    registry.begin_create(&ticket, clock(0)).unwrap();
    registry.set_lease_limit(policy(100), clock(80)).unwrap();
    let snapshot = registry.inspect(request, &cap).unwrap().unwrap();
    assert_eq!(snapshot.lease_duration(), Some(Duration::from_secs(100)));
    assert_eq!(snapshot.expires_wall_ms(), Some(101000));
    assert_eq!(
        registry.runtime_deadline(&ticket).unwrap().at,
        now + Duration::from_secs(100)
    );
    registry.set_lease_limit(policy(600), clock(90)).unwrap();
    assert_eq!(
        registry.inspect(request, &cap).unwrap().unwrap().expires_wall_ms(),
        Some(101000)
    );
    assert_eq!(
        registry
            .renew_lease(request, &cap, clock(95))
            .unwrap()
            .expires_wall_ms(),
        Some(196000)
    );
    registry.set_lease_limit(policy(20), clock(130)).unwrap();
    assert_eq!(
        registry.inspect(request, &cap).unwrap().unwrap().state(),
        State::Closing
    );
    assert_eq!(
        registry.renew_lease(request, &cap, clock(131)).unwrap().state(),
        State::Closing
    );
}

#[test]
fn failed_policy_reload_still_limits_queued_admission_and_renewal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ownership");
    let registry = Registry::open(&path).unwrap();
    let cap = Capability::from_bytes([92; 32]);
    let request = capsem_crypto::uuid_v4();
    let Reservation::New(ticket) = registry.reserve(request, &cap).unwrap() else {
        panic!()
    };
    let now = Instant::now();
    let clock = LeaseClock::new(1000, now);
    let policy = |seconds| LeasePolicy::new(Duration::from_secs(seconds)).unwrap();
    let corrupt = path.join(format!("{}.json", capsem_crypto::uuid_v4()));
    std::fs::write(&corrupt, b"corrupt").unwrap();
    assert!(registry.set_lease_limit(policy(20), clock).is_err());
    std::fs::remove_file(corrupt).unwrap();
    let admitted = registry.start_lease(&ticket, policy(300), clock).unwrap();
    assert_eq!(admitted.lease_duration(), Some(Duration::from_secs(20)));
    registry.begin_create(&ticket, clock).unwrap();
    registry.set_lease_limit(policy(10), clock).unwrap();
    let renewed = registry
        .renew_lease(request, &cap, LeaseClock::new(2000, now + Duration::from_secs(1)))
        .unwrap();
    assert_eq!(renewed.lease_duration(), Some(Duration::from_secs(10)));
    assert_eq!(renewed.expires_wall_ms(), Some(12000));
}

#[test]
fn unavailable_policy_denies_admission_and_renewal_until_validated_reload() {
    let directory = tempfile::tempdir().unwrap();
    let registry = Registry::open(&directory.path().join("ownership")).unwrap();
    let cap = Capability::from_bytes([95; 32]);
    let request = capsem_crypto::uuid_v4();
    let Reservation::New(ticket) = registry.reserve(request, &cap).unwrap() else {
        panic!()
    };
    let clock = LeaseClock::new(1000, Instant::now());
    let policy = LeasePolicy::new(Duration::from_secs(10)).unwrap();
    registry.deny_lease_admission().unwrap();
    assert!(registry.start_lease(&ticket, policy, clock).is_err());
    registry.set_lease_limit(policy, clock).unwrap();
    registry.start_lease(&ticket, policy, clock).unwrap();
    registry.begin_create(&ticket, clock).unwrap();
    registry.deny_lease_admission().unwrap();
    assert!(registry.renew_lease(request, &cap, clock).is_err());
    registry.close(request, &cap).unwrap();
    assert_eq!(
        registry.inspect(request, &cap).unwrap().unwrap().state(),
        State::Closing
    );
}

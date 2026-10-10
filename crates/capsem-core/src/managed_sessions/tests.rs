use super::*;

#[test]
fn recovery_inventory_is_canonical_and_refuses_unknown_or_linked_records() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let store = Registry::open(&path).unwrap();
    let request = Uuid::new_v4();
    let cap = Capability::from_bytes([51; 32]);
    let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
        panic!()
    };
    let inventory = store.recovery_inventory().unwrap();
    assert_eq!(inventory.len(), 1);
    assert_eq!(inventory[0].request(), request);
    assert_eq!(inventory[0].generation(), ticket.generation());
    assert!(!format!("{inventory:?}").contains("capability_hash"));
    std::fs::write(path.join("unknown.json"), b"{} ").unwrap();
    assert!(store.recovery_inventory().is_err());
    std::fs::remove_file(path.join("unknown.json")).unwrap();
    let record = path.join(format!("{request}.json"));
    let outside = root.path().join("outside");
    std::fs::rename(&record, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &record).unwrap();
    assert!(store.recovery_inventory().is_err());
    std::fs::remove_file(&record).unwrap();
    std::fs::write(&record, b"broken").unwrap();
    assert!(store.recovery_inventory().is_err());
}

#[test]
fn prepared_spawn_identity_survives_restart_without_claiming_a_live_vm() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let store = Registry::open(&path).unwrap();
    let request = Uuid::new_v4();
    let cap = Capability::from_bytes([30; 32]);
    let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
        panic!()
    };
    initialize_test_lease(&store, &ticket);
    store.begin_create(&ticket, test_lease_clock()).unwrap();
    let intent = VmBinding::new("chosen-before-spawn".into(), Uuid::new_v4()).unwrap();
    let prepared = store
        .prepare_spawn(&ticket, intent.clone(), test_lease_clock())
        .unwrap();
    assert_eq!(prepared.state(), State::Creating);
    assert!(prepared.vm().is_none());
    assert_eq!(prepared.spawn_intent(), Some(&intent));
    let other = VmBinding::new(intent.id().into(), Uuid::new_v4()).unwrap();
    assert!(store.prepare_spawn(&ticket, other.clone(), test_lease_clock()).is_err());
    assert!(store.bind_created(&ticket, other, test_lease_clock()).is_err());
    drop(store);
    let restarted = Registry::open(&path).unwrap();
    let (_, unknown) = restarted.recover_interrupted(request).unwrap().unwrap();
    assert_eq!(unknown.state(), State::Unknown);
    assert!(unknown.vm().is_none());
    assert_eq!(unknown.spawn_intent(), Some(&intent));
    assert_eq!(unknown.generation(), ticket.generation());
}

#[test]
fn close_and_expiry_refuse_unprepared_vm_side_effects() {
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let store = Registry::open(&root.path().join("ownership")).unwrap();
    let cap = Capability::from_bytes([31; 32]);
    let now = Instant::now();
    for closing in [false, true] {
        let request = Uuid::new_v4();
        let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
            panic!()
        };
        store
            .start_lease(
                &ticket,
                LeasePolicy::new(Duration::from_secs(2)).unwrap(),
                LeaseClock::new(1000, now),
            )
            .unwrap();
        store.begin_create(&ticket, LeaseClock::new(1000, now)).unwrap();
        let clock = if closing {
            store.close(request, &cap).unwrap();
            LeaseClock::new(1000, now)
        } else {
            LeaseClock::new(4000, now + Duration::from_secs(3))
        };
        assert!(store
            .prepare_spawn(
                &ticket,
                VmBinding::new("never-spawn".into(), Uuid::new_v4()).unwrap(),
                clock
            )
            .is_err());
        let pending = store.inspect(request, &cap).unwrap().unwrap();
        assert_eq!(pending.state(), State::Closing);
        assert!(pending.spawn_intent().is_none());
    }
}

fn test_lease_clock() -> LeaseClock {
    LeaseClock::new(1000, std::time::Instant::now())
}

fn initialize_test_lease(store: &Registry, ticket: &Ticket) {
    store
        .start_lease(
            ticket,
            LeasePolicy::new(std::time::Duration::from_secs(60)).unwrap(),
            test_lease_clock(),
        )
        .unwrap();
}

#[test]
fn reservation_is_durable_hash_only_and_never_replays_creation() {
    let root = tempfile::tempdir().unwrap();
    let store = Registry::open(&root.path().join("ownership")).unwrap();
    let request = uuid::Uuid::new_v4();
    let capability = Capability::from_bytes(*b"high-entropy-test-capability-123");
    assert!(!format!("{capability:?}").contains("high-entropy"));
    let first = store.reserve(request, &capability).unwrap();
    let Reservation::New(ticket) = first else {
        panic!("first request must reserve creation");
    };
    assert_eq!(ticket.request(), request);
    let generation = ticket.generation();
    assert!(!generation.is_nil());
    let bytes = std::fs::read(root.path().join("ownership").join(format!("{request}.json"))).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("high-entropy-test-capability"));
    let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_ne!(
        record["capability_hash"],
        serde_json::json!(*b"high-entropy-test-capability-123"),
        "the store must not disguise plaintext capability bytes as a hash"
    );
    let reopened = Registry::open(&root.path().join("ownership")).unwrap();
    let Reservation::Existing(existing) = reopened.reserve(request, &capability).unwrap() else {
        panic!("restart must never replay creation");
    };
    assert_eq!(existing.generation(), generation);
    assert_eq!(existing.state(), State::Reserved);
    let wrong = Capability::from_bytes([9; 32]);
    assert!(reopened.reserve(request, &wrong).is_err());
    assert_eq!(
        std::fs::read(root.path().join("ownership").join(format!("{request}.json"))).unwrap(),
        bytes
    );
}

#[test]
fn contained_ownership_record_refuses_links_corruption_and_nil_request() {
    let root = tempfile::tempdir().unwrap();
    let store = Registry::open(&root.path().join("ownership")).unwrap();
    let request = uuid::Uuid::new_v4();
    let capability = Capability::from_bytes([7; 32]);
    store.reserve(request, &capability).unwrap();
    let path = root.path().join("ownership").join(format!("{request}.json"));
    let original = std::fs::read(&path).unwrap();
    let outside = root.path().join("outside");
    std::fs::write(&outside, &original).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(store.reserve(request, &capability).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), original);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"broken JSON").unwrap();
    assert!(store.reserve(request, &capability).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"broken JSON");
    assert!(store.reserve(uuid::Uuid::nil(), &capability).is_err());
}

#[test]
fn close_before_create_is_a_durable_tombstone() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let store = Registry::open(&path).unwrap();
    let request = Uuid::new_v4();
    let cap = Capability::from_bytes([3; 32]);
    let closed = store.close(request, &cap).unwrap();
    assert_eq!(closed.state(), State::Closed);
    let reopened = Registry::open(&path).unwrap();
    let Reservation::Existing(existing) = reopened.reserve(request, &cap).unwrap() else {
        panic!("close must prevent later creation");
    };
    assert_eq!(existing.generation(), closed.generation());
    assert_eq!(existing.state(), State::Closed);
    assert!(reopened.close(request, &Capability::from_bytes([4; 32])).is_err());
}

#[test]
fn close_racing_create_requires_matching_vm_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let store = Registry::open(&root.path().join("ownership")).unwrap();
    let request = Uuid::new_v4();
    let cap = Capability::from_bytes([5; 32]);
    let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
        panic!()
    };
    initialize_test_lease(&store, &ticket);
    assert!(store.begin_create(&ticket, test_lease_clock()).unwrap());
    assert!(!store.begin_create(&ticket, test_lease_clock()).unwrap());
    let binding = VmBinding::new("vm-123".into(), Uuid::new_v4()).unwrap();
    store
        .prepare_spawn(&ticket, binding.clone(), test_lease_clock())
        .unwrap();
    assert_eq!(store.close(request, &cap).unwrap().state(), State::Closing);
    let late = store
        .bind_created(&ticket, binding.clone(), test_lease_clock())
        .unwrap();
    assert_eq!(late.state(), State::Closing);
    assert_eq!(late.vm(), Some(&binding));
    let wrong = VmBinding::new("vm-123".into(), Uuid::new_v4()).unwrap();
    assert!(store.complete_cleanup(&ticket, &wrong).is_err());
    assert_eq!(store.close(request, &cap).unwrap().state(), State::Closing);
    let closed = store.complete_cleanup(&ticket, &binding).unwrap();
    assert_eq!(closed.state(), State::Closed);
    assert!(closed.vm().is_none());
    assert!(
        store.runtime_deadlines().unwrap().is_empty(),
        "closed owners must release runtime lease tracking"
    );
    assert!(store.bind_created(&ticket, binding, test_lease_clock()).is_err());
}

#[test]
fn create_and_close_transitions_refuse_stale_tickets_and_vm_rebinding() {
    let root = tempfile::tempdir().unwrap();
    let store = Registry::open(&root.path().join("ownership")).unwrap();
    let request = Uuid::new_v4();
    let cap = Capability::from_bytes([6; 32]);
    let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
        panic!()
    };
    initialize_test_lease(&store, &ticket);
    let stale = Ticket {
        request,
        generation: Uuid::new_v4(),
    };
    assert!(store.begin_create(&stale, test_lease_clock()).is_err());
    assert!(store.begin_create(&ticket, test_lease_clock()).unwrap());
    let binding = VmBinding::new("vm-456".into(), Uuid::new_v4()).unwrap();
    store
        .prepare_spawn(&ticket, binding.clone(), test_lease_clock())
        .unwrap();
    assert_eq!(
        store
            .bind_created(&ticket, binding.clone(), test_lease_clock())
            .unwrap()
            .state(),
        State::Active
    );
    assert!(store.complete_cleanup(&ticket, &binding).is_err());
    assert!(store
        .bind_created(
            &ticket,
            VmBinding::new("other".into(), Uuid::new_v4()).unwrap(),
            test_lease_clock()
        )
        .is_err());
    assert_eq!(
        store
            .bind_created(&ticket, binding.clone(), test_lease_clock())
            .unwrap()
            .state(),
        State::Active
    );
    store.close(request, &cap).unwrap();
    assert!(store.complete_cleanup(&stale, &binding).is_err());
    store.complete_cleanup(&ticket, &binding).unwrap();
    assert!(!store.begin_create(&ticket, test_lease_clock()).unwrap());
    assert!(VmBinding::new("".into(), Uuid::new_v4()).is_err());
    assert!(VmBinding::new("valid".into(), Uuid::nil()).is_err());
}

#[test]
fn restart_never_replays_unfinished_create_and_keeps_unknown_outcome() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let store = Registry::open(&path).unwrap();
    let cap = Capability::from_bytes([8; 32]);
    let request = Uuid::new_v4();
    let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
        panic!()
    };
    initialize_test_lease(&store, &ticket);
    store.begin_create(&ticket, test_lease_clock()).unwrap();
    drop(store);
    let restarted = Registry::open(&path).unwrap();
    let (recovered, snapshot) = restarted.recover_interrupted(request).unwrap().unwrap();
    assert_eq!(recovered.generation(), ticket.generation());
    assert_eq!(snapshot.state(), State::Unknown);
    assert!(!restarted.begin_create(&recovered, test_lease_clock()).unwrap());
    assert!(restarted.inspect(request, &Capability::from_bytes([9; 32])).is_err());
    assert_eq!(
        restarted.inspect(request, &cap).unwrap().unwrap().state(),
        State::Unknown
    );
    let binding = VmBinding::new("orphan".into(), Uuid::new_v4()).unwrap();
    assert!(restarted
        .bind_created(&recovered, binding.clone(), test_lease_clock())
        .is_err());
    assert_eq!(
        restarted.reconcile_vm(&recovered, binding.clone()).unwrap().state(),
        State::Closing
    );
    assert!(restarted.confirm_absent(&recovered).is_err());
    restarted.complete_cleanup(&recovered, &binding).unwrap();
    assert_eq!(
        restarted.inspect(request, &cap).unwrap().unwrap().state(),
        State::Closed
    );
}

#[test]
fn recovery_of_reserved_and_active_records_is_conservative_and_durable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let store = Registry::open(&path).unwrap();
    let cap = Capability::from_bytes([10; 32]);
    let reserved = Uuid::new_v4();
    let Reservation::New(ticket) = store.reserve(reserved, &cap).unwrap() else {
        panic!()
    };
    assert!(store.confirm_absent(&ticket).is_err());
    let (recovered, _) = store.recover_interrupted(reserved).unwrap().unwrap();
    assert!(!store.begin_create(&ticket, test_lease_clock()).unwrap());
    store.close(reserved, &cap).unwrap();
    store.confirm_absent(&recovered).unwrap();
    let active = Uuid::new_v4();
    let Reservation::New(active_ticket) = store.reserve(active, &cap).unwrap() else {
        panic!()
    };
    initialize_test_lease(&store, &active_ticket);
    store.begin_create(&active_ticket, test_lease_clock()).unwrap();
    let binding = VmBinding::new("live".into(), Uuid::new_v4()).unwrap();
    store
        .prepare_spawn(&active_ticket, binding.clone(), test_lease_clock())
        .unwrap();
    store
        .bind_created(&active_ticket, binding.clone(), test_lease_clock())
        .unwrap();
    drop(store);
    let restarted = Registry::open(&path).unwrap();
    let (recovered_active, snapshot) = restarted.recover_interrupted(active).unwrap().unwrap();
    assert_eq!(snapshot.state(), State::Unknown);
    assert_eq!(snapshot.vm(), Some(&binding));
    assert!(restarted
        .reconcile_vm(
            &recovered_active,
            VmBinding::new("live".into(), Uuid::new_v4()).unwrap()
        )
        .is_err());
    assert!(restarted.confirm_absent(&recovered_active).is_err());
    restarted.close(active, &cap).unwrap();
    restarted.complete_cleanup(&recovered_active, &binding).unwrap();
    let (_, terminal) = restarted.recover_interrupted(active).unwrap().unwrap();
    assert_eq!(terminal.state(), State::Closed);
    assert!(restarted.inspect(Uuid::new_v4(), &cap).unwrap().is_none());
    let original = restarted.inspect(reserved, &cap).unwrap().unwrap();
    assert_eq!(original.state(), State::Closed);
    assert_eq!(original.generation(), ticket.generation());
}

#[test]
fn managed_lease_renewal_is_bounded_and_cannot_resurrect_expired_ownership() {
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let store = Registry::open(&root.path().join("ownership")).unwrap();
    let cap = Capability::from_bytes([11; 32]);
    let request = Uuid::new_v4();
    let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
        panic!()
    };
    let now = Instant::now();
    let policy = LeasePolicy::new(Duration::from_secs(10)).unwrap();
    let clock = LeaseClock::new(1000, now);
    let leased = store.start_lease(&ticket, policy, clock).unwrap();
    assert_eq!(leased.expires_wall_ms(), Some(11000));
    assert!(store.start_lease(&ticket, policy, clock).is_err());
    store.begin_create(&ticket, clock).unwrap();
    let binding = VmBinding::new("leased".into(), Uuid::new_v4()).unwrap();
    store.prepare_spawn(&ticket, binding.clone(), clock).unwrap();
    store.bind_created(&ticket, binding.clone(), clock).unwrap();
    let renewed = store
        .renew_lease(request, &cap, LeaseClock::new(4000, now + Duration::from_secs(3)))
        .unwrap();
    assert_eq!(renewed.expires_wall_ms(), Some(14000));
    assert_eq!(renewed.state(), State::Active);
    assert!(store
        .renew_lease(
            request,
            &Capability::from_bytes([12; 32]),
            LeaseClock::new(5000, now + Duration::from_secs(4))
        )
        .is_err());
    let expired = store
        .expire_lease(&ticket, LeaseClock::new(14000, now + Duration::from_secs(13)))
        .unwrap();
    assert_eq!(expired.state(), State::Closing);
    let late = store
        .renew_lease(request, &cap, LeaseClock::new(15000, now + Duration::from_secs(14)))
        .unwrap();
    assert_eq!(late.state(), State::Closing);
    assert_eq!(late.expires_wall_ms(), Some(14000));
    store.complete_cleanup(&ticket, &binding).unwrap();
    assert_eq!(
        store
            .renew_lease(request, &cap, LeaseClock::new(16000, now + Duration::from_secs(15)))
            .unwrap()
            .state(),
        State::Closed
    );
    assert!(LeasePolicy::new(Duration::ZERO).is_err());
    assert!(LeasePolicy::new(Duration::from_nanos(1)).is_err());
}

#[test]
fn restart_and_wall_rollback_cannot_extend_a_managed_lease() {
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let store = Registry::open(&path).unwrap();
    let cap = Capability::from_bytes([13; 32]);
    let now = Instant::now();
    let mut requests = Vec::new();
    for id in ["rollback", "restarted", "monotonic"] {
        let request = Uuid::new_v4();
        let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
            panic!()
        };
        store
            .start_lease(
                &ticket,
                LeasePolicy::new(Duration::from_secs(20)).unwrap(),
                LeaseClock::new(10000, now),
            )
            .unwrap();
        store.begin_create(&ticket, LeaseClock::new(10000, now)).unwrap();
        let binding = VmBinding::new(id.into(), Uuid::new_v4()).unwrap();
        store
            .prepare_spawn(&ticket, binding.clone(), LeaseClock::new(10000, now))
            .unwrap();
        store
            .bind_created(&ticket, binding, LeaseClock::new(10000, now))
            .unwrap();
        requests.push((request, ticket));
    }
    let rolled = store
        .renew_lease(requests[0].0, &cap, LeaseClock::new(9999, now + Duration::from_secs(1)))
        .unwrap();
    assert_eq!(rolled.state(), State::Closing);
    assert_eq!(rolled.expires_wall_ms(), Some(30000));
    // A wall clock that advanced less than elapsed monotonic time still
    // cannot keep abandoned ownership alive past the runtime deadline.
    assert_eq!(
        store
            .expire_lease(&requests[2].1, LeaseClock::new(10001, now + Duration::from_secs(21)))
            .unwrap()
            .state(),
        State::Closing
    );
    drop(store);
    let restarted = Registry::open(&path).unwrap();
    assert_eq!(
        restarted
            .inspect(requests[1].0, &cap)
            .unwrap()
            .unwrap()
            .expires_wall_ms(),
        Some(30000)
    );
    let result = restarted
        .renew_lease(
            requests[1].0,
            &cap,
            LeaseClock::new(11000, now + Duration::from_secs(1)),
        )
        .unwrap();
    assert_eq!(result.state(), State::Closing);
    assert_eq!(result.expires_wall_ms(), Some(30000));
}

#[test]
fn expired_reservation_cannot_admit_a_create_side_effect() {
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let store = Registry::open(&root.path().join("ownership")).unwrap();
    let request = Uuid::new_v4();
    let cap = Capability::from_bytes([14; 32]);
    let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
        panic!()
    };
    let earlier = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .expect("the monotonic clock has at least two seconds of history");
    store
        .start_lease(
            &ticket,
            LeasePolicy::new(Duration::from_secs(1)).unwrap(),
            LeaseClock::new(1000, earlier),
        )
        .unwrap();
    assert!(
        !store.begin_create(&ticket, test_lease_clock()).unwrap(),
        "a delayed continuation must not start after its lease elapsed"
    );
    assert_eq!(store.inspect(request, &cap).unwrap().unwrap().state(), State::Closed);
}

#[test]
fn late_create_result_after_lease_expiry_is_bound_only_for_cleanup() {
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let store = Registry::open(&root.path().join("ownership")).unwrap();
    let request = Uuid::new_v4();
    let cap = Capability::from_bytes([15; 32]);
    let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
        panic!()
    };
    assert!(
        store.begin_create(&ticket, test_lease_clock()).is_err(),
        "uninitialized lifetime must never admit creation"
    );
    let now = Instant::now();
    store
        .start_lease(
            &ticket,
            LeasePolicy::new(Duration::from_secs(2)).unwrap(),
            LeaseClock::new(1000, now),
        )
        .unwrap();
    assert!(store.begin_create(&ticket, LeaseClock::new(1000, now)).unwrap());
    let binding = VmBinding::new("late-leased".into(), Uuid::new_v4()).unwrap();
    store
        .prepare_spawn(&ticket, binding.clone(), LeaseClock::new(1000, now))
        .unwrap();
    let late = store
        .bind_created(
            &ticket,
            binding.clone(),
            LeaseClock::new(4000, now + Duration::from_secs(3)),
        )
        .unwrap();
    assert_eq!(late.state(), State::Closing);
    assert_eq!(late.vm(), Some(&binding));
    store.complete_cleanup(&ticket, &binding).unwrap();
    assert_eq!(store.inspect(request, &cap).unwrap().unwrap().state(), State::Closed);
}

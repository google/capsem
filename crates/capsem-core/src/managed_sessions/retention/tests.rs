use super::super::*;
use std::time::{Duration, Instant};

const RETENTION: Duration = Duration::from_secs(86_400);

#[test]
fn completed_tombstone_retains_replay_protection_across_restart_and_clock_rollback() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let store = Registry::open(&path).unwrap();
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([71; 32]);
    store.close(request, &cap).unwrap();
    let completed = store.read(request).unwrap().unwrap().closed_wall_ms.unwrap();
    drop(store);
    let store = Registry::open(&path).unwrap();
    let clock = |wall| LeaseClock::new(wall, Instant::now());
    assert_eq!(store.prune_closed(RETENTION, clock(completed - 1)).unwrap(), 0);
    assert_eq!(store.prune_closed(RETENTION, clock(completed + 86_399_999)).unwrap(), 0);
    assert!(matches!(
        store.reserve(request, &cap).unwrap(),
        Reservation::Existing(_)
    ));
    assert!(store.reserve(request, &Capability::from_bytes([72; 32])).is_err());
    assert_eq!(store.prune_closed(RETENTION, clock(completed + 86_400_000)).unwrap(), 1);
    assert!(store.inspect(request, &cap).unwrap().is_none());
}

#[test]
fn retention_never_removes_live_or_unresolved_cleanup_authority() {
    let root = tempfile::tempdir().unwrap();
    let store = Registry::open(&root.path().join("ownership")).unwrap();
    let cap = Capability::from_bytes([73; 32]);
    let clock = LeaseClock::new(100, Instant::now());
    let mut requests = Vec::new();
    for state in [State::Reserved, State::Closing, State::Unknown] {
        let request = capsem_crypto::uuid_v4();
        let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
            panic!()
        };
        if state != State::Reserved {
            store
                .start_lease(&ticket, LeasePolicy::new(Duration::from_secs(10)).unwrap(), clock)
                .unwrap();
            store.begin_create(&ticket, clock).unwrap();
            if state == State::Closing {
                store.close(request, &cap).unwrap();
            } else {
                store.creation_failed(&ticket).unwrap();
            }
        }
        requests.push((request, state));
    }
    assert_eq!(
        store
            .prune_closed(RETENTION, LeaseClock::new(u64::MAX, Instant::now()))
            .unwrap(),
        0
    );
    for (request, state) in requests {
        assert_eq!(store.inspect(request, &cap).unwrap().unwrap().state(), state);
    }
    assert!(store.prune_closed(Duration::ZERO, clock).is_err());
}

#[test]
fn legacy_completion_gets_a_full_window_and_pruning_refuses_linked_inventory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let store = Registry::open(&path).unwrap();
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([74; 32]);
    store.close(request, &cap).unwrap();
    let file = path.join(format!("{request}.json"));
    let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("closed_wall_ms");
    std::fs::write(&file, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let clock = |wall| LeaseClock::new(wall, Instant::now());
    assert_eq!(store.prune_closed(RETENTION, clock(1000)).unwrap(), 0);
    assert_eq!(store.read(request).unwrap().unwrap().closed_wall_ms, Some(1000));
    let outside = root.path().join("outside");
    std::fs::write(&outside, b"untouched").unwrap();
    let linked = path.join(format!("{}.json", capsem_crypto::uuid_v4()));
    std::os::unix::fs::symlink(&outside, &linked).unwrap();
    assert!(store.prune_closed(RETENTION, clock(86_401_000)).is_err());
    assert!(file.exists());
    assert_eq!(std::fs::read(&outside).unwrap(), b"untouched");
    std::fs::remove_file(linked).unwrap();
    assert_eq!(store.prune_closed(RETENTION, clock(86_401_000)).unwrap(), 1);
}

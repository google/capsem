use super::*;

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
    assert!(store.begin_create(&ticket).unwrap());
    assert!(!store.begin_create(&ticket).unwrap());
    assert_eq!(store.close(request, &cap).unwrap().state(), State::Closing);
    let binding = VmBinding::new("vm-123".into(), Uuid::new_v4()).unwrap();
    let late = store.bind_created(&ticket, binding.clone()).unwrap();
    assert_eq!(late.state(), State::Closing);
    assert_eq!(late.vm(), Some(&binding));
    let wrong = VmBinding::new("vm-123".into(), Uuid::new_v4()).unwrap();
    assert!(store.complete_cleanup(&ticket, &wrong).is_err());
    assert_eq!(store.close(request, &cap).unwrap().state(), State::Closing);
    let closed = store.complete_cleanup(&ticket, &binding).unwrap();
    assert_eq!(closed.state(), State::Closed);
    assert!(closed.vm().is_none());
    assert!(store.bind_created(&ticket, binding).is_err());
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
    let stale = Ticket {
        request,
        generation: Uuid::new_v4(),
    };
    assert!(store.begin_create(&stale).is_err());
    assert!(store.begin_create(&ticket).unwrap());
    let binding = VmBinding::new("vm-456".into(), Uuid::new_v4()).unwrap();
    assert_eq!(
        store.bind_created(&ticket, binding.clone()).unwrap().state(),
        State::Active
    );
    assert!(store.complete_cleanup(&ticket, &binding).is_err());
    assert!(store
        .bind_created(&ticket, VmBinding::new("other".into(), Uuid::new_v4()).unwrap())
        .is_err());
    assert_eq!(
        store.bind_created(&ticket, binding.clone()).unwrap().state(),
        State::Active
    );
    store.close(request, &cap).unwrap();
    assert!(store.complete_cleanup(&stale, &binding).is_err());
    store.complete_cleanup(&ticket, &binding).unwrap();
    assert!(!store.begin_create(&ticket).unwrap());
    assert!(VmBinding::new("".into(), Uuid::new_v4()).is_err());
    assert!(VmBinding::new("valid".into(), Uuid::nil()).is_err());
}

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

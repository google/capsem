use super::*;

#[test]
fn acknowledged_requests_disappear_for_reconnected_clients_without_reusing_ids() {
    let session = SessionBrowser::default();
    session.request("https://example.org/first").unwrap();
    assert!(session.acknowledge(1));
    assert!(session.subscribe().borrow().is_none());
    assert!(session.acknowledge(1), "same metadata ack is idempotent");
    session.request("https://example.org/second").unwrap();
    assert_eq!(session.subscribe().borrow().as_ref().unwrap().id, 2);
    assert!(
        session.acknowledge(1),
        "already handled ack must preserve newer pending request"
    );
    assert_eq!(session.subscribe().borrow().as_ref().unwrap().id, 2);
}

#[test]
fn delayed_ack_cannot_clear_a_superseding_request_or_accept_unknown_ids() {
    let session = SessionBrowser::default();
    session.request("https://example.org/first").unwrap();
    session.request("https://example.org/second").unwrap();
    assert!(!session.acknowledge(1));
    assert!(!session.acknowledge(0));
    assert!(!session.acknowledge(3));
    assert_eq!(session.subscribe().borrow().as_ref().unwrap().id, 2);
    assert!(session.acknowledge(2));
    assert_eq!(session.revision(), 2);
}

#[tokio::test]
async fn requests_survive_viewer_reconnect_without_history_or_unsafe_urls() {
    let session = SessionBrowser::default();
    session.request("https://example.org/login?state=one").unwrap();
    let mut viewer = session.subscribe();
    assert_eq!(viewer.borrow_and_update().as_ref().unwrap().id, 1);
    assert!(session.request("file:///etc/passwd").is_err());
    assert!(!viewer.has_changed().unwrap());
    session.request("https://example.org/login?state=two").unwrap();
    viewer.changed().await.unwrap();
    let latest = session.subscribe().borrow().clone().unwrap();
    assert_eq!(latest.id, 2);
    assert!(latest.url.ends_with("state=two"));
    assert!(SessionBrowser::default().subscribe().borrow().is_none());
}

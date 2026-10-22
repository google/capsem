use super::*;

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

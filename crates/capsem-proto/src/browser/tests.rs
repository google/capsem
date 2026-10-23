use super::*;

#[test]
fn browser_navigation_rejects_non_web_targets_and_credentials() {
    for value in [
        "",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,x",
        "mailto:a@b.c",
        "//example.org",
        "https://user@example.org",
        "https://user:pass@example.org",
        "https://example.org/\nfoo",
    ] {
        assert!(browser_url(value).is_err(), "accepted {value:?}");
    }
    assert!(browser_url(&format!("https://example.org/{}", "x".repeat(MAX_BROWSER_URL_BYTES))).is_err());
    assert_eq!(
        browser_url("HTTPS://EXAMPLE.ORG/login?state=a%2Bb").unwrap(),
        "https://example.org/login?state=a%2Bb"
    );
}

#[test]
fn browser_request_is_named_messagepack_and_debug_redacts_auth_url() {
    let request = BrowserRequest {
        owner_generation: Some("11111111-1111-4111-8111-111111111111".into()),
        id: 42,
        url: browser_url("https://example.org/?secret=challenge").unwrap(),
    };
    let encoded = rmp_serde::to_vec_named(&request).unwrap();
    assert_eq!(rmp_serde::from_slice::<BrowserRequest>(&encoded).unwrap(), request);
    assert!(!format!("{request:?}").contains("challenge"));
    let message = crate::GuestToHost::OpenUrl {
        url: BrowserUrl(request.url),
    };
    assert!(!format!("{message:?}").contains("challenge"));
}

#[test]
fn browser_ack_is_metadata_only_and_correlates_separately_from_browser_revision() {
    let command = crate::ipc::ServiceToProcess::AcknowledgeBrowserRequest { id: 7, request_id: 42 };
    let reply = crate::ipc::ProcessToService::BrowserRequestAcknowledged {
        id: 7,
        acknowledged: true,
    };
    assert_eq!(command.request_id(), Some(7));
    assert_eq!(reply.reply_id(), Some(7));
    assert!(!format!("{command:?}").contains("url"));
    let bytes = rmp_serde::to_vec_named(&command).unwrap();
    let decoded: crate::ipc::ServiceToProcess = rmp_serde::from_slice(&bytes).unwrap();
    assert_eq!(decoded.request_id(), Some(7));
    assert_eq!(
        crate::ipc::ProcessToService::BrowserRequestCleared { request_id: 42 }.reply_id(),
        None
    );
}

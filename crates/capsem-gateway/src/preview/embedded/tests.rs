use super::*;
use crate::preview::parse_head;
use std::time::{Duration, Instant};

fn scope() -> Scope {
    Scope {
        vm_id: "box".into(),
        exposure_id: "preview".into(),
        owner_generation: "owner".into(),
        surface: true,
        expires: Instant::now() + Duration::from_secs(60),
        sessions: vec![("scoped".into(), Instant::now() + Duration::from_secs(60))],
    }
}
fn head(origin: &str) -> Head {
    parse_head(format!("GET / HTTP/1.1\r\nHost: preview.localhost:19444\r\nUpgrade: websocket\r\nOrigin: {origin}\r\nSec-WebSocket-Protocol: binary, capsem-preview.scoped\r\n\r\n").as_bytes()).unwrap()
}

#[test]
fn embedded_auth_works_without_cookies_and_requires_the_pinned_client_origin() {
    let mut head = head("http://preview.localhost:19444");
    assert!(head.cookie.is_none());
    assert_eq!(session_token(&head, &scope()).unwrap(), Some("scoped"));
    head.cookie = Some("old-standalone-session".into());
    assert_eq!(session_token(&head, &scope()).unwrap(), Some("scoped"));
    for origin in [
        "http://other.localhost:19444",
        "http://preview.localhost:19445",
        "https://attacker.test",
        "null",
    ] {
        head.origin = Some(origin.into());
        assert!(session_token(&head, &scope()).is_err());
    }
}

#[test]
fn credentials_refuse_http_other_scopes_expiry_and_body_framing() {
    let mut request = head("http://preview.localhost:19444");
    let mut scoped = scope();
    scoped.sessions.clear();
    assert!(session_token(&request, &scoped).is_err());
    scoped = scope();
    scoped.sessions[0].1 = Instant::now();
    assert!(session_token(&request, &scoped).is_err());
    scoped = scope();
    scoped.surface = false;
    assert!(session_token(&request, &scoped).is_err());
    request.websocket = false;
    assert!(session_token(&request, &scope()).is_err());
    request.websocket = true;
    request.content_length = 1;
    assert!(session_token(&request, &scope()).is_err());
    request.content_length = 0;
    request.transfer_encoded = true;
    assert!(session_token(&request, &scope()).is_err());
}

#[test]
fn embedding_parents_use_the_exact_capsem_origin_allowlist() {
    for origin in ["http://127.0.0.1:5173", "http://localhost:19222", "tauri://localhost"] {
        assert_eq!(trusted_parent(&head(origin)).unwrap(), origin);
    }
    for origin in [
        "null",
        "https://attacker.test",
        "http://localhost.evil.test",
        "http://127.0.0.1:5173/path",
    ] {
        assert!(trusted_parent(&head(origin)).is_err());
    }
    let mut request = head("http://localhost:5173");
    request.origin = None;
    assert!(trusted_parent(&request).is_err());
}

#[test]
fn parent_from_launcher_is_validated_independently_of_its_trusted_initiator() {
    let request = head("http://127.0.0.1:19222");
    assert_eq!(
        parent_from_form("parent_origin=http%3A%2F%2Flocalhost%3A5173", &request).unwrap(),
        "http://localhost:5173"
    );
    assert_eq!(
        parent_from_form("bootstrap_token=x", &request).unwrap(),
        "http://127.0.0.1:19222"
    );
    for body in [
        "parent_origin=https%3A%2F%2Fattacker.test",
        "parent_origin=http%3A%2F%2Flocalhost&parent_origin=http%3A%2F%2Flocalhost",
    ] {
        assert!(parent_from_form(body, &request).is_err());
    }
    assert!(parent_from_form("parent_origin=http%3A%2F%2Flocalhost", &head("https://attacker.test")).is_err());
}

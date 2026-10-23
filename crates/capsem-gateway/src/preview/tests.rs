use super::*;

mod admission;
use crate::{auth::AuthFailureTracker, service_client::ServiceClient, status::StatusCache};
use axum::body::{to_bytes, Body};
use axum::routing::post;
use axum::{Json, Router};
use std::sync::atomic::{AtomicBool, Ordering};
use tower::ServiceExt;

#[test]
fn preview_vm_ids_follow_the_service_and_stream_identifier_rule() {
    for valid in ["a", "9lives", "my-vm", "project_alpha", &"a".repeat(64)] {
        assert!(safe_id(valid), "valid VM ID {valid:?} was refused");
    }
    for invalid in ["", "-vm", "_vm", "my/vm", "my.vm", "vm\0id", "naïve", &"a".repeat(65)] {
        assert!(!safe_id(invalid), "invalid VM ID {invalid:?} was admitted");
    }
    assert!(safe_dns_label("preview-1"));
    for invalid in ["_preview", "-preview", "preview-", "project_alpha", &"a".repeat(64)] {
        assert!(
            !safe_dns_label(invalid),
            "invalid exposure label {invalid:?} was admitted"
        );
    }
}

#[test]
fn preview_origin_requires_one_safe_localhost_label() {
    let parsed = parse_head(
        b"GET /assets/app.js HTTP/1.1\r\nHost: 0199df26-d0f2-74f2-a304-ef67b79d1217.localhost:19444\r\nCookie: app=x; capsem_preview=session-secret\r\n\r\n",
    )
    .unwrap();
    assert_eq!(parsed.label, "0199df26-d0f2-74f2-a304-ef67b79d1217");
    assert_eq!(parsed.port, 19444);
    assert_eq!(parsed.cookie.as_deref(), Some("session-secret"));
    assert!(!parsed.websocket);

    for host in [
        "localhost:19444",
        "a.b.localhost:19444",
        "bad_.localhost:19444",
        "evil.test:19444",
    ] {
        let request = format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n");
        assert!(parse_head(request.as_bytes()).is_err(), "{host} must be refused");
    }
}

#[test]
fn websocket_admission_is_detected_without_consuming_the_request() {
    let parsed = parse_head(
        b"GET /live HTTP/1.1\r\nHost: preview.localhost:19444\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nCookie: capsem_preview=s\r\n\r\n",
    )
    .unwrap();
    assert!(parsed.websocket);
    assert_eq!(parsed.cookie.as_deref(), Some("s"));
}

#[test]
fn bootstrap_is_a_bounded_post_body_and_never_a_url_secret() {
    let parsed =
        parse_head(b"POST /_capsem/bootstrap HTTP/1.1\r\nHost: preview.localhost:19444\r\nContent-Length: 31\r\n\r\n")
            .unwrap();
    assert_eq!(parsed.method, "POST");
    assert_eq!(parsed.path, "/_capsem/bootstrap");
    assert_eq!(parsed.content_length, 31);
    let public = PreviewSessionResponse {
        url: "http://preview.localhost:19444/_capsem/bootstrap".into(),
        bootstrap_token: "post-body-only".into(),
        expires_in_seconds: 30,
    };
    assert!(!public.url.contains(&public.bootstrap_token));
    assert!(!public.url.contains('?'));
    assert_eq!(
        parse_bootstrap_token("ignored=value&bootstrap_token=single-use").unwrap(),
        "single-use"
    );
    assert!(parse_bootstrap_token("bootstrap_token=one&bootstrap_token=two").is_err());
}

#[test]
fn chunked_bootstrap_and_oversized_headers_fail_closed() {
    let parsed = parse_head(
        b"POST /_capsem/bootstrap HTTP/1.1\r\nHost: preview.localhost:19444\r\nTransfer-Encoding: chunked\r\n\r\n",
    )
    .unwrap();
    assert!(parsed.transfer_encoded);
    assert!(validate_bootstrap_head(&parsed).is_err());
}

#[test]
fn duplicate_origin_or_control_cookie_is_refused() {
    for request in [
        b"GET / HTTP/1.1\r\nHost: preview.localhost:19444\r\nHost: evil.localhost:19444\r\n\r\n".as_slice(),
        b"GET / HTTP/1.1\r\nHost: preview.localhost:19444\r\nCookie: capsem_preview=one; capsem_preview=two\r\n\r\n"
            .as_slice(),
    ] {
        assert!(parse_head(request).is_err());
    }
}

#[test]
fn embedded_credentials_and_parent_origins_cannot_be_ambiguous() {
    for headers in [
        "Origin: http://127.0.0.1:5173\r\nOrigin: https://attacker.test\r\n",
        "Sec-WebSocket-Protocol: binary, capsem-preview.first, capsem-preview.second\r\n",
        "Sec-WebSocket-Protocol: capsem-preview.first\r\nSec-WebSocket-Protocol: capsem-preview.second\r\n",
        "Sec-WebSocket-Protocol: capsem-preview.\r\n",
        "Sec-WebSocket-Protocol: capsem-preview.invalid%20token\r\n",
    ] {
        let request = format!("GET / HTTP/1.1\r\nHost: preview.localhost:19444\r\n{headers}\r\n");
        assert!(parse_head(request.as_bytes()).is_err(), "ambiguous credential accepted");
    }
}

#[test]
fn origin_port_is_part_of_preview_isolation() {
    let parsed = parse_head(b"GET / HTTP/1.1\r\nHost: preview.localhost:19445\r\n\r\n").unwrap();
    assert!(validate_origin_port(&parsed, 19444).is_err());
}

#[tokio::test]
async fn bootstrap_is_posted_once_on_its_scoped_origin_and_becomes_an_http_only_cookie() {
    assert_bootstrap_is_scoped(false).await;
}

#[tokio::test]
async fn embedded_bootstrap_returns_a_parent_bound_client_without_cookies() {
    assert_bootstrap_is_scoped(true).await;
}

async fn assert_bootstrap_is_scoped(surface: bool) {
    const SESSION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const EXPOSURE: &str = "0199df26-d0f2-74f2-a304-ef67b79d1217";
    let dir = tempfile::tempdir().unwrap();
    let service_path = dir.path().join("service.sock");
    let service_listener = tokio::net::UnixListener::bind(&service_path).unwrap();
    let exchanged = Arc::new(AtomicBool::new(false));
    let exchange_state = Arc::clone(&exchanged);
    let service = Router::new()
        .route(
            "/internal/vms/box/exposures/0199df26-d0f2-74f2-a304-ef67b79d1217/preview-session",
            post(|| async {
                Json(PreviewSessionMaterial {
                    exposure: capsem_api::ExposureInfo::preview(
                        EXPOSURE.into(),
                        8080,
                        capsem_api::ExposureTarget::Container,
                    ),
                    owner_generation: "42".into(),
                    bootstrap_token: "single-use-bootstrap".into(),
                    expires_in_seconds: 30,
                })
            }),
        )
        .route(
            "/internal/vms/box/exposures/0199df26-d0f2-74f2-a304-ef67b79d1217/preview-bootstrap",
            post(move |Json(body): Json<PreviewBootstrapExchangeRequest>| {
                let exchange_state = Arc::clone(&exchange_state);
                async move {
                    if body.bootstrap_token != "single-use-bootstrap" || exchange_state.swap(true, Ordering::SeqCst) {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                    Json(PreviewBootstrapExchangeResponse {
                        session_token: SESSION.into(),
                        expires_in_seconds: 900,
                    })
                    .into_response()
                }
            }),
        );
    tokio::spawn(async move { axum::serve(service_listener, service).await.unwrap() });

    let preview_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let preview_port = preview_listener.local_addr().unwrap().port();
    let state = Arc::new(crate::AppState {
        token: "admin-token".into(),
        uds_path: service_path.clone(),
        service_client: ServiceClient::new(&service_path),
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(4).0,
        previews: PreviewState::new(preview_port, 19222),
    });
    let control = Router::new()
        .route(
            "/vms/{vm_id}/exposures/{exposure_id}/preview-session",
            post(create_session),
        )
        .with_state(Arc::clone(&state));
    let response = if surface {
        mint(&state, "box".into(), EXPOSURE.into(), true).await
    } else {
        control
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/vms/box/exposures/{EXPOSURE}/preview-session"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    };
    assert_eq!(response.status(), StatusCode::OK);
    let session: PreviewSessionResponse =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert!(!session.url.contains(&session.bootstrap_token));
    tokio::spawn(serve(preview_listener, Arc::clone(&state)));

    let exchange = |label: &str, parent: Option<&str>| {
        let label = label.to_owned();
        let origin = parent.map(|value| format!("Origin: {value}\r\n")).unwrap_or_default();
        let path = if surface { embedded::PATH } else { "/_capsem/bootstrap" };
        async move {
            let mut client = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, preview_port))
                .await
                .unwrap();
            let body = "bootstrap_token=single-use-bootstrap";
            client
            .write_all(
                format!(
                    "POST {path} HTTP/1.1\r\nHost: {label}.localhost:{preview_port}\r\n{origin}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
            let mut response = Vec::new();
            if let Err(error) = client.read_to_end(&mut response).await {
                assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
            }
            String::from_utf8(response).unwrap()
        }
    };
    let parent = surface.then_some("tauri://localhost");
    if surface {
        let denied = exchange(EXPOSURE, Some("https://attacker.test")).await;
        assert!(denied.starts_with("HTTP/1.1 403 Forbidden"));
        assert!(
            !exchanged.load(Ordering::SeqCst),
            "untrusted parent cannot consume bootstrap"
        );
    }
    let response = exchange(EXPOSURE, parent).await;
    if surface {
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("frame-ancestors tauri://localhost;"));
        assert!(response.contains(&format!("capsem-preview.{SESSION}")));
        assert!(!response.contains("Set-Cookie:") && !response.contains("X-Frame-Options:"));
    } else {
        assert!(response.starts_with("HTTP/1.1 303 See Other\r\n"), "{response}");
        assert!(
            response.contains(&format!(
                "Set-Cookie: capsem_preview={SESSION}; Path=/; HttpOnly; SameSite=Lax; Max-Age=900"
            )),
            "{response}"
        );
    }
    // Fail closed means no session is minted. A refusal is answered with a
    // status rather than a dropped socket, which a browser shows as an empty
    // response instead of an expired preview.
    let refused = |response: &str| response.starts_with("HTTP/1.1 4") && !response.contains("Set-Cookie");
    let replay = exchange(EXPOSURE, parent).await;
    assert!(refused(&replay), "bootstrap replay must fail closed: {replay}");
    let foreign = exchange("1199df26-d0f2-74f2-a304-ef67b79d1217", parent).await;
    assert!(
        refused(&foreign),
        "another exposure origin must not share this scope: {foreign}"
    );
}

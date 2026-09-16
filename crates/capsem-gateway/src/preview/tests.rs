use super::*;
use crate::{auth::AuthFailureTracker, service_client::ServiceClient, status::StatusCache};
use axum::body::{to_bytes, Body};
use axum::routing::post;
use axum::{Json, Router};
use std::sync::atomic::{AtomicBool, Ordering};
use tower::ServiceExt;

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
fn origin_port_is_part_of_preview_isolation() {
    let parsed = parse_head(b"GET / HTTP/1.1\r\nHost: preview.localhost:19445\r\n\r\n").unwrap();
    assert!(validate_origin_port(&parsed, 19444).is_err());
}

#[tokio::test]
async fn bootstrap_is_posted_once_on_its_scoped_origin_and_becomes_an_http_only_cookie() {
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
                        session_token: "scoped-session".into(),
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
        previews: PreviewState::new(preview_port),
    });
    let control = Router::new()
        .route(
            "/vms/{vm_id}/exposures/{exposure_id}/preview-session",
            post(create_session),
        )
        .with_state(Arc::clone(&state));
    let response = control
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/vms/box/exposures/{EXPOSURE}/preview-session"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let session: PreviewSessionResponse =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert!(!session.url.contains(&session.bootstrap_token));
    tokio::spawn(serve(preview_listener, Arc::clone(&state)));

    let exchange = |label: &str| {
        let label = label.to_owned();
        async move {
            let mut client = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, preview_port))
                .await
                .unwrap();
            let body = "bootstrap_token=single-use-bootstrap";
            client
            .write_all(
                format!(
                    "POST /_capsem/bootstrap HTTP/1.1\r\nHost: {label}.localhost:{preview_port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
    let response = exchange(EXPOSURE).await;
    assert!(response.starts_with("HTTP/1.1 303 See Other\r\n"), "{response}");
    assert!(
        response.contains("Set-Cookie: capsem_preview=scoped-session; Path=/; HttpOnly; SameSite=Strict; Max-Age=900"),
        "{response}"
    );
    assert!(exchange(EXPOSURE).await.is_empty(), "bootstrap replay must fail closed");
    assert!(
        exchange("1199df26-d0f2-74f2-a304-ef67b79d1217").await.is_empty(),
        "another exposure origin must not share this scope"
    );
}

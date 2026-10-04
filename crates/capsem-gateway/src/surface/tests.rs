use super::*;
use crate::{auth::AuthFailureTracker, service_client::ServiceClient, status::StatusCache};
use axum::body::{to_bytes, Body};
use axum::routing::{get, post};
use axum::Json;
use capsem_api::{
    ContainerState, ContainerSurface, ExposureInfo, ExposureTarget, PreviewBootstrapExchangeResponse,
    PreviewSessionMaterial, PreviewSessionResponse,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const SURFACE: &str = "0199df26-d0f2-74f2-a304-ef67b79d1217";
const PLAIN: &str = "1199df26-d0f2-74f2-a304-ef67b79d1217";
const TOKEN: &str = "admin-token";

fn status(surface: Option<ContainerSurface>) -> ContainerStatusResponse {
    ContainerStatusResponse {
        state: ContainerState::Running,
        image: "ghcr.io/google/capsem/claude-desktop:1".into(),
        digest: None,
        exit_code: None,
        error: None,
        surface,
        resolved: None,
    }
}

fn xpra(exposure_id: Option<&str>) -> Option<ContainerSurface> {
    Some(ContainerSurface {
        kind: ContainerSurfaceKind::Xpra,
        port: 14500,
        exposure_id: exposure_id.map(str::to_owned),
    })
}

fn material(exposure: &str) -> PreviewSessionMaterial {
    PreviewSessionMaterial {
        exposure: ExposureInfo::preview(exposure.into(), 14500, ExposureTarget::Container),
        owner_generation: "42".into(),
        bootstrap_token: format!("bootstrap-{exposure}"),
        expires_in_seconds: 30,
    }
}

/// A service that knows four VMs -- `gui` with a granted surface, `pending`
/// whose surface has no exposure yet, `terminal` with no surface, and none
/// for anything else -- and counts preview admissions, refusing them all.
async fn service(dir: &std::path::Path, admissions: Arc<AtomicUsize>) -> std::path::PathBuf {
    let path = dir.join("service.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let router = axum::Router::new()
        .route(
            "/vms/{id}/container",
            get(|Path(id): Path<String>| async move {
                match id.as_str() {
                    "gui" => Json(status(xpra(Some(SURFACE)))).into_response(),
                    "pending" => Json(status(xpra(None))).into_response(),
                    "terminal" => Json(status(None)).into_response(),
                    _ => (StatusCode::NOT_FOUND, "VM has no container workload").into_response(),
                }
            }),
        )
        .route(
            "/internal/vms/{id}/exposures/{exposure_id}/preview-session",
            post(|Path((_, exposure)): Path<(String, String)>| async move { Json(material(&exposure)) }),
        )
        .route(
            "/internal/vms/{id}/exposures/{exposure_id}/preview-bootstrap",
            post(|Path((_, exposure)): Path<(String, String)>| async move {
                Json(PreviewBootstrapExchangeResponse {
                    session_token: format!("session-{exposure}"),
                    expires_in_seconds: 900,
                })
            }),
        )
        .route(
            "/internal/vms/{id}/exposures/{exposure_id}/preview-admission",
            post(move || {
                let admissions = Arc::clone(&admissions);
                async move {
                    admissions.fetch_add(1, Ordering::SeqCst);
                    StatusCode::UNAUTHORIZED
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    path
}

fn state(service_path: &std::path::Path, preview_port: u16) -> Arc<AppState> {
    Arc::new(AppState {
        token: TOKEN.into(),
        uds_path: service_path.to_path_buf(),
        service_client: ServiceClient::new(service_path),
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(4).0,
        previews: crate::preview::PreviewState::new(preview_port),
    })
}

/// The gateway's real route table behind its real authentication.
fn gateway(state: &Arc<AppState>) -> axum::Router {
    routes()
        .merge(crate::service_proxy_routes())
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(state),
            crate::auth::auth_middleware,
        ))
        .with_state(Arc::clone(state))
}

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, String) {
    let mut request = http::Request::builder()
        .method(method)
        .uri(path)
        .header(http::header::HOST, "127.0.0.1:19222");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
    let (parts, body) = response.into_parts();
    let body = String::from_utf8(to_bytes(body, 1 << 20).await.unwrap().to_vec()).unwrap();
    (parts.status, parts.headers, body)
}

const BEARER: (&str, &str) = ("authorization", "Bearer admin-token");

#[tokio::test]
async fn the_launcher_is_served_without_a_token_and_holds_none() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(&service(dir.path(), Arc::default()).await, 19444);
    let app = gateway(&state);

    let (status, headers, page) = call(&app, "GET", "/vms/gui/surface/", &[]).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(!page.contains(TOKEN), "the launcher carries no secret");
    assert!(page.contains("data-auto=\"true\""), "{page}");
    assert!(page.contains("<script src=\"launch.js\" defer></script>"));
    let policy = headers[http::header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(policy.contains("script-src 'self';"), "{policy}");
    assert!(policy.contains("form-action http://*.localhost:19444;"), "{policy}");
    assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
    assert_eq!(headers[http::header::X_FRAME_OPTIONS], "DENY");

    let (status, headers, script) = call(&app, "GET", "/vms/gui/surface/launch.js", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[http::header::CONTENT_TYPE], "text/javascript; charset=utf-8");
    assert!(script.contains("form.method = \"POST\"") && script.contains("bootstrap_token"));
    assert!(!script.contains("?token") && !script.contains("location.search"));
}

#[tokio::test]
async fn a_launcher_another_site_opened_waits_for_a_click() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(&service(dir.path(), Arc::default()).await, 19444);
    let app = gateway(&state);
    let (_, _, page) = call(&app, "GET", "/vms/gui/surface/", &[("sec-fetch-site", "cross-site")]).await;
    assert!(page.contains("data-auto=\"false\""), "{page}");
    for site in ["none", "same-origin", "same-site"] {
        let (_, _, page) = call(&app, "GET", "/vms/gui/surface/", &[("sec-fetch-site", site)]).await;
        assert!(page.contains("data-auto=\"true\""), "{site}");
    }
}

#[tokio::test]
async fn only_the_static_launcher_skips_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(&service(dir.path(), Arc::default()).await, 19444);
    let app = gateway(&state);
    for (method, path) in [
        ("POST", "/vms/gui/surface/session"),
        ("POST", "/vms/gui/surface/"),
        ("GET", "/vms/gui/surface/session"),
        ("GET", "/vms/gui/surface/other.js"),
        ("GET", "/vms/gui/surface"),
        ("GET", "/vms/gui/container"),
    ] {
        let (status, _, body) = call(&app, method, path, &[]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}: {body}");
    }
    // A foreign Host is refused before the exemption: a rebinding page
    // cannot reach the launcher.
    let request = http::Request::builder()
        .uri("/vms/gui/surface/")
        .header(http::header::HOST, "evil.example:19222")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_vm_without_a_running_surface_has_no_session() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(&service(dir.path(), Arc::default()).await, 19444);
    let app = gateway(&state);
    for vm in ["terminal", "pending", "nothing"] {
        let (status, _, body) = call(&app, "POST", &format!("/vms/{vm}/surface/session"), &[BEARER]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{vm}: {body}");
        assert!(body.contains("no running app surface"), "{vm}: {body}");
    }
    let (status, _, _) = call(&app, "POST", "/vms/bad.id/surface/session", &[BEARER]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(&app, "GET", "/vms/bad.id/surface/", &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// What a browser on the preview origin sends and gets back.
async fn browse(port: u16, label: &str, request: &str) -> String {
    let mut client = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let request = request.replace("{host}", &format!("{label}.localhost:{port}"));
    client.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    if let Err(error) = client.read_to_end(&mut response).await {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    String::from_utf8_lossy(&response).into_owned()
}

fn bootstrap(token: &str) -> String {
    let body = format!("bootstrap_token={token}");
    format!(
        "POST /_capsem/bootstrap HTTP/1.1\r\nHost: {{host}}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn get_with_cookie(path: &str, cookie: Option<&str>) -> String {
    let cookie = cookie
        .map(|c| format!("Cookie: capsem_preview={c}\r\n"))
        .unwrap_or_default();
    format!("GET {path} HTTP/1.1\r\nHost: {{host}}\r\n{cookie}Connection: close\r\n\r\n")
}

#[tokio::test]
async fn a_surface_session_opens_the_client_on_its_own_origin_only_with_its_cookie() {
    let dir = tempfile::tempdir().unwrap();
    let admissions = Arc::new(AtomicUsize::new(0));
    let service_path = service(dir.path(), Arc::clone(&admissions)).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = state(&service_path, port);
    tokio::spawn(crate::preview::serve(listener, Arc::clone(&state)));
    let app = gateway(&state);

    let (status, _, body) = call(&app, "POST", "/vms/gui/surface/session", &[BEARER]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session: PreviewSessionResponse = serde_json::from_str(&body).unwrap();
    assert_eq!(
        session.url,
        format!("http://{SURFACE}.localhost:{port}/_capsem/bootstrap")
    );
    assert!(!session.url.contains(&session.bootstrap_token));

    // Before the bootstrap there is no session, so no client.
    let refused = browse(port, SURFACE, &get_with_cookie("/_capsem/surface/", None)).await;
    assert!(refused.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{refused}");

    let exchanged = browse(port, SURFACE, &bootstrap(&session.bootstrap_token)).await;
    assert!(exchanged.starts_with("HTTP/1.1 303 See Other\r\n"), "{exchanged}");
    assert!(exchanged.contains("\r\nLocation: /_capsem/surface/\r\n"), "{exchanged}");
    let cookie = format!("session-{SURFACE}");
    assert!(exchanged.contains(&format!(
        "Set-Cookie: capsem_preview={cookie}; Path=/; HttpOnly; SameSite=Lax"
    )));

    let page = browse(port, SURFACE, &get_with_cookie("/_capsem/surface/", Some(&cookie))).await;
    assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
    assert!(page.contains(&format!("connect-src 'self' ws://{SURFACE}.localhost:{port};")));
    assert!(page.contains("<script src=\"capsem-surface.js\"></script>"));
    let script = browse(
        port,
        SURFACE,
        &get_with_cookie("/_capsem/surface/js/Client.js", Some(&cookie)),
    )
    .await;
    assert!(script.starts_with("HTTP/1.1 200 OK\r\n") && script.contains("class XpraClient"));
    for forged in ["session-forged", &format!("session-{PLAIN}")] {
        let refused = browse(port, SURFACE, &get_with_cookie("/_capsem/surface/", Some(forged))).await;
        assert!(
            refused.starts_with("HTTP/1.1 401 Unauthorized\r\n"),
            "{forged}: {refused}"
        );
    }
    assert_eq!(
        admissions.load(Ordering::SeqCst),
        0,
        "the client is served without reaching the VM owner"
    );

    // The client's websocket is the workload's, admitted by the VM owner.
    let upgrade = "GET / HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: binary\r\nCookie: capsem_preview=COOKIE\r\n\r\n"
        .replace("COOKIE", &cookie);
    let denied = browse(port, SURFACE, &upgrade).await;
    assert!(denied.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{denied}");
    assert_eq!(admissions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_ordinary_preview_never_serves_the_client() {
    let dir = tempfile::tempdir().unwrap();
    let admissions = Arc::new(AtomicUsize::new(0));
    let service_path = service(dir.path(), Arc::clone(&admissions)).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = state(&service_path, port);
    tokio::spawn(crate::preview::serve(listener, Arc::clone(&state)));
    let app = gateway(&state);

    let (status, _, body) = call(
        &app,
        "POST",
        &format!("/vms/web/exposures/{PLAIN}/preview-session"),
        &[BEARER],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session: PreviewSessionResponse = serde_json::from_str(&body).unwrap();
    let exchanged = browse(port, PLAIN, &bootstrap(&session.bootstrap_token)).await;
    assert!(exchanged.contains("\r\nLocation: /\r\n"), "{exchanged}");
    let cookie = format!("session-{PLAIN}");
    // Its /_capsem/surface/ belongs to the workload, through admission.
    let response = browse(port, PLAIN, &get_with_cookie("/_capsem/surface/", Some(&cookie))).await;
    assert!(response.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{response}");
    assert_eq!(admissions.load(Ordering::SeqCst), 1);
}

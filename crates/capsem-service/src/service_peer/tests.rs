use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, make_test_state};
use tower::ServiceExt;

#[test]
fn owner_routes_are_exact_and_bound_to_the_registered_vm() {
    assert!(owner_route("box", "/networks/private/resolve"));
    assert!(owner_route("box", "/internal/vms/box/metrics"));
    assert!(!owner_route("box", "/internal/vms/sibling/metrics"));
    assert!(!owner_route("box", "/vms/list"));
}

#[tokio::test]
async fn registered_owner_cannot_call_the_public_service_api() {
    let state = make_test_state();
    insert_fake_instance_with_session_dir(&state, "box", std::process::id(), state.run_dir.join("sessions/box"));
    let peer = ServicePeer(Some(capsem_foundation::unix::peer::PeerIdentity {
        pid: capsem_foundation::unix::process::ProcessId::try_from(std::process::id()).unwrap(),
        uid: capsem_foundation::unix::process::current_uid(),
    }));
    let app = crate::router_runtime::build_service_router(state);
    let request = axum::http::Request::get("/version")
        .extension(axum::extract::ConnectInfo(peer))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn caller_supplied_gateway_headers_cannot_reach_the_gateway_status_route() {
    let state = make_test_state();
    let app = crate::router_runtime::build_service_router(state);
    let request = axum::http::Request::get("/internal/gateway/status")
        .header("x-capsem-actor", "operator")
        .header("x-capsem-gateway", "true")
        .header("authorization", "Bearer forged-gateway-identity")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn direct_service_api_refuses_even_a_bearer_without_gateway_transport() {
    let state = make_test_state();
    let app = crate::router_runtime::build_service_router(state);
    let request = axum::http::Request::get("/vms/list")
        .header("authorization", format!("Bearer {}", "A".repeat(64)))
        .header("x-capsem-actor", "operator")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
}

#[test]
fn direct_bootstrap_and_admin_exceptions_are_exact_read_only_status_probes() {
    for path in ["/version", "/status", "/system/status", "/assets/status"] {
        assert!(direct_status_route(&axum::http::Method::GET, path));
        for method in [
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
        ] {
            assert!(!direct_status_route(&method, path));
        }
    }
    for path in [
        "/vms/list",
        "/credentials/inject",
        "/settings/edit",
        "/corp/edit",
        "/status/",
        "/assets/status/edit",
    ] {
        assert!(!direct_status_route(&axum::http::Method::GET, path));
    }
}

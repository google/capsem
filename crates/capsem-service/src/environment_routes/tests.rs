use crate::tests::{make_test_state, route_request};
use axum::http::{Method, StatusCode};
use capsem_logger::events::PreparedEnvironmentRevision;

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

async fn seed(state: &crate::ServiceState) {
    state
        .environments
        .commit(
            PreparedEnvironmentRevision {
                environment_id: id(1),
                revision_id: id(2),
                previous_revision_id: None,
                fork_parent: None,
                timestamp_unix_ms: 2,
                actor: "operator".into(),
                reason: "approved environment".into(),
                request_id: None,
                payload_event_id: "abcdef012345".into(),
                specification: Box::new(toml::from_str("name='development'\n[variables]\nEDITOR='vim'").unwrap()),
            },
            10,
        )
        .wait()
        .await
        .unwrap();
}

#[tokio::test]
async fn current_environment_route_returns_verified_content_and_committed_provenance() {
    let state = make_test_state();
    seed(&state).await;
    let app = crate::router_runtime::build_service_router(state);
    let (status, body) = route_request(app, Method::GET, &format!("/environments/{}/info", id(1)), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: capsem_api::environments::EnvironmentCurrentResponse = serde_json::from_value(body).unwrap();
    assert_eq!(view.document.identity.id, id(1));
    assert_eq!(view.document.identity.revision_id, id(2));
    assert_eq!(view.document.specification.variables["EDITOR"], "vim");
    assert_eq!(view.timestamp_unix_ms, 2);
    assert_eq!(view.actor, "operator");
    assert_eq!(view.reason, "approved environment");
    assert!(!format!("{view:?}").contains("vim"));
}

#[tokio::test]
async fn current_environment_route_distinguishes_absent_and_unverified_state() {
    let state = make_test_state();
    let app = crate::router_runtime::build_service_router(state.clone());
    assert_eq!(
        route_request(app.clone(), Method::GET, &format!("/environments/{}/info", id(1)), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        route_request(app.clone(), Method::GET, "/environments/not-a-uuid/info", None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    seed(&state).await;
    let mut changed = state.environments.load_current(&id(1)).await.unwrap().document;
    changed.specification.name = "unreviewed-edit".into();
    capsem_foundation::unix::fs::atomic_write_private(
        &state
            .run_dir
            .parent()
            .unwrap()
            .join("environments")
            .join(id(1))
            .join("environment.toml"),
        changed.to_toml().unwrap().as_bytes(),
    )
    .unwrap();
    assert_eq!(
        route_request(app, Method::GET, &format!("/environments/{}/info", id(1)), None)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

fn delegate(
    state: &crate::ServiceState,
    operation: capsem_api::AgentOperation,
    environments: Vec<String>,
) -> (uuid::Uuid, String) {
    use capsem_core::{
        agent_access::Reservation,
        managed_sessions::{Capability, LeaseClock},
    };
    let access = capsem_crypto::uuid_v4();
    let scope = capsem_api::AgentAccessScope::new(vec![operation], environments, vec![]).unwrap();
    let Reservation::New(admission) = state
        .caller_authority
        .agents
        .reserve(
            access,
            &Capability::from_bytes([0x11; 32]),
            scope,
            std::time::Duration::from_secs(60),
            || LeaseClock::now().ok(),
        )
        .unwrap()
    else {
        panic!("new admission")
    };
    admission.commit(|| LeaseClock::now().ok()).unwrap();
    (access, format!("Bearer {access}.{}", "11".repeat(32)))
}

#[tokio::test]
async fn current_environment_route_requires_exact_live_inspect_scope() {
    use crate::tests::OperatorRequestExt;
    use axum::{body::Body, http::Request};
    let state = make_test_state();
    seed(&state).await;
    let app = crate::router_runtime::build_service_router(state.clone());
    for (operation, targets, expected) in [
        (capsem_api::AgentOperation::Inspect, vec![id(1)], StatusCode::OK),
        (capsem_api::AgentOperation::Inspect, vec![id(9)], StatusCode::FORBIDDEN),
        (capsem_api::AgentOperation::Inspect, vec![], StatusCode::FORBIDDEN),
        (capsem_api::AgentOperation::Discover, vec![id(1)], StatusCode::FORBIDDEN),
    ] {
        let (access, bearer) = delegate(&state, operation, targets);
        let request = || {
            Request::get(format!("/environments/{}/info", id(1)))
                .header("authorization", &bearer)
                .body(Body::empty())
                .unwrap()
        };
        assert_eq!(
            app.clone().operator_request(request()).await.unwrap().status(),
            expected
        );
        state.caller_authority.agents.revoke(access).unwrap();
        assert_eq!(
            app.clone().operator_request(request()).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let (access, bearer) = delegate(&state, capsem_api::AgentOperation::Inspect, vec![id(1)]);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("authorization", bearer.parse().unwrap());
    let admitted = state.caller_authority.authenticate_gateway(&headers).unwrap();
    assert!(super::permits_inspect(&admitted, &id(1)));
    state.caller_authority.agents.revoke(access).unwrap();
    assert!(
        !super::permits_inspect(&admitted, &id(1)),
        "an existing actor snapshot must not outlive revocation"
    );
}

#[tokio::test]
async fn current_environment_route_refuses_direct_transport_and_forged_actor() {
    use tower::ServiceExt;
    let app = crate::router_runtime::build_service_router(make_test_state());
    let request = axum::http::Request::get(format!("/environments/{}/info", id(1)))
        .header("authorization", format!("Bearer {}", "A".repeat(64)))
        .header("x-capsem-actor", "operator")
        .body(axum::body::Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
}

mod creates;

mod catalogs;

mod changes;

mod readiness;

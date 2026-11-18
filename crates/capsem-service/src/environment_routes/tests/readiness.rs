use super::*;
use capsem_config::environment::{EnvironmentImage, EnvironmentStartup};
use serde_json::json;

async fn commit(state: &crate::ServiceState, revision: u64, previous: Option<u64>, seconds: u32, image: bool) {
    let mut specification: capsem_config::environment::EnvironmentSpecification =
        toml::from_str("name='wait-proof'\n[variables]\nPRIVATE_VALUE='never-return'").unwrap();
    specification.startup = image.then(|| EnvironmentStartup {
        script: "printf script-must-stay-private".into(),
        timeout_seconds: seconds,
    });
    specification.image = image.then(|| EnvironmentImage {
        reference: "registry.example/app:1".into(),
        resolved_digest: format!("sha256:{}", "a".repeat(64)),
        update_policy: Default::default(),
    });
    state
        .environments
        .commit(
            PreparedEnvironmentRevision {
                environment_id: id(1),
                revision_id: id(revision),
                previous_revision_id: previous.map(id),
                fork_parent: None,
                timestamp_unix_ms: revision as i64,
                actor: "operator".into(),
                reason: "approved wait".into(),
                request_id: None,
                payload_event_id: format!("{revision:012x}"),
                specification: Box::new(specification),
            },
            revision as i64,
        )
        .wait()
        .await
        .unwrap();
}

fn path(environment: u64, revision: u64) -> String {
    format!("/environments/{}/revisions/{}/readiness", id(environment), id(revision))
}

#[tokio::test]
async fn readiness_returns_retained_deadline_without_reading_shared_head_or_content() {
    let state = make_test_state();
    commit(&state, 2, None, 600, true).await;
    commit(&state, 3, Some(2), u32::MAX, true).await;
    commit(&state, 4, Some(3), 1, false).await;
    // This view is historical ledger metadata; a broken current file cannot
    // retarget or prevent a retained exact-revision read.
    std::fs::write(
        state
            .run_dir
            .parent()
            .unwrap()
            .join("environments")
            .join(id(1))
            .join("environment.toml"),
        "broken",
    )
    .unwrap();
    let app = crate::router_runtime::build_service_router(state);
    for (revision, seconds) in [(2, 600), (3, u32::MAX), (4, 0)] {
        let (status, body) = route_request(app.clone(), Method::GET, &path(1, revision), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!({"environment": {"environment_id": id(1), "revision_id": id(revision)}, "startup_timeout_seconds": seconds})
        );
    }
}

#[tokio::test]
async fn readiness_refuses_invalid_or_unavailable_qualified_history() {
    let state = make_test_state();
    seed(&state).await;
    let app = crate::router_runtime::build_service_router(state);
    for selected in [path(9, 2), path(1, 9)] {
        assert_eq!(
            route_request(app.clone(), Method::GET, &selected, None).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    for selected in [
        format!("/environments/no/revisions/{}/readiness", id(2)),
        format!("/environments/{}/revisions/no/readiness", id(1)),
        format!("/environments/{}/revisions/{}/readiness", id(1), uuid::Uuid::nil()),
    ] {
        assert_eq!(
            route_request(app.clone(), Method::GET, &selected, None).await.0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn readiness_requires_live_exact_inspection_scope_and_sealed_transport() {
    use crate::tests::OperatorRequestExt;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let state = make_test_state();
    seed(&state).await;
    let app = crate::router_runtime::build_service_router(state.clone());
    for (operation, targets, expected) in [
        (capsem_api::AgentOperation::Inspect, vec![id(1)], StatusCode::OK),
        (capsem_api::AgentOperation::Inspect, vec![id(9)], StatusCode::FORBIDDEN),
        (capsem_api::AgentOperation::Discover, vec![id(1)], StatusCode::FORBIDDEN),
    ] {
        let (access, bearer) = delegate(&state, operation, targets);
        let request = || {
            Request::get(path(1, 2))
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
    let request = Request::get(path(1, 2))
        .header("x-capsem-actor", "operator")
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn readiness_refuses_prepared_and_recovered_unapplied_revisions() {
    use capsem_logger::{
        events::{EnvironmentRevisionState, EnvironmentRevisionTransition},
        WriteOp,
    };
    let state = make_test_state();
    seed(&state).await;
    let specification = state
        .environments
        .load_current(&id(1))
        .await
        .unwrap()
        .document
        .specification;
    state
        .host_ledger
        .write(WriteOp::PrepareEnvironmentRevision(PreparedEnvironmentRevision {
            environment_id: id(1),
            revision_id: id(3),
            previous_revision_id: Some(id(2)),
            fork_parent: None,
            timestamp_unix_ms: 3,
            actor: "operator".into(),
            reason: "not committed".into(),
            request_id: None,
            payload_event_id: "000000000003".into(),
            specification: Box::new(specification),
        }))
        .await
        .unwrap();
    state.host_ledger.flush().await.unwrap();
    let app = crate::router_runtime::build_service_router(state.clone());
    assert_eq!(
        route_request(app.clone(), Method::GET, &path(1, 3), None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    state
        .host_ledger
        .write(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: id(1),
            revision_id: id(3),
            state: EnvironmentRevisionState::RecoveredUnapplied,
            timestamp_unix_ms: 4,
            actor: "operator".into(),
        }))
        .await
        .unwrap();
    state.host_ledger.flush().await.unwrap();
    assert_eq!(
        route_request(app, Method::GET, &path(1, 3), None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

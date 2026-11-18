use super::*;
use capsem_api::{requests::RequestApplicationState, EnvironmentMutationResponse};
use serde_json::json;

fn intent(n: u64) -> serde_json::Value {
    json!({"request_id": id(n), "base_revision_id":id(2), "specification":{"name":"development","variables":{"EDITOR":"emacs"}},"reason":"change development environment"})
}

#[tokio::test]
async fn operator_changes_publish_linear_history_and_reconcile_without_reapplying() {
    let state = make_test_state();
    seed(&state).await;
    let app = crate::router_runtime::build_service_router(state.clone());
    let path = format!("/environments/{}/changes", id(1));
    let (status, body) = route_request(app.clone(), Method::POST, &path, Some(intent(100))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let changed: EnvironmentMutationResponse = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(changed.environment_id, id(1));
    assert_eq!(changed.application, RequestApplicationState::Applied);
    let current = state.environments.load_current(&id(1)).await.unwrap();
    assert_eq!(current.revision.previous_revision_id, Some(id(2)));
    assert_eq!(current.document.specification.variables["EDITOR"], "emacs");
    assert_eq!(
        current.document.identity.revision_id,
        changed.result.unwrap().revision_id
    );
    assert_eq!(
        route_request(app.clone(), Method::POST, &path, Some(intent(100))).await,
        (StatusCode::OK, body)
    );
    let (status, _) = route_request(app.clone(), Method::POST, &path, Some(intent(101))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(state
        .host_ledger
        .environment_request_snapshot(&id(101))
        .await
        .unwrap()
        .is_none());
    let mut contradictory = intent(100);
    contradictory["reason"] = "changed meaning".into();
    assert_eq!(
        route_request(app.clone(), Method::POST, &path, Some(contradictory))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (status, view) = route_request(app, Method::GET, &format!("/environments/{}/info", id(1)), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["document"]["specification"]["variables"]["EDITOR"], "emacs");
}

#[tokio::test]
async fn changes_reject_forged_authority_requesters_and_self_attested_pins() {
    let state = make_test_state();
    seed(&state).await;
    let app = crate::router_runtime::build_service_router(state.clone());
    let path = format!("/environments/{}/changes", id(1));
    for field in ["actor", "requesting_sandbox", "pin"] {
        let mut value = intent(100);
        match field {
            "actor" => value["actor"] = "operator".into(),
            "requesting_sandbox" => value["requesting_sandbox"] = json!({"session_id":id(3),"generation":id(4)}),
            _ => {
                value["specification"]["image"] =
                    json!({"reference":"development:stable","resolved_digest":format!("sha256:{}","a".repeat(64))})
            }
        }
        let (status, body) = route_request(app.clone(), Method::POST, &path, Some(value)).await;
        assert!(
            matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY),
            "{status}: {body}"
        );
        assert!(!body.to_string().contains("emacs"));
    }
    assert!(state
        .host_ledger
        .environment_request_snapshot(&id(100))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn changes_distinguish_malformed_missing_and_untrusted_callers() {
    use crate::tests::OperatorRequestExt;
    let state = make_test_state();
    seed(&state).await;
    let (_, bearer) = delegate(&state, capsem_api::AgentOperation::Propose, vec![id(1)]);
    let app = crate::router_runtime::build_service_router(state);
    for (id, expected) in [
        ("not-a-uuid".to_string(), StatusCode::BAD_REQUEST),
        (id(99), StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            route_request(
                app.clone(),
                Method::POST,
                &format!("/environments/{id}/changes"),
                Some(intent(100))
            )
            .await
            .0,
            expected
        );
    }
    let request = axum::http::Request::post(format!("/environments/{}/changes", id(1)))
        .header("authorization", bearer)
        .header("content-type", "application/json")
        .header("x-capsem-actor", "operator")
        .body(axum::body::Body::from(serde_json::to_vec(&intent(100)).unwrap()))
        .unwrap();
    assert_eq!(
        app.operator_request(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

use super::*;
use axum::body::{to_bytes, Body};
use http::{Request, StatusCode};
use tower::ServiceExt;

fn app() -> Router {
    let (_, state) = crate::tests::health_app("/tmp/capsem-schema-no-service.sock");
    routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::auth_middleware,
        ))
        .with_state(state)
}

#[tokio::test]
async fn schema_requires_the_existing_gateway_authentication() {
    let response = app()
        .oneshot(Request::builder().uri("/openapi.json").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authenticated_schema_matches_the_export_without_a_service() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .header("Authorization", "Bearer test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 2 * 1024 * 1024).await.unwrap();
    let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(document, serde_json::to_value(capsem_api::openapi()).unwrap());
}

#[tokio::test]
async fn every_documented_operation_is_forwarded_by_the_real_gateway_router() {
    let (_, state) = crate::tests::health_app("/tmp/capsem-schema-no-service.sock");
    let app = crate::service_proxy_routes()
        .route("/status", axum::routing::get(crate::status::handle_status))
        .with_state(state);
    let document = serde_json::to_value(capsem_api::openapi()).unwrap();
    for (path, methods) in document["paths"].as_object().unwrap() {
        for method in methods.as_object().unwrap().keys() {
            let uri = path.replace("{id}", "schema-test-vm");
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.to_uppercase().as_str())
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let expected = if path == "/status" {
                StatusCode::OK
            } else {
                StatusCode::BAD_GATEWAY
            };
            assert_eq!(
                response.status(),
                expected,
                "{method} {path} must reach its production handler"
            );
        }
    }
}

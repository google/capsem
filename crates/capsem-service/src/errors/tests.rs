//! Tests for AppError JSON shape, status code preservation, and the
//! `app_error_logged!` macro (compile-time integration only -- the
//! tracing line itself is covered by tracing-test if/when added; for
//! now we just assert the macro builds an AppError with the right
//! status + msg shape).

use super::*;
use axum::body::to_bytes;
use axum::http::StatusCode;

#[tokio::test]
async fn app_error_formats_json() {
    let err = AppError(StatusCode::BAD_REQUEST, "invalid sandbox name".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "invalid sandbox name");
}

#[tokio::test]
async fn app_error_internal_server() {
    let err = AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "db connection failed".into(),
    );
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "db connection failed");
}

#[tokio::test]
async fn app_error_conflict() {
    let err = AppError(StatusCode::CONFLICT, "sandbox already exists".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "sandbox already exists");
}

#[tokio::test]
async fn app_error_preserves_arbitrary_status() {
    let err = AppError(StatusCode::IM_A_TEAPOT, "no coffee here".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "no coffee here");
}

#[tokio::test]
async fn app_error_preserves_empty_message() {
    let err = AppError(StatusCode::FORBIDDEN, String::new());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "");
}

#[test]
fn app_error_logged_bare_form_builds_correct_appe() {
    let id = "vm-test";
    let err = app_error_logged!(
        error,
        StatusCode::INTERNAL_SERVER_ERROR,
        "exec failed for {id}: io error"
    );
    assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(err.1.contains("vm-test"));
}

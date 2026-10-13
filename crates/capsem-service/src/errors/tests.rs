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
    let err = AppError::new(StatusCode::BAD_REQUEST, "invalid sandbox name".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "invalid sandbox name");
}

#[tokio::test]
async fn app_error_internal_server() {
    let err = AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "db connection failed".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "db connection failed");
}

#[tokio::test]
async fn app_error_conflict() {
    let err = AppError::new(StatusCode::CONFLICT, "sandbox already exists".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "sandbox already exists");
}

#[tokio::test]
async fn app_error_preserves_arbitrary_status() {
    let err = AppError::new(StatusCode::IM_A_TEAPOT, "no coffee here".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "no coffee here");
}

#[tokio::test]
async fn app_error_preserves_empty_message() {
    let err = AppError::new(StatusCode::FORBIDDEN, String::new());
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
    assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(err.body.error.contains("vm-test"));
}

#[tokio::test]
async fn app_error_structured_builders_serialize_fields() {
    let err = AppError::vm_not_found("vm-42");
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "sandbox not found: vm-42");
    assert_eq!(json["code"], "vm_not_found");
    assert_eq!(json["vm_id"], "vm-42");
    assert!(json.get("timeout_secs").is_none());
}

#[tokio::test]
async fn app_error_exec_timeout_serializes_fields_and_504() {
    let err = AppError::exec_timeout("command timed out after 30s", 30);
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "command timed out after 30s");
    assert_eq!(json["code"], "exec_timeout");
    assert_eq!(json["timeout_secs"], 30);
    assert!(json.get("vm_id").is_none());
}

#[tokio::test]
async fn app_error_create_timeout_and_into_message() {
    let err = AppError::create_timeout("vm-99", 4);
    assert_eq!(err.status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(err.body.code, Some(ErrorCode::CreateTimeout));
    assert_eq!(err.body.vm_id.as_deref(), Some("vm-99"));
    let msg = err.into_message();
    assert!(msg.contains("vm-99"));
}

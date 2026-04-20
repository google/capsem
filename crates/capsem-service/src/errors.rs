//! HTTP error type used by every axum handler in the service.
//!
//! `ErrorResponse` (the on-the-wire JSON shape) lives in `api.rs` so the public
//! API surface stays in one place; this module re-exports it for ergonomics.

use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;

pub use crate::api::ErrorResponse;

/// Tuple of (HTTP status, error message). Implements `IntoResponse` so handlers
/// can `?` against `Result<T, AppError>` and get a JSON `{"error": "..."}`
/// body with the right status code.
#[derive(Debug)]
pub struct AppError(pub StatusCode, pub String);

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            self.0,
            Json(ErrorResponse {
                error: self.1,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
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
        let err = AppError(StatusCode::INTERNAL_SERVER_ERROR, "db connection failed".into());
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
        // Non-standard status codes (418 I'm a teapot, etc.) round-trip cleanly.
        let err = AppError(StatusCode::IM_A_TEAPOT, "no coffee here".into());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "no coffee here");
    }

    #[tokio::test]
    async fn app_error_preserves_empty_message() {
        // Empty strings are still valid JSON values; assert no panic and shape stays intact.
        let err = AppError(StatusCode::FORBIDDEN, String::new());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "");
    }
}

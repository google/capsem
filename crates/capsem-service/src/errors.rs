//! HTTP error type used by every axum handler in the service.
//!
//! `ErrorResponse` (the on-the-wire JSON shape) lives in `api.rs` so the public
//! API surface stays in one place; this module re-exports it for ergonomics.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

pub use crate::api::ErrorResponse;

/// Tuple of (HTTP status, error message). Implements `IntoResponse` so handlers
/// can `?` against `Result<T, AppError>` and get a JSON `{"error": "..."}`
/// body with the right status code.
///
/// Every `AppError` is automatically logged as it goes out the door (see the
/// `IntoResponse` impl below). 5xx → `tracing::error!`, 4xx → `tracing::warn!`,
/// other → `info!`. The operator sees a structured `target = "service"` line
/// for every error response without per-site work. Pre-W3.5: the operator
/// got a 500 in the response and nothing in the log to trace back from.
#[derive(Debug)]
pub struct AppError(pub StatusCode, pub String);

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let status = self.0;
        let msg = self.1.as_str();
        if status.is_server_error() {
            ::tracing::error!(
                target: "service",
                status = status.as_u16(),
                "{}",
                msg
            );
        } else if status.is_client_error() {
            ::tracing::warn!(
                target: "service",
                status = status.as_u16(),
                "{}",
                msg
            );
        } else {
            ::tracing::info!(
                target: "service",
                status = status.as_u16(),
                "{}",
                msg
            );
        }

        (self.0, Json(ErrorResponse { error: self.1 })).into_response()
    }
}

/// Construct an `AppError` and emit an early `tracing` event at the call
/// site (in addition to the late one fired by `IntoResponse`). Use this
/// when you want the log line BEFORE the response is built -- e.g. so a
/// span timer sees the error inside the operation -- or when the bare
/// (status, msg) is enough but you want the operator to see it
/// twice-with-different-fields. Most sites can rely on the
/// `IntoResponse` auto-log alone; reach for this macro only when context
/// would be lost otherwise.
///
/// Usage: `return Err(app_error_logged!(error, StatusCode::INTERNAL_SERVER_ERROR, "exec failed: {e}"));`
#[macro_export]
macro_rules! app_error_logged {
    ($lvl:ident, $status:expr, $($fmt:tt)+) => {{
        let __msg = format!($($fmt)+);
        ::tracing::$lvl!(
            target: "service",
            status = $status.as_u16(),
            "{}", __msg
        );
        $crate::errors::AppError($status, __msg)
    }};
}

#[cfg(test)]
mod tests;

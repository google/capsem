//! HTTP error type used by every axum handler in the service.
//!
//! `ErrorResponse` (the on-the-wire JSON shape) lives in `api.rs` so the public
//! API surface stays in one place; this module re-exports it for ergonomics.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

pub use crate::api::{ErrorCode, ErrorResponse};

/// Structured `(HTTP status, ErrorResponse)` returned by service route handlers.
/// Implements `IntoResponse` so handlers can `?` against `Result<T, AppError>`
/// and get a JSON `{"error": "..."}` body with the right status code.
///
/// Every `AppError` is automatically logged as it goes out the door (see the
/// `IntoResponse` impl below). 5xx → `tracing::error!`, 4xx → `tracing::warn!`,
/// other → `info!`. The operator sees a structured `target = "service"` line
/// for every error response without per-site work. Pre-W3.5: the operator
/// got a 500 in the response and nothing in the log to trace back from.
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub body: ErrorResponse,
}

impl AppError {
    /// Construct a plain `(status, error)` response without structured metadata.
    pub fn new(status: StatusCode, error: String) -> Self {
        Self {
            status,
            body: ErrorResponse {
                error,
                code: None,
                vm_id: None,
                timeout_secs: None,
            },
        }
    }

    pub fn into_message(self) -> String {
        self.body.error
    }

    pub fn with_code(mut self, code: ErrorCode) -> Self {
        self.body.code = Some(code);
        self
    }

    pub fn with_vm_id(mut self, vm_id: impl Into<String>) -> Self {
        self.body.vm_id = Some(vm_id.into());
        self
    }

    pub fn with_timeout_secs(mut self, timeout_secs: u64) -> Self {
        self.body.timeout_secs = Some(timeout_secs);
        self
    }

    pub fn vm_not_found(id: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("sandbox not found: {id}"))
            .with_code(ErrorCode::VmNotFound)
            .with_vm_id(id)
    }

    pub fn exec_timeout(error: impl Into<String>, timeout_secs: u64) -> Self {
        Self::new(StatusCode::GATEWAY_TIMEOUT, error.into())
            .with_code(ErrorCode::ExecTimeout)
            .with_timeout_secs(timeout_secs)
    }

    pub fn create_timeout(id: &str, attempts: u32) -> Self {
        ::tracing::warn!(vm_id = id, attempts, "container create readiness timed out");
        Self::new(
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "container workload for VM {id} did not become ready before the HTTP deadline; setup continues under service ownership"
            ),
        )
        .with_code(ErrorCode::CreateTimeout)
        .with_vm_id(id)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status;
        let msg = self.body.error.as_str();
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

        (self.status, Json(self.body)).into_response()
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
        $crate::errors::AppError::new($status, __msg)
    }};
}

#[cfg(test)]
mod tests;

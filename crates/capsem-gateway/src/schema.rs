//! The gateway serves the contract used to generate its clients.

use std::sync::Arc;

use axum::{routing::get, Json, Router};

use crate::AppState;

pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/openapi.json", get(|| async { Json(capsem_api::openapi()) }))
}

#[cfg(test)]
mod tests;

//! Xpra surfaces: the GUI an image declares, opened in the user's browser.
//!
//! The service grants the image's declared port as an `http_preview`
//! exposure and reports it in the VM's container status. The client must run
//! on that exposure's preview origin: its websocket is authenticated by the
//! origin's host-only, SameSite=Lax session cookie, which a page on any other
//! site cannot send. So the gateway serves the pinned xpra-html5 client there
//! (see `client` and `preview`), and its own origin serves only a launcher:
//! `GET /vms/{id}/surface/` reads the gateway token like the Capsem UI does,
//! mints a single-use bootstrap with `POST /vms/{id}/surface/session`, and
//! POSTs it to the surface's origin, which sets the cookie and opens the
//! client. No token appears in any URL.

mod client;

pub(crate) use client::{response as client_response, CLIENT_ROOT};

use crate::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use capsem_api::{ContainerStatusResponse, ContainerSurfaceKind};
use std::sync::Arc;

const LAUNCHER: &str = include_str!("surface/assets/launcher.html");
const LAUNCH_SCRIPT: &str = include_str!("surface/assets/launch.js");

fn no_surface() -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({"error": "VM has no running app surface"})),
    )
        .into_response()
}

/// POST /vms/{id}/surface/session -- a single-use bootstrap for the VM's Xpra
/// surface, whose exposure the service recorded in the container status.
pub async fn create_session(State(state): State<Arc<AppState>>, Path(vm_id): Path<String>) -> Response {
    if !crate::preview::safe_id(&vm_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let path = format!("/vms/{vm_id}/container");
    let status: ContainerStatusResponse =
        match crate::preview::service_json(&state, http::Method::GET, &path, None).await {
            Ok(status) => status,
            Err(response) if response.status() == StatusCode::NOT_FOUND => return no_surface(),
            Err(response) => return response,
        };
    let Some(exposure_id) = status
        .surface
        .filter(|surface| surface.kind == ContainerSurfaceKind::Xpra)
        .and_then(|surface| surface.exposure_id)
    else {
        return no_surface();
    };
    crate::preview::mint(&state, vm_id, exposure_id, true).await
}

/// The launcher's policy: its script, same-origin calls, and a form posted
/// only to a preview origin on this gateway's preview port.
fn launcher_policy(preview_port: u16) -> String {
    format!(
        "default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; \
         form-action http://*.localhost:{preview_port}; frame-ancestors 'none'; base-uri 'none'"
    )
}

fn hardened(response: impl IntoResponse, content_type: &'static str, policy: String) -> Response {
    let mut response = response.into_response();
    let headers = response.headers_mut();
    for (name, value) in [
        (http::header::CONTENT_TYPE, content_type.to_owned()),
        (http::header::CONTENT_SECURITY_POLICY, policy),
        (http::header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        (http::header::X_FRAME_OPTIONS, "DENY".to_owned()),
        (http::header::REFERRER_POLICY, "no-referrer".to_owned()),
        (http::header::CACHE_CONTROL, "no-store".to_owned()),
        (
            http::HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin".to_owned(),
        ),
    ] {
        if let Ok(value) = http::HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    }
    response
}

/// GET /vms/{id}/surface/ -- the launcher page. It holds nothing secret, so
/// it is served without a bearer token. A page another site opened must be
/// clicked through: it can open this tab, but not open the app in it.
pub async fn launcher(State(state): State<Arc<AppState>>, Path(vm_id): Path<String>, headers: HeaderMap) -> Response {
    if !crate::preview::safe_id(&vm_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let cross_site = headers
        .get("sec-fetch-site")
        .is_some_and(|site| site.as_bytes().eq_ignore_ascii_case(b"cross-site"));
    let page = LAUNCHER.replace("{{auto}}", if cross_site { "false" } else { "true" });
    hardened(page, "text/html; charset=utf-8", launcher_policy(state.previews.port()))
}

/// GET /vms/{id}/surface/launch.js -- the launcher's script.
pub async fn launcher_script(State(state): State<Arc<AppState>>) -> Response {
    hardened(
        LAUNCH_SCRIPT,
        "text/javascript; charset=utf-8",
        launcher_policy(state.previews.port()),
    )
}

#[cfg(test)]
mod tests;

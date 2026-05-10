use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tracing::{info, warn};

use crate::AppState;

/// Classify the shape of an authorization header for diagnostic logs. Never
/// returns token bytes -- only the structural shape -- so it's safe to log
/// at info level without leaking secrets.
fn classify_auth_header(raw: Option<&http::HeaderValue>) -> &'static str {
    let Some(v) = raw else { return "absent" };
    let bytes = v.as_bytes();
    if bytes.is_empty() {
        return "empty";
    }
    let Ok(s) = v.to_str() else {
        return "non-ascii";
    };
    if let Some(rest) = s.strip_prefix("Bearer ") {
        if rest.is_empty() {
            "bearer-empty"
        } else if rest.trim().is_empty() {
            "bearer-whitespace"
        } else {
            "bearer-present"
        }
    } else if s.starts_with("Bearer") {
        "bearer-no-space"
    } else if s.starts_with("bearer") {
        "bearer-lowercase"
    } else if s.starts_with("Basic ") {
        "basic"
    } else {
        "unknown-scheme"
    }
}

/// Maximum auth failures per window before returning 429.
const MAX_AUTH_FAILURES: u32 = 20;
/// Time window for counting auth failures.
const AUTH_FAILURE_WINDOW: Duration = Duration::from_secs(60);

/// Tracks auth failure rate to throttle brute-force attempts.
pub struct AuthFailureTracker {
    inner: tokio::sync::Mutex<(Instant, u32)>,
}

impl AuthFailureTracker {
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::Mutex::new((Instant::now(), 0)),
        }
    }

    /// Record a failure. Returns true if the caller should be throttled (429).
    pub async fn record_failure(&self) -> bool {
        let mut guard = self.inner.lock().await;
        let (ref mut window_start, ref mut count) = *guard;
        if window_start.elapsed() > AUTH_FAILURE_WINDOW {
            *window_start = Instant::now();
            *count = 1;
            false
        } else {
            *count += 1;
            *count > MAX_AUTH_FAILURES
        }
    }
}

/// Runtime file state for cleanup on shutdown.
#[derive(Clone)]
pub struct AuthState {
    pub token_path: PathBuf,
    pub port_path: PathBuf,
    pub pid_path: PathBuf,
}

impl AuthState {
    /// Generate runtime files: token (600), port, pid.
    pub fn new(run_dir: &Path, token: &str, port: u16) -> Result<Self> {
        std::fs::create_dir_all(run_dir)
            .with_context(|| format!("failed to create run dir: {}", run_dir.display()))?;

        let token_path = run_dir.join("gateway.token");
        let port_path = run_dir.join("gateway.port");
        let pid_path = run_dir.join("gateway.pid");

        std::fs::write(&token_path, token)
            .with_context(|| format!("failed to write {}", token_path.display()))?;

        // chmod 600 on token file
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))?;
        }

        std::fs::write(&port_path, port.to_string())?;
        std::fs::write(&pid_path, std::process::id().to_string())?;

        info!(
            token_path = %token_path.display(),
            port_path = %port_path.display(),
            pid_path = %pid_path.display(),
            "runtime files written"
        );

        Ok(Self {
            token_path,
            port_path,
            pid_path,
        })
    }

    /// Remove runtime files on shutdown.
    pub fn cleanup(&self) {
        for path in [&self.token_path, &self.port_path, &self.pid_path] {
            if let Err(e) = std::fs::remove_file(path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = %path.display(), error = %e, "failed to remove runtime file");
                }
            }
        }
        info!("runtime files cleaned up");
    }
}

/// Generate a 64-character alphanumeric random token.
pub fn generate_token() -> String {
    use rand::Rng;
    rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}

/// Axum middleware: require Bearer token on all routes except `GET /health` and `GET /token`.
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // Health check and token endpoint are unauthenticated (token has its own IP check)
    let path = req.uri().path();
    if req.method() == http::Method::GET && (path == "/" || path == "/health" || path == "/token") {
        return next.run(req).await;
    }

    let header_valid = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t == state.token);

    // For WebSocket paths: allow ?token= query param as fallback
    // (browser WebSocket API cannot set custom headers).
    // Only the "token" param is recognized; all others are dropped.
    let query_valid = !header_valid
        && (path.starts_with("/terminal/") || path == "/events")
        && req
            .uri()
            .query()
            .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("token=")))
            .is_some_and(|t| t == state.token);

    if header_valid || query_valid {
        next.run(req).await
    } else {
        let shape = classify_auth_header(req.headers().get("authorization"));
        let method = req.method().clone();
        let throttled = state.auth_failures.record_failure().await;
        if throttled {
            warn!(%method, path, shape, "auth rejected (429 throttled)");
            (
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(serde_json::json!({"error": "too many failed auth attempts"})),
            )
                .into_response()
        } else {
            info!(%method, path, shape, "auth rejected (401)");
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error": "unauthorized"})),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests;

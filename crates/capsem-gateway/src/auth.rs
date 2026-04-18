use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tracing::info;

use crate::AppState;

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
    if req.method() == http::Method::GET
        && (path == "/" || path == "/health" || path == "/token")
    {
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
            .and_then(|q| {
                q.split('&')
                    .find_map(|pair| pair.strip_prefix("token="))
            })
            .is_some_and(|t| t == state.token);

    if header_valid || query_valid {
        next.run(req).await
    } else {
        let throttled = state.auth_failures.record_failure().await;
        if throttled {
            (
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(serde_json::json!({"error": "too many failed auth attempts"})),
            )
                .into_response()
        } else {
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error": "unauthorized"})),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use axum::Router;
    use http::Request;
    use tower::ServiceExt;

    use crate::status::StatusCache;

    fn test_state(token: &str) -> Arc<AppState> {
        Arc::new(AppState {
            token: token.to_string(),
            uds_path: "/tmp/nonexistent.sock".into(),
            status_cache: StatusCache::new(),
            auth_failures: AuthFailureTracker::new(),
            events_tx: tokio::sync::broadcast::channel(16).0,
        })
    }

    fn test_app(token: &str) -> Router {
        let state = test_state(token);
        Router::new()
            .route("/", get(|| async { "health" }))
            .route("/health", get(|| async { "health" }))
            .route("/token", get(|| async { "token" }))
            .route("/list", get(|| async { "ok" }))
            .route("/status", get(|| async { "status" }))
            .route("/terminal/{id}", get(|| async { "terminal" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state)
    }

    // --- Token generation ---

    #[test]
    fn token_is_64_chars_alphanumeric() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn token_is_unique() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn token_uniqueness_over_100_samples() {
        let tokens: std::collections::HashSet<String> =
            (0..100).map(|_| generate_token()).collect();
        assert_eq!(tokens.len(), 100);
    }

    // --- AuthState file lifecycle ---

    #[test]
    fn auth_state_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::new(dir.path(), "test-token", 19222).unwrap();

        assert!(state.token_path.exists());
        assert_eq!(std::fs::read_to_string(&state.token_path).unwrap(), "test-token");
        assert_eq!(std::fs::read_to_string(&state.port_path).unwrap(), "19222");
        assert!(state.pid_path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&state.token_path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }

        state.cleanup();
        assert!(!state.token_path.exists());
        assert!(!state.port_path.exists());
        assert!(!state.pid_path.exists());
    }

    #[test]
    fn auth_state_creates_run_dir_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested/deep");
        assert!(!nested.exists());

        let state = AuthState::new(&nested, "tok", 9999).unwrap();
        assert!(nested.exists());
        assert!(state.token_path.exists());
        state.cleanup();
    }

    #[test]
    fn auth_state_pid_file_contains_current_pid() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::new(dir.path(), "tok", 1234).unwrap();
        let pid: u32 = std::fs::read_to_string(&state.pid_path).unwrap().parse().unwrap();
        assert_eq!(pid, std::process::id());
        state.cleanup();
    }

    #[test]
    fn cleanup_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::new(dir.path(), "tok", 1234).unwrap();
        state.cleanup();
        state.cleanup(); // second call should not panic
    }

    // --- Auth middleware ---

    #[tokio::test]
    async fn health_endpoint_requires_no_auth() {
        let app = test_app("secret-token");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_request_without_token() {
        let app = test_app("secret-token");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "unauthorized");
    }

    #[tokio::test]
    async fn rejects_request_with_wrong_token() {
        let app = test_app("correct-token");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn accepts_request_with_valid_token() {
        let app = test_app("my-token");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "Bearer my-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_malformed_auth_header() {
        let app = test_app("tok");

        // No "Bearer " prefix
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "tok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Basic auth instead of Bearer
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "Basic dG9rOg==")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_empty_bearer_token() {
        let app = test_app("tok");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "Bearer ")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn post_to_health_requires_auth() {
        let app = test_app("tok");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // POST / is not the health check (only GET / is exempt)
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn all_non_root_paths_require_auth() {
        let app = test_app("tok");
        for path in ["/status", "/list"] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "{path} should require auth"
            );
        }
    }

    // --- Security edge cases ---

    #[tokio::test]
    async fn rejects_double_space_bearer() {
        let app = test_app("tok");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "Bearer  tok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_lowercase_bearer() {
        let app = test_app("tok");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "bearer tok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_tab_separated_bearer() {
        let app = test_app("tok");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "Bearer\ttok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_token_with_trailing_whitespace() {
        let app = test_app("tok");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "Bearer tok ")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_non_ascii_auth_header() {
        let app = test_app("tok");
        // HeaderValue::from_bytes accepts 0x80, but to_str() rejects non-visible-ASCII
        let hv = http::HeaderValue::from_bytes(&[0x80, 0x81]).unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", hv)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn health_exempt_only_exact_paths() {
        let app = test_app("tok");
        // /healthz is NOT exempt
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // GET / is exempt
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/?foo=bar")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // GET /health is exempt
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn token_endpoint_exempt_from_auth() {
        let app = test_app("tok");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Should pass through middleware without 401 (handler may still fail
        // without ConnectInfo in unit test, but the point is it's not rejected
        // by auth middleware).
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // --- WebSocket query-param auth (terminal paths only) ---

    #[tokio::test]
    async fn terminal_accepts_query_param_token() {
        let app = test_app("my-secret");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/terminal/vm1?token=my-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn terminal_rejects_wrong_query_param_token() {
        let app = test_app("correct");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/terminal/vm1?token=wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn non_terminal_path_ignores_query_param_token() {
        let app = test_app("tok");
        // /list with ?token= should still require header auth
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list?token=tok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn terminal_extra_query_params_ignored() {
        let app = test_app("tok");
        // Extra params present but only token is checked
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/terminal/vm1?evil=payload&token=tok&other=stuff")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn terminal_header_auth_still_works() {
        let app = test_app("tok");
        // Header auth should still work on terminal paths (no query needed)
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/terminal/vm1")
                    .header("authorization", "Bearer tok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn post_to_token_requires_auth() {
        let app = test_app("tok");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // POST /token is not exempt (only GET is)
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn delete_method_on_root_requires_auth() {
        let app = test_app("tok");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // --- Rate limiting (issue #3) ---

    #[tokio::test]
    async fn auth_failure_tracker_allows_initial_failures() {
        let tracker = AuthFailureTracker::new();
        for _ in 0..MAX_AUTH_FAILURES {
            assert!(!tracker.record_failure().await, "should not throttle within limit");
        }
    }

    #[tokio::test]
    async fn auth_failure_tracker_throttles_after_limit() {
        let tracker = AuthFailureTracker::new();
        for _ in 0..MAX_AUTH_FAILURES {
            tracker.record_failure().await;
        }
        assert!(tracker.record_failure().await, "should throttle after exceeding limit");
    }

    #[tokio::test]
    async fn auth_failure_tracker_resets_after_window() {
        let tracker = AuthFailureTracker::new();
        // Exhaust the limit
        for _ in 0..=MAX_AUTH_FAILURES {
            tracker.record_failure().await;
        }
        assert!(tracker.record_failure().await);

        // Simulate window expiry by backdating the window start
        {
            let mut guard = tracker.inner.lock().await;
            guard.0 = Instant::now() - AUTH_FAILURE_WINDOW - Duration::from_secs(1);
        }
        // After window reset, should allow again
        assert!(!tracker.record_failure().await);
    }

    #[tokio::test]
    async fn returns_429_after_too_many_failures() {
        let state = test_state("secret");
        // Exhaust the failure budget
        {
            for _ in 0..=MAX_AUTH_FAILURES {
                state.auth_failures.record_failure().await;
            }
        }

        let app = Router::new()
            .route("/", get(|| async { "health" }))
            .route("/list", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "Bearer wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "too many failed auth attempts");
    }

    #[tokio::test]
    async fn valid_auth_succeeds_even_after_many_failures() {
        let state = test_state("correct-token");
        // Exhaust some failures but still under limit
        for _ in 0..5 {
            state.auth_failures.record_failure().await;
        }

        let app = Router::new()
            .route("/list", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/list")
                    .header("authorization", "Bearer correct-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

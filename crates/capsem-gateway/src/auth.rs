use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tracing::info;

use crate::AppState;

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

/// Axum middleware: require Bearer token on all routes except `GET /`.
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // Health check is unauthenticated
    if req.uri().path() == "/" && req.method() == http::Method::GET {
        return next.run(req).await;
    }

    let valid = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t == state.token);

    if valid {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

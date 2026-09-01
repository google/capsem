//! Ownership-safe cleanup for ephemeral sessions.

use super::*;

#[cfg(test)]
mod tests;

pub(super) async fn preserve_failed_run_shutdown_result(
    state: Arc<ServiceState>,
    id: String,
    shutdown_result: Option<(PathBuf, bool, u32)>,
) -> Result<Option<PathBuf>, AppError> {
    let Some((session_dir, persistent, _pid)) = shutdown_result else {
        tracing::debug!(id, "failed session preservation already owned by child watcher");
        return Ok(None);
    };
    if persistent {
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("one-shot session {id} unexpectedly used persistent storage"),
        ));
    }
    tokio::task::spawn_blocking(move || state.preserve_failed_session_dir(&session_dir, &id))
        .await
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed session preservation task: {error}"),
            )
        })
}

async fn ensure_failed_session_preserved(
    state: Arc<ServiceState>,
    id: String,
    shutdown_result: Option<(PathBuf, bool, u32)>,
) -> Result<Option<PathBuf>, AppError> {
    if let Some(path) = preserve_failed_run_shutdown_result(Arc::clone(&state), id.clone(), shutdown_result).await? {
        return Ok(Some(path));
    }
    let original = state.run_dir.join("sessions").join(&id);
    tokio::task::spawn_blocking(move || {
        for _ in 0..20 {
            if let Some(path) = find_failed_session_dir(&state.run_dir, &id) {
                return Ok(Some(path));
            }
            if original.exists() {
                return Ok(state.preserve_failed_session_dir(&original, &id));
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        Ok(None)
    })
    .await
    .map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed session preservation task: {error}"),
        )
    })?
}

pub(super) async fn finalize_one_shot_session(
    state: Arc<ServiceState>,
    id: String,
    shutdown_result: Option<(PathBuf, bool, u32)>,
    failed: bool,
) -> Result<Option<PathBuf>, AppError> {
    if failed {
        return ensure_failed_session_preserved(state, id, shutdown_result).await;
    }
    let session_dir = shutdown_result
        .map(|(path, _, _)| path)
        .unwrap_or_else(|| state.run_dir.join("sessions").join(id));
    if !session_dir.exists() {
        return Ok(None);
    }
    tokio::task::spawn_blocking(move || state.delete_session_dir(&session_dir))
        .await
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("one-shot cleanup task failed: {error}"),
            )
        })?
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("one-shot cleanup failed: {error:#}"),
            )
        })?;
    Ok(None)
}

pub(super) async fn handle_preserve_failure(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let shutdown_result = shutdown_vm_process(&state, &id, ShutdownMode::Retain).await?;
    let _preserved = ensure_failed_session_preserved(Arc::clone(&state), id.clone(), shutdown_result)
        .await?
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
    Ok(Json(json!({ "success": true })))
}

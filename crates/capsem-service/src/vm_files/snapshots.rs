use super::*;
#[cfg(test)]
mod tests;

pub(crate) async fn handle_vm_snapshots_status(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::SnapshotsStatus>, AppError> {
    if let Some(uds_path) = {
        let instances = state.instances.lock().unwrap();
        instances.get(&id).map(|instance| instance.uds_path.clone())
    } {
        let request_id = state.job_counter.fetch_add(1, Ordering::SeqCst);
        let response = send_ipc_command(&uds_path, ServiceToProcess::SnapshotStatus { id: request_id }, Some(5))
            .await
            .map_err(|error| AppError(StatusCode::BAD_GATEWAY, error))?;
        return match response {
            ProcessToService::SnapshotStatusResult {
                id: response_id,
                status,
            } if response_id == request_id => snapshot_response(status).map(Json),
            other => Err(AppError(
                StatusCode::BAD_GATEWAY,
                format!("unexpected snapshot status IPC response: {other:?}"),
            )),
        };
    }

    let session_dir = resolve_session_dir(&state, &id)?;
    let status = state
        .off_worker(move |_| snapshot_status_from_session_dir(&session_dir))
        .await?;
    Ok(Json(status))
}

pub(crate) async fn handle_vm_snapshots_list(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::SnapshotsList>, AppError> {
    let Json(status) = handle_vm_snapshots_status(State(state), Path(id)).await?;
    Ok(Json(api::SnapshotsList {
        total: status.total,
        snapshots: status.snapshots,
    }))
}

fn snapshot_response(status: capsem_proto::ipc::SnapshotStatus) -> Result<api::SnapshotsStatus, AppError> {
    let snapshots = status
        .snapshots
        .into_iter()
        .map(|slot| {
            let origin = match slot.origin.as_str() {
                "auto" => api::SnapshotOrigin::Auto,
                "manual" => api::SnapshotOrigin::Manual,
                other => {
                    return Err(AppError(
                        StatusCode::BAD_GATEWAY,
                        format!("unknown snapshot origin: {other}"),
                    ))
                }
            };
            Ok(api::SnapshotInfo {
                checkpoint: slot.checkpoint,
                slot: slot.slot,
                origin,
                name: slot.name,
                timestamp: slot.timestamp,
                hash: slot.hash,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(api::SnapshotsStatus {
        total: status.total,
        auto_count: status.auto_count,
        manual_count: status.manual_count,
        manual_available: status.manual_available,
        snapshots,
    })
}

pub(crate) async fn handle_vm_changes(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<api::ChangesQuery>,
) -> Result<Json<api::ChangesResponse>, AppError> {
    let slot = params
        .checkpoint
        .strip_prefix("cp-")
        .filter(|slot| !slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|slot| slot.parse::<usize>().ok())
        .ok_or_else(|| {
            AppError(
                StatusCode::BAD_REQUEST,
                "checkpoint must be cp-<slot> from snapshots.list()".into(),
            )
        })?;
    let session_dir = resolve_session_dir(&state, &id)?;
    let workspace = capsem_core::guest_share_dir(&session_dir).join("workspace");
    let result = state
        .off_worker(move |_| {
            let scheduler = capsem_core::auto_snapshot::AutoSnapshotScheduler::new(
                session_dir,
                10,
                12,
                std::time::Duration::from_secs(300),
            );
            let snapshot = scheduler.get_snapshot(slot).ok_or_else(|| {
                AppError(
                    StatusCode::NOT_FOUND,
                    format!("checkpoint not found: {}", params.checkpoint),
                )
            })?;
            let changes = capsem_core::auto_snapshot::changes::workspace_changes(&snapshot.workspace_path, &workspace)
                .map_err(|error| {
                    AppError(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("workspace comparison failed: {error}"),
                    )
                })?;
            let total = changes.len();
            let changes = changes
                .into_iter()
                .skip(params.offset)
                .take(params.limit.unwrap_or(200).min(2000))
                .map(|change| api::FileChange {
                    path: change.path,
                    size: change.size,
                    is_symlink: change.is_symlink,
                    kind: match change.kind {
                        capsem_core::auto_snapshot::changes::ChangeKind::Created => api::FileChangeKind::Created,
                        capsem_core::auto_snapshot::changes::ChangeKind::Modified => api::FileChangeKind::Modified,
                        capsem_core::auto_snapshot::changes::ChangeKind::Deleted => api::FileChangeKind::Deleted,
                    },
                })
                .collect::<Vec<_>>();
            Ok::<_, AppError>(api::ChangesResponse {
                checkpoint: params.checkpoint,
                has_more: params.offset.saturating_add(changes.len()) < total,
                total,
                changes,
            })
        })
        .await??;
    Ok(Json(result))
}

pub(crate) fn snapshot_status_from_session_dir(session_dir: &std::path::Path) -> api::SnapshotsStatus {
    let scheduler = capsem_core::auto_snapshot::AutoSnapshotScheduler::new(
        session_dir.to_path_buf(),
        10,
        12,
        std::time::Duration::from_secs(300),
    );
    let snapshots = scheduler.list_snapshots();
    let auto_count = snapshots
        .iter()
        .filter(|slot| slot.origin == capsem_core::auto_snapshot::SnapshotOrigin::Auto)
        .count();
    let manual_count = snapshots.len().saturating_sub(auto_count);
    let snapshots = snapshots
        .into_iter()
        .map(|slot| api::SnapshotInfo {
            checkpoint: format!("cp-{}", slot.slot),
            slot: slot.slot,
            origin: match slot.origin {
                capsem_core::auto_snapshot::SnapshotOrigin::Auto => api::SnapshotOrigin::Auto,
                capsem_core::auto_snapshot::SnapshotOrigin::Manual => api::SnapshotOrigin::Manual,
            },
            name: slot.name,
            timestamp: humantime::format_rfc3339(slot.timestamp).to_string(),
            hash: slot.hash,
        })
        .collect();
    api::SnapshotsStatus {
        total: auto_count + manual_count,
        auto_count,
        manual_count,
        manual_available: scheduler.available_manual_slots(),
        snapshots,
    }
}

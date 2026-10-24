//! An admitted update belongs to the service, not the request's response future.

use super::*;
use tokio::sync::OwnedMutexGuard;
use tokio_util::sync::CancellationToken;

pub(super) async fn settle(
    state: Arc<ServiceState>,
    plan: api::UpdateCommandPlan,
    guard: OwnedMutexGuard<()>,
    kind: UpdateCommandKind,
) -> Result<(api::UpdateActionResponse, Option<UpdateRuntimeDisposition>), AppError> {
    let cancelled = CancellationToken::new();
    let _caller = cancelled.clone().drop_guard();
    // As with asset reconciliation, a service-owned task retains state and
    // admitted ownership across a caller dropping its result waiter. Apply
    // deliberately ignores caller cancellation: its installer must settle.
    tokio::spawn(async move {
        let _guard = guard;
        let result: Result<(api::UpdateActionResponse, Option<UpdateRuntimeDisposition>), AppError> = async {
            let completed = update_command::execute(plan, kind, cancelled).await?;
            let disposition = if matches!(kind, UpdateCommandKind::Apply) && completed.succeeded {
                Some(
                    state
                        .off_worker(|state| reload_activated_update_runtime(&state))
                        .await??,
                )
            } else {
                None
            };
            Ok((completed.response?, disposition))
        }
        .await;
        if let Err(error) = &result {
            warn!(error = %error.body.error, "service-owned update failed");
        }
        result
    })
    .await
    .map_err(|error| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("update owner failed: {error}"),
        )
    })?
}

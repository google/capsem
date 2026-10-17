use super::*;

/// Start a background reconciliation of the runtime asset set. Answers
/// whether this call started one: service startup and `/assets/ensure` share
/// one rail, so a second request while one runs is a no-op.
pub(super) fn start_asset_ensure(state: &Arc<ServiceState>) -> bool {
    if claim_asset_reconcile(state).is_err() {
        return false;
    }
    if let Err(error) = update_asset_reconcile_state(state, |status| {
        *status = AssetReconcileState {
            in_progress: true,
            ..Default::default()
        };
    }) {
        state.asset_reconcile_inflight.store(false, Ordering::Release);
        warn!(error = %error, "failed to start asset reconciliation");
        return false;
    }
    let state = Arc::clone(state);
    tokio::spawn(async move {
        match ensure_assets_after_claim(Arc::clone(&state)).await {
            Ok(downloaded) => info!(downloaded, "asset reconciliation finished"),
            Err(error) => warn!(error = %error, "asset reconciliation failed"),
        }
        state.asset_reconcile_inflight.store(false, Ordering::Release);
    });
    true
}

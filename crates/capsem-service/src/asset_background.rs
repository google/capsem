use super::*;

pub(super) fn start_profile_ensure(state: &Arc<ServiceState>, profile: &ProfileConfigFile) -> Result<bool, AppError> {
    if claim_asset_reconcile(state).is_err() {
        return Ok(false);
    }
    if let Err(error) = update_asset_reconcile_state(state, |status| {
        *status = AssetReconcileState {
            in_progress: true,
            ..Default::default()
        };
    }) {
        state.asset_reconcile_inflight.store(false, Ordering::Release);
        return Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, error));
    }

    let state = Arc::clone(state);
    let profile = profile.clone();
    tokio::spawn(async move {
        if let Err(error) = ensure_profile_assets_after_claim(Arc::clone(&state), &profile).await {
            warn!(profile = %profile.id, error = %error, "profile asset reconciliation failed");
        }
        if let Err(AppError(_, error)) = rebuild_profile_status_cache(&state) {
            warn!(profile = %profile.id, error = %error, "failed to refresh profile status after asset reconciliation");
        }
        state.asset_reconcile_inflight.store(false, Ordering::Release);
    });
    Ok(true)
}

pub(super) fn start_startup_ensure(state: Arc<ServiceState>) {
    if let Err(error) = claim_asset_reconcile(&state) {
        warn!(error = %error, "startup asset reconciliation was already running");
        return;
    }
    if let Err(error) = update_asset_reconcile_state(&state, |status| {
        *status = AssetReconcileState {
            in_progress: true,
            ..Default::default()
        };
    }) {
        state.asset_reconcile_inflight.store(false, Ordering::Release);
        warn!(error = %error, "failed to start startup asset reconciliation");
        return;
    }
    tokio::spawn(async move {
        match ensure_assets_after_claim(Arc::clone(&state)).await {
            Ok(downloaded) => info!(downloaded, "startup asset reconciliation finished"),
            Err(error) => warn!(error = %error, "startup asset reconciliation failed"),
        }
        if let Err(AppError(_, error)) = rebuild_profile_status_cache(&state) {
            warn!(error = %error, "failed to refresh profile status after startup asset reconciliation");
        }
        state.asset_reconcile_inflight.store(false, Ordering::Release);
    });
}

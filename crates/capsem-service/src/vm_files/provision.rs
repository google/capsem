use super::*;

pub(crate) async fn handle_provision(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, AppError> {
    let _launch = state
        .lifecycle
        .admit()
        .map_err(|e| AppError(StatusCode::CONFLICT, e.to_string()))?;
    let profile_id = validate_profile_route_id(payload.profile_id.clone())?;
    if let Some(reason) = vm_asset_block_reason(&state, &profile_id) {
        return Err(AppError(StatusCode::PRECONDITION_FAILED, reason));
    }

    let existing = state.off_worker(|state| existing_session_names(&state)).await?;
    let name = payload
        .name
        .clone()
        .unwrap_or_else(|| generate_profile_session_name(&profile_id, existing.iter().map(|s| s.as_str())));
    let persistent = payload.persistent || payload.name.is_some() || payload.from.is_some();
    if existing.iter().any(|existing| existing == &name) {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!("persistent VM \"{}\" already exists", name),
        ));
    }
    let id = new_persistent_vm_id();

    let profile = state
        .cached_profile_config(&profile_id)
        .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;
    let resources = resolve_profile_vm_resources(&profile, payload.ram_mb, payload.cpus);
    let ram_mb = resources.ram_mb;
    let cpus = resources.cpus;
    let scratch_disk_size_gb = resources.scratch_disk_size_gb;

    // Retry budget for the launchd-cleanup transient. Failed attempts
    // fast-fail in ~500ms (capsem-process spawn -> validateWithError
    // crash -> child-exit handler -> instances-map removal observable
    // here), so 8s covers ~5-8 attempts including backoff. Successful
    // attempts return on the first poll iteration regardless of timeout.
    // Backoff lets launchd tick at least one PETRIFIED-cleanup entry
    // (9s wall-clock per entry) between retries; under a real cascade
    // the second attempt usually lands once one entry has drained.
    let opts = capsem_foundation::poll::PollOpts {
        label: "provision-launchd-drain",
        timeout: std::time::Duration::from_secs(8),
        initial_delay: std::time::Duration::from_millis(200),
        max_delay: std::time::Duration::from_millis(500),
    };

    let id_for_loop = id.clone();
    let attempt_num = std::sync::atomic::AtomicU32::new(0);
    let result = capsem_foundation::poll::poll_until(opts, || {
        let state = Arc::clone(&state);
        let id = id_for_loop.clone();
        let name = name.clone();
        let payload_env = payload.env.clone();
        let payload_from = payload.from.clone();
        let payload_profile_id = profile_id.clone();
        let payload_persistent = persistent;
        let attempt = attempt_num.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        async move {
            // Before retry attempts (>1), clear any state the prior
            // failed attempt left behind so provision_sandbox does not
            // reject with "already exists". The child-exit handler has
            // already done its own cleanup (instances.remove +
            // preserve_failed_session_dir) by the time we observe
            // crash-before-ready; we only need to undo registration of
            // the persistent entry.
            if attempt > 1 {
                let stale_name = name.clone();
                let _ = state
                    .off_worker(move |state| {
                        let _ = state.persistent_registry.lock().unwrap().unregister(&stale_name);
                    })
                    .await;
                state.instances.lock().unwrap().remove(&id);
                warn!(id, attempt, "retrying provision after launchd-cleanup transient");
            }

            let outcome = provision_attempt(
                &state,
                &id,
                &name,
                ram_mb,
                cpus,
                scratch_disk_size_gb,
                payload_profile_id,
                payload_persistent,
                payload_env,
                payload_from,
            )
            .await;
            // Log structured context BEFORE losing the outcome to classify_*.
            // BootCrash/ProvisionError still produce a user-facing error
            // body via classify_attempt_decision; these logs are for
            // operators reading service.log.
            if let ProvisionAttemptOutcome::BootCrash { ref tail } = outcome {
                // The tail goes to the caller in the 500 body; without it here
                // service.log records that a boot died but never why, and the
                // reason survives only inside the session's process.log.
                error!(
                    id,
                    cause = capsem_core::session::boot_failure_summary(tail),
                    "capsem-process exited before reaching ready"
                );
            } else if let ProvisionAttemptOutcome::ProvisionError(ref e) = outcome {
                error!(id, error = %e, "provision failed");
            }
            match classify_attempt_decision(outcome, &id) {
                AttemptDecision::Succeed(uds_path) => Some(Ok(uds_path)),
                AttemptDecision::RetryAfterCleanup => None, // poll_until retries
                AttemptDecision::BailWithError(err) => Some(Err(err)),
            }
        }
    })
    .await;

    match result {
        Ok(Ok(uds_path)) => provision_response_for_running(&state, id, uds_path).map(Json),
        Ok(Err(app_err)) => Err(app_err),
        Err(timed_out) => {
            // Exhausted retries on launchd transient. Surface the most
            // recent failed-attempt tail so the user sees what VZ said,
            // even though the actual cause is launchd-side saturation.
            let tail = failed_process_log_tail(&state, &id).await;
            error!(
                id,
                attempts = timed_out.attempts,
                "provision: launchd-cleanup retries exhausted"
            );
            Err(AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "sandbox {id} could not be provisioned after {} attempts ({}). \
                     This typically clears within 10s; please retry. process.log tail:\n\n{tail}\n\n\
                     (full logs: `capsem logs {id}`)",
                    timed_out.attempts, timed_out
                ),
            ))
        }
    }
}

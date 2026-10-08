use super::*;

pub(crate) async fn handle_provision(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, AppError> {
    let _launch = state
        .lifecycle
        .admit()
        .map_err(|e| AppError::new(StatusCode::CONFLICT, e.to_string()))?;
    validate_vm_labels(payload.labels.as_ref()).map_err(|e| AppError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
    let labels = non_empty_labels(payload.labels.clone());
    if let Some(reason) = state.off_worker(|state| vm_asset_block_reason(&state)).await? {
        return Err(AppError::new(StatusCode::PRECONDITION_FAILED, reason));
    }

    let existing = state.off_worker(|state| existing_session_names(&state)).await?;
    let name = payload
        .name
        .clone()
        .unwrap_or_else(|| generate_session_name(existing.iter().map(|s| s.as_str())));
    let persistent = payload.persistent || payload.name.is_some() || payload.from.is_some();
    if existing.iter().any(|existing| existing == &name) {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            format!("persistent VM \"{}\" already exists", name),
        ));
    }
    let id = new_persistent_vm_id();
    let networks = network_routes::resolve_network_names(&*state.networks.lock().await, &payload.networks)?;

    let resources = resolve_vm_resources(payload.ram_mb, payload.cpus);
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
        let payload_env = capsem_core::container::session_env(payload.env.clone(), payload.container.is_some());
        let payload_labels = labels.clone();
        let payload_from = payload.from.clone().map(|source| crate::CloneFrom {
            source,
            replace_image: payload.container.is_some(),
        });
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
                payload_persistent,
                payload_env,
                payload_labels,
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
                error!(id, error = format!("{e:#}"), "provision failed");
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
        Ok(Ok(_)) => finish_create(&state, &id, &networks, payload.container).await.map(Json),
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
            Err(AppError::new(
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

/// Everything `POST /vms/create` does once the VM is registered and running.
/// A create that fails here must leave nothing behind: the caller never
/// learns the VM's id, and a named VM would keep its name. A readiness
/// timeout is the exception -- setup continues under service ownership and
/// the 504 names the VM.
pub(crate) async fn finish_create(
    state: &Arc<ServiceState>,
    id: &str,
    networks: &[uuid::Uuid],
    container: Option<api::ContainerSpec>,
) -> Result<ProvisionResponse, AppError> {
    let created = complete_create(state, id, networks, container).await;
    if matches!(&created, Err(error) if error.status != StatusCode::GATEWAY_TIMEOUT) {
        discard_failed_create(state, id).await;
    }
    created
}

pub(crate) async fn complete_create(
    state: &Arc<ServiceState>,
    id: &str,
    networks: &[uuid::Uuid],
    container: Option<api::ContainerSpec>,
) -> Result<ProvisionResponse, AppError> {
    let response = provision_response_for_running(state, id.to_owned())?;
    let socket = state
        .instances
        .lock()
        .unwrap()
        .get(id)
        .map(|instance| instance.uds_path.clone());
    if let Some(socket) = socket {
        crate::credential_routes::sync_memory(state, &socket)
            .await
            .map_err(|error| AppError::new(StatusCode::SERVICE_UNAVAILABLE, error))?;
    }
    network_routes::attach_provisioned(state, id, networks).await?;
    if container.is_none() {
        // A `--from` clone boots its source's staged image, if it had one.
        container_setup::restore_ready(state, id).await?;
    }
    if let Some(spec) = container {
        container_setup::start(state, id.to_owned(), spec);
        container_setup::require_ready(state, id).await?;
    }
    Ok(response)
}

/// The VM goes and its name is freed, but its ledger and logs are kept as a
/// failed session: they hold the record of why the create failed -- a
/// policy-refused pull's audit row lives in this VM's own ledger. Retain, not
/// Discard: the owner exits cleanly so that ledger is flushed before it moves.
async fn discard_failed_create(state: &Arc<ServiceState>, id: &str) {
    // Resolved first: a teardown that fails -- a ledger too damaged to read --
    // must not also lose the ledger it could not read.
    let session_dir = resolve_session_dir(state, id).ok();
    if let Err(error) = shutdown_vm_process(state, id, ShutdownMode::Retain, None).await {
        error!(vm_id = id, error = %error.body.error, "failed create did not shut down cleanly");
    }
    if let Some(session_dir) = session_dir {
        let (owner, vm_id) = (Arc::clone(state), id.to_owned());
        let kept = tokio::task::spawn_blocking(move || owner.preserve_failed_session_dir(&session_dir, &vm_id)).await;
        if !matches!(kept, Ok(Some(_))) {
            warn!(vm_id = id, "failed create's session could not be kept for post-mortem");
        }
    }
    if let Some(key) = persistent_registry_key_for_route_id(state, id) {
        match state.off_worker(move |state| state.forget_persistent_entry(&key)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => error!(vm_id = id, error = %error, "failed create kept its name"),
            Err(error) => error!(vm_id = id, error = %error.body.error, "failed create kept its name"),
        }
    }
    network_routes::vm_deleted(state, id).await;
}

/// The caller's view of a failed provision: the whole cause chain, since the
/// top context alone ("load the policy inputs") never says why.
pub(crate) fn provision_failure(e: &anyhow::Error) -> AppError {
    let message = format!("{e:#}");
    let status = if message.contains("already exists") {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    AppError::new(status, format!("provision failed: {message}"))
}

#[cfg(test)]
mod tests;

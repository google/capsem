//! One-shot VM execution and its owned cleanup.

use super::*;

/// One-shot exec: provision a temp VM, run a command, return output, destroy VM.
pub(crate) async fn handle_run(
    actor: crate::caller_auth::AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<RunRequest>,
) -> Result<Json<ExecResponse>, AppError> {
    actor.require_operator()?;
    payload
        .validate_environment()
        .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error.into()))?;
    let timeout_secs =
        capsem_api::exec_timeout_secs(payload.timeout_secs).map_err(|e| AppError::new(StatusCode::BAD_REQUEST, e))?;
    if let Some(reference) = &payload.environment {
        crate::environment_launch::validate_command_selection(&state, reference).await?;
    }
    // Once accepted, the same task owns launch, execution and retirement even
    // if the HTTP waiter disappears while a guest is already running.
    tokio::spawn(run_owned(state, payload, timeout_secs))
        .await
        .map_err(|error| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("one-shot execution task: {error}"),
            )
        })?
}

async fn run_owned(
    state: Arc<ServiceState>,
    payload: RunRequest,
    timeout_secs: u64,
) -> Result<Json<ExecResponse>, AppError> {
    let _launch = state
        .lifecycle
        .admit()
        .map_err(|e| AppError::new(StatusCode::CONFLICT, e.to_string()))?;
    if let Some(reason) = state.off_worker(|state| vm_asset_block_reason(&state)).await? {
        return Err(AppError::new(StatusCode::PRECONDITION_FAILED, reason));
    }

    let name = {
        let existing = state.off_worker(|state| existing_session_names(&state)).await?;
        generate_session_name(existing.iter().map(|s| s.as_str()))
    };
    let environment = payload
        .environment
        .map(|reference| capsem_logger::events::EnvironmentRevisionParent {
            environment_id: reference.environment_id,
            revision_id: reference.revision_id,
        });
    let saved = environment.is_some();
    let id = if saved { new_persistent_vm_id() } else { name.clone() };
    let generation = capsem_crypto::uuid_v4();
    let uds_path = state
        .instance_socket_path(&id)
        .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let resources = resolve_vm_resources(payload.ram_mb, payload.cpus);
    let ram_mb = resources.ram_mb;
    let cpus = resources.cpus;
    let scratch_disk_size_gb = resources.scratch_disk_size_gb;

    // 1. Provision ephemeral VM. `provision_sandbox` is synchronous and
    // does heavy I/O (APFS clonefile, rootfs.img fsync, child spawn);
    // offload to the blocking pool, matching `handle_provision` -- the
    // tokio::process::Command::spawn inside still works because
    // spawn_blocking preserves the runtime handle via thread-locals.
    let state_clone = Arc::clone(&state);
    let id_clone = id.clone();
    let version = state.current_version.clone();
    let env = payload.env;
    {
        let _vz_guard = state.lifecycle.vz.read().await;
        let _vz_host_guard = acquire_vz_host_lock(startup::VzHostLockMode::Shared).await?;
        let provision_result = tokio::task::spawn_blocking(move || {
            state_clone.provision_sandbox_generation(
                ProvisionOptions {
                    environment,
                    id: &id_clone,
                    name: &name,
                    ram_mb,
                    cpus,
                    scratch_disk_size_gb,
                    version_override: Some(version),
                    persistent: false,
                    env,
                    labels: None,
                    from: None,
                    description: None,
                },
                generation,
            )
        })
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("provision task: {e}")))?;
        provision_result.map_err(|e| provision_failure(&e))?;
        // The blocking worker has published its child. Executing the user's
        // command is no longer launch admission work for service shutdown.
        drop(_launch);

        // 3. Wait for VM socket to appear while still holding the VZ
        // lifecycle rail. The child does its Apple VZ start/restore before it
        // writes the ready sentinel; releasing earlier reintroduces the
        // sibling-service overlap this lock exists to prevent.
        if let Err(e) = wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id)).await {
            drop(_vz_host_guard);
            drop(_vz_guard);
            // Wait for the child to actually exit before renaming. Rename on
            // an open-for-write dir is safe (fds survive) but any path-based
            // reopens the child might do during shutdown (log rotation, db
            // reopen) would ENOENT -- so we let it finish flushing first.
            // shutdown_vm_process now blocks until exit (5s budget, SIGKILL
            // fallback) and cleans the UDS socket inline. Graceful because
            // preserve_failed_session_dir inspects session logs that capsem-process
            // is still flushing.
            let shutdown_result = shutdown_vm_process(&state, &id, ShutdownMode::Retain, Some(generation)).await?;
            preserve_failed_run_shutdown_result(Arc::clone(&state), id.clone(), shutdown_result).await?;
            return Err(AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e));
        }
    }

    let result = execute(&state, &id, &uds_path, payload.command, timeout_secs, saved).await;

    // 3. Tear down VM process and build response. shutdown_vm_process
    // blocks until the process is actually gone -- the leak detector needs
    // that guarantee. Route handlers must not mine session.db before returning;
    // durable telemetry is recovered by the ledger rails.
    let shutdown_result = shutdown_vm_process(&state, &id, ShutdownMode::Retain, Some(generation)).await?;
    let finalized = finalize_one_shot_session(Arc::clone(&state), id.clone(), shutdown_result, result.is_err()).await;
    if saved {
        crate::environment_launch::release_one_shot_references(&state, &id, generation)
            .await
            .map_err(|error| AppError::new(StatusCode::SERVICE_UNAVAILABLE, error))?;
    }
    finalized?;
    result.map(Json)
}

async fn execute(
    state: &Arc<ServiceState>,
    id: &str,
    socket: &StdPath,
    command: String,
    timeout_secs: u64,
    saved: bool,
) -> Result<ExecResponse, AppError> {
    if saved {
        crate::environment_launch::initialize(state, id)
            .await
            .map_err(|error| AppError::new(StatusCode::SERVICE_UNAVAILABLE, error))?;
    }
    let exec_result = send_ipc_command(
        state,
        socket,
        ServiceToProcess::Exec {
            id: state.next_job_id(),
            command,
            target: capsem_proto::ipc::ExecTarget::Vm,
        },
        Some(timeout_secs),
    )
    .await;
    match exec_result {
        Ok(ProcessToService::ExecResult {
            stdout,
            stderr,
            exit_code,
            truncated,
            ..
        }) => Ok(ExecResponse {
            stdout: ExecOutput::from_bytes(stdout),
            stderr: ExecOutput::from_bytes(stderr),
            exit_code,
            truncated,
        }),
        Ok(_) => Err(AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response".into(),
        )),
        Err(e) => Err(e.into_exec_app_error(true)),
    }
}

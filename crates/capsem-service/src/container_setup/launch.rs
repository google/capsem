//! One generation-owned image admission, staging and execution pipeline.
use super::*;

/// Reuse admitted immutable bytes while sending every freshly prepared control
/// through the ordinary import boundary. Retirement joins blocking publishers.
pub(crate) async fn stage_carried_plan(
    state: &Arc<ServiceState>,
    owner: &Arc<crate::owner_connection::OwnerConnection>,
    image: &str,
    manifest: &str,
    files: Vec<stage::StagedFile>,
) -> Result<(), String> {
    owner.validate(state, false)?;
    if files.is_empty() {
        return Ok(());
    }
    let checked_owner = Arc::clone(owner);
    state
        .off_worker(move |state| {
            checked_owner.validate(&state, false)?;
            if !super::relaunch::needs_relaunch(&state, checked_owner.id()) {
                return Err("running workload controls cannot be replaced".to_string());
            }
            Ok(())
        })
        .await
        .map_err(AppError::into_message)??;
    let generation = {
        let instances = state.instances.lock().unwrap();
        let _instance = instances
            .get(owner.id())
            .filter(|instance| instance.generation == owner.generation())
            .ok_or("carried workload owner changed before staging")?;
        // Admit the setup record while this original owner is still registered.
        let generation = state.containers.begin(owner.id(), image);
        drop(instances);
        generation
    };
    let lease = state
        .containers
        .work_lease(owner.id(), generation)
        .ok_or("carried workload staging cancelled")?;
    tokio::select! {
        () = state.containers.wait_for_predecessors(owner.id(), generation) => {},
        () = lease.cancelled() => return Err("carried workload staging cancelled".into()),
    }
    owner.validate(state, false)?;
    if !state.containers.advance(owner.id(), generation, |status| {
        status.state = ContainerState::Staging;
        status.digest = Some(manifest.to_owned());
        status.resolved = Some(image.to_owned());
    }) {
        return Err("carried workload staging cancelled".into());
    }
    for file in files {
        owner.validate(state, false)?;
        stage_file(state, owner.id(), generation, file).await?;
    }
    owner.validate(state, false)
}

/// Pull, stage and start `spec` in VM `id` in the background.
pub(crate) fn start(state: &Arc<ServiceState>, id: String, spec: ContainerSpec) {
    start_workload(state, id, spec, None);
}

pub(crate) fn start_selected(
    state: &Arc<ServiceState>,
    id: String,
    spec: ContainerSpec,
    selected: crate::environment_launch::SelectedImage,
) {
    start_workload(state, id, spec, Some(selected));
}

fn start_workload(
    state: &Arc<ServiceState>,
    id: String,
    spec: ContainerSpec,
    selected: Option<crate::environment_launch::SelectedImage>,
) {
    let owner = match &selected {
        Some(selected) => Ok(Arc::clone(&selected.owner)),
        None => state
            .instances
            .lock()
            .unwrap()
            .get(&id)
            .ok_or_else(|| format!("sandbox not found: {id}"))
            .and_then(crate::owner_connection::OwnerConnection::capture)
            .map(Arc::new),
    };
    let generation = state.containers.begin(&id, &spec.image);
    let Some(lease) = state.containers.work_lease(&id, generation) else {
        return;
    };
    let task = tokio::spawn({
        let state = Arc::clone(state);
        let id = id.clone();
        async move {
            let _lease = lease;
            let result = match owner {
                Ok(owner) => run(&state, &id, generation, spec, selected, &owner).await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                warn!(vm_id = id.as_str(), error = %error, "container setup failed");
                state.containers.advance(&id, generation, |status| {
                    status.state = ContainerState::Failed;
                    status.error = Some(error);
                });
            }
        }
    });
    state.containers.attach_task(&id, generation, task.abort_handle());
}

pub(super) async fn run(
    state: &Arc<ServiceState>,
    id: &str,
    generation: u64,
    spec: ContainerSpec,
    selected: Option<crate::environment_launch::SelectedImage>,
    owner: &crate::owner_connection::OwnerConnection,
) -> Result<(), String> {
    state.containers.wait_for_predecessors(id, generation).await;
    if !state.containers.advance(id, generation, |_| {}) {
        return Err("container setup was cancelled".into());
    }
    owner.validate(state, false)?;
    let started = std::time::Instant::now();
    // One resolver for every entrypoint: a catalog name becomes the pinned
    // digest it names here, and is an explicit reference from then on.
    let mut decision = images::Decision::new(state).await?;
    let requested = match &selected {
        Some(selected) => {
            let requested = decision.resolve_saved(&selected.image).await?;
            decision.admit_saved(&requested).await?;
            owner.validate(state, false)?;
            requested
        }
        None => decision.resolve(&spec.image).await?,
    };
    // Sources are checked before any registry access: a refused registry is
    // never contacted.
    decision.check_source(&requested).await?;
    let admission = ServiceToProcess::AdmitContainerPull {
        id: state.next_job_id(),
        image: requested.pull.clone(),
        registry: requested.registry.clone(),
        digest: requested.identity.digest().map(ToString::to_string),
    };
    let (uds_path, vm_generation) = (owner.uds_path.clone(), owner.generation());
    let vm = capsem_core::managed_sessions::VmBinding::new(id.to_owned(), vm_generation)
        .map_err(|error| error.to_string())?;
    // Admission and staging are host work. The launched owner has its
    // security context and ledger; guest readiness is needed only to exec.
    use crate::vm_files::launch::{wait_for_launch, LaunchWait};
    match wait_for_launch(
        &uds_path.with_extension("ready"),
        &uds_path.with_extension("launched"),
        || state.instances.lock().unwrap().contains_key(id),
        std::time::Duration::from_secs(30),
    )
    .await
    {
        LaunchWait::Ready | LaunchWait::Launched => {}
        LaunchWait::Crashed => return Err("container owner exited before launch".into()),
        LaunchWait::TimedOut => return Err("container owner did not launch within 30s".into()),
    }
    match crate::vm_files::send_owner_command(state, owner, admission, Some(5)).await? {
        ProcessToService::ContainerPullAdmission { error: None, .. } => {}
        ProcessToService::ContainerPullAdmission {
            error: Some(error),
            policy_refused,
            ..
        } => {
            let kind = if policy_refused {
                "policy refused"
            } else {
                "security admission failed"
            };
            return Err(format!("container pull {kind}: {error}"));
        }
        other => return Err(format!("unexpected container pull admission reply: {other:?}")),
    }
    debug!(
        vm_id = id,
        stage = "owner_admission",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "container startup stage"
    );
    let started = std::time::Instant::now();
    let access = decision
        .registry_access(&requested, spec.registry.unwrap_or_default())
        .await?;
    let image = state
        .containers
        .source
        .pull(
            requested.pull.clone(),
            access.clone(),
            decision.parent.clone(),
            ImageFetch::PreferCached.into(),
        )
        .await
        .map_err(|e| format!("pull {}: {e:#}", requested.pull))?;
    debug!(
        vm_id = id,
        stage = "image_pull",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "container startup stage"
    );
    let started = std::time::Instant::now();
    // Admission is on the resolved digest, before anything is staged.
    let resolved = decision.admit(&requested, &image).await?;
    if !state.containers.set_app(id, generation, decision.app(&resolved).await) {
        return Err("container setup was cancelled".into());
    }
    let rootfs = decision
        .published_root(&requested, &image, &resolved, access, ImageFetch::PreferCached.into())
        .await?;
    debug!(
        vm_id = id,
        stage = "published_root",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "container startup stage"
    );
    let started = std::time::Instant::now();
    // An image whose surface labels are not exactly one valid declaration is
    // refused here, before staging: nothing of it runs and nothing is exposed.
    let declared = tokio::task::spawn_blocking({
        let root = image.root.clone();
        move || stage::image_surface(&root)
    })
    .await
    .map_err(|e| format!("read image surface: {e}"))?
    .map_err(|e| format!("container image refused: {e:#}"))?;
    if !state.containers.advance(id, generation, |status| {
        status.state = ContainerState::Staging;
        status.digest = Some(image.digest.clone());
        status.surface = api_surface(declared);
        status.resolved = Some(resolved.to_string());
    }) {
        return Ok(());
    }

    let attach = spec.attach;
    let (ram_mb, cpus) = state
        .instances
        .lock()
        .unwrap()
        .get(id)
        .map(|vm| (vm.ram_mb, vm.cpus))
        .ok_or_else(|| format!("VM {id} stopped before its container was staged"))?;
    let resources = capsem_core::container::workload_resources(ram_mb, cpus).map_err(|e| format!("{e:#}"))?;
    let (manifest, rootfs) = share_image(state, id, generation, &image, rootfs).await?;
    let cache_owner = image.cache_key.clone().map(|key| cache_owners::CacheOwner { key, vm });
    if !state
        .containers
        .pin_image(id, generation, Some(manifest.clone()), cache_owner)
    {
        return Ok(());
    }
    let environment = selected
        .as_ref()
        .map(|selected| selected.context())
        .transpose()
        .map_err(|error| format!("invalid environment launch context: {error:#}"))?;
    owner.validate(state, false)?;
    let staged_image = stage::StageImage {
        manifest: &manifest,
        rootfs: rootfs.as_ref().map(|(digest, integrity, size)| stage::RootfsFile {
            digest,
            integrity,
            size: *size,
        }),
        environment: environment.as_ref(),
        startup: selected.as_ref().and_then(|selected| selected.startup.as_ref()),
        repositories: selected
            .as_ref()
            .map_or(&[], |selected| selected.repositories.as_slice()),
    };
    let plan = stage::stage_plan(staged_image, &spec.args, &spec.env, resources, declared)
        .map_err(|e| format!("plan stage: {e:#}"))?;
    for file in plan {
        if !state.containers.advance(id, generation, |_| {}) {
            return Ok(());
        }
        stage_file(state, id, generation, file).await?;
    }
    debug!(
        vm_id = id,
        stage = "image_stage",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "container startup stage"
    );
    let started = std::time::Instant::now();

    wait_for_vm_ready(&uds_path, 30, Some(state), Some(id))
        .await
        .map_err(|error| format!("container guest did not become ready: {error}"))?;
    debug!(
        vm_id = id,
        stage = "guest_ready_wait",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "container startup stage"
    );
    if attach {
        // A `container` stream starts it; the launch record is written then.
        state
            .containers
            .advance(id, generation, |status| status.state = ContainerState::Staged);
        return Ok(());
    }
    if !state
        .containers
        .advance(id, generation, |status| status.state = ContainerState::Starting)
    {
        return Ok(());
    }
    let preparation = crate::environment_launch::preparation_policy(state, owner).await?;
    let started = std::time::Instant::now();
    let reply = crate::vm_files::send_owner_command(
        state,
        owner,
        ServiceToProcess::Exec {
            id: state.next_job_id(),
            command: capsem_core::container::detached_launch_command(),
            target: capsem_proto::ipc::ExecTarget::Vm,
        },
        Some(30),
    )
    .await?;
    drop(preparation);
    match reply {
        ProcessToService::ExecResult { exit_code: 0, .. } => record_launched(state, id)?,
        ProcessToService::ExecResult { exit_code, .. } => return Err(format!("container launcher exited {exit_code}")),
        other => return Err(format!("unexpected launch reply: {other:?}")),
    }
    debug!(
        vm_id = id,
        stage = "launcher_dispatch",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "container startup stage"
    );
    let started = std::time::Instant::now();
    grant_surface(state, id, generation).await;
    debug!(
        vm_id = id,
        stage = "workload_ready",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "container startup stage"
    );
    Ok(())
}

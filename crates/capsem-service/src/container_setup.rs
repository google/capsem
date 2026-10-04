//! Service-owned container workloads: pull an OCI image on the host, stage it
//! into the VM's workspace and start the guest launcher.
//!
//! The service coordinates; it never forwards workload bytes. Registry access
//! lives only in the setup task and is dropped with it.

use super::*;
use capsem_api::{
    ContainerSpec, ContainerState, ContainerStatusResponse, ContainerSurface, ContainerSurfaceKind, RegistryAccess,
};
use capsem_core::container::stage::{self, StagedContent};
use capsem_foundation::unix::contained::{ContainedOpenOptions, EntryKind};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;

const CREATE_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(110);

/// A verified image layout on the host, kept alive while it is staged.
pub(crate) struct PulledImage {
    pub(crate) root: PathBuf,
    pub(crate) files: Vec<PathBuf>,
    pub(crate) digest: String,
    /// What the reference resolved to (the index, when there was one).
    pub(crate) image_digest: String,
    pub(crate) _hold: Box<dyn Send + Sync>,
}

pub(crate) type PullFuture = Pin<Box<dyn Future<Output = anyhow::Result<PulledImage>> + Send>>;

/// Where images come from, and which may be fetched and run. Production
/// pulls from registries under the installation's policy; tests substitute.
pub(crate) trait ImageSource: Send + Sync {
    fn pull(&self, image: String, access: RegistryAccess, parent: PathBuf) -> PullFuture;

    /// The image policy for one decision, read fresh each time.
    fn policy(&self) -> PolicyFuture {
        Box::pin(async {
            tokio::task::spawn_blocking(|| {
                let (settings, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
                capsem_core::container::admission::ImagePolicy::from_files(&settings, &corp, Vec::new())
            })
            .await?
        })
    }
}

pub(crate) type PolicyFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<capsem_core::container::admission::ImagePolicy>> + Send>>;

pub(crate) struct RegistryImages;

impl ImageSource for RegistryImages {
    fn pull(&self, image: String, access: RegistryAccess, parent: PathBuf) -> PullFuture {
        Box::pin(async move {
            capsem_assets::oci::image_reference(&image)
                .context("container image expects docker://IMAGE or registry/repository:tag")?;
            let authentication = match (access.username, access.password) {
                (Some(username), Some(password)) => capsem_assets::oci::RegistryAuth::Basic(username, password),
                (None, None) => capsem_assets::oci::RegistryAuth::Anonymous,
                _ => anyhow::bail!("registry access needs both username and password"),
            };
            let puller = capsem_assets::oci::Puller::new_with_root_certificate(
                stage::oci_architecture()?,
                authentication,
                access.ca_pem.as_deref().map(str::as_bytes),
            )?;
            let layout = puller.pull(&image, &parent).await?;
            Ok(PulledImage {
                root: layout.path().to_path_buf(),
                files: layout.files().to_vec(),
                digest: layout.source_digest.clone(),
                image_digest: layout.image_digest.clone(),
                _hold: Box::new(layout),
            })
        })
    }
}

struct ContainerRecord {
    generation: u64,
    status: ContainerStatusResponse,
    task: Option<tokio::task::AbortHandle>,
}

/// Every VM's container workload, keyed by VM id.
pub(crate) struct ContainerSetups {
    records: Mutex<HashMap<String, ContainerRecord>>,
    generation: AtomicU64,
    source: Box<dyn ImageSource>,
}

impl Default for ContainerSetups {
    fn default() -> Self {
        Self::with_source(Box::new(RegistryImages))
    }
}

impl ContainerSetups {
    pub(crate) fn with_source(source: Box<dyn ImageSource>) -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            source,
        }
    }

    pub(crate) fn status(&self, id: &str) -> Option<ContainerStatusResponse> {
        self.records.lock().unwrap().get(id).map(|record| record.status.clone())
    }

    /// Stop an in-flight setup and forget the VM's workload. A setup that
    /// finishes afterwards finds its generation gone and changes nothing.
    pub(crate) fn cancel(&self, id: &str) {
        if let Some(task) = self.records.lock().unwrap().remove(id).and_then(|record| record.task) {
            task.abort();
        }
    }

    fn begin(&self, id: &str, image: &str) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let replaced = self.records.lock().unwrap().insert(
            id.to_owned(),
            ContainerRecord {
                generation,
                status: ContainerStatusResponse {
                    state: ContainerState::Pulling,
                    image: image.to_owned(),
                    digest: None,
                    exit_code: None,
                    error: None,
                    surface: None,
                },
                task: None,
            },
        );
        if let Some(task) = replaced.and_then(|record| record.task) {
            task.abort();
        }
        generation
    }

    /// Apply `update` if this generation still owns the VM's record.
    fn advance(&self, id: &str, generation: u64, update: impl FnOnce(&mut ContainerStatusResponse)) -> bool {
        let mut records = self.records.lock().unwrap();
        match records.get_mut(id) {
            Some(record) if record.generation == generation => {
                update(&mut record.status);
                true
            }
            _ => false,
        }
    }

    /// Claim a staged workload for an attached start. Exactly one stream wins;
    /// the claim is the move from `staged` to `starting`.
    pub(crate) fn claim_attach(&self, id: &str) -> Result<u64, String> {
        let mut records = self.records.lock().unwrap();
        match records.get_mut(id) {
            Some(record) if record.status.state == ContainerState::Staged => {
                record.status.state = ContainerState::Starting;
                Ok(record.generation)
            }
            Some(record) => Err(format!(
                "container workload is {}, not staged for attach",
                serde_json::to_value(record.status.state)
                    .ok()
                    .and_then(|state| state.as_str().map(str::to_owned))
                    .unwrap_or_default()
            )),
            None => Err("VM has no container workload".into()),
        }
    }

    /// A workload already staged for attach, for stream tests.
    #[cfg(test)]
    pub(crate) fn stage_for_tests(&self, id: &str, image: &str) -> u64 {
        let generation = self.begin(id, image);
        self.advance(id, generation, |status| status.state = ContainerState::Staged);
        generation
    }

    /// Record how an attached workload ended.
    pub(crate) fn finish_attach(&self, id: &str, generation: u64, outcome: Result<i32, String>) {
        self.advance(id, generation, |status| match outcome {
            Ok(code) => {
                status.state = ContainerState::Exited;
                status.exit_code = Some(code);
            }
            Err(error) => {
                status.state = ContainerState::Failed;
                status.error = Some(error);
            }
        });
    }

    fn attach_task(&self, id: &str, generation: u64, task: tokio::task::AbortHandle) {
        let mut records = self.records.lock().unwrap();
        match records.get_mut(id) {
            Some(record) if record.generation == generation => record.task = Some(task),
            // Cancelled before the handle arrived: nothing may keep running.
            _ => task.abort(),
        }
    }
}

/// Admit a pulled image by either of its content addresses: what the
/// reference resolved to, or the platform manifest it selected.
fn admit(
    policy: &capsem_core::container::admission::ImagePolicy,
    requested: &capsem_assets::oci::ImageReference,
    image: &PulledImage,
) -> Result<(), String> {
    let mut refusal = None;
    for digest in [&image.image_digest, &image.digest] {
        let resolved = capsem_assets::oci::Digest::parse(digest)
            .and_then(|digest| requested.clone().resolve(digest))
            .and_then(|resolved| policy.admit(&resolved));
        match resolved {
            Ok(()) => return Ok(()),
            Err(error) => refusal = Some(format!("container image refused: {error:#}")),
        }
    }
    Err(refusal.unwrap_or_else(|| "container image refused".into()))
}

/// Pull, stage and start `spec` in VM `id` in the background.
pub(crate) fn start(state: &Arc<ServiceState>, id: String, spec: ContainerSpec) {
    let generation = state.containers.begin(&id, &spec.image);
    let task = tokio::spawn({
        let state = Arc::clone(state);
        let id = id.clone();
        async move {
            if let Err(error) = run(&state, &id, generation, spec).await {
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

async fn run(state: &Arc<ServiceState>, id: &str, generation: u64, spec: ContainerSpec) -> Result<(), String> {
    let reference = capsem_assets::oci::image_reference(&spec.image)
        .map_err(|error| format!("container image expects docker://IMAGE or registry/repository:tag: {error:#}"))?;
    // Sources are checked before any registry access: a refused registry is
    // never contacted.
    let requested = capsem_assets::oci::ImageReference::try_from(&reference).map_err(|e| format!("{e:#}"))?;
    let policy = state
        .containers
        .source
        .policy()
        .await
        .map_err(|e| format!("image policy: {e:#}"))?;
    policy.check_source(&requested).map_err(|e| format!("{e:#}"))?;
    let admission = ServiceToProcess::AdmitContainerPull {
        id: state.next_job_id(),
        image: spec.image.clone(),
        registry: reference.resolve_registry().to_owned(),
        digest: reference.digest().map(str::to_owned),
    };
    let uds_path = running_uds_path(state, id).map_err(|error| error.1)?;
    wait_for_vm_ready(&uds_path, 30, Some(state), Some(id))
        .await
        .map_err(|error| format!("container owner did not become ready: {error}"))?;
    match send_ipc_command(&uds_path, admission, Some(5)).await? {
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
    let parent = state.run_dir.join("container-pulls");
    tokio::task::spawn_blocking({
        let parent = parent.clone();
        move || capsem_foundation::unix::fs::ensure_private_dir(&parent)
    })
    .await
    .map_err(|e| format!("prepare pull directory: {e}"))?
    .map_err(|e| format!("prepare pull directory: {e}"))?;
    let image = state
        .containers
        .source
        .pull(spec.image.clone(), spec.registry.unwrap_or_default(), parent)
        .await
        .map_err(|e| format!("pull {}: {e:#}", spec.image))?;
    // Admission is on the resolved digest, before anything is staged.
    admit(&policy, &requested, &image)?;
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
    let plan = tokio::task::spawn_blocking({
        let (root, files) = (image.root.clone(), image.files.clone());
        move || stage::stage_plan(&root, &files, &spec.args, &spec.env, resources, declared)
    })
    .await
    .map_err(|e| format!("plan stage: {e}"))?
    .map_err(|e| format!("plan stage: {e:#}"))?;
    for file in plan {
        if !state.containers.advance(id, generation, |_| {}) {
            return Ok(());
        }
        stage_file(state, id, file).await?;
    }

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
    let uds_path = running_uds_path(state, id).map_err(|e| e.1)?;
    let reply = send_ipc_command(
        &uds_path,
        ServiceToProcess::Exec {
            id: state.next_job_id(),
            command: capsem_core::container::detached_launch_command(),
        },
        Some(30),
    )
    .await?;
    match reply {
        ProcessToService::ExecResult { exit_code: 0, .. } => record_launched(state, id)?,
        ProcessToService::ExecResult { exit_code, .. } => return Err(format!("container launcher exited {exit_code}")),
        other => return Err(format!("unexpected launch reply: {other:?}")),
    }
    grant_surface(state, id, generation).await;
    Ok(())
}

/// How long a launched GUI workload may take to report itself running before
/// its surface is given up on. Unpacking a large desktop image dominates.
const SURFACE_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

fn api_surface(declared: stage::DeclaredSurface) -> Option<ContainerSurface> {
    match declared {
        stage::DeclaredSurface::Terminal => None,
        stage::DeclaredSurface::Xpra { port } => Some(ContainerSurface {
            kind: ContainerSurfaceKind::Xpra,
            port,
            exposure_id: None,
        }),
    }
}

/// Once the workload runs, grant the surface its image declared: one
/// `http_preview` exposure of the declared container loopback port, made
/// through the ordinary exposure path so the VM's security engine admits it
/// like any other. It never binds a host listener; the gateway's
/// authenticated preview origin is the only way in. A refusal leaves the
/// workload running without a surface.
pub(crate) async fn grant_surface(state: &Arc<ServiceState>, id: &str, generation: u64) {
    let Some(surface) = state.containers.status(id).and_then(|status| status.surface) else {
        return;
    };
    if surface.exposure_id.is_some() {
        return;
    }
    let options = capsem_foundation::poll::PollOpts::new("container-surface-running", SURFACE_READY_TIMEOUT);
    match wait_observed(state, id, options).await {
        Ok(status) if status.state == ContainerState::Running => {}
        Ok(status) => {
            info!(vm_id = id, state = ?status.state, "container ended before its surface was granted");
            return;
        }
        Err(timed_out) => {
            warn!(
                vm_id = id,
                attempts = timed_out.attempts,
                "container never reported running; surface not granted"
            );
            return;
        }
    }
    let request = capsem_api::ExposureRequest {
        guest_port: surface.port,
        host_port: 0,
        target: capsem_api::ExposureTarget::Container,
        access: capsem_api::ExposureAccess::HttpPreview,
    };
    let exposure = match crate::router_runtime::exposures::create_exposure(state, id, request).await {
        Ok(exposure) => exposure,
        Err(AppError(status, error)) => {
            warn!(vm_id = id, %status, %error, "container surface exposure refused");
            return;
        }
    };
    let recorded = state.containers.advance(id, generation, |status| {
        if let Some(surface) = status.surface.as_mut() {
            surface.exposure_id = Some(exposure.id.clone());
        }
    });
    if !recorded {
        return;
    }
    info!(
        vm_id = id,
        exposure_id = exposure.id.as_str(),
        port = surface.port,
        "container surface granted"
    );
    if let Err(error) = record_launched(state, id) {
        warn!(vm_id = id, %error, "container surface not recorded for a service restart");
    }
}

/// Grant the surface of a workload an attached stream just started, beside
/// the stream that runs it.
pub(crate) fn grant_surface_in_background(state: &Arc<ServiceState>, id: &str, generation: u64) {
    if state
        .containers
        .status(id)
        .is_none_or(|status| status.surface.is_none())
    {
        return;
    }
    let (state, id) = (Arc::clone(state), id.to_owned());
    tokio::spawn(async move { grant_surface(&state, &id, generation).await });
}

/// Host-side record of a launched workload, next to the session ledger and
/// outside the guest share, so status survives a service restart.
const LAUNCH_RECORD: &str = "container.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct LaunchRecord {
    image: String,
    digest: String,
    /// The declared surface and, once granted, its exposure: the exposure
    /// lives in the VM owner, which outlives a service restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    surface: Option<ContainerSurface>,
}

/// Wait for the workload `POST /vms/create` started to settle. A detached
/// launcher runs in the background, so readiness is read the way the status
/// route reads it -- through the guest's ready marker -- not only from the
/// live record, which stays `starting` for a detached workload. The shared
/// poll primitive provides the deadline and backoff; callers never retry the
/// mutation that created the VM.
pub(crate) async fn wait_for_create(
    state: &Arc<ServiceState>,
    id: &str,
) -> Result<ContainerStatusResponse, capsem_proto::poll::TimedOut> {
    wait_observed(
        state,
        id,
        capsem_foundation::poll::PollOpts::new("container-create-ready", CREATE_READY_TIMEOUT),
    )
    .await
}

async fn wait_observed(
    state: &Arc<ServiceState>,
    id: &str,
    options: capsem_foundation::poll::PollOpts,
) -> Result<ContainerStatusResponse, capsem_proto::poll::TimedOut> {
    capsem_foundation::poll::poll_until(options, || async {
        let live = state.containers.status(id);
        let id = id.to_owned();
        let observed = state.off_worker(move |state| observe(&state, &id, live)).await;
        let Ok(Ok(Some(status))) = observed else {
            return None;
        };
        (!matches!(
            status.state,
            ContainerState::Pulling | ContainerState::Staging | ContainerState::Starting
        ))
        .then_some(status)
    })
    .await
}

pub(crate) fn record_launched(state: &ServiceState, id: &str) -> Result<(), String> {
    let session_dir = resolve_session_dir(state, id).map_err(|e| e.1)?;
    let Some(status) = state.containers.status(id) else {
        return Ok(());
    };
    let record = serde_json::to_vec(&LaunchRecord {
        image: status.image,
        digest: status.digest.unwrap_or_default(),
        surface: status.surface,
    })
    .map_err(|e| format!("encode launch record: {e}"))?;
    capsem_foundation::unix::fs::atomic_write_private(&session_dir.join(LAUNCH_RECORD), &record)
        .map_err(|e| format!("write launch record: {e}"))
}

/// GET /vms/{id}/container -- the VM's container workload status.
///
/// Whether VM `id` runs a container workload, live or recorded: the files API
/// resolves absolute paths against what the container sees.
pub(crate) async fn runs_container(state: &Arc<ServiceState>, id: &str) -> bool {
    let live = state.containers.status(id);
    let id = id.to_owned();
    matches!(
        state.off_worker(move |state| observe(&state, &id, live)).await,
        Ok(Ok(Some(_)))
    )
}

/// Resolve a files-API path against what VM `id` shows its guest.
pub(crate) async fn guest_file(
    state: &Arc<ServiceState>,
    id: &str,
    query: &capsem_service::fs_utils::FileContentQuery,
) -> Result<capsem_service::fs_utils::FilePath, AppError> {
    capsem_service::fs_utils::resolve_file_path(&query.path, query.exact, runs_container(state, id).await)
}

/// `running` is guest-reported: the launcher writes its ready marker into the
/// shared workspace once the image is unpacked and the workload started.
pub(crate) async fn handle_container_status(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<ContainerStatusResponse>, AppError> {
    let live = state.containers.status(&id);
    state
        .off_worker(move |state| observe(&state, &id, live))
        .await??
        .map(Json)
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, "VM has no container workload".into()))
}

fn observe(
    state: &ServiceState,
    id: &str,
    live: Option<ContainerStatusResponse>,
) -> Result<Option<ContainerStatusResponse>, AppError> {
    let session_dir = resolve_session_dir(state, id)?;
    let Some(mut status) = live.or_else(|| {
        let record = std::fs::read(session_dir.join(LAUNCH_RECORD)).ok()?;
        let record: LaunchRecord = serde_json::from_slice(&record).ok()?;
        Some(ContainerStatusResponse {
            state: ContainerState::Starting,
            image: record.image,
            digest: Some(record.digest),
            exit_code: None,
            error: None,
            surface: record.surface,
        })
    }) else {
        return Ok(None);
    };
    if status.state == ContainerState::Starting {
        let ready = format!("{}/ready", capsem_core::container::STAGE);
        let launched = resolve_workspace_target(state, id, &ready, false)
            .and_then(|(parent, name)| parent.entry_kind(&name).map_err(workspace_io_error))
            .is_ok_and(|kind| kind == Some(EntryKind::File));
        if launched {
            status.state = ContainerState::Running;
        }
    }
    Ok(Some(status))
}

/// Write one staged file into the VM's workspace through the same contained,
/// no-follow writer and import ledger a file upload uses.
async fn stage_file(state: &Arc<ServiceState>, id: &str, file: stage::StagedFile) -> Result<(), String> {
    let path = format!("{}/{}", capsem_core::container::STAGE, file.name);
    let (parent, name) = resolve_workspace_target(state, id, &path, true).map_err(|e| e.1)?;
    let (preview, size) = match &file.content {
        StagedContent::Bytes(bytes) => (file_security_preview_bytes(bytes), bytes.len() as u64),
        StagedContent::File(source) => {
            let source = source.clone();
            tokio::task::spawn_blocking(move || -> std::io::Result<(Vec<u8>, u64)> {
                use std::io::Read;
                let input = std::fs::File::open(&source)?;
                let size = input.metadata()?.len();
                let mut preview = Vec::new();
                input
                    .take(FILE_SECURITY_CONTENT_PREVIEW_MAX as u64)
                    .read_to_end(&mut preview)?;
                Ok((preview, size))
            })
            .await
            .map_err(|e| format!("stage {}: {e}", file.name))?
            .map_err(|e| format!("stage {}: {e}", file.name))?
        }
    };
    if log_file_boundary(state, id, FileBoundaryAction::Import, path, preview, size, None)
        .await
        .map_err(|e| e.1)?
        .is_some()
    {
        return Err(format!("file import policy rewrote staged image file {}", file.name));
    }
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        use std::io::Write;
        let mut output = parent
            .open_file(&name, ContainedOpenOptions::write_create_truncate(0o644))
            .map_err(|e| format!("open staged {}: {e}", file.name))?;
        match file.content {
            StagedContent::Bytes(bytes) => output.write_all(&bytes),
            StagedContent::File(source) => {
                std::fs::File::open(source).and_then(|mut input| std::io::copy(&mut input, &mut output).map(drop))
            }
        }
        .map_err(|e| format!("write staged {}: {e}", file.name))
    })
    .await
    .map_err(|e| format!("stage task: {e}"))?
}

#[cfg(test)]
mod tests;

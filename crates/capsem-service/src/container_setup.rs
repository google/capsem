//! Service-owned container workloads: pull an OCI image on the host, stage it
//! into the VM's workspace and start the guest launcher.
//!
//! The service coordinates; it never forwards workload bytes. Registry access
//! lives only in the setup task and is dropped with it.

use super::*;
use capsem_api::{ContainerSpec, ContainerState, ContainerStatusResponse, RegistryAccess};
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
    pub(crate) _hold: Box<dyn Send + Sync>,
}

pub(crate) type PullFuture = Pin<Box<dyn Future<Output = anyhow::Result<PulledImage>> + Send>>;

/// Where images come from. Production pulls from registries; tests substitute.
pub(crate) trait ImageSource: Send + Sync {
    fn pull(&self, image: String, access: RegistryAccess, parent: PathBuf) -> PullFuture;
}

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
    if !state.containers.advance(id, generation, |status| {
        status.state = ContainerState::Staging;
        status.digest = Some(image.digest.clone());
    }) {
        return Ok(());
    }

    let attach = spec.attach;
    let plan = tokio::task::spawn_blocking({
        let (root, files) = (image.root.clone(), image.files.clone());
        move || stage::stage_plan(&root, &files, &spec.args, &spec.env)
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
        ProcessToService::ExecResult { exit_code: 0, .. } => record_launched(state, id),
        ProcessToService::ExecResult { exit_code, .. } => Err(format!("container launcher exited {exit_code}")),
        other => Err(format!("unexpected launch reply: {other:?}")),
    }
}

/// Host-side record of a launched workload, next to the session ledger and
/// outside the guest share, so status survives a service restart.
const LAUNCH_RECORD: &str = "container.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct LaunchRecord {
    image: String,
    digest: String,
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
    })
    .map_err(|e| format!("encode launch record: {e}"))?;
    capsem_foundation::unix::fs::atomic_write_private(&session_dir.join(LAUNCH_RECORD), &record)
        .map_err(|e| format!("write launch record: {e}"))
}

/// GET /vms/{id}/container -- the VM's container workload status.
///
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

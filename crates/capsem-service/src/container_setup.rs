//! Service-owned container workloads: pull an OCI image on the host, stage it
//! into the VM's workspace and start the guest launcher.
//!
//! The service coordinates; it never forwards workload bytes. Registry access
//! lives only in the setup task and is dropped with it.

use super::*;
use capsem_api::{
    ContainerSpec, ContainerState, ContainerStatusResponse, ContainerSurface, ContainerSurfaceKind, RegistryAccess,
};
use capsem_core::container::admission::CatalogSource;
use capsem_core::container::stage;
use capsem_foundation::unix::contained::{ContainedOpenOptions, EntryKind};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;

mod activity;
mod cache_owners;
mod registry;
use registry::RegistryImages;
pub(crate) mod images;
mod relaunch;
#[cfg(test)]
pub(crate) use relaunch::restore;
pub(crate) use relaunch::{carry_launch_record, drop_carried_image, forget_previous_run, restore_ready};

const CREATE_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(110);

/// A verified image layout on the host, kept alive while it is staged.
pub(crate) struct PulledImage {
    pub(crate) root: PathBuf,
    pub(crate) files: Vec<PathBuf>,
    pub(crate) digest: String,
    /// What the reference resolved to (the index, when there was one).
    pub(crate) image_digest: String,
    pub(crate) cache_key: Option<capsem_assets::oci::CacheKey>,
    pub(crate) _hold: Box<dyn Send + Sync>,
}

pub(crate) type PullFuture = Pin<Box<dyn Future<Output = anyhow::Result<PulledImage>> + Send>>;
pub(crate) type RootfsFuture = Pin<Box<dyn Future<Output = anyhow::Result<capsem_assets::oci::RootfsLayout>> + Send>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageFetch {
    Fresh,
    PreferCached,
}

/// Where images come from, and which may be fetched and run. Production
/// pulls from registries under the installation's policy; tests substitute.
pub(crate) trait ImageSource: Send + Sync {
    /// Read the local owner's cheap snapshot without fetching or verifying.
    fn cache_state(
        &self,
        _key: &capsem_assets::oci::CacheKey,
    ) -> anyhow::Result<Option<capsem_assets::oci::CacheState>> {
        Ok(None)
    }

    /// Schedule local owner proof; polling never waits for hashes or registry.
    fn observe_cache(&self, _keys: &[capsem_assets::oci::CacheKey], _parent: &StdPath) -> anyhow::Result<()> {
        Ok(())
    }

    fn pull(&self, image: String, access: RegistryAccess, parent: PathBuf, mode: ImageFetch) -> PullFuture;

    /// Read the catalog `source` names, anonymously: catalogs are public.
    fn fetch_catalog(&self, source: CatalogSource, parent: PathBuf) -> images::CatalogFuture;

    /// Fetch a filesystem bound to an already admitted platform image.
    fn fetch_rootfs(
        &self,
        _reference: String,
        _subject: String,
        _access: RegistryAccess,
        _parent: PathBuf,
        _mode: ImageFetch,
        _cache_key: Option<capsem_assets::oci::CacheKey>,
    ) -> RootfsFuture {
        Box::pin(async { anyhow::bail!("image source does not provide published filesystems") })
    }

    /// The image policy for one decision, read fresh each time.
    fn policy(&self) -> PolicyFuture {
        Box::pin(async {
            tokio::task::spawn_blocking(|| {
                let (settings, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
                capsem_core::container::admission::ImagePolicy::from_files(&settings, &corp)
            })
            .await?
        })
    }
}

pub(crate) type PolicyFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<capsem_core::container::admission::ImagePolicy>> + Send>>;

struct ContainerRecord {
    generation: u64,
    status: ContainerStatusResponse,
    /// The manifest digest the image share holds the image under, once
    /// published: what the launch record pins for a relaunch.
    manifest: Option<String>,
    cache_owner: Option<cache_owners::CacheOwner>,
    task: Option<tokio::task::AbortHandle>,
    activity: activity::Activity,
}

/// Every VM's container workload, keyed by VM id.
pub(crate) struct ContainerSetups {
    records: Mutex<HashMap<String, ContainerRecord>>,
    generation: AtomicU64,
    source: Box<dyn ImageSource>,
    /// The last good image catalog, shared by every image request.
    catalog: images::CatalogCache,
    retiring: activity::Retiring,
}

impl Default for ContainerSetups {
    fn default() -> Self {
        Self::with_source(Box::new(RegistryImages::default()))
    }
}

impl ContainerSetups {
    pub(crate) fn with_source(source: Box<dyn ImageSource>) -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            source,
            catalog: Default::default(),
            retiring: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn status(&self, id: &str) -> Option<ContainerStatusResponse> {
        self.records.lock().unwrap().get(id).map(|record| record.status.clone())
    }

    /// Stop an in-flight setup and forget the VM's workload. A setup that
    /// finishes afterwards finds its generation gone and changes nothing.
    pub(crate) fn cancel(&self, id: &str) {
        let mut records = self.records.lock().unwrap();
        if let Some(record) = records.remove(id) {
            record.activity.retire();
            drop(records);
            if let Some(task) = record.task {
                task.abort();
            }
        }
    }

    pub(crate) async fn cancel_and_wait(&self, id: &str) {
        self.cancel(id);
        let retiring = self.retiring.lock().unwrap().get(id).cloned().unwrap_or_default();
        for activity in retiring {
            activity.wait().await;
        }
    }

    fn work_lease(&self, id: &str, generation: u64) -> Option<activity::Lease> {
        let records = self.records.lock().unwrap();
        records
            .get(id)
            .filter(|record| record.generation == generation)
            .map(|record| record.activity.enter())
    }

    fn begin(&self, id: &str, image: &str) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let mut records = self.records.lock().unwrap();
        let replaced = records.insert(
            id.to_owned(),
            ContainerRecord {
                generation,
                status: ContainerStatusResponse {
                    state: ContainerState::Pulling,
                    image: image.to_owned(),
                    digest: None,
                    resolved: None,
                    exit_code: None,
                    error: None,
                    surface: None,
                },
                manifest: None,
                cache_owner: None,
                task: None,
                activity: activity::Activity::new(id, generation, &self.retiring),
            },
        );
        if let Some(record) = replaced {
            record.activity.retire();
            drop(records);
            if let Some(task) = record.task {
                task.abort();
            }
        } else {
            drop(records);
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
/// reference resolved to, or the platform manifest it selected. Answers with
/// what the reference resolved to, which is what a session records.
fn admit(
    policy: &capsem_core::container::admission::ImagePolicy,
    requested: &capsem_assets::oci::ImageReference,
    image: &PulledImage,
) -> Result<capsem_assets::oci::ResolvedImage, String> {
    let bind =
        |digest: &str| capsem_assets::oci::Digest::parse(digest).and_then(|digest| requested.clone().resolve(digest));
    // The refusal reported is the one for what the reference resolved to: a
    // pinned reference cannot bind its platform manifest at all, and that
    // mismatch used to replace the policy's reason.
    let mut refusal = None;
    for digest in [&image.image_digest, &image.digest] {
        match bind(digest).and_then(|resolved| policy.admit(&resolved)) {
            Ok(()) => return bind(&image.image_digest).map_err(|e| format!("container image refused: {e:#}")),
            Err(error) => {
                refusal.get_or_insert_with(|| format!("container image refused: {error:#}"));
            }
        }
    }
    Err(refusal.unwrap_or_else(|| "container image refused".into()))
}

/// Pull, stage and start `spec` in VM `id` in the background.
pub(crate) fn start(state: &Arc<ServiceState>, id: String, spec: ContainerSpec) {
    let generation = state.containers.begin(&id, &spec.image);
    let Some(lease) = state.containers.work_lease(&id, generation) else {
        return;
    };
    let task = tokio::spawn({
        let state = Arc::clone(state);
        let id = id.clone();
        async move {
            let _lease = lease;
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
    // One resolver for every entrypoint: a catalog name becomes the pinned
    // digest it names here, and is an explicit reference from then on.
    let mut decision = images::Decision::new(state).await?;
    let requested = decision.resolve(&spec.image).await?;
    // Sources are checked before any registry access: a refused registry is
    // never contacted.
    decision.check_source(&requested).await?;
    let admission = ServiceToProcess::AdmitContainerPull {
        id: state.next_job_id(),
        image: requested.pull.clone(),
        registry: requested.registry.clone(),
        digest: requested.identity.digest().map(ToString::to_string),
    };
    let (uds_path, vm_generation) = {
        let instances = state.instances.lock().unwrap();
        let instance = instances.get(id).ok_or_else(|| format!("sandbox not found: {id}"))?;
        let snapshot = (instance.uds_path.clone(), instance.generation);
        drop(instances);
        snapshot
    };
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
    match send_ipc_command(state, &uds_path, admission, Some(5)).await? {
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
    let access = spec.registry.unwrap_or_default();
    let image = state
        .containers
        .source
        .pull(
            requested.pull.clone(),
            access.clone(),
            decision.parent.clone(),
            ImageFetch::PreferCached,
        )
        .await
        .map_err(|e| format!("pull {}: {e:#}", requested.pull))?;
    // Admission is on the resolved digest, before anything is staged.
    let resolved = decision.admit(&requested, &image).await?;
    let rootfs = decision
        .published_root(&requested, &image, &resolved, access, ImageFetch::PreferCached)
        .await?;
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
    let staged_image = stage::StageImage {
        manifest: &manifest,
        rootfs: rootfs
            .as_ref()
            .map(|(digest, size)| stage::RootfsFile { digest, size: *size }),
    };
    let plan = stage::stage_plan(staged_image, &spec.args, &spec.env, resources, declared)
        .map_err(|e| format!("plan stage: {e:#}"))?;
    for file in plan {
        if !state.containers.advance(id, generation, |_| {}) {
            return Ok(());
        }
        stage_file(state, id, generation, file).await?;
    }

    wait_for_vm_ready(&uds_path, 30, Some(state), Some(id))
        .await
        .map_err(|error| format!("container guest did not become ready: {error}"))?;
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
        state,
        &uds_path,
        ServiceToProcess::Exec {
            id: state.next_job_id(),
            command: capsem_core::container::detached_launch_command(),
            target: capsem_proto::ipc::ExecTarget::Vm,
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
    let Some(lease) = state.containers.work_lease(id, generation) else {
        return;
    };
    let Some(surface) = state.containers.status(id).and_then(|status| status.surface) else {
        return;
    };
    if surface.exposure_id.is_some() {
        return;
    }
    let options = capsem_foundation::poll::PollOpts::new("container-surface-running", SURFACE_READY_TIMEOUT);
    let observed = tokio::select! {
        observed = wait_observed(state, id, options) => observed,
        () = lease.cancelled() => return,
    };
    match observed {
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
    if !state.containers.advance(id, generation, |_| {}) {
        return;
    }
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
        // The admission RPC can outlive this workload generation. Its new
        // exposure belongs to this attempt and must not escape retirement.
        let revoked = crate::router_runtime::exposures::handle_delete_exposure(
            State(Arc::clone(state)),
            Path((id.to_owned(), exposure.id.clone())),
        )
        .await;
        if let Err(AppError(status, error)) = revoked {
            if status != StatusCode::NOT_FOUND {
                warn!(vm_id = id, exposure_id = exposure.id.as_str(), %status, %error, "late container exposure could not be revoked");
            }
        }
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
    /// The pinned `repository@digest`; absent in records written before
    /// sessions recorded it.
    #[serde(default)]
    resolved: Option<String>,
    /// The manifest digest the session's image share holds the image under,
    /// and the one its launcher unpacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest: Option<String>,
    /// Producer identity only; no cached receipt or legacy pin invents this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_key: Option<String>,
}

impl LaunchRecord {
    /// The status a recorded workload has before its launcher reports.
    fn starting(self) -> ContainerStatusResponse {
        ContainerStatusResponse {
            state: ContainerState::Starting,
            image: self.image,
            digest: Some(self.digest),
            resolved: self.resolved,
            exit_code: None,
            error: None,
            surface: self.surface,
        }
    }
}

fn read_launch_record(session_dir: &std::path::Path) -> Option<LaunchRecord> {
    serde_json::from_slice(&std::fs::read(session_dir.join(LAUNCH_RECORD)).ok()?).ok()
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
        capsem_foundation::poll::PollOpts {
            label: "container-create-ready",
            ..vm_ready_poll_opts(CREATE_READY_TIMEOUT.as_secs())
        },
    )
    .await
}

/// Create, resume and carried-image clones share the same workload barrier.
/// A timeout leaves setup with its service owner; terminal failures are errors.
pub(crate) async fn require_ready(state: &Arc<ServiceState>, id: &str) -> Result<(), AppError> {
    let status = wait_for_create(state, id).await.map_err(|timed_out| {
        warn!(vm_id = id, attempts = timed_out.attempts, "container workload readiness timed out");
        AppError(
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "container workload for VM {id} did not become ready before the HTTP deadline; setup continues under service ownership"
            ),
        )
    })?;
    match status.state {
        ContainerState::Running | ContainerState::Staged => Ok(()),
        ContainerState::Exited | ContainerState::Failed => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "container workload for VM {id} reached {:?}: {}",
                status.state,
                status.error.unwrap_or_else(|| status
                    .exit_code
                    .map(|code| format!("exit code {code}"))
                    .unwrap_or_else(|| "no failure detail".into()))
            ),
        )),
        ContainerState::Pulling | ContainerState::Staging | ContainerState::Starting => {
            unreachable!("container readiness returned a pending state")
        }
    }
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
    let Some(record) = state.containers.launch_record(id) else {
        return Ok(());
    };
    let record = serde_json::to_vec(&record).map_err(|e| format!("encode launch record: {e}"))?;
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

/// `running` is guest-reported: runc's poststart hook writes the launcher's
/// running marker into the shared workspace once the workload's process
/// started, so an exec sent after `running` finds it. A launch that ended
/// without starting it marks itself failed.
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
    let Some(mut status) = live.or_else(|| read_launch_record(&session_dir).map(LaunchRecord::starting)) else {
        return Ok(None);
    };
    if matches!(status.state, ContainerState::Starting | ContainerState::Running) {
        if let Some(exited) = exited_marker(state, id) {
            status.state = ContainerState::Exited;
            status.exit_code = exited.code;
        } else if status.state == ContainerState::Starting {
            if staged_marker(state, id, capsem_core::container::STAGE_RUNNING) {
                status.state = ContainerState::Running;
            } else if staged_marker(state, id, capsem_core::container::STAGE_FAILED) {
                status.state = ContainerState::Failed;
                status.error = Some("the workload did not start; `capsem logs` shows why".into());
            }
        }
    }
    Ok(Some(status))
}

/// A workload that ran and ended, as its stage records it.
struct Exited {
    /// `None` when the marker's code does not parse.
    code: Option<i32>,
}

/// Whether the workload of VM `id` ran and ended, and how. The guest writes
/// the marker, so it is opened without following links and read only a few
/// bytes deep.
fn exited_marker(state: &ServiceState, id: &str) -> Option<Exited> {
    use std::io::Read as _;
    let path = format!(
        "{}/{}",
        capsem_core::container::STAGE,
        capsem_core::container::STAGE_EXITED
    );
    let (parent, name) = resolve_workspace_target(state, id, &path, false).ok()?;
    let file = parent.open_file(&name, ContainedOpenOptions::read_only()).ok()?;
    let mut text = String::new();
    file.take(32).read_to_string(&mut text).ok()?;
    Some(Exited {
        code: text.trim().parse().ok(),
    })
}

/// Whether the launcher left `marker` in VM `id`'s stage.
fn staged_marker(state: &ServiceState, id: &str, marker: &str) -> bool {
    let path = format!("{}/{marker}", capsem_core::container::STAGE);
    resolve_workspace_target(state, id, &path, false)
        .and_then(|(parent, name)| parent.entry_kind(&name).map_err(workspace_io_error))
        .is_ok_and(|kind| kind == Some(EntryKind::File))
}

/// Publish `image`'s verified blobs into VM `id`'s read-only image share and
/// answer the manifest digest they are held under. Every blob is linked from
/// the pull's private layout, which nothing but the service can write; the
/// guest-writable workspace is never a source.
async fn share_image(
    state: &ServiceState,
    id: &str,
    generation: u64,
    image: &PulledImage,
    rootfs: Option<capsem_assets::oci::RootfsLayout>,
) -> Result<(String, Option<(String, u64)>), String> {
    let lease = state
        .containers
        .work_lease(id, generation)
        .ok_or("container share cancelled")?;
    let session_dir = resolve_session_dir(state, id).map_err(|e| e.1)?;
    let (root, files) = (image.root.clone(), image.files.clone());
    tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let _lease = lease;
        let mut image = stage::image_blobs(&root, &files)?;
        let blobs = capsem_foundation::unix::contained::ContainedDir::open_root(&root)?
            .walk(std::path::Path::new("blobs/sha256"))?;
        let rootfs = rootfs
            .map(|rootfs| -> anyhow::Result<_> {
                use capsem_foundation::unix::{contained::ContainedDir, tree_clone};
                let path = rootfs.path();
                let name = path
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("rootfs has no blob name"))?;
                let source =
                    ContainedDir::open_root(path.parent().ok_or_else(|| anyhow::anyhow!("rootfs has no parent"))?)?;
                let size = source
                    .open_file(name, ContainedOpenOptions::read_only())?
                    .metadata()?
                    .len();
                tree_clone::link_file(&source, name, &blobs, name)?;
                image.blobs.push(
                    name.to_str()
                        .ok_or_else(|| anyhow::anyhow!("rootfs name is not UTF-8"))?
                        .to_owned(),
                );
                Ok((rootfs.digest().to_string(), size))
            })
            .transpose()?;
        capsem_core::session::publish_image_share(&session_dir, &blobs, &image.blobs)?;
        Ok((image.manifest, rootfs))
    })
    .await
    .map_err(|e| format!("share image: {e}"))?
    .map_err(|e| format!("share image: {e:#}"))
}

/// Write one staged control file into the VM's workspace through the same
/// contained, no-follow writer and import ledger a file upload uses.
async fn stage_file(
    state: &Arc<ServiceState>,
    id: &str,
    generation: u64,
    file: stage::StagedFile,
) -> Result<(), String> {
    let lease = state
        .containers
        .work_lease(id, generation)
        .ok_or("container stage cancelled")?;
    let path = format!("{}/{}", capsem_core::container::STAGE, file.name);
    let target_id = id.to_owned();
    let target_path = path.clone();
    let (parent, name, lease) = state
        .off_worker(move |state| {
            let (parent, name) = resolve_workspace_target(&state, &target_id, &target_path, true)?;
            Ok::<_, AppError>((parent, name, lease))
        })
        .await
        .map_err(|e| e.1)?
        .map_err(|e| e.1)?;
    let preview = file_security_preview_bytes(&file.bytes);
    let size = file.bytes.len() as u64;
    let uds_path = running_uds_path(state, id).map_err(|e| e.1)?;
    if log_file_boundary_on_owner(state, &uds_path, FileBoundaryAction::Import, path, preview, size, None)
        .await
        .map_err(|e| e.1)?
        .is_some()
    {
        return Err(format!("file import policy rewrote staged image file {}", file.name));
    }
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let _lease = lease;
        use std::io::Write;
        parent
            .open_file(&name, ContainedOpenOptions::write_create_truncate(0o644))
            .and_then(|mut output| output.write_all(&file.bytes))
            .map_err(|e| format!("write staged {}: {e}", file.name))
    })
    .await
    .map_err(|e| format!("stage task: {e}"))?
}

#[cfg(test)]
mod tests;

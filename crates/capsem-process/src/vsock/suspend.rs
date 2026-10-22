//! Coordinated VM and terminal continuation, published as one completed pair.

use super::*;
use capsem_core::session::TerminalCheckpointStore;
use capsem_core::terminal::SessionTerminal;
use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;

/// Open before confinement, then retain only the session's granted handles.
pub(crate) struct Files {
    owner: ContainedDir,
    overlay: File,
    terminal: TerminalCheckpointStore,
}

impl Files {
    pub(crate) fn prepare(session: &Path) -> Result<Self> {
        let terminal = TerminalCheckpointStore::prepare(session)?;
        Ok(Self {
            owner: ContainedDir::open_root(session)?.descend(OsStr::new(capsem_core::session::OWNER_STATE_DIR))?,
            overlay: capsem_core::session::open_system_overlay(session)?,
            terminal,
        })
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.owner.path().join(crate::OWNER_CHECKPOINT_FILE)
    }

    fn marker_path(&self) -> PathBuf {
        capsem_foundation::paths::checkpoint_complete_path(&self.path())
    }

    fn remove(&self, path: &Path) -> Result<()> {
        match self
            .owner
            .remove_non_directory(path.file_name().context("checkpoint name missing")?)
        {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn begin(&self) -> Result<()> {
        self.remove(&self.marker_path())?;
        self.owner.sync()?;
        self.remove(&self.path())?;
        self.owner.sync()?;
        Ok(())
    }

    pub(crate) fn load_terminal(&self) -> Result<Arc<SessionTerminal>> {
        self.terminal.load(proto::TERMINAL_SUBSCRIBER_UPDATES)
    }

    pub(crate) fn consume_restored(&self) -> Result<()> {
        self.begin()?;
        self.terminal.clear()
    }

    fn capture(self: &Arc<Self>, terminal: &SessionTerminal) -> Result<Captured> {
        self.terminal.save(terminal)?;
        Ok(Captured(Arc::clone(self)))
    }
}

/// A successful terminal publication is required to complete the VM checkpoint.
struct Captured(Arc<Files>);

impl Captured {
    fn complete(self) -> Result<()> {
        self.0
            .owner
            .open_single_link_file(
                OsStr::new(crate::OWNER_CHECKPOINT_FILE),
                ContainedOpenOptions::read_only(),
            )?
            .sync_all()?;
        self.0.overlay.sync_all()?;
        let marker = self.0.marker_path();
        capsem_core::session::publish_contained_private(
            &self.0.owner,
            marker
                .file_name()
                .and_then(OsStr::to_str)
                .context("checkpoint marker name missing")?,
            b"ok\n",
        )
    }
}

type SharedVm = Arc<tokio::sync::Mutex<Box<dyn capsem_core::hypervisor::VmHandle>>>;

pub(super) struct Suspend {
    pub vm: SharedVm,
    pub hub: mpsc::Sender<HostToGuest>,
    pub jobs: Arc<JobStore>,
    pub ipc: broadcast::Sender<ProcessToService>,
    pub id: String,
    pub shutdown: Arc<tokio::sync::Mutex<crate::Shutdown>>,
    pub terminal: Arc<SessionTerminal>,
    pub files: Arc<Files>,
}

async fn on_vm(
    vm: SharedVm,
    operation: impl FnOnce(&mut dyn capsem_core::hypervisor::VmHandle) -> Result<()> + Send + 'static,
) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        return capsem_core::hypervisor::apple_vz::run_on_main_thread(move || operation(vm.blocking_lock().as_mut()));
        #[cfg(not(target_os = "macos"))]
        operation(vm.blocking_lock().as_mut())
    })
    .await
    .context("join VM checkpoint operation")?
}

impl Suspend {
    async fn save(&self) -> Result<()> {
        self.terminal.seal_views();
        let files = Arc::clone(&self.files);
        tokio::task::spawn_blocking(move || files.begin()).await??;
        let bound = std::time::Duration::from_secs(10);
        crate::job_store::with_quiescence(&self.hub, &self.jobs, bound, || async {
            on_vm(Arc::clone(&self.vm), |vm| vm.pause()).await.context("pause VM")?;
            let bridge = self
                .shutdown
                .lock()
                .await
                .terminal_bridge
                .take()
                .context("terminal transport missing")?;
            tokio::time::timeout(bound, bridge.checkpoint())
                .await
                .context("terminal checkpoint drain timed out")??;
            let files = Arc::clone(&self.files);
            let terminal = Arc::clone(&self.terminal);
            let captured = tokio::task::spawn_blocking(move || files.capture(&terminal)).await??;
            let path = self.files.path();
            on_vm(Arc::clone(&self.vm), move |vm| vm.save_state(&path))
                .await
                .context("save VM")?;
            tokio::task::spawn_blocking(move || captured.complete()).await??;
            Ok(())
        })
        .await
    }

    /// Run inline in the command mux: commands admitted before Suspend are
    /// settled, while later commands cannot mutate the checkpoint's guest.
    pub(super) async fn run(self) -> ! {
        let started = std::time::Instant::now();
        let result = self.save().await;
        let failed = result.is_err();
        let message = match result {
            Ok(()) => ProcessToService::StateChanged {
                id: self.id,
                state: "Suspended".into(),
                trigger: "suspend_requested".into(),
            },
            Err(error) => {
                error!(error = %error, elapsed_ms = started.elapsed().as_millis() as u64, "coordinated suspend failed");
                let files = Arc::clone(&self.files);
                if let Err(cleanup) = tokio::task::spawn_blocking(move || files.begin())
                    .await
                    .unwrap_or_else(|join| Err(join.into()))
                {
                    error!(%cleanup, "invalidate failed checkpoint");
                }
                ProcessToService::SuspendFailed {
                    id: self.id,
                    error: format!("{error:#}"),
                }
            }
        };
        info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            failed, "coordinated suspend finished"
        );
        capsem_core::try_send!("ipc_suspend_result", self.ipc.send(message));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        crate::drain_background_owners(&self.shutdown).await;
        std::process::exit(i32::from(failed));
    }
}

#[cfg(test)]
mod tests;

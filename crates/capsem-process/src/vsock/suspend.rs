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
        self.invalidate()?;
        self.remove(&self.path())?;
        self.owner.sync()?;
        Ok(())
    }

    fn invalidate(&self) -> Result<()> {
        self.remove(&self.marker_path())?;
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
    pub hub: crate::guest_control::Sender,
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
    async fn save(&self, parked: &mut Option<super::terminal::Checkpoint>) -> Result<()> {
        self.terminal.seal_views();
        let files = Arc::clone(&self.files);
        tokio::task::spawn_blocking(move || files.begin()).await??;
        let bound = std::time::Duration::from_secs(10);
        crate::job_store::snapshot_transition(&self.hub, &self.jobs, bound, true).await?;
        on_vm(Arc::clone(&self.vm), |vm| vm.pause()).await.context("pause VM")?;
        *parked = Some(
            self.shutdown
                .lock()
                .await
                .terminal_bridge
                .as_ref()
                .context("terminal transport missing")?
                .park()?,
        );
        tokio::time::timeout(bound, parked.as_mut().unwrap().settled())
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
        // The saved guest retains its frozen filesystem flag. Its first
        // restored control connection thaws before admitting guest commands.
        Ok(())
    }

    /// Run inline in the command mux: commands admitted before Suspend are
    /// settled, while later commands cannot mutate the checkpoint's guest.
    pub(super) async fn run(self) {
        let _sequence = self.jobs.quiescence.lock().await;
        let started = std::time::Instant::now();
        let mut parked = None;
        let result = self.save(&mut parked).await;
        let failed = result.is_err();
        let message = match result {
            Ok(()) => ProcessToService::StateChanged {
                id: self.id,
                state: "Suspended".into(),
                trigger: "suspend_requested".into(),
            },
            Err(error) => {
                error!(error = %error, elapsed_ms = started.elapsed().as_millis() as u64, "coordinated suspend failed");
                match self.rollback(parked.take()).await {
                    Ok(()) => {
                        capsem_core::try_send!(
                            "ipc_suspend_rollback",
                            self.ipc.send(ProcessToService::SuspendRolledBack {
                                id: self.id,
                                error: format!("{error:#}"),
                            })
                        );
                        return;
                    }
                    Err(rollback) => {
                        error!(%rollback, "suspend rollback failed; retiring owner");
                        ProcessToService::SuspendFailed {
                            id: self.id,
                            error: format!("{error:#}; rollback failed: {rollback:#}"),
                        }
                    }
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

    async fn rollback(&self, parked: Option<super::terminal::Checkpoint>) -> Result<()> {
        let files = Arc::clone(&self.files);
        tokio::task::spawn_blocking(move || files.invalidate()).await??;
        on_vm(Arc::clone(&self.vm), |vm| resume_running_vm(vm)).await?;
        crate::job_store::snapshot_transition(&self.hub, &self.jobs, std::time::Duration::from_secs(10), false)
            .await
            .context("confirm guest thaw after rejected suspend")?;
        if let Some(parked) = parked {
            tokio::time::timeout(std::time::Duration::from_secs(10), parked.resume())
                .await
                .context("terminal rollback timed out")??;
        }
        self.terminal.resume_views();
        Ok(())
    }
}

fn resume_running_vm(vm: &dyn capsem_core::hypervisor::VmHandle) -> Result<()> {
    use capsem_core::hypervisor::VmState;
    if vm.state() == VmState::Paused {
        vm.resume().context("resume VM after rejected suspend")?;
    }
    anyhow::ensure!(
        vm.state() == VmState::Running,
        "VM did not return to Running after rejected suspend"
    );
    Ok(())
}

#[cfg(test)]
mod tests;

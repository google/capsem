//! The per-session logger DB handles the service holds for its routes:
//! registration, replacement on a rebound path, and startup hydration.

use super::*;

pub(super) fn session_db_path_for_session_dir(session_dir: &StdPath) -> PathBuf {
    session_dir.join("session.db")
}

/// The one place the service opens a per-session external reader. Blocking:
/// callers on the runtime reach it through `spawn_blocking`.
fn open_session_db_reader(db_path: &StdPath) -> Result<capsem_logger::DbHandle, String> {
    capsem_logger::DbHandle::open_external_reader(db_path).map_err(|error| error.to_string())
}

impl ServiceState {
    /// Register `vm_id`'s ledger reader, opening it on this thread.
    ///
    /// Opening a reader is a blocking SQLite open and schema check, so this is
    /// for code already off the runtime: startup hydration and `resume_sandbox`.
    /// Async routes call [`Self::register_session_db_handle_async`].
    pub(crate) fn register_session_db_handle(
        &self,
        vm_id: &str,
        session_dir: &StdPath,
    ) -> anyhow::Result<Arc<capsem_logger::DbHandle>> {
        let db_path = session_db_path_for_session_dir(session_dir);
        let started = std::time::Instant::now();
        if let Some(handle) = self.registered_session_db_handle(vm_id, &db_path, started) {
            return Ok(handle);
        }
        let opened = open_session_db_reader(&db_path);
        self.install_session_db_handle(vm_id, db_path, opened, started)
    }

    /// Register `vm_id`'s ledger reader without blocking a tokio worker.
    ///
    /// Same answer as [`Self::register_session_db_handle`]; the open runs on
    /// the blocking pool, where a slow disk or a large schema check stalls one
    /// blocking thread instead of every request sharing the worker.
    pub(crate) async fn register_session_db_handle_async(
        &self,
        vm_id: &str,
        session_dir: &StdPath,
    ) -> anyhow::Result<Arc<capsem_logger::DbHandle>> {
        let db_path = session_db_path_for_session_dir(session_dir);
        let started = std::time::Instant::now();
        if let Some(handle) = self.registered_session_db_handle(vm_id, &db_path, started) {
            return Ok(handle);
        }
        let open_path = db_path.clone();
        let opened = tokio::task::spawn_blocking(move || open_session_db_reader(&open_path))
            .await
            .unwrap_or_else(|error| Err(format!("session DB open task failed: {error}")));
        self.install_session_db_handle(vm_id, db_path, opened, started)
    }

    /// The handle already registered for `vm_id` at `db_path`, if any. A
    /// handle for another path is left for the caller to replace.
    fn registered_session_db_handle(
        &self,
        vm_id: &str,
        db_path: &StdPath,
        started: std::time::Instant,
    ) -> Option<Arc<capsem_logger::DbHandle>> {
        let handle = self.session_db_handles.lock().unwrap().get(vm_id).cloned()?;
        if handle.path() == db_path {
            tracing::debug!(
                vm_id,
                db_path = %db_path.display(),
                operation = "register_session_db_handle",
                duration_ms = started.elapsed().as_millis(),
                "reused existing session DB handle"
            );
            return Some(handle);
        }
        warn!(
            vm_id,
            cached_db_path = %handle.path().display(),
            db_path = %db_path.display(),
            operation = "register_session_db_handle",
            "replacing session DB handle for rebound session path"
        );
        None
    }

    fn install_session_db_handle(
        &self,
        vm_id: &str,
        db_path: PathBuf,
        opened: Result<capsem_logger::DbHandle, String>,
        started: std::time::Instant,
    ) -> anyhow::Result<Arc<capsem_logger::DbHandle>> {
        let handle = match opened {
            Ok(handle) => Arc::new(handle),
            Err(error) => {
                error!(
                    vm_id,
                    db_path = %db_path.display(),
                    operation = "register_session_db_handle",
                    duration_ms = started.elapsed().as_millis(),
                    error = %error,
                    "failed to register session DB handle"
                );
                return Err(anyhow!(
                    "failed to open session DB handle for {vm_id}: {}: {error}",
                    db_path.display()
                ));
            }
        };
        self.session_db_handles
            .lock()
            .unwrap()
            .insert(vm_id.to_string(), Arc::clone(&handle));
        info!(
            vm_id,
            db_path = %db_path.display(),
            operation = "register_session_db_handle",
            duration_ms = started.elapsed().as_millis(),
            "registered session DB handle"
        );
        Ok(handle)
    }

    pub(crate) fn unregister_session_db_handle(&self, vm_id: &str) {
        let removed = self.session_db_handles.lock().unwrap().remove(vm_id);
        forget_session_responses(self, vm_id);
        if removed.is_some() {
            info!(
                vm_id,
                operation = "unregister_session_db_handle",
                "unregistered session DB handle"
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn rename_session_db_handle(&self, old_vm_id: &str, new_vm_id: &str) {
        let mut handles = self.session_db_handles.lock().unwrap();
        if let Some(handle) = handles.remove(old_vm_id) {
            handles.insert(new_vm_id.to_string(), handle);
            drop(handles);
            info!(
                old_vm_id,
                new_vm_id,
                operation = "rename_session_db_handle",
                "renamed session DB handle"
            );
        }
    }

    pub(crate) fn session_db_handle(&self, vm_id: &str) -> Option<Arc<capsem_logger::DbHandle>> {
        self.session_db_handles.lock().unwrap().get(vm_id).cloned()
    }

    pub(crate) fn hydrate_session_db_handles(&self) {
        let mut candidates: Vec<(String, PathBuf)> = {
            let instances = self.instances.lock().unwrap();
            instances
                .values()
                .map(|info| (info.id.clone(), info.session_dir.clone()))
                .collect()
        };
        {
            let registry = self.persistent_registry.lock().unwrap();
            candidates.extend(
                registry
                    .data
                    .vms
                    .values()
                    .map(|entry| (persistent_entry_vm_id(entry), entry.session_dir.clone())),
            );
        }

        let mut hydrated = 0usize;
        for (vm_id, session_dir) in candidates {
            let db_path = session_db_path_for_session_dir(&session_dir);
            if !db_path.exists() {
                info!(
                    vm_id,
                    operation = "hydrate_session_db_handle",
                    db_path = %db_path.display(),
                    "session DB absent during startup handle hydration"
                );
                continue;
            }
            match self.register_session_db_handle(&vm_id, &session_dir) {
                Ok(_) => hydrated += 1,
                Err(error) => {
                    warn!(
                        vm_id,
                        operation = "hydrate_session_db_handle",
                        db_path = %db_path.display(),
                        error = %error,
                        "failed to hydrate session DB handle"
                    );
                }
            }
        }
        info!(
            operation = "hydrate_session_db_handles",
            hydrated, "startup session DB handle hydration complete"
        );
    }
}

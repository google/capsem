//! The per-session logger DB handles the service holds for its routes:
//! registration, replacement on a rebound path, and startup hydration.

use super::*;

impl ServiceState {
    pub(crate) fn register_session_db_handle(
        &self,
        vm_id: &str,
        session_dir: &StdPath,
    ) -> anyhow::Result<Arc<capsem_logger::DbHandle>> {
        let db_path = session_db_path_for_session_dir(session_dir);
        let started = std::time::Instant::now();
        let handles = self.session_db_handles.lock().unwrap();
        if let Some(handle) = handles.get(vm_id) {
            if handle.path() == db_path.as_path() {
                tracing::debug!(
                    vm_id,
                    db_path = %db_path.display(),
                    operation = "register_session_db_handle",
                    duration_ms = started.elapsed().as_millis(),
                    "reused existing session DB handle"
                );
                return Ok(Arc::clone(handle));
            }
            warn!(
                vm_id,
                cached_db_path = %handle.path().display(),
                db_path = %db_path.display(),
                operation = "register_session_db_handle",
                "replacing session DB handle for rebound session path"
            );
        }
        drop(handles);
        let handle = match capsem_logger::DbHandle::open_external_reader(&db_path) {
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
        let mut handles = self.session_db_handles.lock().unwrap();
        handles.insert(vm_id.to_string(), Arc::clone(&handle));
        drop(handles);
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

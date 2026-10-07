//! The coordinator's session and spawn, never a worker-reported path, select
//! the endpoint to which a handoff may grant descriptors.
use super::*;

pub(crate) struct OwnerHandoff {
    id: String,
    generation: uuid::Uuid,
    endpoint: PathBuf,
    pub(crate) uds_path: PathBuf,
}

impl OwnerHandoff {
    pub(crate) async fn acquire(state: &Arc<ServiceState>, id: &str) -> Result<Self, AppError> {
        let (id, generation, uds_path) = {
            let instances = state.instances.lock().unwrap();
            let instance = instances
                .get(id)
                .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not running: {id}")))?;
            let captured = (instance.id.clone(), instance.generation, instance.uds_path.clone());
            drop(instances);
            captured
        };
        let session_id = id.clone();
        // The stable fallback helper may create/check its private directory.
        let endpoint = state
            .off_worker(move |state| capsem_foundation::uds::private_handoff_socket_path(&state.run_dir, &session_id))
            .await?
            .map_err(|error| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("derive owner handoff: {error}"),
                )
            })?;
        Ok(Self {
            id,
            generation,
            endpoint,
            uds_path,
        })
    }

    pub(crate) fn validate(self, state: &ServiceState, reported: &str) -> Result<PathBuf, AppError> {
        // Path equality normalizes '.' components. Authorize exactly the
        // spelling we derived, without resolving any worker-controlled path.
        if self.endpoint.as_os_str() != std::ffi::OsStr::new(reported) {
            return Err(AppError(
                StatusCode::BAD_GATEWAY,
                "handoff endpoint does not belong to this VM owner".into(),
            ));
        }
        if !state
            .instances
            .lock()
            .unwrap()
            .get(&self.id)
            .is_some_and(|instance| instance.generation == self.generation)
        {
            return Err(AppError(
                StatusCode::BAD_GATEWAY,
                "VM owner changed during handoff admission".into(),
            ));
        }
        Ok(self.endpoint)
    }
}

#[cfg(test)]
mod tests;

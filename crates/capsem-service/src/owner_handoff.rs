//! The coordinator's session and spawn, never a worker-reported path, select
//! the endpoint to which a handoff may grant descriptors.
use super::*;
use std::os::fd::AsFd;

pub(crate) struct OwnerHandoff {
    id: String,
    generation: uuid::Uuid,
    endpoint: PathBuf,
    pub(crate) owner: crate::owner_connection::OwnerConnection,
}

impl OwnerHandoff {
    pub(crate) async fn acquire(state: &Arc<ServiceState>, id: &str) -> Result<Self, AppError> {
        let (id, generation, owner) = {
            let instances = state.instances.lock().unwrap();
            let instance = instances
                .get(id)
                .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not running: {id}")))?;
            let owner = crate::owner_connection::OwnerConnection::capture(instance)
                .map_err(|e| AppError(StatusCode::BAD_GATEWAY, e))?;
            let captured = (instance.id.clone(), instance.generation, owner);
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
            owner,
        })
    }

    pub(crate) fn validate(&self, state: &ServiceState, reported: &str) -> Result<PathBuf, AppError> {
        // Path equality normalizes '.' components. Authorize exactly the
        // spelling we derived, without resolving any worker-controlled path.
        if self.endpoint.as_os_str() != std::ffi::OsStr::new(reported) {
            return Err(AppError(
                StatusCode::BAD_GATEWAY,
                "handoff endpoint does not belong to this VM owner".into(),
            ));
        }
        self.validate_current(state)?;
        Ok(self.endpoint.clone())
    }

    pub(crate) async fn connect(&self, state: &ServiceState) -> Result<std::os::unix::net::UnixStream, AppError> {
        self.validate_current(state)?;
        let socket = tokio::net::UnixStream::connect(&self.endpoint)
            .await
            .and_then(|socket| socket.into_std())
            .map_err(|error| {
                AppError(
                    StatusCode::BAD_GATEWAY,
                    format!("VM owner handoff unavailable: {error}"),
                )
            })?;
        capsem_foundation::unix::peer::require(
            socket.as_fd(),
            capsem_foundation::unix::peer::PeerIdentity {
                pid: capsem_foundation::unix::process::ProcessId::try_from(self.owner.pid())
                    .map_err(|error| AppError(StatusCode::BAD_GATEWAY, error.to_string()))?,
                uid: self.owner.uid(),
            },
        )
        .map_err(|error| {
            AppError(
                StatusCode::BAD_GATEWAY,
                format!("authenticate VM owner handoff: {error}"),
            )
        })?;
        self.validate_current(state)?;
        Ok(socket)
    }

    fn validate_current(&self, state: &ServiceState) -> Result<(), AppError> {
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
        Ok(())
    }
}

#[cfg(test)]
mod tests;

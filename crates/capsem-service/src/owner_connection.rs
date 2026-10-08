//! A coordinator-owned spawn grants IPC authority; a pathname or Hello does not.
use super::*;
use capsem_foundation::unix::{peer, process};
use std::os::fd::AsFd;

pub(crate) struct OwnerConnection {
    id: String,
    generation: uuid::Uuid,
    identity: peer::PeerIdentity,
    pub(crate) uds_path: PathBuf,
}

impl OwnerConnection {
    pub(crate) fn current(
        state: &ServiceState,
        id: &str,
        identity: Option<peer::PeerIdentity>,
    ) -> Result<Self, String> {
        let owner = state
            .instances
            .lock()
            .unwrap()
            .get(id)
            .map(Self::capture)
            .transpose()?
            .ok_or_else(|| format!("VM {id} has no running owner"))?;
        let identity = identity.ok_or_else(|| "service peer identity unavailable".to_string())?;
        owner.authenticate_identity(state, identity)?;
        Ok(owner)
    }

    pub(crate) fn pid(&self) -> u32 {
        self.identity.pid.get()
    }

    pub(crate) fn uid(&self) -> u32 {
        self.identity.uid
    }

    pub(crate) fn authenticate(
        &self,
        state: &ServiceState,
        socket: &std::os::unix::net::UnixStream,
    ) -> Result<(), String> {
        peer::require(socket.as_fd(), self.identity).map_err(|e| format!("VM owner authentication: {e}"))?;
        self.validate(state, false)
    }

    pub(crate) fn authenticate_identity(
        &self,
        state: &ServiceState,
        identity: peer::PeerIdentity,
    ) -> Result<(), String> {
        if identity != self.identity {
            return Err("Unix peer is not the granted VM owner".into());
        }
        self.validate(state, false)
    }

    pub(crate) fn capture(instance: &InstanceInfo) -> Result<Self, String> {
        Ok(Self {
            id: instance.id.clone(),
            generation: instance.generation,
            identity: peer::PeerIdentity {
                pid: process::ProcessId::try_from(instance.pid).map_err(|e| e.to_string())?,
                uid: process::current_uid(),
            },
            uds_path: instance.uds_path.clone(),
        })
    }

    pub(crate) fn acquire(state: &ServiceState, path: &StdPath) -> Result<Self, String> {
        let instances = state.instances.lock().unwrap();
        let mut matching = instances
            .values()
            .filter(|instance| instance.uds_path.as_os_str() == path.as_os_str());
        let owner = Self::capture(matching.next().ok_or("VM owner is not registered")?)?;
        if matching.next().is_some() {
            return Err("VM owner endpoint is ambiguous".into());
        }
        drop(instances);
        Ok(owner)
    }

    pub(crate) fn validate(&self, state: &ServiceState, retiring: bool) -> Result<(), String> {
        let instances = state.instances.lock().unwrap();
        let current = match instances.get(&self.id) {
            Some(instance) => {
                instance.generation == self.generation
                    && instance.pid == self.identity.pid.get()
                    && instance.uds_path.as_os_str() == self.uds_path.as_os_str()
            }
            // Only the lifecycle owner that captured and claimed this spawn
            // may finish its shutdown after removing its registry entry.
            None => retiring,
        };
        drop(instances);
        if !current {
            return Err("VM owner changed during IPC admission".into());
        }
        Ok(())
    }

    pub(crate) async fn open(
        &self,
        state: &ServiceState,
        role: &str,
        retiring: bool,
    ) -> Result<(Sender<ServiceToProcess>, Receiver<ProcessToService>), String> {
        self.validate(state, retiring)?;
        let socket = tokio::net::UnixStream::connect(&self.uds_path)
            .await
            .and_then(|socket| socket.into_std())
            .map_err(|e| format!("VM owner unavailable: {e}"))?;
        peer::require(socket.as_fd(), self.identity).map_err(|e| format!("VM owner authentication: {e}"))?;
        self.validate(state, retiring)?;
        let (socket, _) = capsem_foundation::ipc_handshake::negotiate_initiator_off_worker(
            socket,
            role,
            capsem_foundation::telemetry::current_parent_traceparent(),
        )
        .await
        .map_err(|e| format!("VM owner handshake: {e}"))?;
        self.validate(state, retiring)?;
        channel_from_std(socket).map_err(|e| format!("VM owner channel: {e}"))
    }
}

#[cfg(test)]
mod tests;

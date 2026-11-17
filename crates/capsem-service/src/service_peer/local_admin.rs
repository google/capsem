//! One named local administration operation. Neither same UID nor a bearer
//! alone grants access to the service API on its public socket.
use crate::{caller_auth::AuthenticatedActor, ServiceState};
use axum::http::HeaderMap;
use capsem_foundation::unix::{peer::PeerIdentity, process::ProcessHandle};
use std::{path::PathBuf, sync::Arc};

pub(super) async fn admit(
    state: &Arc<ServiceState>,
    peer: Option<PeerIdentity>,
    headers: &HeaderMap,
) -> Option<AuthenticatedActor> {
    admit_expected(
        state,
        peer,
        headers,
        crate::service_runtime::find_sibling_binary("capsem"),
    )
    .await
}

async fn admit_expected(
    state: &Arc<ServiceState>,
    peer: Option<PeerIdentity>,
    headers: &HeaderMap,
    expected: PathBuf,
) -> Option<AuthenticatedActor> {
    state
        .caller_authority
        .authenticate_gateway(headers)
        .ok()?
        .require_operator()
        .ok()?;
    let peer = peer?;
    if peer.uid != capsem_foundation::unix::process::current_uid() {
        return None;
    }
    let matches = state
        .off_worker(move |_| {
            let handle = ProcessHandle::open(peer.pid).ok()?;
            let executable = handle.executable_path().ok()?;
            Some(executable.canonicalize().ok()? == expected.canonicalize().ok()?)
        })
        .await
        .ok()??;
    if !matches {
        return None;
    }
    // The blocking peer check cannot extend a replaced or ended operator epoch.
    let actor = state.caller_authority.authenticate_gateway(headers).ok()?;
    actor.require_operator().ok()?;
    Some(actor)
}

#[cfg(test)]
mod tests;

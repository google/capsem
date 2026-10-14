use super::ServiceState;

/// Atomically claim teardown ownership for an instance. The child watcher
/// uses the same map removal as its ownership token, so only one side wins.
pub(super) fn claim_shutdown_instance(state: &ServiceState, id: &str, generation: uuid::Uuid) -> bool {
    let Some(info) = state.evict_instance(id, generation) else {
        return false;
    };
    state.connections.fence_runtime(
        id,
        generation,
        super::connection_routes::lifecycle::stop_policy(info.persistent, true),
    );
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShutdownMode {
    Retain,
    Discard,
    /// Service-wide stop preserves the existing two-second guest grace.
    ServiceStop,
}

impl ShutdownMode {
    pub(super) fn retains_state(self) -> bool {
        matches!(self, Self::Retain | Self::ServiceStop)
    }

    /// How long the guest is given to exit on its own before SIGKILL.
    ///
    /// `Retain` is the graceful path: the guest syncs its filesystem and
    /// writes bash history before teardown, and whatever has not reached the
    /// disk when the budget expires is lost. Five seconds was sized for an
    /// idle developer machine. On a loaded runner with several VMs competing
    /// for I/O the sync did not finish, the guest was killed, and a persistent
    /// VM resumed without files it had acknowledged writing.
    ///
    /// `Discard` keeps its short budget: the session directory is about to be
    /// removed, so there is nothing for a sync to preserve.
    pub(super) fn exit_timeout(self) -> std::time::Duration {
        match self {
            Self::Retain => std::time::Duration::from_secs(60),
            Self::Discard => std::time::Duration::from_secs(1),
            Self::ServiceStop => std::time::Duration::from_secs(2),
        }
    }
}

#[cfg(test)]
mod tests;

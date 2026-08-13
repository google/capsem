use super::{PollOpts, ServiceState};

pub(super) fn process_exit_poll_options(timeout: std::time::Duration) -> PollOpts {
    let cadence = std::time::Duration::from_millis(20);
    PollOpts {
        label: "vm-process-exit",
        timeout,
        initial_delay: cadence,
        max_delay: cadence,
    }
}

/// Atomically claim teardown ownership for an instance. The child watcher
/// uses the same map removal as its ownership token, so only one side wins.
pub(super) fn claim_shutdown_instance(state: &ServiceState, id: &str) -> bool {
    state.instances.lock().unwrap().remove(id).is_some()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShutdownMode {
    Retain,
    Discard,
}

impl ShutdownMode {
    pub(super) fn retains_state(self) -> bool {
        matches!(self, Self::Retain)
    }

    pub(super) fn exit_timeout(self) -> std::time::Duration {
        match self {
            Self::Retain => std::time::Duration::from_secs(5),
            Self::Discard => std::time::Duration::from_secs(1),
        }
    }
}

use capsem_proto::ipc::ProcessToService;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SuspendConfirmation {
    Suspended,
    RolledBack(String),
    Failed(String),
    ChannelClosed,
    TimedOut,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SuspendFailure {
    RolledBack(String),
    Fatal { outcome: &'static str, error: String },
}

pub(super) fn observe_suspend_message(message: ProcessToService, suspended: &mut bool) -> Option<SuspendConfirmation> {
    match message {
        ProcessToService::StateChanged { state, .. } if state == "Suspended" => {
            *suspended = true;
            None
        }
        ProcessToService::SuspendFailed { error, .. } => Some(SuspendConfirmation::Failed(error)),
        ProcessToService::SuspendRolledBack { error, .. } => Some(SuspendConfirmation::RolledBack(error)),
        _ => None,
    }
}

pub(super) fn suspend_channel_closed(suspended: bool) -> SuspendConfirmation {
    if suspended {
        SuspendConfirmation::Suspended
    } else {
        SuspendConfirmation::ChannelClosed
    }
}

pub(super) fn suspend_failure(confirmation: SuspendConfirmation) -> Option<SuspendFailure> {
    match confirmation {
        SuspendConfirmation::Suspended => None,
        SuspendConfirmation::RolledBack(error) => Some(SuspendFailure::RolledBack(format!(
            "suspend failed; session remains running: {error}"
        ))),
        SuspendConfirmation::Failed(error) => Some(SuspendFailure::Fatal {
            outcome: "failed",
            error: format!("suspend failed before checkpoint completion: {error} (process killed)"),
        }),
        SuspendConfirmation::ChannelClosed => Some(SuspendFailure::Fatal {
            outcome: "channel-closed",
            error: "suspend process exited before checkpoint confirmation (process killed)".into(),
        }),
        SuspendConfirmation::TimedOut => Some(SuspendFailure::Fatal {
            outcome: "timed-out",
            error: "suspend timed out: VM did not confirm suspended state (process killed)".into(),
        }),
    }
}

#[cfg(test)]
mod tests;

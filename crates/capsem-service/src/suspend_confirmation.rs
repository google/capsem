use capsem_proto::ipc::ProcessToService;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SuspendConfirmation {
    Suspended,
    Failed(String),
    ChannelClosed,
    TimedOut,
}

pub(super) fn observe_suspend_message(message: ProcessToService, suspended: &mut bool) -> Option<SuspendConfirmation> {
    match message {
        ProcessToService::StateChanged { state, .. } if state == "Suspended" => {
            *suspended = true;
            None
        }
        ProcessToService::SuspendFailed { error, .. } => Some(SuspendConfirmation::Failed(error)),
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

pub(super) fn suspend_failure(confirmation: SuspendConfirmation) -> Option<(&'static str, String)> {
    match confirmation {
        SuspendConfirmation::Suspended => None,
        SuspendConfirmation::Failed(error) => Some((
            "failed",
            format!("suspend failed before checkpoint completion: {error} (process killed)"),
        )),
        SuspendConfirmation::ChannelClosed => Some((
            "channel-closed",
            "suspend process exited before checkpoint confirmation (process killed)".into(),
        )),
        SuspendConfirmation::TimedOut => Some((
            "timed-out",
            "suspend timed out: VM did not confirm suspended state (process killed)".into(),
        )),
    }
}

#[cfg(test)]
mod tests;

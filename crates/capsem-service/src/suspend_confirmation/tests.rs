use super::*;

#[test]
fn checkpoint_failure_stays_distinct_from_timeout() {
    let mut suspended = false;
    assert_eq!(
        observe_suspend_message(
            ProcessToService::SuspendFailed {
                id: "vm-checkpoint".into(),
                error: "VirtioFS inode 41 is not reopenable".into(),
            },
            &mut suspended,
        ),
        Some(SuspendConfirmation::Failed(
            "VirtioFS inode 41 is not reopenable".into()
        ))
    );
    assert!(!suspended);

    assert_eq!(
        observe_suspend_message(
            ProcessToService::StateChanged {
                id: "vm-checkpoint".into(),
                state: "Suspended".into(),
                trigger: "suspend_requested".into(),
            },
            &mut suspended,
        ),
        None
    );
    assert!(suspended);
    assert_eq!(observe_suspend_message(ProcessToService::Pong, &mut suspended), None);
    assert_eq!(suspend_channel_closed(suspended), SuspendConfirmation::Suspended);
    assert_eq!(suspend_channel_closed(false), SuspendConfirmation::ChannelClosed);
    assert_ne!(SuspendConfirmation::ChannelClosed, SuspendConfirmation::TimedOut);
    assert_eq!(suspend_failure(SuspendConfirmation::Suspended), None);
    assert_eq!(
        suspend_failure(SuspendConfirmation::Failed("checkpoint cause".into())),
        Some((
            "failed",
            "suspend failed before checkpoint completion: checkpoint cause (process killed)".into()
        ))
    );
    assert_eq!(
        suspend_failure(SuspendConfirmation::ChannelClosed),
        Some((
            "channel-closed",
            "suspend process exited before checkpoint confirmation (process killed)".into()
        ))
    );
    assert_eq!(
        suspend_failure(SuspendConfirmation::TimedOut),
        Some((
            "timed-out",
            "suspend timed out: VM did not confirm suspended state (process killed)".into()
        ))
    );
}

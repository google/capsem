use super::*;
use crate::client;

fn session(status: client::VmLifecycleState) -> SessionInfo {
    SessionInfo {
        id: "vm1".into(),
        profile_id: "code".into(),
        name: Some("dev".into()),
        pid: 0,
        status,
        persistent: true,
        ram_mb: None,
        cpus: None,
        version: None,
        forked_from: None,
        description: None,
        created_at: None,
        uptime_secs: None,
        total_input_tokens: None,
        total_output_tokens: None,
        total_estimated_cost: None,
        total_tool_calls: None,
        total_requests: None,
        allowed_requests: None,
        denied_requests: None,
        total_file_events: None,
        model_call_count: None,
        last_error: None,
        can_resume: false,
        resume_blocked_reason: None,
    }
}

#[test]
fn session_blocked_reason_distils_a_crashed_boot_to_its_error_line() {
    let mut vm = session(client::VmLifecycleState::Defunct);
    vm.last_error = Some(
        "INFO capsem_process: booting\n\
         ERROR capsem_process: failed to build VmConfig: rootfs hash mismatch\n"
            .into(),
    );

    assert_eq!(
        session_blocked_reason(&vm),
        Some("ERROR capsem_process: failed to build VmConfig: rootfs hash mismatch")
    );
}

#[test]
fn session_blocked_reason_explains_a_stopped_vm_the_service_will_not_resume() {
    // The reachable case `capsem list` used to render as a bare "Stopped" row:
    // the VM never crashed, so there is no last_error, but asset validation
    // fails and the service refuses to start it.
    let mut vm = session(client::VmLifecycleState::Stopped);
    vm.resume_blocked_reason = Some("rootfs asset file is missing".into());

    assert_eq!(session_blocked_reason(&vm), Some("rootfs asset file is missing"));
}

#[test]
fn session_blocked_reason_stays_quiet_for_a_healthy_session() {
    assert_eq!(
        session_blocked_reason(&session(client::VmLifecycleState::Running)),
        None
    );

    let mut resumable = session(client::VmLifecycleState::Stopped);
    resumable.can_resume = true;
    assert_eq!(session_blocked_reason(&resumable), None);
}

//! Tests for `host_state` (extracted from inline `mod tests`).

use super::*;

// -------------------------------------------------------------------
// HostState transitions
// -------------------------------------------------------------------

#[test]
fn non_terminal_have_transitions() {
    let non_terminal = [
        HostState::Created,
        HostState::Booting,
        HostState::VsockConnected,
        HostState::Handshaking,
        HostState::Running,
        HostState::ShuttingDown,
        HostState::Error,
    ];
    for state in non_terminal {
        assert!(
            !state.valid_transitions().is_empty(),
            "{state:?} should have valid transitions"
        );
    }
}

#[test]
fn stopped_is_terminal() {
    assert!(HostState::Stopped.valid_transitions().is_empty());
}

#[test]
fn all_valid_transitions_succeed() {
    let all_states = [
        HostState::Created,
        HostState::Booting,
        HostState::VsockConnected,
        HostState::Handshaking,
        HostState::Running,
        HostState::ShuttingDown,
        HostState::Stopped,
        HostState::Error,
    ];
    for from in all_states {
        for &to in from.valid_transitions() {
            let mut sm = StateMachine::new(from, |s| s.valid_transitions());
            assert!(
                sm.transition(to, "test").is_ok(),
                "transition {from:?} -> {to:?} should succeed"
            );
        }
    }
}

#[test]
fn invalid_transitions_fail() {
    let mut sm = HostStateMachine::new_host();
    assert!(sm.transition(HostState::Running, "test").is_err());
}

#[test]
fn created_to_booting() {
    let mut sm = HostStateMachine::new_host();
    assert_eq!(sm.state(), HostState::Created);
    let t = sm.transition(HostState::Booting, "vm_started").unwrap();
    assert_eq!(t.from, HostState::Created);
    assert_eq!(t.to, HostState::Booting);
    assert_eq!(t.trigger, "vm_started");
    assert_eq!(sm.state(), HostState::Booting);
}

#[test]
fn error_reachable_from_all_non_terminal() {
    let states = [
        HostState::Created,
        HostState::Booting,
        HostState::VsockConnected,
        HostState::Handshaking,
        HostState::Running,
        HostState::ShuttingDown,
    ];
    for state in states {
        assert!(
            state.valid_transitions().contains(&HostState::Error),
            "{state:?} should be able to transition to Error"
        );
    }
}

#[test]
fn same_state_transition_fails() {
    let mut sm = HostStateMachine::new_host();
    assert!(sm.transition(HostState::Created, "test").is_err());
}

// -------------------------------------------------------------------
// StateMachine generic behavior
// -------------------------------------------------------------------

#[test]
fn new_starts_in_initial_state() {
    let sm = HostStateMachine::new_host();
    assert_eq!(sm.state(), HostState::Created);
    assert!(sm.history().is_empty());
}

#[test]
fn elapsed_does_not_panic() {
    let sm = HostStateMachine::new_host();
    let _ = sm.elapsed();
}

#[test]
fn transition_records_history() {
    let mut sm = HostStateMachine::new_host();
    sm.transition(HostState::Booting, "vm_started").unwrap();
    sm.transition(HostState::VsockConnected, "vsock_ports_connected")
        .unwrap();
    assert_eq!(sm.history().len(), 2);
    assert_eq!(sm.history()[0].from, HostState::Created);
    assert_eq!(sm.history()[0].to, HostState::Booting);
    assert_eq!(sm.history()[0].trigger, "vm_started");
    assert_eq!(sm.history()[1].from, HostState::Booting);
    assert_eq!(sm.history()[1].to, HostState::VsockConnected);
}

#[test]
fn trigger_strings_preserved() {
    let mut sm = HostStateMachine::new_host();
    sm.transition(HostState::Booting, "vm_started").unwrap();
    assert_eq!(sm.history()[0].trigger, "vm_started");
}

#[test]
fn format_perf_log_not_empty() {
    let mut sm = HostStateMachine::new_host();
    sm.transition(HostState::Booting, "vm_started").unwrap();
    let log = sm.format_perf_log();
    assert!(log.contains("Created"));
    assert!(log.contains("Booting"));
    assert!(log.contains("vm_started"));
    assert!(log.contains("ms"));
}

#[test]
fn format_perf_log_empty_for_no_transitions() {
    let sm = HostStateMachine::new_host();
    assert_eq!(sm.format_perf_log(), "");
}

#[test]
fn full_host_lifecycle() {
    let mut sm = HostStateMachine::new_host();
    sm.transition(HostState::Booting, "vm_started").unwrap();
    sm.transition(HostState::VsockConnected, "vsock_ports_connected")
        .unwrap();
    sm.transition(HostState::Handshaking, "ready_received")
        .unwrap();
    sm.transition(HostState::Running, "boot_ready_received")
        .unwrap();
    sm.transition(HostState::ShuttingDown, "shutdown_requested")
        .unwrap();
    sm.transition(HostState::Stopped, "vm_stopped").unwrap();
    assert_eq!(sm.state(), HostState::Stopped);
    assert_eq!(sm.history().len(), 6);
}

#[test]
fn error_then_stopped() {
    let mut sm = HostStateMachine::new_host();
    sm.transition(HostState::Error, "boot_failed").unwrap();
    sm.transition(HostState::Stopped, "cleanup").unwrap();
    assert_eq!(sm.state(), HostState::Stopped);
}

// -------------------------------------------------------------------
// validate_host_msg
// -------------------------------------------------------------------

#[test]
fn boot_config_in_handshaking() {
    let msg = HostToGuest::BootConfig {
        epoch_secs: 1000,
        traceparent: String::new(),
    };
    assert!(validate_host_msg(&msg, HostState::Handshaking).is_ok());
}

#[test]
fn boot_config_rejected_in_other_states() {
    let msg = HostToGuest::BootConfig {
        epoch_secs: 1000,
        traceparent: String::new(),
    };
    for state in [
        HostState::Created,
        HostState::Booting,
        HostState::VsockConnected,
        HostState::Running,
        HostState::ShuttingDown,
    ] {
        assert!(
            validate_host_msg(&msg, state).is_err(),
            "BootConfig should be rejected in {state:?}"
        );
    }
}

#[test]
fn shutdown_accepted_in_all_states() {
    for state in [
        HostState::Created,
        HostState::Booting,
        HostState::VsockConnected,
        HostState::Handshaking,
        HostState::Running,
        HostState::ShuttingDown,
        HostState::Stopped,
        HostState::Error,
    ] {
        assert!(
            validate_host_msg(&HostToGuest::Shutdown, state).is_ok(),
            "Shutdown should be accepted in {state:?}"
        );
    }
}

#[test]
fn resize_only_in_running() {
    let msg = HostToGuest::Resize { cols: 80, rows: 24 };
    assert!(validate_host_msg(&msg, HostState::Running).is_ok());
    assert!(validate_host_msg(&msg, HostState::Booting).is_err());
    assert!(validate_host_msg(&msg, HostState::VsockConnected).is_err());
}

#[test]
fn exec_only_in_running() {
    let msg = HostToGuest::Exec {
        id: 1,
        command: "ls".into(),
    };
    assert!(validate_host_msg(&msg, HostState::Running).is_ok());
    assert!(validate_host_msg(&msg, HostState::Handshaking).is_err());
}

#[test]
fn ping_only_in_running() {
    assert!(validate_host_msg(&HostToGuest::Ping { epoch_secs: 0 }, HostState::Running).is_ok());
    assert!(validate_host_msg(&HostToGuest::Ping { epoch_secs: 0 }, HostState::Created).is_err());
}

#[test]
fn set_env_in_handshaking() {
    let msg = HostToGuest::SetEnv {
        key: "TERM".into(),
        value: "xterm-256color".into(),
    };
    assert!(validate_host_msg(&msg, HostState::Handshaking).is_ok());
    assert!(validate_host_msg(&msg, HostState::Running).is_err());
    assert!(validate_host_msg(&msg, HostState::Booting).is_err());
}

#[test]
fn boot_config_done_in_handshaking() {
    assert!(validate_host_msg(&HostToGuest::BootConfigDone, HostState::Handshaking).is_ok());
    assert!(validate_host_msg(&HostToGuest::BootConfigDone, HostState::Running).is_err());
    assert!(validate_host_msg(&HostToGuest::BootConfigDone, HostState::Booting).is_err());
}

#[test]
fn file_write_in_handshaking_and_running() {
    let msg = HostToGuest::FileWrite {
        id: 1,
        path: "/root/.gemini/settings.json".into(),
        data: b"{}".to_vec(),
        mode: 0o644,
    };
    assert!(validate_host_msg(&msg, HostState::Handshaking).is_ok());
    assert!(validate_host_msg(&msg, HostState::Running).is_ok());
    assert!(validate_host_msg(&msg, HostState::Booting).is_err());
}

// -------------------------------------------------------------------
// validate_guest_msg (host validates untrusted guest messages)
// -------------------------------------------------------------------

#[test]
fn ready_in_vsock_connected() {
    let msg = GuestToHost::Ready {
        version: "0.3.0".into(),
    };
    assert!(validate_guest_msg(&msg, HostState::VsockConnected).is_ok());
}

#[test]
fn ready_rejected_in_other_states() {
    let msg = GuestToHost::Ready {
        version: "0.3.0".into(),
    };
    for state in [
        HostState::Created,
        HostState::Booting,
        HostState::Handshaking,
        HostState::Running,
    ] {
        assert!(
            validate_guest_msg(&msg, state).is_err(),
            "Ready should be rejected in {state:?}"
        );
    }
}

#[test]
fn boot_ready_in_handshaking() {
    assert!(validate_guest_msg(&GuestToHost::BootReady, HostState::Handshaking).is_ok());
}

#[test]
fn boot_ready_rejected_in_other_states() {
    for state in [
        HostState::Created,
        HostState::Booting,
        HostState::VsockConnected,
        HostState::Running,
    ] {
        assert!(
            validate_guest_msg(&GuestToHost::BootReady, state).is_err(),
            "BootReady should be rejected in {state:?}"
        );
    }
}

#[test]
fn exec_done_in_running() {
    let msg = GuestToHost::ExecDone {
        id: 1,
        exit_code: 0,
    };
    assert!(validate_guest_msg(&msg, HostState::Running).is_ok());
}

#[test]
fn pong_in_running() {
    assert!(validate_guest_msg(&GuestToHost::Pong, HostState::Running).is_ok());
}

#[test]
fn pong_rejected_in_booting() {
    assert!(validate_guest_msg(&GuestToHost::Pong, HostState::Booting).is_err());
}

// -------------------------------------------------------------------
// Display + Serde
// -------------------------------------------------------------------

#[test]
fn display() {
    assert_eq!(format!("{}", HostState::Running), "Running");
    assert_eq!(format!("{}", HostState::Created), "Created");
}

#[test]
fn serde_roundtrip() {
    let state = HostState::Running;
    let json = serde_json::to_string(&state).unwrap();
    let decoded: HostState = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, state);
}

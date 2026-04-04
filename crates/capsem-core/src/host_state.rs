//! Host-side VM lifecycle state machine.
//!
//! All state tracking and message validation lives on the host. The guest
//! agent is treated as untrusted (zero-trust model) and has no state machine
//! of its own -- all protocol enforcement happens here.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::{GuestToHost, HostToGuest};

// ---------------------------------------------------------------------------
// Host state enum
// ---------------------------------------------------------------------------

/// Host-side VM lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HostState {
    /// VM created but not yet started.
    Created,
    /// VM started, serial console active, waiting for vsock.
    Booting,
    /// Vsock terminal + control ports connected from guest.
    VsockConnected,
    /// Boot handshake: Ready received, BootConfig sent, awaiting BootReady.
    Handshaking,
    /// BootReady received, terminal interactive.
    Running,
    /// Graceful shutdown requested.
    ShuttingDown,
    /// VM stopped.
    Stopped,
    /// Unrecoverable error.
    Error,
}

impl HostState {
    /// Returns the set of states reachable from this state.
    pub fn valid_transitions(&self) -> &'static [HostState] {
        match self {
            Self::Created => &[Self::Booting, Self::Error],
            Self::Booting => &[Self::VsockConnected, Self::Error, Self::Stopped],
            Self::VsockConnected => &[Self::Handshaking, Self::Error, Self::Stopped],
            Self::Handshaking => &[Self::Running, Self::Error, Self::Stopped],
            Self::Running => &[Self::ShuttingDown, Self::Error, Self::Stopped],
            Self::ShuttingDown => &[Self::Stopped, Self::Error],
            Self::Stopped => &[],
            Self::Error => &[Self::Stopped],
        }
    }
}

impl std::fmt::Display for HostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

// ---------------------------------------------------------------------------
// Generic state machine
// ---------------------------------------------------------------------------

/// A recorded state transition.
#[derive(Debug, Clone)]
pub struct Transition<S: Copy> {
    pub from: S,
    pub to: S,
    pub trigger: &'static str,
    pub duration_in_from: Duration,
}

/// Generic state machine with validated transitions and timing history.
pub struct StateMachine<S: Copy + PartialEq + std::fmt::Debug + 'static> {
    current: S,
    entered_at: Instant,
    history: Vec<Transition<S>>,
    validate_fn: fn(&S) -> &'static [S],
}

impl<S: Copy + PartialEq + Eq + std::fmt::Debug + 'static> StateMachine<S> {
    /// Create a new state machine in `initial` state.
    pub fn new(initial: S, validate_fn: fn(&S) -> &'static [S]) -> Self {
        Self {
            current: initial,
            entered_at: Instant::now(),
            history: Vec::new(),
            validate_fn,
        }
    }

    /// Transition to a new state. Returns Err if the transition is invalid.
    pub fn transition(
        &mut self,
        to: S,
        trigger: &'static str,
    ) -> Result<&Transition<S>> {
        let valid = (self.validate_fn)(&self.current);
        if !valid.contains(&to) {
            bail!(
                "invalid transition {:?} -> {:?} (trigger: {trigger})",
                self.current,
                to
            );
        }
        let now = Instant::now();
        let duration = now.duration_since(self.entered_at);
        self.history.push(Transition {
            from: self.current,
            to,
            trigger,
            duration_in_from: duration,
        });
        self.current = to;
        self.entered_at = now;
        Ok(self.history.last().unwrap())
    }

    /// Current state.
    pub fn state(&self) -> S {
        self.current
    }

    /// Time spent in current state.
    pub fn elapsed(&self) -> Duration {
        self.entered_at.elapsed()
    }

    /// Full transition history.
    pub fn history(&self) -> &[Transition<S>] {
        &self.history
    }

    /// Format history as structured log lines.
    pub fn format_perf_log(&self) -> String {
        let mut out = String::new();
        for t in &self.history {
            out.push_str(&format!(
                "{:?} -> {:?} ({}) {:.1}ms\n",
                t.from,
                t.to,
                t.trigger,
                t.duration_in_from.as_secs_f64() * 1000.0
            ));
        }
        out
    }
}

/// Host-side state machine.
pub type HostStateMachine = StateMachine<HostState>;

impl HostStateMachine {
    pub fn new_host() -> Self {
        StateMachine::new(HostState::Created, |s| s.valid_transitions())
    }
}

// ---------------------------------------------------------------------------
// Per-state message validation (host-side only, zero-trust on guest)
// ---------------------------------------------------------------------------

/// Validate a host->guest message against the current host state.
/// Prevents the host from sending messages that are invalid for the
/// current lifecycle stage.
pub fn validate_host_msg(msg: &HostToGuest, state: HostState) -> Result<()> {
    match (msg, state) {
        // Boot handshake messages: BootConfig, SetEnv, FileWrite, BootConfigDone
        (HostToGuest::BootConfig { .. }, HostState::Handshaking) => Ok(()),
        (HostToGuest::BootConfig { .. }, _) => {
            bail!("BootConfig only valid in Handshaking, got {state:?}")
        }
        (HostToGuest::SetEnv { .. }, HostState::Handshaking) => Ok(()),
        (HostToGuest::SetEnv { .. }, _) => {
            bail!("SetEnv only valid in Handshaking, got {state:?}")
        }
        (HostToGuest::BootConfigDone, HostState::Handshaking) => Ok(()),
        (HostToGuest::BootConfigDone, _) => {
            bail!("BootConfigDone only valid in Handshaking, got {state:?}")
        }
        (HostToGuest::FileWrite { .. }, HostState::Handshaking) => Ok(()),
        (HostToGuest::Shutdown, _) => Ok(()),
        (_, HostState::Running) => Ok(()),
        (msg, _) => bail!("{msg:?} only valid in Running, got {state:?}"),
    }
}

/// Validate a guest->host message against the current host state.
/// The guest is untrusted -- the host decides whether to accept or
/// drop each incoming message based on its own state machine.
pub fn validate_guest_msg(msg: &GuestToHost, state: HostState) -> Result<()> {
    match (msg, state) {
        (GuestToHost::Ready { .. }, HostState::VsockConnected) => Ok(()),
        (GuestToHost::Ready { .. }, _) => {
            bail!("Ready only valid in VsockConnected, got {state:?}")
        }
        (GuestToHost::BootReady, HostState::Handshaking) => Ok(()),
        (GuestToHost::BootReady, _) => bail!("BootReady only valid in Handshaking, got {state:?}"),
        (_, HostState::Running) => Ok(()),
        (msg, _) => bail!("{msg:?} only valid in Running, got {state:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
        let msg = HostToGuest::BootConfig { epoch_secs: 1000 };
        assert!(validate_host_msg(&msg, HostState::Handshaking).is_ok());
    }

    #[test]
    fn boot_config_rejected_in_other_states() {
        let msg = HostToGuest::BootConfig { epoch_secs: 1000 };
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
        assert!(validate_host_msg(&HostToGuest::Ping, HostState::Running).is_ok());
        assert!(validate_host_msg(&HostToGuest::Ping, HostState::Created).is_err());
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
}

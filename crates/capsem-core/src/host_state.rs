//! Host-side VM lifecycle state machine.
//!
//! All state tracking and message validation lives on the host. The guest
//! agent is treated as untrusted (zero-trust model) and has no state machine
//! of its own -- all protocol enforcement happens here.

use std::time::{Duration, Instant};

use anyhow::{bail, Result};
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
    pub fn transition(&mut self, to: S, trigger: &'static str) -> Result<&Transition<S>> {
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
mod tests;

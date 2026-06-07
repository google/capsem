pub mod boot;
pub mod config;
pub mod registry;
pub mod terminal;
pub mod vsock;

/// Unified VM lifecycle state.
///
/// Covers both app-level states (before a VM exists) and VZ machine states.
/// String representation matches the IPC protocol -- frontend uses these exact
/// lowercase strings for status display and color mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VmState {
    NotCreated,
    Downloading,
    Booting,
    Starting,
    Running,
    Paused,
    Pausing,
    Resuming,
    Stopping,
    Stopped,
    Saving,
    Restoring,
    Error,
    Unknown,
}

impl VmState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotCreated => "not created",
            Self::Downloading => "downloading",
            Self::Booting => "booting",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Pausing => "pausing",
            Self::Resuming => "resuming",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Saving => "saving",
            Self::Restoring => "restoring",
            Self::Error => "error",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "not created" => Self::NotCreated,
            "downloading" => Self::Downloading,
            "booting" => Self::Booting,
            "starting" => Self::Starting,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "pausing" => Self::Pausing,
            "resuming" => Self::Resuming,
            "stopping" => Self::Stopping,
            "stopped" => Self::Stopped,
            "saving" => Self::Saving,
            "restoring" => Self::Restoring,
            "error" => Self::Error,
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for VmState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_states_roundtrip() {
        let states = [
            VmState::NotCreated,
            VmState::Downloading,
            VmState::Booting,
            VmState::Starting,
            VmState::Running,
            VmState::Paused,
            VmState::Pausing,
            VmState::Resuming,
            VmState::Stopping,
            VmState::Stopped,
            VmState::Saving,
            VmState::Restoring,
            VmState::Error,
            VmState::Unknown,
        ];
        for state in states {
            assert_eq!(VmState::parse(state.as_str()), state);
        }
    }

    #[test]
    fn unknown_input_maps_to_unknown() {
        assert_eq!(VmState::parse("garbage"), VmState::Unknown);
        assert_eq!(VmState::parse(""), VmState::Unknown);
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(format!("{}", VmState::Running), "running");
        assert_eq!(format!("{}", VmState::NotCreated), "not created");
    }
}

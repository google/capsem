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

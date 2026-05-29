use crate::fixture::fixture_state;
use crate::model::{Attention, ServiceStatus, SessionLifecycle};
use crate::ui::render_snapshot;

#[test]
fn fixture_models_global_service_state_and_session_indicators() {
    let state = fixture_state();

    assert_eq!(state.service.status, ServiceStatus::Online);
    assert_eq!(
        state.sessions[0].lifecycle,
        SessionLifecycle::Working,
        "active desktop should be working in the fixture"
    );
    assert!(
        state.sessions[1].attention.contains(&Attention::Bell),
        "fixture needs one terminal-bell attention indicator"
    );
}

#[test]
fn snapshot_contains_light_bar_tabs_and_active_desktop() {
    let snapshot = render_snapshot(&fixture_state(), 100, 24).expect("render snapshot");

    assert!(snapshot.contains("svc=online latency=18ms"));
    assert!(snapshot.contains("Profile V2"));
    assert!(snapshot.contains("Linux OS !"));
    assert!(snapshot.contains("repo: github.com/google/capsem"));
    assert!(snapshot.contains("< > switch desktop"));
}

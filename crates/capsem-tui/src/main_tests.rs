use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{
    apply_refresh_event, terminal_event_closes_connection, ConnectedTerminal, RefreshBridge,
    RefreshEvent,
};
use capsem_tui::app::App;
use capsem_tui::fixture::offline_state;
use capsem_tui::model::ServiceStatus;
use capsem_tui::terminal::TerminalEvent;

#[test]
fn terminal_failure_status_clears_connected_session() {
    let connected = ConnectedTerminal {
        session_id: "vm-1".to_string(),
        cols: 80,
        rows: 23,
    };
    let event = TerminalEvent::Status {
        session_id: "vm-1".to_string(),
        status: "connect failed: refused".to_string(),
    };

    assert!(terminal_event_closes_connection(&event, Some(&connected)));
}

#[test]
fn terminal_connected_status_keeps_connected_session() {
    let connected = ConnectedTerminal {
        session_id: "vm-1".to_string(),
        cols: 80,
        rows: 23,
    };
    let event = TerminalEvent::Status {
        session_id: "vm-1".to_string(),
        status: "connected".to_string(),
    };

    assert!(!terminal_event_closes_connection(&event, Some(&connected)));
}

#[test]
fn refresh_bridge_keeps_slow_gateway_load_off_input_thread() {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let bridge = RefreshBridge::spawn_with_loader(move || {
        started_tx.send(()).expect("signal refresh start");
        release_rx.recv().expect("wait for test release");
        Ok(offline_state())
    });

    let started = Instant::now();
    bridge.request();
    assert!(
        started.elapsed() < Duration::from_millis(20),
        "requesting a refresh must not block the TUI input/render thread"
    );
    started_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("refresh worker should start in the background");

    bridge.request();
    assert!(
        started_rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "a slow refresh must not let periodic ticks queue duplicate gateway loads"
    );
    assert!(bridge.drain_events().is_empty());

    release_tx.send(()).expect("release refresh worker");
    let events = wait_for_refresh_events(&bridge);
    assert_eq!(events.len(), 1);
    assert!(matches!(events.first(), Some(RefreshEvent::Loaded(_))));
}

#[test]
fn failed_refresh_event_marks_service_offline_without_blocking() {
    let mut state = offline_state();
    state.service.reconnect_attempt = None;
    let mut app = App::new(state);
    let changed = apply_refresh_event(&mut app, RefreshEvent::Failed("timeout".to_string()));

    assert!(changed);
    assert_eq!(app.state().service.status, ServiceStatus::Offline);
    assert_eq!(app.state().service.reconnect_attempt, Some(1));
}

fn wait_for_refresh_events(bridge: &RefreshBridge) -> Vec<RefreshEvent> {
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        let events = bridge.drain_events();
        if !events.is_empty() {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for refresh event"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{
    apply_refresh_event, handle_input_event_batch, terminal_event_closes_connection,
    ConnectedTerminal, RefreshBridge, RefreshEvent,
};
use capsem_tui::app::App;
use capsem_tui::fixture::offline_state;
use capsem_tui::model::ServiceStatus;
use capsem_tui::terminal::TerminalEvent;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

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

#[test]
fn input_event_batch_drains_ready_events_before_redraw() {
    let (queued_tx, queued_rx) = mpsc::channel();
    for ch in ['b', 'c', 'd', 'e', 'f', 'g', 'h', 'i'] {
        queued_tx
            .send(Ok(key_event(ch)))
            .expect("queue ready terminal input");
    }
    drop(queued_tx);

    let mut handled = Vec::new();
    let should_exit = handle_input_event_batch(Ok(key_event('a')), &queued_rx, |event| {
        handled.push(key_char(event));
        Ok(false)
    })
    .expect("drain ready input batch");

    assert!(!should_exit);
    assert_eq!(handled, vec!['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i']);
    assert!(
        queued_rx.try_recv().is_err(),
        "all ready input must be handled before the TUI redraws"
    );
}

#[test]
fn input_event_batch_is_bounded_so_rendering_cannot_starve() {
    let (queued_tx, queued_rx) = mpsc::channel();
    for _ in 1..=(super::MAX_INPUT_EVENTS_PER_TICK + 10) {
        queued_tx
            .send(Ok(key_event('x')))
            .expect("queue terminal input flood");
    }
    drop(queued_tx);

    let mut handled = 0usize;
    let should_exit = handle_input_event_batch(Ok(key_event('x')), &queued_rx, |_event| {
        handled += 1;
        Ok(false)
    })
    .expect("handle bounded input batch");

    assert!(!should_exit);
    assert_eq!(handled, super::MAX_INPUT_EVENTS_PER_TICK);
    assert!(
        queued_rx.try_recv().is_ok(),
        "input floods must yield back to the render/gateway loop after one bounded batch"
    );
}

#[test]
fn input_event_batch_stops_on_exit_without_draining_extra_events() {
    let (queued_tx, queued_rx) = mpsc::channel();
    queued_tx
        .send(Ok(key_event('b')))
        .expect("queue exit event");
    queued_tx
        .send(Ok(key_event('c')))
        .expect("queue event after exit");

    let mut handled = Vec::new();
    let should_exit = handle_input_event_batch(Ok(key_event('a')), &queued_rx, |event| {
        let ch = key_char(event);
        handled.push(ch);
        Ok(ch == 'b')
    })
    .expect("stop ready input batch");

    assert!(should_exit);
    assert_eq!(handled, vec!['a', 'b']);
    assert!(
        queued_rx.try_recv().is_ok(),
        "events after an exit action must remain untouched"
    );
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

fn key_event(ch: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
}

fn key_char(event: Event) -> char {
    let Event::Key(key) = event else {
        panic!("expected key event");
    };
    let KeyCode::Char(ch) = key.code else {
        panic!("expected char key");
    };
    ch
}

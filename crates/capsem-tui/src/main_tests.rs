use super::{refresh_state, terminal_event_closes_connection, ConnectedTerminal};
use capsem_tui::app::{App, AppOverlay, ControlAction};
use capsem_tui::fixture::fixture_state;
use capsem_tui::gateway_provider::GatewayProvider;
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
fn refresh_failure_drops_stale_sessions() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind unused port");
    let addr = listener.local_addr().expect("unused addr");
    drop(listener);
    let provider = GatewayProvider::new(format!("http://{addr}"));
    let mut app = App::new(fixture_state());

    assert!(refresh_state(&mut app, Some(&provider)));

    assert_eq!(app.state().service.status, ServiceStatus::Offline);
    assert!(app.state().sessions.is_empty());
    assert!(app.state().profiles.is_empty());
    assert_eq!(app.overlay(), AppOverlay::Confirm);
    assert_eq!(app.pending_action(), Some(&ControlAction::StartService));
}

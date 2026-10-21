use crate::app::{App, AppOverlay};
use crate::fixture::fixture_state;
use crate::terminal::{TerminalEvent, TerminalSurface};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn guest_cursor_is_visible_in_the_terminal_and_hidden_behind_overlays() {
    let mut app = App::new(fixture_state());
    let id = app.state().active_session_id.clone();
    let mut surface = TerminalSurface::new();
    let state = serde_json::from_value(serde_json::json!({
        "revision": 1, "reset": true, "cols": 40, "rows": 9,
        "cursor": {"row": 4, "col": 3, "hidden": false},
        "modes": {"alternate_screen": false, "application_cursor": false, "application_keypad": false, "bracketed_paste": false, "mouse_protocol": "None", "mouse_encoding": "Default"},
        "title": "", "icon_name": "", "bell_count": 0, "last_notification": null, "events": []
    })).unwrap();
    surface.apply(TerminalEvent::State { session_id: id, state });
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|frame| crate::ui::render_app(frame, &app, Some(&surface)))
        .unwrap();
    assert!(terminal.backend().cursor_visible());
    assert_eq!(terminal.backend().cursor_position(), (3, 4).into());
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::ALT));
    assert_eq!(app.overlay(), AppOverlay::Help);
    terminal
        .draw(|frame| crate::ui::render_app(frame, &app, Some(&surface)))
        .unwrap();
    assert!(!terminal.backend().cursor_visible());
}

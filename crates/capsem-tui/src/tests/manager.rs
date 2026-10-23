use super::*;

#[test]
fn manager_shortcuts_new_session_and_context_actions_are_explicit() {
    for shortcut in ['m', '0'] {
        let mut app = App::new(fixture_state());
        app.handle_key(key(KeyCode::Char(shortcut), KeyModifiers::ALT));
        assert_eq!(app.overlay(), AppOverlay::Home);
        let screen = render_app_snapshot(&app, 140, 40).unwrap();
        for label in [
            "Manager: Alt+M",
            "Help: Alt+?",
            "New session [N]",
            "Alt+T: Stop",
            "Alt+F: Fork",
            "Alt+D: Delete",
        ] {
            assert!(screen.contains(label), "missing {label}");
        }
        assert!(!screen.contains("Alt+b browser"));
        app.handle_key(key(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(app.overlay(), AppOverlay::Create);
    }
}

#[test]
fn manager_selects_sessions_and_confirms_lifecycle_actions() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('m'), KeyModifiers::ALT));
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.state().active_session_id, "linux-os");
    app.handle_key(key(KeyCode::Char('t'), KeyModifiers::ALT));
    assert_eq!(app.overlay(), AppOverlay::Confirm);
    assert!(matches!(app.pending_action(), Some(ControlAction::Stop { id, .. }) if id == "linux-os"));
}

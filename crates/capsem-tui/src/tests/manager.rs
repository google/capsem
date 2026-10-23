use super::*;

#[test]
fn footer_anchors_latency_left_and_shortcuts_right_without_overlap() {
    for width in [150, 80, 40] {
        let screen = render_snapshot(&fixture_state(), width, 24).unwrap();
        let footer = screen.lines().last().unwrap();
        assert!(footer.starts_with("  18ms●"));
        assert!(footer.ends_with("Help: Alt+?"));
    }
}

#[test]
fn browser_handoff_is_single_use_across_polls_and_explicitly_retryable_on_failure() {
    let mut state = fixture_state();
    let id = state.active_session_id.clone();
    let request = capsem_sdk::models::BrowserRequest {
        owner_generation: Some("18f59e33-8af6-4992-81d9-71e10873af9a".into()),
        id: 1,
        url: "https://example.org/login".into(),
    };
    state.sessions[0].browser_request = Some(request.clone());
    let mut app = App::new(state.clone());
    let open = key(KeyCode::Char('b'), KeyModifiers::ALT);
    assert!(matches!(
        app.handle_key(open),
        AppAction::Invoke(ControlAction::OpenBrowser { .. })
    ));
    assert_eq!(app.handle_key(open), AppAction::Consumed);
    app.replace_state(state.clone());
    assert!(!render_app_snapshot(&app, 150, 42).unwrap().contains("Alt+b browser"));
    app.complete_browser_open(&id, &request, false);
    assert!(render_app_snapshot(&app, 150, 42).unwrap().contains("Alt+b browser"));
    assert!(matches!(
        app.handle_key(open),
        AppAction::Invoke(ControlAction::OpenBrowser { .. })
    ));
    app.complete_browser_open(&id, &request, true);
    app.replace_state(state);
    assert_eq!(app.handle_key(open), AppAction::Consumed);
    let mut next = fixture_state();
    let newer = capsem_sdk::models::BrowserRequest {
        owner_generation: Some("18f59e33-8af6-4992-81d9-71e10873af9a".into()),
        id: 2,
        url: "https://example.org/next".into(),
    };
    next.sessions[0].browser_request = Some(newer);
    app.replace_state(next);
    assert!(matches!(
        app.handle_key(open),
        AppAction::Invoke(ControlAction::OpenBrowser { .. })
    ));
    app.complete_browser_open(&id, &request, false);
    assert_eq!(
        app.handle_key(open),
        AppAction::Consumed,
        "stale failure cannot restore an older request"
    );
}

#[test]
fn chooser_has_spaced_table_and_bottom_legend_and_direct_arrow_navigation() {
    for (width, height) in [(150, 42), (80, 24)] {
        let mut app = App::new(fixture_state());
        app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
        let request = app.take_catalog_request().unwrap();
        app.apply_image_catalog(request, Ok(image_creation::catalog()));
        app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.create_draft().unwrap().selected_image().unwrap().name, "code");
        let buffer = render_app_test_buffer(&app, width, height).unwrap();
        let (_, name_y) = find_cell(&buffer, "Name");
        let (_, heading_y) = find_cell(&buffer, "Image list");
        let (_, legend_y) = find_cell(&buffer, "Enter creates");
        assert!(heading_y >= name_y + 2);
        assert!(legend_y >= height - 7);
        let screen = render_app_snapshot(&app, width, height).unwrap();
        assert!(!screen.contains("CAPSEM"));
        assert!(!screen.contains("new session"));
        for column in ["Owner", "Image", "Status", "Description"] {
            assert!(screen.contains(column));
        }
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
            AppAction::Invoke(ControlAction::CreateSession { image: Some(_), .. })
        ));
    }
}

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

use super::*;

#[test]
fn stopped_session_renders_start_prompt_and_grey_tab() {
    let mut state = fixture_state();
    state.sessions[0].lifecycle = SessionLifecycle::Idle;
    state.sessions[0].available_actions = vec![VmAction::Start, VmAction::Delete];

    let snapshot = render_snapshot(&state, 100, 24).expect("render stopped snapshot");
    assert!(
        snapshot.contains("Press Enter to start"),
        "stopped sessions should render an explicit recovery affordance instead of a blank pane"
    );
    assert!(snapshot.contains("stopped"));

    let buffer = render_test_buffer(&state, 100, 24).expect("render stopped buffer");
    let row = buffer.area.height - 1;
    let stopped_number = find_cell_x(&buffer, row, "1  Session V2");
    let stopped_label = stopped_number + 3;

    assert_eq!(buffer_cell(&buffer, stopped_number, row).bg, grey());
    assert_eq!(buffer_cell(&buffer, stopped_label, row).fg, grey());
    assert!(
        buffer_cell(&buffer, stopped_label, row)
            .modifier
            .contains(Modifier::DIM),
        "stopped tab labels should read as inactive"
    );
}

#[test]
fn enter_starts_stopped_active_session_instead_of_forwarding_to_terminal() {
    let mut state = fixture_state();
    state.sessions[0].lifecycle = SessionLifecycle::Idle;
    state.sessions[0].available_actions = vec![VmAction::Start, VmAction::Delete];
    let mut app = App::new(state);

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::Start {
            id: "vm-2".to_string(),
            label: "Session V2".to_string()
        })
    );
}

#[test]
fn unresumable_session_blocks_resume_and_explains_recreate() {
    let mut state = fixture_state();
    state.sessions[0].lifecycle = SessionLifecycle::Idle;
    // The service said no without saying why: the TUI supplies the reason.
    state.sessions[0].can_resume = false;
    state.sessions[0].resume_blocked_reason = None;
    state.sessions[0].attention = vec![Attention::CredentialIssue];
    let mut app = App::new(state);
    assert!(app.select_session_by_id("vm-2"));

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render unresumable session");
    assert!(snapshot.contains("cannot resume: session state is not resumable"));
    assert!(!snapshot.contains("Press Enter to resume"));
    assert!(snapshot.contains("Press Enter to create a replacement"));
    assert!(snapshot.contains("Alt+d deletes this session"));

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Create);
    assert_eq!(app.create_draft().expect("create draft").name, "vm-1".to_string());

    app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(
        app.handle_key(key(KeyCode::Char('r'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.pending_action(), None);
    assert_eq!(
        app.state().service.control_message.as_deref(),
        Some("cannot resume: session state is not resumable")
    );
}

#[test]
fn unresumable_sessions_are_hidden_from_tabs_but_stay_in_vm_list() {
    let mut state = fixture_state();
    state.sessions[0].lifecycle = SessionLifecycle::Idle;
    state.sessions[0].can_resume = false;
    state.sessions[0].resume_blocked_reason = Some("session overlay is missing".to_string());
    state.sessions[0].attention = vec![Attention::CredentialIssue];
    let mut app = App::new(state);

    assert_eq!(
        app.state().active_session_id,
        "linux-os",
        "startup focus should move to the first resumable tab instead of an unresumable session"
    );
    let snapshot = render_app_snapshot(&app, 100, 24).expect("render filtered tabs");
    assert!(!snapshot.contains("Session V2"));
    assert!(snapshot.contains("1  Linux OS!"));

    assert_eq!(
        app.handle_key(key(KeyCode::Char('l'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    let list_snapshot = render_app_snapshot(&app, 120, 30).expect("render session inventory");
    assert!(list_snapshot.contains("Session V2"));

    assert_eq!(
        app.handle_key(key(KeyCode::Char('1'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.state().active_session_id,
        "linux-os",
        "tab number 1 should map to the first visible tab, not the hidden unresumable session"
    );
}

#[test]
fn resume_action_is_only_available_for_stopped_or_suspended_sessions() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('r'), KeyModifiers::ALT)),
        AppAction::Forward,
        "running active session should not map Alt+r to resume"
    );

    let mut state = fixture_state();
    state.active_session_id = "linux-os".to_string();
    state.sessions[1].lifecycle = SessionLifecycle::Suspended;
    state.sessions[1].available_actions = vec![VmAction::Resume, VmAction::Delete];
    app = App::new(state);

    assert_eq!(
        app.handle_key(key(KeyCode::Char('r'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.pending_action(),
        Some(&ControlAction::Resume {
            id: "linux-os".to_string(),
            label: "Linux OS".to_string()
        })
    );
}

#[test]
fn suspend_action_requires_service_pause_permission() {
    let mut app = App::new(fixture_state());
    assert_eq!(
        app.handle_key(key(KeyCode::Char('s'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.pending_action(),
        Some(&ControlAction::Suspend {
            id: "vm-2".to_string(),
            label: "Session V2".to_string()
        })
    );

    let mut state = fixture_state();
    state.sessions[0].persistent = false;
    state.sessions[0].available_actions = vec![VmAction::Stop, VmAction::Delete];
    app = App::new(state);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('s'), KeyModifiers::ALT)),
        AppAction::Forward,
        "sessions without pause permission cannot be suspended through the service"
    );
}

#[test]
fn suspend_progress_owns_the_main_terminal_surface() {
    let mut app = App::new(fixture_state());
    app.set_control_progress("suspending");

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render suspend progress");

    assert!(snapshot.contains("suspending..."));
    assert!(
        !snapshot.contains("connecting terminal vm-2"),
        "suspend progress should be visible in the main pane, not only the status bar"
    );
}

#[test]
fn checkpoint_action_is_alt_c_and_uses_checkpoint_label() {
    let mut app = App::new(fixture_state());
    assert_eq!(
        app.handle_key(key(KeyCode::Char('c'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.pending_action(),
        Some(&ControlAction::Checkpoint {
            id: "vm-2".to_string(),
            label: "Session V2".to_string()
        })
    );

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render checkpoint confirm");
    assert!(snapshot.contains("checkpoint"));
    assert!(snapshot.contains("Session V2"));
}

#[test]
fn tui_story_suite_covers_create_stop_resume_navigation_help_latency_and_human_labels() {
    let mut state = state_from_status_json_for_test(gateway_status_body(), std::time::Duration::from_millis(24))
        .expect("parse gateway status for TUI story");
    state.sessions[1].can_resume = true;
    state.sessions[1].available_actions = vec![VmAction::Resume, VmAction::Delete];
    let mut app = App::new(state);

    let initial = render_app_snapshot(&app, 100, 24).expect("render initial TUI story");
    assert!(initial.contains("24ms"), "measured gateway latency is visible");
    assert!(initial.contains("main-session"), "named session is visible");
    assert!(
        !initial.contains("vm-1"),
        "the internal session id must not replace a human session name"
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Right, KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.state().active_session_id, "vm-2");
    assert_eq!(
        app.handle_key(key(KeyCode::Left, KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.state().active_session_id, "vm-1");

    assert_eq!(
        app.handle_key(key(KeyCode::Char('?'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    let help = render_app_snapshot(&app, 100, 24).expect("render TUI story help");
    assert!(help.contains("Alt+Right"));
    assert!(help.contains("stop active session"));
    assert_eq!(
        app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE)),
        AppAction::Consumed
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Char('t'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    let stop = render_app_snapshot(&app, 100, 24).expect("render TUI story stop");
    assert!(stop.contains("main-session"));
    assert!(!stop.contains("vm-1"), "confirmation target is the human name");
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::Stop {
            id: "vm-1".to_string(),
            label: "main-session".to_string()
        }),
        "the stop request still routes by immutable session id"
    );

    let mut stopped = app.state().clone();
    stopped.sessions[0].lifecycle = SessionLifecycle::Idle;
    stopped.sessions[0].can_resume = true;
    stopped.sessions[0].available_actions = vec![VmAction::Start, VmAction::Delete];
    app.replace_state(stopped);
    let stopped = render_app_snapshot(&app, 100, 24).expect("render stopped TUI story");
    assert!(stopped.contains("main-session"));
    assert!(!stopped.contains("vm-1"), "resume prompt uses the human name");
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::Start {
            id: "vm-1".to_string(),
            label: "main-session".to_string()
        }),
        "the resume request still routes by immutable session id"
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::CreateSession { name: None })
    );

    let mut created = app.state().clone();
    let mut session = created.sessions[0].clone();
    session.id = "77777777-7777-4777-8777-777777777777".to_string();
    session.title = "vm-3".to_string();
    session.lifecycle = SessionLifecycle::Working;
    created.sessions.push(session);
    app.focus_session_when_available("77777777-7777-4777-8777-777777777777");
    app.replace_state(created);
    let created = render_app_snapshot(&app, 100, 24).expect("render created TUI story");
    assert!(created.contains("vm-3"));
    assert!(!created.contains("77777777"));
}

#[test]
fn gateway_status_can_resume_false_blocks_tui_resume() {
    let state = state_from_status_json_for_test(
        r#"{
            "service": "running",
            "gateway_version": "test",
            "vm_count": 1,
            "resource_summary": null,
            "vms": [{
                "id": "stale-vm",
                "name": "Stale VM",
                "status": "Stopped",
                "persistent": true,
                "available_actions": [],
                "can_resume": false,
                "resume_blocked_reason": "session overlay is missing"
            }]
        }"#,
        std::time::Duration::from_millis(1),
    )
    .expect("parse service status");
    let mut app = App::new(state);

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render non-resumable session");
    assert!(snapshot.contains("session overlay is missing"));
    assert!(!snapshot.contains("Press Enter to resume"));
    assert_eq!(
        app.handle_key(key(KeyCode::Char('r'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.pending_action(), None);
}

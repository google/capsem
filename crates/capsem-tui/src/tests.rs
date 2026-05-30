use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app::{App, AppAction, AppOverlay, ControlAction};
use crate::fixture::{fixture_state, offline_state};
use crate::gateway_provider::{
    start_service_with_binary, state_from_status_json_for_test, GatewayProvider,
};
use crate::model::{Attention, ServiceStatus, SessionLifecycle};
use crate::ui::{render_app_snapshot, render_app_test_buffer, render_snapshot, render_test_buffer};

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

    assert!(snapshot.contains("  18ms●"));
    assert!(snapshot.contains("1  profile-v2"));
    assert!(snapshot.contains("2  linux-os!"));
    assert!(snapshot.contains("◷ 47m | # 38.4k | $ 0.21 | help: alt+?"));
    assert!(
        !snapshot.contains("github.com/google/capsem"),
        "repo metadata belongs in a popup or future status segment, not the empty terminal surface"
    );
    assert!(
        !snapshot.contains("┌"),
        "minimal UI should not render boxes"
    );
    assert!(
        !snapshot.contains("? help"),
        "help belongs in a popup, not persistent chrome"
    );
}

#[test]
fn no_session_status_bar_keeps_help_hint_on_the_right() {
    let mut state = fixture_state();
    state.active_session_id.clear();
    state.sessions.clear();

    let snapshot = render_snapshot(&state, 100, 24).expect("render empty snapshot");

    assert!(snapshot.contains("no session | help: alt+?"));
}

#[test]
fn offline_empty_state_asks_to_start_service_instead_of_create() {
    let mut app = App::new(offline_state());

    assert_eq!(app.overlay(), AppOverlay::Confirm);
    assert_eq!(app.pending_action(), Some(&ControlAction::StartService));
    assert_eq!(app.create_draft(), None);

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render offline start prompt");
    assert!(snapshot.contains("service offline"));
    assert!(snapshot.contains("Press Enter to start Capsem service"));
    assert!(snapshot.contains("start service"));
    assert!(
        !snapshot.contains("new session"),
        "offline service should ask to start before showing the create flow"
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::StartService)
    );
}

#[test]
fn degraded_empty_state_asks_to_start_service_instead_of_create() {
    let mut state = offline_state();
    state.service.status = ServiceStatus::Degraded;
    let app = App::new(state);

    assert_eq!(app.overlay(), AppOverlay::Confirm);
    assert_eq!(app.pending_action(), Some(&ControlAction::StartService));
    let snapshot = render_app_snapshot(&app, 100, 24).expect("render unavailable start prompt");
    assert!(snapshot.contains("service unavailable"));
    assert!(snapshot.contains("start service"));
}

#[test]
fn empty_state_opens_new_session_modal_with_gradient_logo() {
    let mut state = fixture_state();
    state.active_session_id.clear();
    state.sessions.clear();

    let app = App::new(state);

    assert_eq!(app.overlay(), AppOverlay::Create);
    assert_eq!(app.create_draft().expect("create draft").name, "tmp-1");
    let snapshot = render_app_snapshot(&app, 100, 24).expect("render empty create modal");
    assert!(snapshot.contains("CAPSEM"));
    assert!(snapshot.contains("new session"));

    let buffer = render_app_test_buffer(&app, 100, 24).expect("render logo buffer");
    let (logo_x, logo_y) = find_cell(&buffer, "CAPSEM");
    let first = buffer_cell(&buffer, logo_x, logo_y);
    let last = buffer_cell(&buffer, logo_x + 5, logo_y);
    assert_ne!(
        first.fg, last.fg,
        "logo letters should use a visible gradient, not one flat color"
    );
    assert!(first.modifier.contains(Modifier::BOLD));
    assert!(last.modifier.contains(Modifier::BOLD));
}

#[tokio::test]
async fn start_service_action_uses_local_capsem_binary_without_gateway_token() {
    let binary = if std::path::Path::new("/bin/true").exists() {
        std::path::Path::new("/bin/true")
    } else {
        std::path::Path::new("/usr/bin/true")
    };
    let outcome = start_service_with_binary(binary)
        .await
        .expect("start service command");

    assert_eq!(outcome.message, "service start requested");
    assert_eq!(outcome.focus_session, None);
}

#[test]
fn empty_create_modal_blocks_enter_when_profiles_are_unavailable() {
    let mut state = fixture_state();
    state.active_session_id.clear();
    state.sessions.clear();
    state.profiles.clear();
    let mut app = App::new(state);

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render empty create modal");
    assert!(snapshot.contains("profiles unavailable"));
    assert!(
        !snapshot.contains("▶  default"),
        "the TUI must not invent a default profile when profile discovery failed"
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Consumed,
        "create should be disabled until a real profile list is available"
    );
    assert_eq!(app.overlay(), AppOverlay::Create);
}

#[test]
fn tab_colors_use_selected_yellow_and_unselected_blue_only() {
    let buffer = render_test_buffer(&fixture_state(), 100, 24).expect("render buffer");
    let row = buffer.area.height - 1;
    let selected_number = find_cell_x(&buffer, row, "1  profile-v2");
    let selected_label = selected_number + 3;
    let other_number = find_cell_x(&buffer, row, "2  linux-os!");
    let other_label = other_number + 3;

    assert_eq!(buffer_cell(&buffer, selected_number, row).bg, yellow());
    assert_eq!(buffer_cell(&buffer, selected_label, row).fg, yellow());
    assert!(buffer_cell(&buffer, selected_number, row)
        .modifier
        .contains(Modifier::BOLD));

    assert_eq!(buffer_cell(&buffer, other_number, row).bg, blue());
    assert_eq!(buffer_cell(&buffer, other_label, row).fg, blue());
    assert!(
        !buffer_cell(&buffer, other_label, row)
            .modifier
            .contains(Modifier::BOLD),
        "only the selected tab label should be bold"
    );
}

#[test]
fn stopped_session_renders_resume_prompt_and_grey_tab() {
    let mut state = fixture_state();
    state.sessions[0].lifecycle = SessionLifecycle::Idle;

    let snapshot = render_snapshot(&state, 100, 24).expect("render stopped snapshot");
    assert!(
        snapshot.contains("Press Enter to resume"),
        "stopped sessions should render an explicit recovery affordance instead of a blank pane"
    );
    assert!(snapshot.contains("stopped"));

    let buffer = render_test_buffer(&state, 100, 24).expect("render stopped buffer");
    let row = buffer.area.height - 1;
    let stopped_number = find_cell_x(&buffer, row, "1  profile-v2");
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
fn enter_resumes_stopped_active_session_instead_of_forwarding_to_terminal() {
    let mut state = fixture_state();
    state.sessions[0].lifecycle = SessionLifecycle::Idle;
    let mut app = App::new(state);

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::Resume {
            name: "profile-v2".to_string()
        })
    );
}

#[test]
fn corrupted_profile_session_blocks_resume_and_explains_recreate() {
    let mut state = fixture_state();
    state.sessions[0].lifecycle = SessionLifecycle::Idle;
    state.sessions[0].profile_status = Some("corrupted".to_string());
    state.sessions[0].attention = vec![Attention::CredentialIssue];
    let mut app = App::new(state);
    assert!(app.select_session_by_id("profile-v2"));

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render corrupted profile session");
    assert!(snapshot.contains("cannot resume: profile pin is corrupted"));
    assert!(!snapshot.contains("Press Enter to resume"));
    assert!(snapshot.contains("Press Enter to create a replacement"));
    assert!(snapshot.contains("Alt+d deletes this VM"));

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Create);
    assert_eq!(
        app.create_draft().expect("create draft").name,
        "tmp-1".to_string()
    );

    app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(
        app.handle_key(key(KeyCode::Char('r'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.pending_action(), None);
    assert_eq!(
        app.state().service.control_message.as_deref(),
        Some("cannot resume: profile pin is corrupted; recreate from a signed profile")
    );
}

#[test]
fn corrupted_profile_sessions_are_hidden_from_tabs_but_stay_in_vm_list() {
    let mut state = fixture_state();
    state.sessions[0].lifecycle = SessionLifecycle::Idle;
    state.sessions[0].profile_status = Some("corrupted".to_string());
    state.sessions[0].attention = vec![Attention::CredentialIssue];
    let mut app = App::new(state);

    assert_eq!(
        app.state().active_session_id,
        "linux-os",
        "startup focus should move to the first resumable tab instead of a corrupt profile pin"
    );
    let snapshot = render_app_snapshot(&app, 100, 24).expect("render filtered tabs");
    assert!(!snapshot.contains("profile-v2"));
    assert!(snapshot.contains("1  linux-os!"));

    assert_eq!(
        app.handle_key(key(KeyCode::Char('l'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    let list_snapshot = render_app_snapshot(&app, 120, 30).expect("render session inventory");
    assert!(list_snapshot.contains("Profile V2"));
    assert!(list_snapshot.contains("corrupted"));

    assert_eq!(
        app.handle_key(key(KeyCode::Char('1'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.state().active_session_id,
        "linux-os",
        "tab number 1 should map to the first visible tab, not the hidden corrupt session"
    );
}

#[test]
fn keyboard_navigation_switches_sessions_without_stealing_plain_q() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('q'), KeyModifiers::NONE)),
        AppAction::Forward
    );
    assert_eq!(app.state().active_session_id, "profile-v2");

    assert_eq!(
        app.handle_key(key(KeyCode::Right, KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.state().active_session_id, "linux-os");

    assert_eq!(
        app.handle_key(key(KeyCode::Left, KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.state().active_session_id, "profile-v2");

    assert_eq!(
        app.handle_key(key(KeyCode::Char('2'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.state().active_session_id, "linux-os");

    assert_eq!(
        app.handle_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        AppAction::Forward
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Char('q'), KeyModifiers::ALT)),
        AppAction::Exit
    );
}

#[test]
fn app_can_start_focused_on_session_id_or_title() {
    let mut app = App::new(fixture_state());

    assert!(app.select_session_by_id("linux-os"));
    assert_eq!(app.state().active_session_id, "linux-os");

    assert!(app.select_session_by_id("Profile V2"));
    assert_eq!(app.state().active_session_id, "profile-v2");

    assert!(!app.select_session_by_id("missing-session"));
    assert_eq!(app.state().active_session_id, "profile-v2");
}

#[test]
fn replace_state_preserves_fresh_service_latency_measurement() {
    let mut initial = fixture_state();
    initial.service.latency = std::time::Duration::from_millis(1);
    let mut app = App::new(initial);

    let mut refreshed = fixture_state();
    refreshed.service.latency = std::time::Duration::from_millis(7);
    app.replace_state(refreshed);

    assert_eq!(
        app.state().service.latency,
        std::time::Duration::from_millis(7),
        "TUI should report the measured latency; latency stability belongs in the service hot path"
    );
}

#[test]
fn shell_commands_are_alt_owned() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Create);

    assert_eq!(
        app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE)),
        AppAction::Consumed
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Char('t'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.pending_action(),
        Some(&ControlAction::Stop {
            id: "profile-v2".to_string()
        })
    );
}

#[test]
fn create_overlay_selects_profile_and_edits_prefilled_name() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    let snapshot = render_app_snapshot(&app, 100, 24).expect("render create dialog");
    assert!(snapshot.contains("new session"));
    assert!(snapshot.contains("name"));
    assert!(snapshot.contains("tmp-1"));
    assert!(snapshot.contains("corp-default"));
    assert!(snapshot.contains("linux-builder"));
    assert!(snapshot.contains("active input"));

    assert_eq!(
        app.handle_key(key(KeyCode::Down, KeyModifiers::NONE)),
        AppAction::Consumed
    );
    let focused = render_app_test_buffer(&app, 100, 24).expect("render focused create dialog");
    let (name_x, name_y) = find_cell(&focused, "tmp-1");
    assert_eq!(buffer_cell(&focused, name_x, name_y).bg, selected_bg());
    let (profile_x, profile_y) = find_cell(&focused, "linux-builder");
    assert_eq!(
        buffer_cell(&focused, profile_x, profile_y).bg,
        selected_bg()
    );
    assert!(
        buffer_cell(&focused, profile_x, profile_y)
            .modifier
            .contains(Modifier::BOLD),
        "selected profile row should be visually highlighted"
    );
    for ch in ['-', 'p', 'r', 'o', 'o', 'f'] {
        assert_eq!(
            app.handle_key(key(KeyCode::Char(ch), KeyModifiers::NONE)),
            AppAction::Consumed
        );
    }

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::CreateSession {
            name: "tmp-1-proof".to_string(),
            profile_id: "linux-builder".to_string()
        })
    );
}

#[test]
fn help_lists_save_sessions_status_and_fork_shortcuts() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('/'), KeyModifiers::ALT));

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render help");

    assert!(snapshot.contains("Key"));
    assert!(snapshot.contains("Action"));
    assert!(snapshot.contains("Alt+?"));
    assert!(snapshot.contains("help"));
    assert!(snapshot.contains("Alt+s"));
    assert!(snapshot.contains("suspend"));
    assert!(snapshot.contains("Alt+c"));
    assert!(snapshot.contains("checkpoint"));
    assert!(snapshot.contains("Alt+l"));
    assert!(snapshot.contains("sessions"));
    assert!(snapshot.contains("Alt+i"));
    assert!(snapshot.contains("session info"));
    assert!(snapshot.contains("Alt+f fork"));
    assert!(snapshot.contains("Alt+p"));
    assert!(snapshot.contains("purge"));
}

#[test]
fn fork_overlay_asks_for_name_and_invokes_fork_action() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('f'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Fork);
    let snapshot = render_app_snapshot(&app, 100, 24).expect("render fork dialog");
    assert!(snapshot.contains("fork session"));
    assert!(snapshot.contains("source"));
    assert!(snapshot.contains("profile-v2"));
    assert!(snapshot.contains("profile-v2-fork"));
    assert!(snapshot.contains("active input"));

    for ch in ['-', 'c', 'o', 'p', 'y'] {
        assert_eq!(
            app.handle_key(key(KeyCode::Char(ch), KeyModifiers::NONE)),
            AppAction::Consumed
        );
    }

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::Fork {
            id: "profile-v2".to_string(),
            name: "profile-v2-fork-copy".to_string()
        })
    );
}

#[test]
fn alt_l_lists_sessions_as_table_with_key_fields() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('l'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Home);

    let snapshot = render_app_snapshot(&app, 120, 30).expect("render session list");
    assert!(snapshot.contains("Name"));
    assert!(snapshot.contains("Profile"));
    assert!(snapshot.contains("State"));
    assert!(snapshot.contains("Time"));
    assert!(snapshot.contains("Tokens"));
    assert!(snapshot.contains("Cost"));
    assert!(snapshot.contains("Profile V2"));
    assert!(snapshot.contains("corp-default"));
    assert!(snapshot.contains("linux-builder"));
}

#[test]
fn refresh_preserves_active_session_when_it_still_exists() {
    let mut app = App::new(fixture_state());
    app.select_session(1);

    let mut refreshed = fixture_state();
    refreshed.sessions[1].stats.tokens = 42;
    app.replace_state(refreshed);

    assert_eq!(app.state().active_session_id, "linux-os");
    assert_eq!(
        app.state()
            .active_session()
            .expect("active session")
            .stats
            .tokens,
        42
    );
}

#[test]
fn pending_create_focus_survives_until_new_session_appears() {
    let mut app = App::new(fixture_state());
    app.select_session_by_id("profile-v2");
    app.focus_session_when_available("tmp-2");

    let unchanged = fixture_state();
    app.replace_state(unchanged);
    assert_eq!(
        app.state().active_session_id,
        "profile-v2",
        "focus should not move if the gateway refresh does not list the new VM yet"
    );

    let mut refreshed = fixture_state();
    let mut created = refreshed.sessions[0].clone();
    created.id = "tmp-2".to_string();
    created.title = "tmp-2".to_string();
    refreshed.sessions.push(created);
    app.replace_state(refreshed);

    assert_eq!(
        app.state().active_session_id,
        "tmp-2",
        "pending create focus should apply on the first refresh that contains the new VM"
    );
}

#[test]
fn function_keys_toggle_hidden_overlays() {
    let mut app = App::new(fixture_state());

    assert_eq!(app.overlay(), AppOverlay::None);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('/'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Help);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('?'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::None);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('i'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Stats);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('i'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::None);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('l'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Home);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('l'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::None);
}

#[test]
fn esc_closes_modal_overlays_and_restores_vm_input() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('/'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Help);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
        AppAction::Consumed,
        "modal overlays should own keys while visible"
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::None);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
        AppAction::Forward,
        "plain VM input must forward after the modal closes"
    );
}

#[test]
fn control_keys_require_confirmation_before_invoking_service_actions() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('t'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Confirm);
    assert_eq!(
        app.pending_action(),
        Some(&ControlAction::Stop {
            id: "profile-v2".to_string()
        })
    );
    let modal_snapshot = render_app_snapshot(&app, 100, 24).expect("render confirmation");
    assert!(modal_snapshot.contains("confirm"));
    assert!(modal_snapshot.contains("Enter confirms"));
    assert!(
        modal_snapshot.contains("┌"),
        "confirmation should render as a modal block"
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
        AppAction::Consumed,
        "confirmation overlay owns keys until confirmed or cancelled"
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::Stop {
            id: "profile-v2".to_string()
        })
    );
    assert_eq!(app.overlay(), AppOverlay::None);
    assert_eq!(app.pending_action(), None);
}

#[test]
fn purge_action_is_alt_p_and_requires_confirmation() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('p'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Confirm);
    assert_eq!(
        app.pending_action(),
        Some(&ControlAction::Purge { all: false })
    );

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render purge confirmation");
    assert!(snapshot.contains("purge"));
    assert!(snapshot.contains("temporary sessions"));

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::Purge { all: false })
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
    app = App::new(state);

    assert_eq!(
        app.handle_key(key(KeyCode::Char('r'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.pending_action(),
        Some(&ControlAction::Resume {
            name: "linux-os".to_string()
        })
    );
}

#[test]
fn suspend_action_requires_persistent_running_session() {
    let mut app = App::new(fixture_state());
    assert_eq!(
        app.handle_key(key(KeyCode::Char('s'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.pending_action(),
        Some(&ControlAction::Suspend {
            id: "profile-v2".to_string()
        })
    );

    let mut state = fixture_state();
    state.sessions[0].persistent = false;
    app = App::new(state);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('s'), KeyModifiers::ALT)),
        AppAction::Forward,
        "ephemeral sessions cannot be suspended through the service"
    );
}

#[test]
fn suspend_progress_owns_the_main_terminal_surface() {
    let mut app = App::new(fixture_state());
    app.set_control_progress("suspending");

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render suspend progress");

    assert!(snapshot.contains("suspending..."));
    assert!(
        !snapshot.contains("connecting terminal profile-v2"),
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
            id: "profile-v2".to_string()
        })
    );

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render checkpoint confirm");
    assert!(snapshot.contains("checkpoint"));
    assert!(snapshot.contains("profile-v2"));
}

#[test]
fn stats_overlay_renders_on_demand_without_persistent_help() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('i'), KeyModifiers::ALT));

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render app snapshot");

    assert!(snapshot.contains("session info"));
    assert!(snapshot.contains("Field"));
    assert!(snapshot.contains("Value"));
    assert!(snapshot.contains("profile-v2"));
    assert!(snapshot.contains("tokens"));
    assert!(
        !render_snapshot(&fixture_state(), 100, 24)
            .expect("render base snapshot")
            .contains("Alt+?"),
        "help is hidden until requested"
    );
}

#[test]
fn gateway_status_json_maps_to_tui_state() {
    let state = state_from_status_json_for_test(
        gateway_status_body(),
        std::time::Duration::from_millis(24),
    )
    .expect("parse service list");

    assert_eq!(state.service.status, ServiceStatus::Online);
    assert_eq!(state.service.latency, std::time::Duration::from_millis(24));
    assert_eq!(state.active_session_id, "vm-1");
    assert_eq!(state.sessions.len(), 2);

    let active = &state.sessions[0];
    assert_eq!(active.title, "profile-main");
    assert_eq!(active.profile, "profile-v2");
    assert_eq!(active.lifecycle, SessionLifecycle::Working);
    assert_eq!(active.stats.duration, std::time::Duration::from_secs(2840));
    assert_eq!(active.stats.tokens, 38_912);
    assert_eq!(active.stats.cost_micros, 215_000);
    assert!(
        active.attention.is_empty(),
        "current profile status should not be marked stale"
    );

    let attention = &state.sessions[1];
    assert_eq!(attention.lifecycle, SessionLifecycle::Suspended);
    assert!(attention.attention.contains(&Attention::PolicyDeny));
    assert_eq!(attention.profile_status.as_deref(), Some("corrupted"));
    assert!(
        attention.attention.contains(&Attention::CredentialIssue),
        "corrupted profile status should be surfaced as a credential/profile issue"
    );
}

#[test]
fn malformed_gateway_status_fails_state_mapping() {
    let error = state_from_status_json_for_test(
        r#"{"service":"running","vms":"not a list"}"#,
        std::time::Duration::ZERO,
    )
    .expect_err("malformed gateway status should fail");

    assert!(error.to_string().contains("invalid type"));
}

#[tokio::test]
async fn gateway_provider_loads_status_over_http_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let body = gateway_status_body().to_string();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("GET /status "),
                    "unexpected request: {request:?}"
                );
                assert!(
                    request.contains("authorization: Bearer test-token")
                        || request.contains("Authorization: Bearer test-token"),
                    "missing bearer auth: {request:?}"
                );
                write_json_response(&mut stream, &body).await;
            }
        }
    });

    let state = GatewayProvider::new(format!("http://{addr}"))
        .load_async()
        .await
        .expect("load state over gateway");

    assert_eq!(state.sessions.len(), 2);
    assert_eq!(state.sessions[0].id, "vm-1");

    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_does_not_invent_default_profile_when_profiles_fail() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else if request.contains("GET /status ") {
                write_json_response(&mut stream, gateway_empty_status_body()).await;
            } else {
                assert!(
                    request.contains("GET /profiles "),
                    "unexpected request: {request:?}"
                );
                write_response(
                    &mut stream,
                    "502 Bad Gateway",
                    r#"{"error":"service profile discovery unavailable"}"#,
                )
                .await;
            }
        }
    });

    let state = GatewayProvider::new(format!("http://{addr}"))
        .load_async()
        .await
        .expect("load state over gateway");

    assert!(state.sessions.is_empty());
    assert!(
        state.profiles.is_empty(),
        "profile discovery failure with no sessions must not synthesize default"
    );
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_reuses_token_across_status_refreshes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let body = gateway_status_body().to_string();
    let server = tokio::spawn(async move {
        let mut token_requests = 0;
        let mut status_requests = 0;
        let mut profile_requests = 0;
        for _ in 0..5 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                token_requests += 1;
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else if request.contains("GET /profiles ") {
                profile_requests += 1;
                write_json_response(&mut stream, gateway_profiles_body()).await;
            } else {
                status_requests += 1;
                assert!(
                    request.contains("GET /status "),
                    "unexpected request: {request:?}"
                );
                assert!(
                    request.contains("authorization: Bearer test-token")
                        || request.contains("Authorization: Bearer test-token"),
                    "missing bearer auth: {request:?}"
                );
                write_json_response(&mut stream, &body).await;
            }
        }
        assert_eq!(token_requests, 1, "token should be cached across refreshes");
        assert_eq!(status_requests, 2);
        assert_eq!(
            profile_requests, 2,
            "profile list should stay live across refreshes"
        );
    });

    let provider = GatewayProvider::new(format!("http://{addr}"));
    provider.load_async().await.expect("initial load");
    let refreshed = provider.load_async().await.expect("refresh load");
    assert_eq!(refreshed.profiles.len(), 2);
    assert_eq!(refreshed.profiles[0].id, "corp-default");
    assert!(refreshed.profiles[0].is_default);

    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_invokes_stop_over_authenticated_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("POST /stop/vm-1 "),
                    "unexpected request: {request:?}"
                );
                assert!(
                    request.contains("authorization: Bearer test-token")
                        || request.contains("Authorization: Bearer test-token"),
                    "missing bearer auth: {request:?}"
                );
                write_json_response(&mut stream, r#"{"success":true}"#).await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Stop {
            id: "vm-1".to_string(),
        })
        .await
        .expect("invoke stop");

    assert_eq!(outcome.message, "stopped vm-1");
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_invokes_named_profile_create_over_authenticated_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("POST /provision "),
                    "unexpected request: {request:?}"
                );
                assert!(request.contains(r#""name":"tmp-1-proof""#));
                assert!(request.contains(r#""persistent":true"#));
                assert!(request.contains(r#""profile_id":"linux-builder""#));
                write_json_response(&mut stream, r#"{"id":"tmp-1-proof"}"#).await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::CreateSession {
            name: "tmp-1-proof".to_string(),
            profile_id: "linux-builder".to_string(),
        })
        .await
        .expect("invoke create");

    assert_eq!(outcome.message, "created tmp-1-proof");
    assert_eq!(outcome.focus_session.as_deref(), Some("tmp-1-proof"));
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_invokes_fork_over_authenticated_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("POST /fork/profile-v2 "),
                    "unexpected request: {request:?}"
                );
                assert!(request.contains(r#""name":"profile-v2-fork-copy""#));
                write_json_response(
                    &mut stream,
                    r#"{"name":"profile-v2-fork-copy","size_bytes":1024}"#,
                )
                .await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Fork {
            id: "profile-v2".to_string(),
            name: "profile-v2-fork-copy".to_string(),
        })
        .await
        .expect("invoke fork");

    assert_eq!(outcome.message, "forked profile-v2-fork-copy");
    assert_eq!(
        outcome.focus_session.as_deref(),
        Some("profile-v2-fork-copy")
    );
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_invokes_checkpoint_over_suspend_endpoint() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("POST /suspend/vm-1 "),
                    "unexpected request: {request:?}"
                );
                write_json_response(&mut stream, r#"{"success":true}"#).await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Checkpoint {
            id: "vm-1".to_string(),
        })
        .await
        .expect("invoke checkpoint");

    assert_eq!(outcome.message, "checkpointed vm-1");
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_invokes_purge_over_authenticated_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("POST /purge "),
                    "unexpected request: {request:?}"
                );
                assert!(
                    request.contains("authorization: Bearer test-token")
                        || request.contains("Authorization: Bearer test-token"),
                    "missing bearer auth: {request:?}"
                );
                assert!(request.contains(r#""all":false"#));
                write_json_response(
                    &mut stream,
                    r#"{"purged":3,"persistent_purged":0,"ephemeral_purged":3}"#,
                )
                .await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Purge { all: false })
        .await
        .expect("invoke purge");

    assert_eq!(outcome.message, "purged 3 temporary sessions");
    assert_eq!(outcome.focus_session, None);
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_surfaces_action_error_body() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("DELETE /delete/vm-1 "),
                    "unexpected request: {request:?}"
                );
                write_response(
                    &mut stream,
                    "500 Internal Server Error",
                    r#"{"error":"boom"}"#,
                )
                .await;
            }
        }
    });

    let error = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Delete {
            id: "vm-1".to_string(),
        })
        .await
        .expect_err("delete should fail");

    assert!(error.to_string().contains("500"));
    assert!(error.to_string().contains("boom"));
    server.await.expect("server task");
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn find_cell_x(buffer: &ratatui::buffer::Buffer, row: u16, needle: &str) -> u16 {
    let width = buffer.area.width as usize;
    let row_start = row as usize * width;
    let line = buffer.content()[row_start..row_start + width]
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    let byte_index = line.find(needle).expect("needle in rendered row");
    line[..byte_index].chars().count() as u16
}

fn find_cell(buffer: &ratatui::buffer::Buffer, needle: &str) -> (u16, u16) {
    let width = buffer.area.width as usize;
    for y in 0..buffer.area.height {
        let row_start = y as usize * width;
        let line = buffer.content()[row_start..row_start + width]
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        if let Some(byte_index) = line.find(needle) {
            return (line[..byte_index].chars().count() as u16, y);
        }
    }
    panic!("{needle:?} in rendered buffer");
}

fn buffer_cell(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> &ratatui::buffer::Cell {
    let width = buffer.area.width as usize;
    &buffer.content()[y as usize * width + x as usize]
}

fn yellow() -> Color {
    Color::Rgb(249, 226, 175)
}

fn blue() -> Color {
    Color::Rgb(137, 180, 250)
}

fn grey() -> Color {
    Color::Rgb(127, 137, 180)
}

fn selected_bg() -> Color {
    Color::Rgb(49, 50, 68)
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 256];
    loop {
        let bytes_read = stream.read(&mut buffer).await.expect("read request");
        if bytes_read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..bytes_read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .unwrap_or(request.len());
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .or_else(|| {
            headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length:"))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or_default();
    while request.len().saturating_sub(header_end) < content_length {
        let bytes_read = stream.read(&mut buffer).await.expect("read request body");
        if bytes_read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..bytes_read]);
    }
    String::from_utf8_lossy(&request).into_owned()
}

async fn write_json_response(stream: &mut tokio::net::TcpStream, body: &str) {
    write_response(stream, "200 OK", body).await;
}

async fn write_response(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write response");
}

fn gateway_status_body() -> &'static str {
    r#"{
        "service": "running",
        "gateway_version": "test",
        "vm_count": 2,
        "resource_summary": null,
        "vms": [
            {
                "id": "vm-1",
                "name": "profile-main",
                "status": "Running",
                "persistent": true,
                "profile_id": "profile-v2",
                "profile_revision": "main",
                "profile_status": "current",
                "uptime_secs": 2840,
                "total_input_tokens": 30000,
                "total_output_tokens": 8912,
                "total_estimated_cost": 0.215,
                "total_tool_calls": 7,
                "total_requests": 11,
                "total_file_events": 3
            },
            {
                "id": "vm-2",
                "status": "Suspended",
                "persistent": true,
                "profile_id": "linux-os",
                "profile_status": "corrupted",
                "uptime_secs": 7860,
                "total_input_tokens": 10000,
                "total_output_tokens": 2900,
                "total_estimated_cost": 0.076,
                "denied_requests": 1
            }
        ]
    }"#
}

fn gateway_empty_status_body() -> &'static str {
    r#"{
        "service": "running",
        "gateway_version": "test",
        "vm_count": 0,
        "resource_summary": null,
        "vms": []
    }"#
}

fn gateway_profiles_body() -> &'static str {
    r#"{
        "mode": "settings_profiles_v2",
        "default_profile": "corp-default",
        "profiles": [
            {
                "profile": {
                    "id": "corp-default",
                    "name": "Corp Default",
                    "best_for": "default profile"
                },
                "source": "corp"
            },
            {
                "profile": {
                    "id": "linux-builder",
                    "name": "Linux Builder",
                    "best_for": "kernel and distro work"
                },
                "source": "user"
            }
        ]
    }"#
}

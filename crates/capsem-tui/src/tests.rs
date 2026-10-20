mod gateway_fixtures;
mod image_creation;
mod lifecycle;
mod updates;
use gateway_fixtures::*;

use capsem_sdk::models::VmAction;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app::{App, AppAction, AppOverlay, ControlAction};
use crate::fixture::{fixture_state, offline_state};
use crate::gateway_provider::{
    start_service_with_binary, state_from_status_and_update_json_for_test, state_from_status_json_for_test,
    update_with_binary, GatewayProvider,
};
use crate::model::{Attention, ServiceStatus, SessionLifecycle, UpdateNoticeKind, UpdateTrack};
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
    assert!(snapshot.contains("1  Session V2"));
    assert!(snapshot.contains("2  Linux OS!"));
    assert!(snapshot.contains("◷ 47m | # 38.4k | $ 0.21 | help: alt+?"));
    assert!(
        !snapshot.contains("github.com/google/capsem"),
        "repo metadata belongs in a popup or future status segment, not the empty terminal surface"
    );
    assert!(!snapshot.contains("┌"), "minimal UI should not render boxes");
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
fn status_bar_shows_update_notice_without_hiding_session_stats() {
    let mut state = fixture_state();
    state.update_notice = Some(crate::model::UpdateNotice {
        kind: UpdateNoticeKind::Available(vec![UpdateTrack::Binary, UpdateTrack::VmAssets]),
        channel_url: Some("https://release.capsem.org/health.json".to_string()),
    });

    let snapshot = render_snapshot(&state, 120, 24).expect("render update snapshot");

    assert!(snapshot.contains("updates: binary, assets"));
    assert!(snapshot.contains("help: alt+?"));
}

#[test]
fn status_bar_shows_current_update_state() {
    let mut state = fixture_state();
    state.update_notice = Some(crate::model::UpdateNotice {
        kind: UpdateNoticeKind::Current,
        channel_url: Some("https://release.capsem.org/health.json".to_string()),
    });

    let snapshot = render_snapshot(&state, 120, 24).expect("render current update snapshot");

    assert!(snapshot.contains("updates: current"));
    assert!(snapshot.contains("help: alt+?"));
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
    assert_eq!(app.create_draft().expect("create draft").name, "vm-1");
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
    let outcome = start_service_with_binary(binary).await.expect("start service command");

    assert_eq!(outcome.message, "service start requested");
    assert_eq!(outcome.focus_session, None);
}

#[tokio::test]
async fn update_action_runs_complete_update_with_yes() {
    let (script, log) = fake_capsem_script("binary-update");

    let outcome = update_with_binary(&script).await.expect("run capsem update");

    assert_eq!(outcome.message, "Capsem update finished");
    assert_eq!(outcome.focus_session, None);
    assert_eq!(std::fs::read_to_string(log).expect("read args"), "update --yes\n");
}

#[test]
fn tab_colors_use_selected_yellow_and_unselected_blue_only() {
    let buffer = render_test_buffer(&fixture_state(), 100, 24).expect("render buffer");
    let row = buffer.area.height - 1;
    let selected_number = find_cell_x(&buffer, row, "1  Session V2");
    let selected_label = selected_number + 3;
    let other_number = find_cell_x(&buffer, row, "2  Linux OS!");
    let other_label = other_number + 3;

    assert_eq!(buffer_cell(&buffer, selected_number, row).bg, yellow());
    assert_eq!(buffer_cell(&buffer, selected_label, row).fg, yellow());
    assert!(buffer_cell(&buffer, selected_number, row)
        .modifier
        .contains(Modifier::BOLD));

    assert_eq!(buffer_cell(&buffer, other_number, row).bg, blue());
    assert_eq!(buffer_cell(&buffer, other_label, row).fg, blue());
    assert!(
        !buffer_cell(&buffer, other_label, row).modifier.contains(Modifier::BOLD),
        "only the selected tab label should be bold"
    );
}

#[test]
fn keyboard_navigation_switches_sessions_without_stealing_plain_q() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('q'), KeyModifiers::NONE)),
        AppAction::Forward
    );
    assert_eq!(app.state().active_session_id, "vm-2");

    assert_eq!(
        app.handle_key(key(KeyCode::Right, KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.state().active_session_id, "linux-os");

    assert_eq!(
        app.handle_key(key(KeyCode::Left, KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.state().active_session_id, "vm-2");

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

    assert!(app.select_session_by_id("Session V2"));
    assert_eq!(app.state().active_session_id, "vm-2");

    assert!(!app.select_session_by_id("missing-session"));
    assert_eq!(app.state().active_session_id, "vm-2");
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
            id: "vm-2".to_string(),
            label: "Session V2".to_string()
        })
    );
}

#[test]
fn update_action_is_alt_owned_atomic_and_confirmed() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('u'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Confirm);
    assert_eq!(app.pending_action(), Some(&ControlAction::Update));
    let snapshot = render_app_snapshot(&app, 100, 24).expect("render update confirmation");
    assert!(snapshot.contains("complete verified release"));
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::Update)
    );

    assert_eq!(
        app.handle_key(key(KeyCode::Char('a'), KeyModifiers::ALT)),
        AppAction::Forward
    );
    assert_eq!(app.pending_action(), None);
}

#[test]
fn create_overlay_edits_prefilled_name() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    let snapshot = render_app_snapshot(&app, 100, 24).expect("render create dialog");
    assert!(snapshot.contains("new session"));
    assert!(snapshot.contains("name"));
    assert!(snapshot.contains("active input"));
    assert!(snapshot.contains("Enter creates; Esc cancels"));
    assert!(!snapshot.to_ascii_lowercase().contains("profile"));

    let focused = render_app_test_buffer(&app, 100, 24).expect("render focused create dialog");
    let (name_x, name_y) = find_cell(&focused, "vm-1");
    assert_eq!(buffer_cell(&focused, name_x, name_y).bg, selected_bg());
    for ch in ['-', 'p', 'r', 'o', 'o', 'f'] {
        assert_eq!(
            app.handle_key(key(KeyCode::Char(ch), KeyModifiers::NONE)),
            AppAction::Consumed
        );
    }

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::CreateSession {
            name: Some("vm-1-proof".to_string()),
            image: None,
        })
    );
}

#[test]
fn create_overlay_prefills_the_first_free_vm_name() {
    let mut state = fixture_state();
    let mut taken = state.sessions[0].clone();
    taken.id = "vm-1".to_string();
    state.sessions.push(taken);
    let mut app = App::new(state);

    assert_eq!(
        app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    // vm-1 and vm-2 are taken; the service spells new names vm-N.
    assert_eq!(app.create_draft().expect("create draft").name, "vm-3");
}

#[test]
fn create_overlay_default_name_lets_service_assign_id() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::CreateSession {
            name: None,
            image: None
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
    assert!(snapshot.contains("Alt+u"));
    assert!(snapshot.contains("apply complete verified release"));
    assert!(!snapshot.contains("Alt+a"));
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
    assert!(snapshot.contains("Session V2"));
    assert!(snapshot.contains("vm-2-fork"));
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
            id: "vm-2".to_string(),
            name: "vm-2-fork-copy".to_string()
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
    assert!(!snapshot.contains("Profile"));
    assert!(snapshot.contains("State"));
    assert!(snapshot.contains("Time"));
    assert!(snapshot.contains("Tokens"));
    assert!(snapshot.contains("Cost"));
    assert!(snapshot.contains("Session V2"));
    assert!(snapshot.contains("Linux OS"));
}

#[test]
fn refresh_preserves_active_session_when_it_still_exists() {
    let mut app = App::new(fixture_state());
    app.select_session(1);

    let mut refreshed = fixture_state();
    refreshed.sessions[1].stats.tokens = 42;
    app.replace_state(refreshed);

    assert_eq!(app.state().active_session_id, "linux-os");
    assert_eq!(app.state().active_session().expect("active session").stats.tokens, 42);
}

#[test]
fn pending_create_focus_survives_until_new_session_appears() {
    let mut app = App::new(fixture_state());
    app.select_session_by_id("vm-2");
    app.focus_session_when_available("vm-3");

    let unchanged = fixture_state();
    app.replace_state(unchanged);
    assert_eq!(
        app.state().active_session_id,
        "vm-2",
        "focus should not move if the gateway refresh does not list the new session yet"
    );

    let mut refreshed = fixture_state();
    let mut created = refreshed.sessions[0].clone();
    created.id = "vm-3".to_string();
    created.title = "vm-3".to_string();
    refreshed.sessions.push(created);
    app.replace_state(refreshed);

    assert_eq!(
        app.state().active_session_id,
        "vm-3",
        "pending create focus should apply on the first refresh that contains the new session"
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
        "plain terminal input must forward after the modal closes"
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
            id: "vm-2".to_string(),
            label: "Session V2".to_string()
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
            id: "vm-2".to_string(),
            label: "Session V2".to_string()
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
    assert_eq!(app.pending_action(), Some(&ControlAction::Purge { all: false }));

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render purge confirmation");
    assert!(snapshot.contains("purge"));
    assert!(snapshot.contains("temporary and broken sessions"));

    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::Purge { all: false })
    );
}

#[test]
fn stats_overlay_renders_on_demand_without_persistent_help() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('i'), KeyModifiers::ALT));

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render app snapshot");

    assert!(snapshot.contains("session info"));
    assert!(snapshot.contains("Field"));
    assert!(snapshot.contains("Value"));
    assert!(snapshot.contains("vm-2"));
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
    let state = state_from_status_json_for_test(gateway_status_body(), std::time::Duration::from_millis(24))
        .expect("parse service list");

    assert_eq!(state.service.status, ServiceStatus::Online);
    assert_eq!(state.service.latency, std::time::Duration::from_millis(24));
    assert_eq!(state.active_session_id, "vm-1");
    assert_eq!(state.sessions.len(), 2);

    let active = &state.sessions[0];
    assert_eq!(active.title, "main-session");
    assert_eq!(active.lifecycle, SessionLifecycle::Working);
    assert_eq!(active.stats.duration, std::time::Duration::from_secs(2840));
    assert_eq!(active.stats.tokens, 38_912);
    assert_eq!(active.stats.cost_micros, 215_000);
    assert!(active.attention.is_empty(), "a running VM should not be marked stale");

    let attention = &state.sessions[1];
    assert_eq!(attention.lifecycle, SessionLifecycle::Suspended);
    assert!(attention.attention.contains(&Attention::PolicyDeny));
    assert_eq!(attention.branch, None);
    assert_eq!(
        attention.resume_blocked_reason.as_deref(),
        Some("session overlay is missing")
    );
    assert!(!attention.attention.contains(&Attention::CredentialIssue));
}

#[test]
fn malformed_gateway_status_fails_state_mapping() {
    let error =
        state_from_status_json_for_test(r#"{"service":"running","vms":"not a list"}"#, std::time::Duration::ZERO)
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
                assert!(request.contains("GET /status "), "unexpected request: {request:?}");
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
async fn gateway_provider_loads_update_status_over_http_gateway() {
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
                assert!(request.contains("GET /status "), "unexpected request: {request:?}");
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-token"));
                let mut overview: serde_json::Value = serde_json::from_str(gateway_empty_status_body()).unwrap();
                overview["updates"] = serde_json::from_str(gateway_update_status_body()).unwrap();
                write_json_response(&mut stream, &overview.to_string()).await;
            }
        }
    });

    let state = GatewayProvider::new(format!("http://{addr}"))
        .load_async()
        .await
        .expect("load state over gateway");

    assert_eq!(
        state.update_notice.expect("update notice").kind,
        UpdateNoticeKind::Available(vec![UpdateTrack::Binary, UpdateTrack::VmAssets])
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
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                token_requests += 1;
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                status_requests += 1;
                assert!(request.contains("GET /status "), "unexpected request: {request:?}");
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
    });

    let provider = GatewayProvider::new(format!("http://{addr}"));
    provider.load_async().await.expect("initial load");
    let refreshed = provider.load_async().await.expect("refresh load");
    assert_eq!(refreshed.sessions.len(), 2);

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

fn fake_capsem_script(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("capsem-tui-{label}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fake capsem dir");
    let script = dir.join("capsem");
    let log = dir.join("args.txt");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\nprintf '%s\\n' 'Capsem update finished'\n",
            log.display()
        ),
    )
    .expect("write fake capsem");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod fake capsem");
    }
    (script, log)
}

pub(crate) async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
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
        .or_else(|| headers.lines().find_map(|line| line.strip_prefix("Content-Length:")))
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

pub(crate) async fn write_json_response(stream: &mut tokio::net::TcpStream, body: &str) {
    write_response(stream, "200 OK", body).await;
}

pub(crate) async fn write_response(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await.expect("write response");
}

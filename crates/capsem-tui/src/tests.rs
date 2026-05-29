use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app::{App, AppAction, AppOverlay, ControlAction};
use crate::fixture::fixture_state;
use crate::gateway_provider::{state_from_status_json_for_test, GatewayProvider};
use crate::model::{Attention, ServiceStatus, SessionLifecycle};
use crate::ui::{render_app_snapshot, render_snapshot, render_test_buffer};

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

    assert!(snapshot.contains("   18ms●"));
    assert!(snapshot.contains("1  profile-v2"));
    assert!(snapshot.contains("2  linux-os!"));
    assert!(snapshot.contains("◷ 47m | # 38.4k | $ 0.21"));
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
fn shell_commands_are_alt_owned() {
    let mut app = App::new(fixture_state());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Confirm);
    assert_eq!(app.pending_action(), Some(&ControlAction::CreateEphemeral));

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
fn stats_overlay_renders_on_demand_without_persistent_help() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('i'), KeyModifiers::ALT));

    let snapshot = render_app_snapshot(&app, 100, 24).expect("render app snapshot");

    assert!(snapshot.contains("stats"));
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
    String::from_utf8_lossy(&request).into_owned()
}

async fn write_json_response(stream: &mut tokio::net::TcpStream, body: &str) {
    write_response(stream, "200 OK", body).await;
}

async fn write_response(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
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
                "uptime_secs": 7860,
                "total_input_tokens": 10000,
                "total_output_tokens": 2900,
                "total_estimated_cost": 0.076,
                "denied_requests": 1
            }
        ]
    }"#
}

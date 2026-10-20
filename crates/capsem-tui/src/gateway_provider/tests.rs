use super::*;
use crate::tests::{read_http_request, write_json_response, write_response};

fn overview(service: &str, lifecycle: &str) -> String {
    serde_json::json!({
        "service": service, "gateway_version": "test", "vm_count": 1,
        "resource_summary": null,
        "vms": [{
            "id": "vm", "name": null, "persistent": true,
            "status": lifecycle, "available_actions": [], "can_resume": false,
            "resume_blocked_reason": "session overlay is missing",
            "total_input_tokens": u64::MAX, "total_output_tokens": 1,
            "total_tool_calls": u64::MAX, "total_requests": u64::MAX,
            "total_file_events": 1, "total_estimated_cost": -1.0
        }]
    })
    .to_string()
}

#[test]
fn typed_overview_preserves_vm_health_and_saturates_counters() {
    for (wire, lifecycle) in [
        ("Running", SessionLifecycle::Working),
        ("Stopped", SessionLifecycle::Idle),
        ("Suspended", SessionLifecycle::Suspended),
        ("Defunct", SessionLifecycle::Failed),
        ("Incompatible", SessionLifecycle::Failed),
    ] {
        let state = state_from_status_json_for_test(&overview("unavailable", wire), Duration::ZERO).unwrap();
        assert_eq!(state.service.status, ServiceStatus::Degraded);
        assert_eq!(state.update_notice.unwrap().kind, UpdateNoticeKind::Unavailable);
        let vm = &state.sessions[0];
        assert_eq!(vm.title, "vm");
        assert_eq!(vm.lifecycle, lifecycle);
        assert_eq!(
            vm.attention.contains(&Attention::StaleData),
            lifecycle == SessionLifecycle::Failed
        );
        assert_eq!(vm.branch, None);
        assert!(!vm.can_resume);
        assert_eq!(vm.resume_blocked_reason.as_deref(), Some("session overlay is missing"));
        assert_eq!(vm.stats.tokens, u64::MAX);
        assert_eq!(vm.stats.jobs, u16::MAX);
        assert_eq!(vm.stats.events, u32::MAX);
        assert_eq!(vm.stats.cost_micros, 0);
    }
}

#[test]
fn unknown_gateway_enums_are_errors_instead_of_idle_sessions() {
    for raw in [overview("failed", "Running"), overview("running", "broken")] {
        let error = state_from_status_json_for_test(&raw, Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("unknown variant"));
    }
}

#[test]
fn gateway_actions_control_tui_keys_and_revoke_stale_intent() {
    use crate::app::{App, AppAction, AppOverlay};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let state = |actions: &[&str], status: &str| {
        let mut value: serde_json::Value = serde_json::from_str(&overview("running", status)).unwrap();
        value["vms"][0]["available_actions"] = serde_json::json!(actions);
        value["vms"][0]["can_resume"] = serde_json::json!(true);
        state_from_status_json_for_test(&value.to_string(), Duration::ZERO).unwrap()
    };
    let alt = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT);
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    for ch in ['s', 'c', 't', 'd', 'f'] {
        let mut app = App::new(state(&[], "Running"));
        assert!(!matches!(app.handle_key(alt(ch)), AppAction::Invoke(_)));
        assert_eq!(app.overlay(), AppOverlay::None, "unadvertised {ch}");
        assert!(app.pending_action().is_none());
    }
    let mut app = App::new(state(&["stop"], "Running"));
    app.handle_key(alt('t'));
    assert_eq!(app.overlay(), AppOverlay::Confirm);
    app.replace_state(state(&[], "Running"));
    assert!(!matches!(app.handle_key(enter), AppAction::Invoke(_)));
    assert_eq!(app.overlay(), AppOverlay::None);
    assert!(app
        .state()
        .service
        .control_message
        .as_deref()
        .unwrap()
        .contains("no longer available"));

    let mut app = App::new(state(&["fork"], "Running"));
    app.handle_key(alt('f'));
    assert_eq!(app.overlay(), AppOverlay::Fork);
    app.replace_state(state(&[], "Running"));
    assert!(!matches!(app.handle_key(enter), AppAction::Invoke(_)));
    assert!(app.fork_draft().is_none());

    for (status, advertised, expected) in [("Stopped", "start", "start"), ("Suspended", "resume", "resume")] {
        let mut app = App::new(state(&[advertised], status));
        let AppAction::Invoke(action) = app.handle_key(enter) else {
            panic!("{status} should {expected}")
        };
        assert_eq!(action.label(), expected);
    }
    let mut app = App::new(state(&[], "Stopped"));
    assert!(
        !matches!(app.handle_key(enter), AppAction::Invoke(_)),
        "can_resume alone grants no action"
    );
    let mut app = App::new(state(&["resume"], "Suspended"));
    app.handle_key(alt('r'));
    let mut revoked = state(&["resume"], "Suspended");
    revoked.sessions[0].can_resume = false;
    app.replace_state(revoked);
    assert!(
        !matches!(app.handle_key(enter), AppAction::Invoke(_)),
        "checkpoint readiness is rechecked at confirmation"
    );
}

#[test]
fn cost_projection_handles_missing_nonfinite_and_extreme_values() {
    for value in [None, Some(f64::NAN), Some(f64::INFINITY), Some(0.0), Some(-1.0)] {
        assert_eq!(cost_to_micros(value), 0);
    }
    assert_eq!(cost_to_micros(Some(f64::MAX)), u64::MAX);
    assert_eq!(cost_to_micros(Some(0.000_001_5)), 2);
}

#[tokio::test]
async fn rotated_token_is_refetched_before_sdk_overview_retry() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for (step, (path, expected_token)) in [
            ("/token", None),
            ("/status", Some("old")),
            ("/status", Some("old")),
            ("/token", None),
            ("/status", Some("new")),
        ]
        .into_iter()
        .enumerate()
        {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            assert!(request.starts_with(&format!("GET {path} ")), "{request}");
            if let Some(token) = expected_token {
                assert!(request
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {token}")));
            }
            match path {
                "/token" => {
                    let token = if step == 0 { "old" } else { "new" };
                    write_json_response(&mut stream, &format!(r#"{{"token":"{token}"}}"#)).await;
                }
                _ if step == 2 => write_response(&mut stream, "401 Unauthorized", "expired").await,
                _ => write_json_response(&mut stream, &overview("running", "Running")).await,
            }
        }
    });
    let provider = GatewayProvider::new(url);
    provider.load_async().await.unwrap();
    let refreshed = provider.load_async().await.unwrap();
    assert_eq!(refreshed.sessions[0].id, "vm");
    server.await.unwrap();
}

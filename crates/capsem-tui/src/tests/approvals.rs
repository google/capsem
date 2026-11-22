use super::*;
use capsem_sdk::models::{SecurityAskDecision, SecurityAskListResponse, SecurityAskRequest, SecurityAskStatus};

#[test]
fn compact_footer_keeps_pending_approval_and_browser_shortcuts() {
    let mut state = fixture_state();
    state.sessions[0].attention.push(Attention::ApprovalRequired);
    for browser in [false, true] {
        state.sessions[0].browser_request = browser.then(|| capsem_sdk::models::BrowserRequest {
            owner_generation: Some("11111111-1111-4111-8111-111111111111".into()),
            id: 1,
            url: "https://example.org/login".into(),
        });
        for width in [80, 40, 32, 26, 20] {
            let snapshot = render_snapshot(&state, width, 24).unwrap();
            let footer = snapshot.lines().last().unwrap();
            assert!(footer.starts_with("  18ms●"), "{footer}");
            assert_eq!(footer.contains("Alt+M"), width >= 26, "{footer}");
            assert!(footer.contains("Alt+A"), "{footer}");
            assert_eq!(footer.contains("Alt+b"), browser && width >= 32, "{footer}");
            assert!(footer.ends_with("Alt+?"), "{footer}");
        }
    }
    state.sessions[0].attention.clear();
    assert!(!render_snapshot(&state, 40, 24)
        .unwrap()
        .lines()
        .last()
        .unwrap()
        .contains("Alt+A"));
}

#[test]
fn pending_approval_badge_comes_from_the_live_count_and_clears_after_resolution() {
    let mut response: serde_json::Value = serde_json::from_str(gateway_status_body()).unwrap();
    response["vms"][0]["pending_approval_count"] = 2.into();
    let state = state_from_status_json_for_test(&response.to_string(), std::time::Duration::ZERO).unwrap();
    assert!(state.sessions[0].attention.contains(&Attention::ApprovalRequired));
    assert!(render_snapshot(&state, 150, 42).unwrap().contains("Alt+A approvals"));
    response["vms"][0]["pending_approval_count"] = 0.into();
    let state = state_from_status_json_for_test(&response.to_string(), std::time::Duration::ZERO).unwrap();
    assert!(!state.sessions[0].attention.contains(&Attention::ApprovalRequired));
    assert!(!render_snapshot(&state, 150, 42).unwrap().contains("Alt+A approvals"));
}

fn list(vm_id: &str) -> SecurityAskListResponse {
    SecurityAskListResponse {
        vm_id: vm_id.into(),
        generation: "11111111-1111-4111-8111-111111111111".into(),
        pending_count: 1,
        requests: vec![SecurityAskRequest {
            ask_id: "22222222-2222-4222-8222-222222222222".into(),
            event_id: "event".into(),
            event_type: "http".into(),
            rule_id: "rule".into(),
            rule_name: "Needs approval".into(),
            summary: "HTTPS request needs approval".into(),
            status: SecurityAskStatus::Pending,
            created_at_unix_ms: 1,
            expires_at_unix_ms: i64::MAX,
            resolved_at_unix_ms: None,
            resolution_reason: None,
        }],
    }
}

fn take_list_request(app: &mut App, vm_id: &str) -> u64 {
    let Some(ControlAction::ListApprovals { id, view_id }) = app.take_approval_request() else {
        panic!("expected an approval list request");
    };
    assert_eq!(id, vm_id);
    view_id
}

#[test]
fn narrow_approval_panel_keeps_all_decision_and_navigation_hints_visible() {
    let mut app = App::new(fixture_state());
    let vm_id = app.state().active_session_id.clone();
    app.handle_key(key(KeyCode::Char('a'), KeyModifiers::ALT));
    let view_id = take_list_request(&mut app, &vm_id);
    app.apply_approval_list(&vm_id, view_id, Ok(list(&vm_id)));
    for width in [100, 40, 32] {
        let screen = render_app_snapshot(&app, width, 24).unwrap();
        let refresh = if width == 100 {
            "R check current state"
        } else {
            "R refresh"
        };
        for hint in ["A approve once", "D deny", refresh, "Esc close", "↑/↓ choose"] {
            assert!(screen.contains(hint), "missing {hint} at {width}:\n{screen}");
        }
    }
}

#[test]
fn approval_overlay_lists_safe_facts_and_decides_once_for_the_observed_generation() {
    let mut app = App::new(fixture_state());
    let vm_id = app.state().active_session_id.clone();
    assert_eq!(
        app.handle_key(key(KeyCode::Char('a'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Approvals);
    let view_id = take_list_request(&mut app, &vm_id);
    let response = list(&vm_id);
    app.apply_approval_list(&vm_id, view_id, Ok(response.clone()));
    let screen = render_app_snapshot(&app, 100, 24).unwrap();
    assert!(screen.contains("HTTPS request needs approval"));
    let action = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::NONE));
    assert_eq!(
        action,
        AppAction::Invoke(ControlAction::ResolveApproval {
            id: vm_id,
            view_id,
            generation: response.generation,
            ask_id: response.requests[0].ask_id.clone(),
            decision: SecurityAskDecision::Approve,
        })
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
        AppAction::Consumed
    );
}

#[test]
fn stale_and_expired_approvals_cannot_be_decided_or_restored_after_closing() {
    let mut app = App::new(fixture_state());
    let vm_id = app.state().active_session_id.clone();
    app.handle_key(key(KeyCode::Char('a'), KeyModifiers::ALT));
    let view_id = take_list_request(&mut app, &vm_id);
    let mut response = list(&vm_id);
    response.requests[0].expires_at_unix_ms = 1;
    app.apply_approval_list(&vm_id, view_id, Ok(response.clone()));
    assert_eq!(
        app.handle_key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
        AppAction::Consumed
    );
    response.vm_id = "different".into();
    app.refresh_approvals();
    assert_eq!(take_list_request(&mut app, &vm_id), view_id);
    app.apply_approval_list(&vm_id, view_id, Ok(response));
    assert!(app.approval_view().unwrap().list.is_none());
    app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));
    app.apply_approval_list(&vm_id, view_id, Ok(list(&vm_id)));
    assert_ne!(app.overlay(), AppOverlay::Approvals);
}

#[test]
fn approval_refresh_keeps_the_exact_reviewed_request_when_rows_move() {
    let mut app = App::new(fixture_state());
    let vm_id = app.state().active_session_id.clone();
    app.handle_key(key(KeyCode::Char('a'), KeyModifiers::ALT));
    let view_id = take_list_request(&mut app, &vm_id);
    let mut response = list(&vm_id);
    let ask_id = response.requests[0].ask_id.clone();
    app.apply_approval_list(&vm_id, view_id, Ok(response.clone()));
    let mut new_request = response.requests[0].clone();
    new_request.ask_id = "33333333-3333-4333-8333-333333333333".into();
    response.requests.insert(0, new_request);
    response.pending_count = 2;
    app.refresh_approvals();
    assert_eq!(take_list_request(&mut app, &vm_id), view_id);
    app.apply_approval_list(&vm_id, view_id, Ok(response));
    let action = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(matches!(action,
        AppAction::Invoke(ControlAction::ResolveApproval { ask_id: selected, .. }) if selected == ask_id
    ));
}

#[test]
fn lost_or_replaced_selection_requires_a_deliberate_new_choice() {
    for replace_generation in [false, true] {
        let mut app = App::new(fixture_state());
        let vm_id = app.state().active_session_id.clone();
        app.handle_key(key(KeyCode::Char('a'), KeyModifiers::ALT));
        let view_id = take_list_request(&mut app, &vm_id);
        let mut response = list(&vm_id);
        app.apply_approval_list(&vm_id, view_id, Ok(response.clone()));
        if replace_generation {
            response.generation = "33333333-3333-4333-8333-333333333333".into();
        } else {
            response.requests[0].ask_id = "33333333-3333-4333-8333-333333333333".into();
        }
        app.refresh_approvals();
        assert_eq!(take_list_request(&mut app, &vm_id), view_id);
        app.apply_approval_list(&vm_id, view_id, Ok(response.clone()));
        let screen = render_app_snapshot(&app, 100, 24).unwrap();
        assert!(screen.contains("Select a request with ↑/↓ before deciding."));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            AppAction::Consumed
        );
        app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('d'), KeyModifiers::NONE)),
            AppAction::Invoke(ControlAction::ResolveApproval {
                id: vm_id,
                view_id,
                generation: response.generation,
                ask_id: response.requests[0].ask_id.clone(),
                decision: SecurityAskDecision::Deny,
            })
        );
    }
}

#[test]
fn delayed_replies_cannot_change_a_reopened_view_of_the_same_sandbox() {
    let mut app = App::new(fixture_state());
    let vm_id = app.state().active_session_id.clone();
    app.handle_key(key(KeyCode::Char('a'), KeyModifiers::ALT));
    let closed = take_list_request(&mut app, &vm_id);
    app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Char('a'), KeyModifiers::ALT));
    let current = take_list_request(&mut app, &vm_id);
    assert_ne!(closed, current);
    let response = list(&vm_id);
    app.apply_approval_list(&vm_id, closed, Ok(response.clone()));
    let view = app.approval_view().unwrap();
    assert!(view.loading && view.list.is_none());
    app.apply_approval_list(&vm_id, current, Ok(response));
    assert!(matches!(
        app.handle_key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
        AppAction::Invoke(_)
    ));
    app.complete_approval_decision(&vm_id, closed, false);
    assert!(app.approval_view().unwrap().resolving);
    assert!(app.take_approval_request().is_none());
    app.complete_approval_decision(&vm_id, current, true);
    assert_eq!(take_list_request(&mut app, &vm_id), current);
}

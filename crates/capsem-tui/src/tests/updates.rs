use super::*;

#[test]
fn gateway_update_status_json_maps_to_tui_notice() {
    let state = state_from_status_and_update_json_for_test(
        gateway_empty_status_body(),
        gateway_update_status_body(),
        std::time::Duration::from_millis(11),
    )
    .expect("parse service and update status");

    let notice = state.update_notice.expect("update notice");
    assert_eq!(
        notice.kind,
        UpdateNoticeKind::Available(vec![UpdateTrack::Binary, UpdateTrack::VmAssets])
    );
    assert_eq!(
        notice.channel_url.as_deref(),
        Some("https://release.capsem.org/health.json")
    );
}

#[test]
fn gateway_current_update_status_maps_to_tui_notice() {
    let state = state_from_status_and_update_json_for_test(
        gateway_empty_status_body(),
        gateway_update_current_status_body(),
        std::time::Duration::from_millis(11),
    )
    .expect("parse current update status");

    let notice = state.update_notice.expect("current update notice");
    assert_eq!(notice.kind, UpdateNoticeKind::Current);
}

#[test]
fn gateway_blocked_update_status_maps_to_tui_notice() {
    let state = state_from_status_and_update_json_for_test(
        gateway_empty_status_body(),
        gateway_update_blocked_image_status_body(),
        std::time::Duration::from_millis(11),
    )
    .expect("parse blocked update status");

    let notice = state.update_notice.as_ref().expect("blocked update notice");
    assert_eq!(notice.kind, UpdateNoticeKind::Blocked(vec![UpdateTrack::Images]));
    let snapshot = render_snapshot(&state, 120, 24).expect("render blocked update notice");
    assert!(snapshot.contains("updates blocked: images"));
}

#[test]
fn gateway_blocked_asset_update_status_maps_to_tui_notice() {
    let state = state_from_status_and_update_json_for_test(
        gateway_empty_status_body(),
        gateway_update_blocked_asset_status_body(),
        std::time::Duration::from_millis(11),
    )
    .expect("parse blocked asset update status");

    let notice = state.update_notice.as_ref().expect("blocked asset update notice");
    assert_eq!(notice.kind, UpdateNoticeKind::Blocked(vec![UpdateTrack::VmAssets]));
    let snapshot = render_snapshot(&state, 120, 24).expect("render blocked asset update notice");
    assert!(snapshot.contains("updates blocked: assets"));
}

#[test]
fn gateway_binary_update_with_blocked_images_keeps_both_tui_labels() {
    let state = state_from_status_and_update_json_for_test(
        gateway_empty_status_body(),
        gateway_update_binary_with_blocked_image_status_body(),
        std::time::Duration::from_millis(11),
    )
    .expect("parse mixed update status");

    let notice = state.update_notice.as_ref().expect("mixed update notice");
    assert_eq!(
        notice.kind,
        UpdateNoticeKind::AvailableWithBlocked {
            available: vec![UpdateTrack::Binary],
            blocked: vec![UpdateTrack::Images],
        }
    );
    let snapshot = render_snapshot(&state, 120, 24).expect("render mixed update notice");
    assert!(snapshot.contains("updates: binary; blocked: images"));
}

#[test]
fn tui_update_smoke_matrix_covers_release_states_and_atomic_action() {
    let cases = [
        (
            "no-update",
            gateway_update_current_status_body().to_string(),
            "updates: current",
        ),
        (
            "binary-update",
            gateway_update_matrix_body(true, false, false, None),
            "updates: binary",
        ),
        (
            "image-update",
            gateway_update_matrix_body(false, false, true, None),
            "updates: images",
        ),
        (
            "asset-update",
            gateway_update_matrix_body(false, true, false, None),
            "updates: assets",
        ),
        (
            "mixed-binary-asset-update",
            gateway_update_matrix_body(true, true, false, None),
            "updates: binary, assets",
        ),
        (
            "channel-error",
            gateway_update_matrix_body(false, false, false, Some("release channel timed out")),
            "updates: unavailable",
        ),
    ];

    for (name, update_body, expected) in cases {
        let state = state_from_status_and_update_json_for_test(
            gateway_status_body(),
            &update_body,
            std::time::Duration::from_millis(11),
        )
        .unwrap_or_else(|error| panic!("{name} update status should parse: {error}"));
        let snapshot = render_snapshot(&state, 120, 24)
            .unwrap_or_else(|error| panic!("{name} TUI snapshot should render: {error}"));
        assert!(snapshot.contains(expected), "{name} missing {expected}");
        assert!(snapshot.contains("help: alt+?"), "{name} lost help hint");
    }

    let mut app = App::new(fixture_state());
    assert_eq!(
        app.handle_key(key(KeyCode::Char('u'), KeyModifiers::ALT)),
        AppAction::Consumed
    );
    assert_eq!(app.pending_action(), Some(&ControlAction::Update));
    app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(
        app.handle_key(key(KeyCode::Char('a'), KeyModifiers::ALT)),
        AppAction::Forward
    );
    assert_eq!(app.pending_action(), None);
}

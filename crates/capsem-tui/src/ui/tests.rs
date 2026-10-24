use super::*;

#[test]
fn guest_metadata_never_emits_terminal_controls_or_uses_them_as_visible_width() {
    assert_eq!(truncate("\u{1b}]52;c;payload\u{7}hello\r\n", 80), "]52;c;payloadhello");
    assert_eq!(truncate("\u{1b}\u{7}abc", 2), "ab...");
    assert_eq!(truncate("normal label", 80), "normal label");
}

#[test]
fn approval_rows_render_semantic_status_colors_without_changing_decision_hints() {
    use capsem_sdk::models::{SecurityAskListResponse, SecurityAskRequest, SecurityAskStatus};

    for (status, expected) in [
        (SecurityAskStatus::Pending, ACTIVE),
        (SecurityAskStatus::Approved, ACTIVE),
        (SecurityAskStatus::Denied, Color::Magenta),
        (SecurityAskStatus::Expired, MUTED),
        (SecurityAskStatus::Cancelled, MUTED),
    ] {
        let heading = format!("{status:?} · review");
        let view = crate::app::ApprovalView {
            vm_id: "vm".into(),
            selected: 0,
            loading: false,
            resolving: false,
            error: None,
            list: Some(SecurityAskListResponse {
                vm_id: "vm".into(),
                generation: "generation".into(),
                pending_count: 0,
                requests: vec![SecurityAskRequest {
                    ask_id: "ask".into(),
                    event_id: "event".into(),
                    event_type: "http".into(),
                    rule_id: "rule".into(),
                    rule_name: "review".into(),
                    summary: "bounded summary".into(),
                    status,
                    created_at_unix_ms: 1,
                    expires_at_unix_ms: 2,
                    resolved_at_unix_ms: None,
                    resolution_reason: None,
                }],
            }),
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| approvals::render(frame, frame.area(), Some(&view)))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let cell = &buffer[(0, 2)];
        assert_eq!(cell.symbol(), &heading[..1]);
        assert_eq!(cell.fg, expected, "{heading}");
        assert_ne!(cell.fg, BAD, "negative approval status must use purple, not red");
        assert_ne!(cell.fg, ONLINE, "approval status must not use terminal green");
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        for hint in ["A approve once", "D deny", "R check current state", "Esc close"] {
            assert!(text.contains(hint), "{heading}: {hint}");
        }
    }
}

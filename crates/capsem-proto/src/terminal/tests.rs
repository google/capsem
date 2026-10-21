use super::*;

#[test]
fn terminal_state_roundtrips_without_losing_side_effect_types() {
    let state = TerminalState {
        revision: 42,
        reset: true,
        cols: 101,
        rows: 33,
        cursor: TerminalCursor {
            row: 7,
            col: 11,
            hidden: false,
        },
        modes: TerminalModes {
            alternate_screen: true,
            bracketed_paste: true,
            ..Default::default()
        },
        title: "agent".into(),
        icon_name: "work".into(),
        bell_count: 3,
        last_notification: Some(TerminalNotification {
            id: "notification-id".into(),
            title: "Ready".into(),
            body: "Review".into(),
        }),
        events: vec![TerminalEvent {
            kind: TerminalEventKind::Bell,
            visual: false,
            notification: None,
        }],
    };
    let bytes = rmp_serde::to_vec_named(&state).unwrap();
    assert_eq!(rmp_serde::from_slice::<TerminalState>(&bytes).unwrap(), state);
    let message = crate::ipc::ProcessToService::TerminalState {
        state: Box::new(state.clone()),
    };
    let bytes = rmp_serde::to_vec_named(&message).unwrap();
    let crate::ipc::ProcessToService::TerminalState { state: received } = rmp_serde::from_slice(&bytes).unwrap() else {
        panic!("wrong variant")
    };
    assert_eq!(*received, state);
}

#[test]
fn focus_is_explicit_on_the_wire() {
    let message = crate::ipc::ServiceToProcess::TerminalFocus { cols: 120, rows: 40 };
    let bytes = rmp_serde::to_vec_named(&message).unwrap();
    assert!(matches!(
        rmp_serde::from_slice(&bytes).unwrap(),
        crate::ipc::ServiceToProcess::TerminalFocus { cols: 120, rows: 40 }
    ));
}

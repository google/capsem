use super::*;

#[test]
fn terminal_checkpoint_limits_share_the_canonical_configuration() {
    let config: toml::Value = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/runtime-limits.toml"
    )))
    .unwrap();
    for (key, compiled) in [
        ("checkpoint_kib", crate::TERMINAL_CHECKPOINT_KIB),
        ("checkpoint_pending_kib", crate::TERMINAL_CHECKPOINT_PENDING_KIB),
        ("checkpoint_depth", crate::TERMINAL_CHECKPOINT_DEPTH),
    ] {
        assert_eq!(
            config["terminal"][key].as_integer().unwrap() as usize,
            compiled,
            "{key}"
        );
    }
}

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

#[test]
fn terminal_bounds_refuse_hostile_metadata_and_keep_pending_wrap_valid() {
    for (cols, rows) in [(0, 24), (80, 0), (513, 24), (80, 257), (512, 256)] {
        assert!(validate_size(cols, rows).is_err());
    }
    assert!(validate_size(512, 128).is_ok());
    let summary = TerminalSummary {
        title: "x".repeat(crate::TERMINAL_METADATA_BYTES + 1),
        ..Default::default()
    };
    assert!(summary.validate().is_err());
    let summary = TerminalSummary {
        title: "\x1b[31m".into(),
        ..Default::default()
    };
    assert!(summary.validate().is_err());
    let mut state = TerminalState {
        revision: 0,
        reset: true,
        cols: 80,
        rows: 24,
        cursor: TerminalCursor {
            row: 23,
            col: 80,
            hidden: false,
        },
        modes: Default::default(),
        title: "Ready".into(),
        icon_name: String::new(),
        bell_count: 0,
        last_notification: None,
        events: Vec::new(),
    };
    assert!(
        state.validate().is_ok(),
        "pending wrap cursor may equal the column count"
    );
    state.cursor.row = 24;
    assert!(state.validate().is_err());
    state.cursor.row = 0;
    state.cursor.col = 81;
    assert!(state.validate().is_err());
    state.cursor.col = 0;
    state.events.push(TerminalEvent {
        kind: TerminalEventKind::Notification,
        visual: false,
        notification: None,
    });
    assert!(state.validate().is_err());
    state.events[0].notification = Some(TerminalNotification {
        id: "n".into(),
        title: "Ready".into(),
        body: "Good".into(),
    });
    assert!(state.validate().is_ok());
    state.last_notification = Some(TerminalNotification {
        id: "n".into(),
        title: String::new(),
        body: "\x07".into(),
    });
    assert!(state.validate().is_err());
    let summary = TerminalSummary {
        last_notification: state.last_notification,
        ..Default::default()
    };
    assert!(summary.validate().is_err());
}

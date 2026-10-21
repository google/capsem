use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{
    coalesced_terminal_inputs, key_to_terminal_bytes, push_coalesced_event, run_terminal_manager, TerminalColor,
    TerminalCommand, TerminalEvent, TerminalInput, TerminalSurface,
};

#[test]
fn terminal_surface_keeps_recent_plain_output() {
    let mut surface = TerminalSurface::new();
    resize(&mut surface, "vm-1", 80, 2);
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"hello\r\nworld".to_vec(),
    });

    assert_eq!(surface.lines_for("vm-1", 2), vec!["hello", "world"]);
}

#[test]
fn terminal_surface_strips_basic_ansi_sequences() {
    let mut surface = TerminalSurface::new();
    resize(&mut surface, "vm-1", 80, 3);
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"\x1b[31mred\x1b[0m\n\x1b[2Jfresh".to_vec(),
    });

    assert!(
        surface.lines_for("vm-1", 3).iter().any(|line| line.contains("fresh")),
        "clear-screen output should leave fresh text on the parsed screen"
    );
}

#[test]
fn terminal_surface_preserves_xterm_colors() {
    let mut surface = TerminalSurface::new();
    resize(&mut surface, "vm-1", 80, 3);
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"\x1b[31mred\x1b[0m plain \x1b[1;32mgreen\x1b[0m".to_vec(),
    });

    let lines = surface.styled_lines_for("vm-1", 3);
    let spans = lines[0].spans();
    assert_eq!(spans[0].text, "red");
    assert_eq!(spans[0].style.fg, TerminalColor::Indexed(1));
    assert_eq!(spans[1].text, " plain ");
    assert_eq!(spans[2].text, "green");
    assert_eq!(spans[2].style.fg, TerminalColor::Indexed(2));
    assert!(spans[2].style.bold);
}

#[test]
fn terminal_surface_resize_same_dimensions_preserves_screen() {
    let mut surface = TerminalSurface::new();
    resize(&mut surface, "vm-1", 80, 4);
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"Antigravity CLI 1.0.8\r\n> write me a poem\r\ncreated poem.md".to_vec(),
    });
    let before = surface.lines_for("vm-1", 4);

    for _ in 0..10 {
        resize(&mut surface, "vm-1", 80, 4);
    }

    assert_eq!(surface.lines_for("vm-1", 4), before);
}

#[test]
fn terminal_surface_renders_agy_style_control_screen() {
    let mut surface = TerminalSurface::new();
    resize(&mut surface, "vm-1", 100, 12);
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: concat!(
            "\x1b[?1049h",
            "\x1b]0;Antigravity CLI\x07",
            "\x1b[2J\x1b[H",
            "\x1b[34mAntigravity CLI 1.0.8\x1b[0m\r\n",
            "user@example.com (Antigravity Starter Quota)\r\n",
            "Gemini 3.5 Flash (Medium)\r\n",
            "\r\n> hey!\r\n",
            "\x1b[31mThere was a network issue connecting to the server, please try again.\x1b[0m\r\n",
            "\x1b[6;1H> write me a poem in poem.md\r\n",
            "\x1b[7;1H\x1b[2KThought for 2s, 542 tokens\r\n",
            "\x1b[8;1H\x1b[32mCreate\x1b[0m(/root/poem.md)\r\n",
            ""
        )
        .as_bytes()
        .to_vec(),
    });

    let rendered = surface.lines_for("vm-1", 12).join("\n");
    assert!(rendered.trim().len() > 80, "{rendered}");
    assert!(rendered.contains("Antigravity CLI 1.0.8"), "{rendered}");
    assert!(rendered.contains("write me a poem in poem.md"), "{rendered}");
    assert!(rendered.contains("Thought for 2s, 542 tokens"), "{rendered}");
    assert!(rendered.contains("Create(/root/poem.md)"), "{rendered}");
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"\x1b[?1049l".to_vec(),
    });
    assert!(
        surface.lines_for("vm-1", 12).join("").trim().is_empty(),
        "leaving alternate screen restores primary, not stale agent contents"
    );
}

#[test]
fn terminal_events_coalesce_adjacent_output() {
    let mut events = Vec::new();
    push_coalesced_event(
        &mut events,
        TerminalEvent::Output {
            session_id: "vm-1".into(),
            bytes: b"hel".to_vec(),
        },
    );
    push_coalesced_event(
        &mut events,
        TerminalEvent::Output {
            session_id: "vm-1".into(),
            bytes: b"lo".to_vec(),
        },
    );

    assert_eq!(
        events,
        vec![TerminalEvent::Output {
            session_id: "vm-1".into(),
            bytes: b"hello".to_vec()
        }]
    );
}

#[test]
fn terminal_inputs_coalesce_adjacent_bytes_without_crossing_resize_boundaries() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tx.send(TerminalInput::Bytes(b"b".to_vec()))
        .expect("queue adjacent bytes");
    tx.send(TerminalInput::Bytes(b"c".to_vec()))
        .expect("queue adjacent bytes");
    tx.send(TerminalInput::Resize { cols: 100, rows: 24 })
        .expect("queue resize boundary");
    tx.send(TerminalInput::Bytes(b"d".to_vec()))
        .expect("queue bytes after resize");
    tx.send(TerminalInput::Bytes(b"e".to_vec()))
        .expect("queue bytes after resize");

    let inputs = coalesced_terminal_inputs(TerminalInput::Bytes(b"a".to_vec()), &mut rx);

    assert_eq!(
        inputs,
        vec![
            TerminalInput::Bytes(b"abc".to_vec()),
            TerminalInput::Resize { cols: 100, rows: 24 },
            TerminalInput::Bytes(b"de".to_vec()),
        ]
    );
}

#[test]
fn terminal_input_coalescing_is_bounded_even_for_all_byte_floods() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for _ in 1..=(super::MAX_TERMINAL_INPUTS_PER_SEND + 10) {
        tx.send(TerminalInput::Bytes(b"x".to_vec())).expect("queue byte flood");
    }
    drop(tx);

    let inputs = coalesced_terminal_inputs(TerminalInput::Bytes(b"x".to_vec()), &mut rx);

    assert_eq!(inputs.len(), 1);
    let TerminalInput::Bytes(bytes) = &inputs[0] else {
        panic!("expected coalesced byte input");
    };
    assert_eq!(bytes.len(), super::MAX_TERMINAL_INPUTS_PER_SEND);
    assert!(
        rx.try_recv().is_ok(),
        "terminal byte floods must yield back to the websocket read loop"
    );
}

#[test]
fn key_encoding_forwards_agent_input_keys() {
    assert_eq!(
        key_to_terminal_bytes(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        Some(b"q".to_vec())
    );
    assert_eq!(
        key_to_terminal_bytes(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Some(vec![b'\r'])
    );
    assert_eq!(
        key_to_terminal_bytes(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        Some(b"\x1b[C".to_vec())
    );
    assert_eq!(
        key_to_terminal_bytes(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Some(vec![3])
    );
}

#[test]
fn key_encoding_does_not_forward_super_shortcuts() {
    assert_eq!(
        key_to_terminal_bytes(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::SUPER)),
        None
    );
}

#[tokio::test]
async fn terminal_manager_reconnects_same_session_after_connection_task_exits() {
    let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
    let (event_tx, mut event_rx) = super::event_channel();
    let manager = tokio::spawn(run_terminal_manager(
        "http://127.0.0.1:9".to_string(),
        command_rx,
        event_tx,
    ));

    command_tx
        .send(TerminalCommand::Connect {
            session_id: "vm-1".to_string(),
            cols: 80,
            rows: 23,
        })
        .expect("send first connect");
    let first = recv_status(&mut event_rx).await;
    assert!(first.contains("token failed"), "{first}");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    command_tx
        .send(TerminalCommand::Connect {
            session_id: "vm-1".to_string(),
            cols: 80,
            rows: 23,
        })
        .expect("send reconnect");
    let second = recv_status(&mut event_rx).await;
    assert!(second.contains("token failed"), "{second}");

    command_tx.send(TerminalCommand::Shutdown).expect("send shutdown");
    manager.await.expect("terminal manager exits cleanly");
}

async fn recv_status(rx: &mut tokio::sync::mpsc::Receiver<TerminalEvent>) -> String {
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("receive terminal status")
        .expect("terminal status event");
    match event {
        TerminalEvent::Status { session_id, status } => {
            assert_eq!(session_id, "vm-1");
            status
        }
        event => panic!("expected status event, got {event:?}"),
    }
}

mod stream_protocol {
    use super::super::*;
    use capsem_sdk::models::stream::{
        decode_client_frame, encode_data, encode_status, ClientFrame, StreamChannel, StreamControl, StreamKind,
        StreamStatus,
    };

    #[test]
    fn a_terminal_stream_opens_with_start_then_its_window_size() {
        let frames = stream_start_frames(120, 40).unwrap();
        assert_eq!(
            decode_client_frame(&frames[0]).unwrap(),
            ClientFrame::Control(StreamControl::Start {
                kind: StreamKind::Terminal,
                command: None
            })
        );
        assert_eq!(
            decode_client_frame(&frames[1]).unwrap(),
            ClientFrame::Control(StreamControl::Focus { cols: 120, rows: 40 })
        );
    }

    #[test]
    fn input_becomes_stdin_and_resize_frames_and_zero_sizes_are_not_sent() {
        let stdin = stream_input_frames(TerminalInput::Bytes(b"ls\n".to_vec())).unwrap();
        assert_eq!(decode_client_frame(&stdin[0]).unwrap(), ClientFrame::Stdin(b"ls\n"));
        let resize = stream_input_frames(TerminalInput::Resize { cols: 90, rows: 20 }).unwrap();
        assert_eq!(
            decode_client_frame(&resize[0]).unwrap(),
            ClientFrame::Control(StreamControl::Resize { cols: 90, rows: 20 })
        );
        assert!(stream_input_frames(TerminalInput::Resize { cols: 0, rows: 20 })
            .unwrap()
            .is_empty());
    }

    #[test]
    fn server_frames_become_output_or_status() {
        assert_eq!(
            stream_server_event(&encode_data(StreamChannel::Stdout, b"$ \xff").unwrap()),
            Some(StreamEvent::Output(b"$ \xff".to_vec()))
        );
        assert_eq!(
            stream_server_event(
                &encode_status(&StreamStatus::Error {
                    message: "terminal closed".into()
                })
                .unwrap()
            ),
            Some(StreamEvent::Status("terminal closed".into()))
        );
        assert_eq!(
            stream_server_event(&encode_status(&StreamStatus::Started).unwrap()),
            Some(StreamEvent::Status("connected".into()))
        );
        assert!(matches!(stream_server_event(&[9, 1]),
            Some(StreamEvent::Status(message)) if message.starts_with("protocol error:")));
    }

    #[test]
    fn a_large_paste_is_split_without_losing_binary_bytes() {
        let bytes = (0..protocol::MAX_STREAM_DATA_BYTES * 2 + 7)
            .map(|index| index as u8)
            .collect::<Vec<_>>();
        let frames = stream_input_frames(TerminalInput::Bytes(bytes.clone())).unwrap();
        assert_eq!(frames.len(), 3);
        let mut received = Vec::new();
        for frame in frames {
            let ClientFrame::Stdin(chunk) = decode_client_frame(&frame).unwrap() else {
                panic!("paste is stdin");
            };
            assert!(chunk.len() <= protocol::MAX_STREAM_DATA_BYTES);
            received.extend_from_slice(chunk);
        }
        assert_eq!(received, bytes);
    }

    #[test]
    fn stream_url_names_the_vm_route_and_carries_the_token_for_the_upgrade() {
        assert_eq!(
            stream_ws_url("http://127.0.0.1:19222/", "vm 1", "t/k"),
            "ws://127.0.0.1:19222/vms/vm%201/stream?token=t%2Fk"
        );
        assert!(stream_ws_url("https://host", "vm", "t").starts_with("wss://host/vms/vm/stream"));
    }
}

fn resize(surface: &mut TerminalSurface, id: &str, cols: u16, rows: u16) {
    let state = state(cols, rows);
    surface.apply(TerminalEvent::State {
        session_id: id.into(),
        state,
    });
}

fn state(cols: u16, rows: u16) -> Box<super::protocol::TerminalState> {
    serde_json::from_value(serde_json::json!({
        "revision": 1, "reset": false, "cols": cols, "rows": rows,
        "cursor": {"row": 0, "col": 0, "hidden": false},
        "modes": {"alternate_screen": false, "application_cursor": false, "application_keypad": false, "bracketed_paste": false, "mouse_protocol": "None", "mouse_encoding": "Default"},
        "title": "", "icon_name": "", "bell_count": 0, "last_notification": null, "events": []
    })).unwrap()
}

#[tokio::test]
async fn terminal_event_delivery_backpressures_and_unblocks_when_the_view_leaves() {
    use std::future::Future;
    use std::task::Poll;
    let (tx, rx) = super::event_channel();
    let mut producer = Box::pin(async move {
        for _ in 0..=super::protocol::MAX_TERMINAL_VIEW_UPDATES {
            super::send_status(&tx, "box", "event").await;
        }
    });
    std::future::poll_fn(|cx| {
        assert!(
            producer.as_mut().poll(cx).is_pending(),
            "terminal events must apply backpressure instead of growing without a bound"
        );
        Poll::Ready(())
    })
    .await;
    drop(rx);
    tokio::time::timeout(std::time::Duration::from_secs(1), producer)
        .await
        .expect("dropping the view releases the blocked producer");
}

#[test]
fn modified_navigation_keys_reach_the_application() {
    assert_eq!(
        key_to_terminal_bytes(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
        Some(b"\x1b[1;5A".to_vec())
    );
    assert_eq!(
        key_to_terminal_bytes(KeyEvent::new(KeyCode::F(5), KeyModifiers::SHIFT)),
        Some(b"\x1b[15;2~".to_vec())
    );
}

#[test]
fn session_modes_control_navigation_paste_and_cursor_projection() {
    let mut surface = TerminalSurface::new();
    let mut state = state(80, 24);
    state.modes.application_cursor = true;
    state.modes.bracketed_paste = true;
    state.cursor.row = 23;
    state.cursor.col = 80;
    surface.apply(TerminalEvent::State {
        session_id: "vm".into(),
        state: state.clone(),
    });
    assert_eq!(
        surface.input_for("vm", KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        Some(b"\x1bOA".to_vec())
    );
    assert_eq!(
        surface.input_for("vm", KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
        Some(b"\x1b[1;5A".to_vec())
    );
    assert_eq!(surface.paste_for("vm", "line\nnext"), b"\x1b[200~line\nnext\x1b[201~");
    assert_eq!(surface.cursor_for("vm", 80, 10), Some((79, 9)));
    assert_eq!(surface.cursor_for("vm", 40, 10), None);
    state.cursor.row = 0;
    assert!(state.validate().is_ok());
    surface.apply(TerminalEvent::State {
        session_id: "vm".into(),
        state: state.clone(),
    });
    assert_eq!(surface.cursor_for("vm", 80, 10), None);
    state.cursor.hidden = true;
    state.modes.application_cursor = false;
    state.modes.bracketed_paste = false;
    surface.apply(TerminalEvent::State {
        session_id: "vm".into(),
        state,
    });
    assert_eq!(surface.cursor_for("vm", 80, 24), None);
    assert_eq!(
        surface.input_for("vm", KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        Some(b"\x1b[A".to_vec())
    );
    assert_eq!(surface.paste_for("vm", "plain"), b"plain");
}

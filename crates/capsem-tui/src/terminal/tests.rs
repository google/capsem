use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{
    key_to_terminal_bytes, push_coalesced_event, run_terminal_manager, TerminalColor,
    TerminalCommand, TerminalEvent, TerminalSurface,
};

#[test]
fn terminal_surface_keeps_recent_plain_output() {
    let mut surface = TerminalSurface::new();
    surface.resize("vm-1", 80, 2);
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"hello\r\nworld".to_vec(),
    });

    assert_eq!(surface.lines_for("vm-1", 2), vec!["hello", "world"]);
}

#[test]
fn terminal_surface_strips_basic_ansi_sequences() {
    let mut surface = TerminalSurface::new();
    surface.resize("vm-1", 80, 3);
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"\x1b[31mred\x1b[0m\n\x1b[2Jfresh".to_vec(),
    });

    assert!(
        surface
            .lines_for("vm-1", 3)
            .iter()
            .any(|line| line.contains("fresh")),
        "clear-screen output should leave fresh text on the parsed screen"
    );
}

#[test]
fn terminal_surface_preserves_xterm_colors() {
    let mut surface = TerminalSurface::new();
    surface.resize("vm-1", 80, 3);
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
    surface.resize("vm-1", 80, 4);
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"Antigravity CLI 1.0.8\r\n> write me a poem\r\ncreated poem.md".to_vec(),
    });
    let before = surface.lines_for("vm-1", 4);

    for _ in 0..10 {
        surface.resize("vm-1", 80, 4);
    }

    assert_eq!(surface.lines_for("vm-1", 4), before);
}

#[test]
fn terminal_surface_renders_agy_style_control_screen() {
    let mut surface = TerminalSurface::new();
    surface.resize("vm-1", 100, 12);
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
            "\x1b[?1049l"
        )
        .as_bytes()
        .to_vec(),
    });

    let rendered = surface.lines_for("vm-1", 12).join("\n");
    assert!(rendered.trim().len() > 80, "{rendered}");
    assert!(rendered.contains("Antigravity CLI 1.0.8"), "{rendered}");
    assert!(
        rendered.contains("write me a poem in poem.md"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Thought for 2s, 542 tokens"),
        "{rendered}"
    );
    assert!(rendered.contains("Create(/root/poem.md)"), "{rendered}");
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
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let event_rx = std::sync::Arc::new(std::sync::Mutex::new(event_rx));
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
    let first = recv_status(event_rx.clone()).await;
    assert!(first.contains("token failed"), "{first}");
    std::thread::sleep(std::time::Duration::from_millis(50));

    command_tx
        .send(TerminalCommand::Connect {
            session_id: "vm-1".to_string(),
            cols: 80,
            rows: 23,
        })
        .expect("send reconnect");
    let second = recv_status(event_rx.clone()).await;
    assert!(second.contains("token failed"), "{second}");

    command_tx
        .send(TerminalCommand::Shutdown)
        .expect("send shutdown");
    manager.await.expect("terminal manager exits cleanly");
}

async fn recv_status(
    rx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<TerminalEvent>>>,
) -> String {
    let event = tokio::task::spawn_blocking(move || {
        rx.lock()
            .expect("lock terminal event receiver")
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("terminal status event")
    })
    .await
    .expect("receive terminal status");
    match event {
        TerminalEvent::Status { session_id, status } => {
            assert_eq!(session_id, "vm-1");
            status
        }
        event => panic!("expected status event, got {event:?}"),
    }
}

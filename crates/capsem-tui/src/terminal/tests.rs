use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{key_to_terminal_bytes, TerminalEvent, TerminalSurface};

#[test]
fn terminal_surface_keeps_recent_plain_output() {
    let mut surface = TerminalSurface::new();
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"hello\r\nworld".to_vec(),
    });

    assert_eq!(surface.lines_for("vm-1", 2), vec!["hello", "world"]);
}

#[test]
fn terminal_surface_strips_basic_ansi_sequences() {
    let mut surface = TerminalSurface::new();
    surface.apply(TerminalEvent::Output {
        session_id: "vm-1".into(),
        bytes: b"\x1b[31mred\x1b[0m\n\x1b[2Jfresh".to_vec(),
    });

    assert_eq!(surface.lines_for("vm-1", 3), vec!["fresh"]);
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

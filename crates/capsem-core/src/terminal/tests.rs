use super::*;

fn display(update: &TerminalUpdate) -> vt100::Parser {
    let mut client = vt100::Parser::new(update.state.rows, update.state.cols, 0);
    client.process(&update.render);
    client
}

#[test]
fn attach_reconstructs_the_screen_instead_of_truncating_control_history() {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"\x1b[31m");
    terminal.publish(vec![b'x'; 70_000]);
    terminal.publish(b"\x1b[Hred");
    let (_, snapshot, _) = terminal.subscribe();
    let client = display(&snapshot);
    assert_eq!(client.screen().cell(0, 0).unwrap().fgcolor(), vt100::Color::Idx(1));
    assert!(client.screen().contents().starts_with("red"));
    assert!(snapshot.render.len() < 70_000);
}

#[test]
fn parser_state_survives_split_utf8_csi_and_osc_and_preserves_modes() {
    let terminal = SessionTerminal::new(8);
    let input = "\x1b[?1049h\x1b[?2004h\x1b[?1h\x1b[?25l\x1b[38;2;10;20;30m界e\u{301}\x1b]2;Agent working\x1b\\";
    for byte in input.as_bytes() {
        terminal.publish(vec![*byte]);
    }
    let (_, snapshot, _) = terminal.subscribe();
    assert_eq!(snapshot.state.title, "Agent working");
    assert!(snapshot.state.modes.alternate_screen);
    assert!(snapshot.state.modes.bracketed_paste);
    assert!(snapshot.state.modes.application_cursor);
    assert!(snapshot.state.cursor.hidden);
    let client = display(&snapshot);
    assert_eq!(client.screen().cell(0, 0).unwrap().contents(), "界");
    assert_eq!(client.screen().cell(0, 2).unwrap().contents(), "e\u{301}");
    assert_eq!(
        client.screen().cell(0, 0).unwrap().fgcolor(),
        vt100::Color::Rgb(10, 20, 30)
    );
}

#[tokio::test]
async fn alternate_screen_and_saved_cursor_restore_without_client_history() {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"shell\x1b7\x1b[?1049happ");
    let (_, snapshot, mut live) = terminal.subscribe();
    let mut client = display(&snapshot);
    assert!(client.screen().contents().contains("app"));
    terminal.publish(b"\x1b[?1049l\x1b8!");
    let update = live.recv().await.unwrap();
    client.process(&update.render);
    assert!(!client.screen().alternate_screen());
    assert!(client.screen().contents().starts_with("shell!"));
}

#[tokio::test]
async fn events_are_live_only_and_identified_state_survives_attach() {
    let terminal = SessionTerminal::new(8);
    let (_, _, mut live) = terminal.subscribe();
    terminal.publish(b"\x1b]0;Title\x07\x07\x1b]777;notify;Review;Ready\x07");
    let update = live.recv().await.unwrap();
    assert_eq!(update.state.title, "Title");
    assert_eq!(update.state.icon_name, "Title");
    assert_eq!(update.state.bell_count, 1);
    assert_eq!(update.state.events.len(), 2);
    assert_eq!(
        update.state.events[0].kind,
        capsem_proto::terminal::TerminalEventKind::Bell
    );
    assert!(!update.state.events[0].visual);
    let (_, snapshot, _) = terminal.subscribe();
    assert!(snapshot.state.events.is_empty());
    assert_eq!(
        snapshot.state.last_notification.as_ref().unwrap().id,
        update.state.last_notification.as_ref().unwrap().id
    );
    assert_eq!(snapshot.state.last_notification.as_ref().unwrap().body, "Ready");
    assert_eq!(snapshot.state.bell_count, 1);
    assert!(!snapshot.render.contains(&7));
}

#[tokio::test]
async fn focused_view_owns_dimensions_and_snapshot_precedes_live_updates() {
    let terminal = SessionTerminal::new(8);
    let (a, snapshot, mut live) = terminal.subscribe();
    assert!(terminal.focus(a, 101, 33).unwrap());
    let resized = live.recv().await.unwrap();
    assert_eq!((resized.state.rows, resized.state.cols), (33, 101));
    assert!(resized.state.reset);
    assert!(resized.state.revision > snapshot.state.revision);
    let (b, _, _) = terminal.subscribe();
    assert!(terminal.focus(b, 60, 20).unwrap());
    assert!(!terminal.resize(a, 150, 45).unwrap());
    assert!(!terminal.controls(a));
    assert!(terminal.controls(b));
    assert!(terminal.resize(b, 0, 1).is_err());
    assert!(terminal.resize(b, u16::MAX, u16::MAX).is_err());
    terminal.release(a);
    assert!(terminal.controls(b));
    terminal.release(b);
    assert!(!terminal.controls(b));
}

#[test]
fn terminal_answers_queries_once_at_the_session() {
    let terminal = SessionTerminal::new(8);
    let reply = terminal.publish(b"hello\x1b[6n\x1b[18t");
    assert_eq!(reply, b"\x1b[1;6R\x1b[8;24;80t");
    let (_, snapshot, _) = terminal.subscribe();
    assert!(!snapshot.render.windows(4).any(|bytes| bytes == b"\x1b[6n"));
}

#[tokio::test]
async fn metadata_watch_retains_latest_state_without_any_viewers() {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"\x1b]2;First\x07\x07");
    terminal.publish(b"\x1b]2;Latest\x07\x07");
    let mut summaries = terminal.subscribe_summary();
    assert_eq!(summaries.borrow_and_update().title, "Latest");
    assert_eq!(summaries.borrow().bell_count, 2);
    terminal.publish(b"ordinary output");
    assert!(
        !summaries.has_changed().unwrap(),
        "cursor/output does not wake inventory observers"
    );
    terminal.publish(b"\x1b]9;Ready\x07");
    summaries.changed().await.unwrap();
    assert_eq!(summaries.borrow().last_notification.as_ref().unwrap().body, "Ready");
}

#[test]
fn first_focus_propagates_dimensions_even_when_the_grid_matches() {
    let terminal = SessionTerminal::new(8);
    let (view, _, _) = terminal.subscribe();
    assert!(terminal.focus(view, 80, 24).unwrap());
    assert!(
        !terminal.focus(view, 80, 24).unwrap(),
        "unchanged focus must not repeatedly redraw the application"
    );
}

#[test]
fn reconstructed_display_preserves_ansi_256_and_true_color() {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"\x1b[91mA\x1b[38;5;202mB\x1b[48;2;40;50;60mC");
    let (_, snapshot, _) = terminal.subscribe();
    let client = display(&snapshot);
    assert_eq!(client.screen().cell(0, 0).unwrap().fgcolor(), vt100::Color::Idx(9));
    assert_eq!(client.screen().cell(0, 1).unwrap().fgcolor(), vt100::Color::Idx(202));
    assert_eq!(
        client.screen().cell(0, 2).unwrap().bgcolor(),
        vt100::Color::Rgb(40, 50, 60)
    );
}

#[test]
fn oversized_control_strings_are_discarded_and_the_terminal_recovers() {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"\x1b]2;Retained\x07\x1b]2;");
    for _ in 0..256 {
        terminal.publish(vec![b'x'; 4096]);
    }
    terminal.publish(b"\x07visible\x1b]9;Ready\x1b\\");
    let (_, snapshot, _) = terminal.subscribe();
    assert_eq!(
        snapshot.state.title, "Retained",
        "oversized OSC must not reach the parser"
    );
    assert_eq!(snapshot.state.last_notification.as_ref().unwrap().body, "Ready");
    assert_eq!(display(&snapshot).screen().contents(), "visible");
}

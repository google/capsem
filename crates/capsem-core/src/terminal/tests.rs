use super::*;

#[test]
fn suspend_seals_view_authority_without_dropping_accepted_guest_output() {
    let terminal = SessionTerminal::new(8);
    let (view, _, _) = terminal.subscribe();
    terminal.focus(view, 100, 30).unwrap();
    terminal.seal_views();
    assert!(!terminal.controls(view));
    assert!(terminal.focus(view, 120, 40).is_err());
    assert!(terminal.resize(view, 120, 40).is_err());
    let (late, _, _) = terminal.subscribe();
    assert!(terminal.focus(late, 120, 40).is_err());
    terminal.publish(b"accepted\x1b]2;Saved\x07");
    let state = terminal.subscribe().1.state;
    assert_eq!((state.cols, state.rows), (100, 30));
    assert_eq!(state.title, "Saved");
    let restored = SessionTerminal::from_checkpoint(&terminal.checkpoint().unwrap(), 8).unwrap();
    let (fresh, _, _) = restored.subscribe();
    assert!(restored.focus(fresh, 100, 30).unwrap());
    assert!(restored.controls(fresh));
}

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

#[test]
fn every_utf8_split_preserves_ascii_before_the_next_multibyte_character() {
    let input = "\x1b[38;2;23;89;144m漢字 e\u{301} 🦀\x1b]2;Ready\x07";
    let whole = SessionTerminal::new(8);
    whole.publish(input.as_bytes());
    let (_, expected, _) = whole.subscribe();
    for split in 0..=input.len() {
        let terminal = SessionTerminal::new(8);
        terminal.publish(&input.as_bytes()[..split]);
        terminal.publish(&input.as_bytes()[split..]);
        let (_, snapshot, _) = terminal.subscribe();
        assert_eq!(snapshot.render, expected.render, "split at byte {split}");
        assert_eq!(snapshot.state.cursor, expected.state.cursor);
        assert_eq!(snapshot.state.title, "Ready");
        assert_eq!(snapshot.state.bell_count, 0);
    }
}

#[test]
fn interrupted_utf8_keeps_control_sequences_and_invalid_bytes_consistent() {
    for input in [
        b"\xc3\xc3\xa9 visible".as_slice(),
        b"\xe2\x82\x1b[31mred",
        b"\xe0\x80 invalid",
        b"\x80\x9f\xff\xfe visible",
        b"\xf0\x9f\xa6\x1b]2;Title\x07visible",
    ] {
        let whole = SessionTerminal::new(8);
        whole.publish(input);
        let (_, expected, _) = whole.subscribe();
        for split in 0..=input.len() {
            let terminal = SessionTerminal::new(8);
            terminal.publish(&input[..split]);
            terminal.publish([]);
            terminal.publish(&input[split..]);
            let (_, snapshot, _) = terminal.subscribe();
            assert_eq!(snapshot.render, expected.render, "{input:?} split {split}");
            assert_eq!(snapshot.state.title, expected.state.title);
            assert_eq!(snapshot.state.bell_count, expected.state.bell_count);
        }
    }
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

#[test]
fn owner_reflows_soft_wrapped_text_when_the_view_grows() {
    let terminal = SessionTerminal::new(8);
    let (view, _, _) = terminal.subscribe();
    terminal.focus(view, 8, 4).unwrap();
    terminal.publish(b"abcdefghijk");
    terminal.resize(view, 12, 4).unwrap();
    let (_, snapshot, _) = terminal.subscribe();
    assert_eq!(display(&snapshot).screen().contents(), "abcdefghijk");
}

#[test]
fn owner_projects_explicit_links_and_dynamic_palette_colors() {
    let terminal = SessionTerminal::new(8);
    let url = b"https://example.invalid/owned-link";
    terminal.publish(
        b"\x1b]4;1;rgb:12/34/56\x07\x1b[31m\x1b]8;id=owned;https://example.invalid/owned-link\x1b\\click\x1b]8;;\x1b\\",
    );
    let (_, snapshot, _) = terminal.subscribe();
    assert!(snapshot.render.windows(url.len()).any(|bytes| bytes == url));
    let link_start = b"\x1b]8;id=owned;";
    assert!(snapshot
        .render
        .windows(link_start.len())
        .any(|bytes| bytes == link_start));
    assert_eq!(
        display(&snapshot).screen().cell(0, 0).unwrap().fgcolor(),
        vt100::Color::Rgb(0x12, 0x34, 0x56)
    );
}

#[test]
fn unfinished_synchronized_output_flushes_once_when_the_owner_expires_it() {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"before\x1b[?2026hheld\x07\x1b]2;held-title\x07\x1b[6n");
    let deadline = terminal.sync_deadline().expect("missing sync deadline");
    let (_, before, _) = terminal.subscribe();
    assert_eq!(display(&before).screen().contents(), "before");
    assert_eq!(before.state.bell_count, 0);
    assert!(terminal
        .expire_sync_at(deadline.checked_sub(std::time::Duration::from_nanos(1)).unwrap())
        .is_empty());
    assert_eq!(terminal.expire_sync_at(deadline), b"\x1b[1;11R");
    let (_, after, _) = terminal.subscribe();
    assert_eq!(display(&after).screen().contents(), "beforeheld");
    assert_eq!(after.state.title, "held-title");
    assert_eq!(after.state.bell_count, 1);
    assert!(terminal.sync_deadline().is_none());
    assert!(terminal.expire_sync_at(deadline).is_empty());
    assert_eq!(terminal.subscribe().1.state.bell_count, 1);
}

#[test]
fn incremental_projection_matches_a_fresh_view_after_edits_and_combining_input() {
    let terminal = SessionTerminal::new(8);
    let (_, snapshot, mut updates) = terminal.subscribe();
    let mut client = display(&snapshot);
    for input in [
        "hello",
        "\r\x1b[31mred",
        "\r\x1b[0mdefault",
        "\x1b[2;1H界",
        "\u{301}",
        "\x1b[1;5H\x1b[2X",
        "\x1b[1;5H\x1b[K",
        "\x1b[?1049h\x1b[38;2;1;2;3malt",
        "\x1b[?1049l",
        "\x1b]4;1;rgb:12/34/56\x07",
        "\x1b[H\x1b[2K",
    ] {
        terminal.publish(input.as_bytes());
        client.process(&updates.try_recv().unwrap().render);
        let (_, snapshot, _) = terminal.subscribe();
        let expected = display(&snapshot);
        assert_eq!(client.screen().contents(), expected.screen().contents(), "{input:?}");
        for row in 0..snapshot.state.rows {
            for col in 0..snapshot.state.cols {
                let actual = client.screen().cell(row, col).unwrap();
                let expected = expected.screen().cell(row, col).unwrap();
                assert_eq!(
                    (actual.fgcolor(), actual.bgcolor(), actual.bold(), actual.italic()),
                    (
                        expected.fgcolor(),
                        expected.bgcolor(),
                        expected.bold(),
                        expected.italic()
                    ),
                    "{input:?} at {row}:{col}"
                );
            }
        }
    }
}

#[test]
fn owner_retention_and_host_capabilities_remain_bounded() {
    let terminal = SessionTerminal::new(8);
    let (view, _, mut updates) = terminal.subscribe();
    assert!(terminal.focus(view, 1, 24).is_err());
    terminal.publish(b"e");
    terminal.publish("\u{301}".repeat(capsem_proto::TERMINAL_COMBINING_CHARS * 4));
    let (_, snapshot, _) = terminal.subscribe();
    // The independent vt100 display fixture itself caps cell bytes at 22.
    // Verify the actual wire projection rather than that fixture's capacity.
    assert_eq!(
        snapshot
            .render
            .windows(2)
            .filter(|bytes| *bytes == [0xcc, 0x81])
            .count(),
        capsem_proto::TERMINAL_COMBINING_CHARS
    );
    let reply = terminal.publish(b"\x1b]52;c;aGVsbG8=\x07\x1b]52;c;?\x07");
    assert!(reply.is_empty());
    assert!(terminal.subscribe().1.state.events.is_empty());
    while let Ok(update) = updates.try_recv() {
        assert!(update.state.events.is_empty());
    }
    let lines = capsem_proto::TERMINAL_HISTORY_CELLS / snapshot.state.cols as usize + snapshot.state.rows as usize + 10;
    terminal.publish("line\r\n".repeat(lines));
    let inner = terminal.inner.lock().unwrap();
    assert_eq!(
        inner.term.history_size(),
        capsem_proto::TERMINAL_HISTORY_CELLS / inner.term.columns()
    );
    drop(inner);
}

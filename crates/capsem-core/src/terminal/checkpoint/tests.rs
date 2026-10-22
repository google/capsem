use super::{LimitedWriter, Saved, MAX_CHECKPOINT_BYTES};
use crate::terminal::{SessionTerminal, TerminalUpdate};
use serde::Serialize;
use std::io::Write;

fn snapshot(terminal: &SessionTerminal) -> TerminalUpdate {
    terminal.subscribe().1
}

fn assert_same(left: &SessionTerminal, right: &SessionTerminal) {
    let left = snapshot(left);
    let right = snapshot(right);
    assert_eq!(left.state, right.state);
    assert_eq!(left.render, right.render);
}

#[test]
fn checkpoint_preserves_both_screens_saved_cursors_attributes_and_effect_identity() {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"PRIMARY\x1b[3;7H\x1b7\x1b[?1h\x1b[?2004h\x1b[?1049h\x1b[HALTERNATE\x1b[4;8H\x1b7\x1b[38;2;10;20;30m\x1b]4;7;rgb:12/34/56\x07\x1b]0;Saved\x07\x07\x1b]777;notify;Review;Ready\x07");
    let before = snapshot(&terminal);
    let restored = SessionTerminal::from_checkpoint(&terminal.checkpoint().unwrap(), 8).unwrap();
    assert_same(&terminal, &restored);
    let (_, attached, mut updates) = restored.subscribe();
    assert!(attached.state.events.is_empty());
    assert_eq!(attached.state.last_notification, before.state.last_notification);
    assert_eq!(attached.state.bell_count, 1);
    assert!(updates.try_recv().is_err());
    for suffix in [b"\x1b8X\x1b[?1049l".as_slice(), b"\x1b8Y\x1b[6n"] {
        assert_eq!(terminal.publish(suffix), restored.publish(suffix));
        assert_same(&terminal, &restored);
    }
    assert!(snapshot(&restored).render.windows(7).any(|window| window == b"PRIMARY"));
}

#[test]
fn every_fragmented_utf8_csi_and_osc_checkpoint_continues_without_replaying_input() {
    let input = "\x1b[38:2::23:89:144m漢字 e\u{301} 🦀\x1b]0;Ready\x07\x1b[?1049h\x1b[3;4Halt\x1b[?1049l\x1b[6n";
    for split in 0..=input.len() {
        let terminal = SessionTerminal::new(8);
        terminal.publish(&input.as_bytes()[..split]);
        let restored = SessionTerminal::from_checkpoint(&terminal.checkpoint().unwrap(), 8).unwrap();
        assert_same(&terminal, &restored);
        assert_eq!(
            terminal.publish(&input.as_bytes()[split..]),
            restored.publish(&input.as_bytes()[split..]),
            "split {split}"
        );
        assert_same(&terminal, &restored);
    }
}

#[test]
fn restored_history_reflows_when_a_later_view_changes_dimensions() {
    let terminal = SessionTerminal::new(8);
    for line in 0..100 {
        terminal.publish(format!("line-{line:03} {}\r\n", "x".repeat(65)));
    }
    let restored = SessionTerminal::from_checkpoint(&terminal.checkpoint().unwrap(), 8).unwrap();
    let (left, _, _) = terminal.subscribe();
    let (right, _, _) = restored.subscribe();
    for (cols, rows) in [(40, 70), (101, 33), (150, 45)] {
        terminal.focus(left, cols, rows).unwrap();
        restored.focus(right, cols, rows).unwrap();
        assert_same(&terminal, &restored);
    }
}

#[test]
fn discarded_oversized_osc_and_pending_escape_survive_checkpoint() {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"\x1b]2;Retained\x07\x1b]2;");
    terminal.publish(vec![b'x'; capsem_proto::TERMINAL_CONTROL_STRING_BYTES + 1]);
    let restored = SessionTerminal::from_checkpoint(&terminal.checkpoint().unwrap(), 8).unwrap();
    terminal.publish(b"ignored\x07OK");
    restored.publish(b"ignored\x07OK");
    assert_same(&terminal, &restored);
    assert_eq!(snapshot(&restored).state.title, "Retained");
}

#[test]
fn synchronized_output_restores_remaining_time_and_delivers_effects_and_query_once() {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"before\x1b[?2026hheld\x07\x1b]2;Delayed\x07\x1b[6n");
    let bytes = terminal.checkpoint().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(175));
    let restored = SessionTerminal::from_checkpoint(&bytes, 8).unwrap();
    let deadline = restored.sync_deadline().unwrap();
    assert!(
        deadline > std::time::Instant::now(),
        "suspended wall time consumed sync continuation"
    );
    assert!(restored.expire_sync().is_empty());
    let (_, _, mut updates) = restored.subscribe();
    let replies = restored.expire_sync_at(deadline);
    assert_eq!(replies, b"\x1b[1;11R");
    let update = updates.try_recv().unwrap();
    assert_eq!(update.state.title, "Delayed");
    assert_eq!(update.state.bell_count, 1);
    assert_eq!(update.state.events.len(), 1);
    assert!(restored.expire_sync_at(deadline).is_empty());
    assert!(updates.try_recv().is_err());
}

#[test]
fn checkpoint_restore_drops_old_view_authority() {
    let terminal = SessionTerminal::new(8);
    let (old_view, _, _) = terminal.subscribe();
    terminal.focus(old_view, 100, 30).unwrap();
    let restored = SessionTerminal::from_checkpoint(&terminal.checkpoint().unwrap(), 8).unwrap();
    assert!(!restored.controls(old_view));
    assert!(restored.focus(old_view, 100, 30).is_err());
    let (fresh_view, _, _) = restored.subscribe();
    assert!(restored.focus(fresh_view, 100, 30).unwrap());
}

fn saved_fixture() -> Saved {
    let terminal = SessionTerminal::new(8);
    terminal.publish(b"visible\x07\x1b]0;Title\x07\x1b]777;notify;Review;Ready\x07");
    let bytes = terminal.checkpoint().unwrap();
    assert!(SessionTerminal::from_checkpoint(&bytes, 8).is_ok());
    rmp_serde::from_slice(&bytes).unwrap()
}

fn encoded(saved: &Saved) -> Vec<u8> {
    rmp_serde::to_vec_named(saved).unwrap()
}

#[test]
fn rejects_invalid_envelopes_dimensions_metadata_and_osc_continuations() {
    type Mutation = fn(&mut Saved);
    let mutations: &[Mutation] = &[
        |s| s.version += 1,
        |s| s.revision = u64::MAX,
        |s| s.cols = 0,
        |s| s.rows = 0,
        |s| s.cols += 1,
        |s| s.metadata.title.push('\x1b'),
        |s| s.metadata.icon_name = "x".repeat(capsem_proto::TERMINAL_METADATA_BYTES + 1),
        |s| s.metadata.last_notification.as_mut().unwrap().id.clear(),
        |s| s.metadata.last_notification.as_mut().unwrap().body.push('\x07'),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut saved = saved_fixture();
        mutate(&mut saved);
        assert!(
            SessionTerminal::from_checkpoint(&encoded(&saved), 8).is_err(),
            "mutation {index}"
        );
    }
    // A valid re-encoded baseline proves malformed cases reach their intended
    // validation boundary rather than failing because of a fixture format.
    let baseline = serde_json::to_value(saved_fixture()).unwrap();
    let valid: Saved = serde_json::from_value(baseline.clone()).unwrap();
    assert!(SessionTerminal::from_checkpoint(&encoded(&valid), 8).is_ok());
    for osc in [
        serde_json::json!({"escape":true,"osc":true,"discarded":false,"pending":[]}),
        serde_json::json!({"escape":false,"osc":false,"discarded":true,"pending":[]}),
        serde_json::json!({"escape":false,"osc":true,"discarded":false,"pending":[93,7]}),
        serde_json::json!({"escape":false,"osc":false,"discarded":false,"pending":[93]}),
    ] {
        let mut saved = baseline.clone();
        saved["osc_budget"] = osc;
        let saved: Saved = serde_json::from_value(saved).unwrap();
        assert!(SessionTerminal::from_checkpoint(&encoded(&saved), 8).is_err());
    }
    for owner in ["model", "parser"] {
        let mut saved = baseline.clone();
        saved[owner]["version"] = serde_json::json!(0);
        let saved: Saved = serde_json::from_value(saved).unwrap();
        assert!(SessionTerminal::from_checkpoint(&encoded(&saved), 8).is_err());
    }
}

#[test]
fn rejects_truncation_trailing_unknown_deep_and_oversized_input() {
    let saved = saved_fixture();
    let mut bytes = encoded(&saved);
    for length in [0, 1, bytes.len() / 2, bytes.len() - 1] {
        assert!(SessionTerminal::from_checkpoint(&bytes[..length], 8).is_err());
    }
    assert!(SessionTerminal::from_checkpoint(&bytes, 0).is_err());
    bytes.push(0);
    assert!(SessionTerminal::from_checkpoint(&bytes, 8).is_err());
    #[derive(Serialize)]
    struct Extended<'a> {
        #[serde(flatten)]
        saved: &'a Saved,
        unrecognized: bool,
    }
    let unknown = Extended {
        saved: &saved,
        unrecognized: true,
    };
    assert!(SessionTerminal::from_checkpoint(&rmp_serde::to_vec_named(&unknown).unwrap(), 8).is_err());
    let mut deep = vec![0x91; capsem_proto::TERMINAL_CHECKPOINT_DEPTH + 1];
    deep.push(0xc0);
    assert!(SessionTerminal::from_checkpoint(&deep, 8).is_err());
    // Forged allocation hints and truncated payloads must fail without trusting
    // attacker-supplied lengths. The outer byte cap is checked before decoding.
    for marker in [0xdb, 0xdd, 0xdf] {
        assert!(SessionTerminal::from_checkpoint(&[marker, 255, 255, 255, 255], 8).is_err());
    }
    assert!(matches!(
        SessionTerminal::from_checkpoint(&vec![0; MAX_CHECKPOINT_BYTES + 1], 8),
        Err(super::super::CheckpointError::TooLarge)
    ));
}

#[test]
fn checkpoint_encoding_stops_at_the_allocation_budget() {
    let mut writer = LimitedWriter::new(7);
    writer.write_all(b"1234").unwrap();
    writer.write_all(b"567").unwrap();
    assert_eq!(writer.bytes, b"1234567");
    assert!(writer.write_all(b"8").is_err());
    assert_eq!(writer.bytes, b"1234567");
    assert!(writer.bytes.capacity() <= 7);
    let mut writer = LimitedWriter::new(16);
    assert!(saved_fixture()
        .serialize(&mut rmp_serde::Serializer::new(&mut writer).with_struct_map())
        .is_err());
    assert!(writer.bytes.len() <= 16);
    assert!(writer.bytes.capacity() <= 16);
}

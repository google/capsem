use super::*;

#[test]
fn parse_resize_valid() {
    let (cols, rows) = parse_resize_message(r#"{"cols": 80, "rows": 24}"#).unwrap();
    assert_eq!(cols, 80);
    assert_eq!(rows, 24);
}

#[test]
fn parse_resize_large_values() {
    let (cols, rows) = parse_resize_message(r#"{"cols": 320, "rows": 100}"#).unwrap();
    assert_eq!(cols, 320);
    assert_eq!(rows, 100);
}

#[test]
fn parse_resize_missing_cols() {
    assert!(parse_resize_message(r#"{"rows": 24}"#).is_none());
}

#[test]
fn parse_resize_missing_rows() {
    assert!(parse_resize_message(r#"{"cols": 80}"#).is_none());
}

#[test]
fn parse_resize_invalid_json() {
    assert!(parse_resize_message("not json").is_none());
}

#[test]
fn parse_resize_wrong_type() {
    assert!(parse_resize_message(r#"{"cols": "eighty", "rows": 24}"#).is_none());
}

#[test]
fn parse_resize_extra_fields_ignored() {
    let (cols, rows) = parse_resize_message(r#"{"cols": 80, "rows": 24, "extra": true}"#).unwrap();
    assert_eq!(cols, 80);
    assert_eq!(rows, 24);
}

#[test]
fn parse_resize_empty_object() {
    assert!(parse_resize_message("{}").is_none());
}

#[tokio::test]
async fn relay_replays_buffered_output_to_new_subscriber() {
    let relay = TerminalRelay::new(16);
    relay.publish(b"hello ".to_vec());
    relay.publish(b"world".to_vec());

    let (replay, _rx) = relay.subscribe();
    assert_eq!(replay, b"hello world");
}

#[tokio::test]
async fn relay_caps_buffer_at_replay_size() {
    let relay = TerminalRelay::new(16);
    let big = vec![b'x'; REPLAY_BUFFER_SIZE + 512];
    relay.publish(big);

    let (replay, _rx) = relay.subscribe();
    assert_eq!(replay.len(), REPLAY_BUFFER_SIZE);
}

#[tokio::test]
async fn relay_subscribe_then_publish_flows_live() {
    let relay = TerminalRelay::new(16);
    relay.publish(b"before".to_vec());

    let (replay, mut rx) = relay.subscribe();
    assert_eq!(replay, b"before");

    relay.publish(b"after".to_vec());
    let live = rx.recv().await.expect("live byte");
    assert_eq!(live, b"after");
}

#[tokio::test]
async fn relay_empty_buffer_returns_empty_replay() {
    let relay = TerminalRelay::new(16);
    let (replay, _rx) = relay.subscribe();
    assert!(replay.is_empty());
}

/// `cols`/`rows` reached TIOCSWINSZ through `as u16`, so 65536 became 0 and
/// 70000 became 4464. A size outside 1..=u16::MAX is refused, not wrapped.
#[test]
fn resize_message_rejects_out_of_range() {
    for text in [
        r#"{"cols": 65536, "rows": 24}"#,
        r#"{"cols": 80, "rows": 70000}"#,
        r#"{"cols": 0, "rows": 24}"#,
        r#"{"cols": 80, "rows": 0}"#,
        r#"{"cols": -1, "rows": 24}"#,
    ] {
        assert_eq!(parse_resize_message(text), None, "{text}");
    }
    assert_eq!(parse_resize_message(r#"{"cols": 65535, "rows": 1}"#), Some((65535, 1)));
}

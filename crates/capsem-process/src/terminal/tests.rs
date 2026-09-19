use super::*;

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

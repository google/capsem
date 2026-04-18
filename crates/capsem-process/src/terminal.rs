use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use capsem_proto::ipc::ServiceToProcess;
use futures::{sink::SinkExt, stream::StreamExt};

/// Maximum bytes kept in the replay ring buffer. 64 KiB covers typical
/// login banners, MOTD, and a few screenfuls of output -- enough for a
/// freshly connecting WS client to see what the shell printed before it
/// arrived without unbounded memory growth.
pub const REPLAY_BUFFER_SIZE: usize = 64 * 1024;

/// Fan-out relay for PTY output. Live subscribers receive new bytes via the
/// broadcast channel; newly-subscribing clients additionally get the last
/// `REPLAY_BUFFER_SIZE` bytes of output so they see the shell's startup
/// banner even if the shell printed it before the WS connected.
///
/// Thread safety: `publish` and `subscribe` both take the same Mutex, which
/// serializes buffer append + broadcast send with buffer snapshot + broadcast
/// subscribe. This avoids a race where a byte is either duplicated (seen in
/// both replay and live stream) or lost (missed by both).
pub struct TerminalRelay {
    inner: Mutex<RelayInner>,
}

struct RelayInner {
    buffer: VecDeque<u8>,
    broadcast: broadcast::Sender<Vec<u8>>,
}

impl TerminalRelay {
    pub fn new(broadcast_capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(broadcast_capacity);
        Arc::new(Self {
            inner: Mutex::new(RelayInner {
                buffer: VecDeque::with_capacity(REPLAY_BUFFER_SIZE),
                broadcast: tx,
            }),
        })
    }

    /// Publish a chunk of PTY output: append to the replay buffer (evicting
    /// oldest bytes past the cap) and fan out to live subscribers.
    pub fn publish(&self, data: Vec<u8>) {
        let mut inner = self.inner.lock().unwrap();
        inner.buffer.extend(data.iter());
        while inner.buffer.len() > REPLAY_BUFFER_SIZE {
            inner.buffer.pop_front();
        }
        let _ = inner.broadcast.send(data);
    }

    /// Subscribe a new client: returns the current replay snapshot plus a
    /// live receiver. Atomic vs. `publish`, so the caller sees either
    /// "snapshot only" or "snapshot + subsequent live bytes" with no
    /// duplicates and no gaps.
    pub fn subscribe(&self) -> (Vec<u8>, broadcast::Receiver<Vec<u8>>) {
        let inner = self.inner.lock().unwrap();
        let rx = inner.broadcast.subscribe();
        let snapshot: Vec<u8> = inner.buffer.iter().copied().collect();
        (snapshot, rx)
    }
}

pub(crate) async fn handle_terminal_socket(
    ws: axum::extract::ws::WebSocket,
    ctrl_tx: mpsc::Sender<ServiceToProcess>,
    replay: Vec<u8>,
    mut term_rx: broadcast::Receiver<Vec<u8>>,
) {
    let (mut client_write, mut client_read) = ws.split();

    let mut rx_task = tokio::spawn(async move {
        // Replay buffered PTY output first so the client sees the shell's
        // startup banner even if the shell printed it before the WS
        // connected. Skip if there's nothing buffered.
        if !replay.is_empty()
            && client_write.send(axum::extract::ws::Message::Binary(replay.into())).await.is_err()
        {
            return;
        }
        while let Ok(data) = term_rx.recv().await {
            if client_write.send(axum::extract::ws::Message::Binary(data.into())).await.is_err() {
                break;
            }
        }
    });

    let ctrl_tx_c = ctrl_tx.clone();
    let mut tx_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = client_read.next().await {
            match msg {
                axum::extract::ws::Message::Binary(b) => {
                    let _ = ctrl_tx_c.send(ServiceToProcess::TerminalInput { data: b.to_vec() }).await;
                }
                axum::extract::ws::Message::Text(t) => {
                    if let Some((cols, rows)) = parse_resize_message(t.as_str()) {
                        let _ = ctrl_tx_c.send(ServiceToProcess::TerminalResize {
                            cols: cols as u16,
                            rows: rows as u16
                        }).await;
                    }
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = &mut rx_task => { tx_task.abort(); },
        _ = &mut tx_task => { rx_task.abort(); },
    }
}

/// Parse a terminal resize JSON message, returning (cols, rows) if valid.
pub(crate) fn parse_resize_message(text: &str) -> Option<(u64, u64)> {
    let resize: serde_json::Value = serde_json::from_str(text).ok()?;
    let cols = resize.get("cols")?.as_u64()?;
    let rows = resize.get("rows")?.as_u64()?;
    Some((cols, rows))
}

#[cfg(test)]
mod tests {
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
}

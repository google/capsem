use tokio::sync::{broadcast, mpsc};
use capsem_proto::ipc::ServiceToProcess;
use futures::{sink::SinkExt, stream::StreamExt};

pub(crate) async fn handle_terminal_socket(
    ws: axum::extract::ws::WebSocket,
    ctrl_tx: mpsc::Sender<ServiceToProcess>,
    mut term_rx: broadcast::Receiver<Vec<u8>>,
) {
    let (mut client_write, mut client_read) = ws.split();

    let mut rx_task = tokio::spawn(async move {
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
}

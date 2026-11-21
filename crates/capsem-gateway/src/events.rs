//! Relay the service's bounded, typed change feed to operator WebSockets.

use crate::AppState;
use anyhow::{ensure, Context, Result};
use capsem_proto::gateway_grant::events::{SessionEvent, CONTENT_TYPE, FRAME_BYTES, PATH};
use http::header;
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::broadcast;

pub(crate) const IO_TIMEOUT: Duration = Duration::from_secs(5);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub(crate) enum Event {
    Message(String),
    Discontinuity,
}

pub(crate) async fn start(state: &Arc<AppState>) -> tokio::task::JoinHandle<()> {
    let first = open(state).await;
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let mut feed = first;
        loop {
            let result = match feed {
                Ok(body) => relay(body, &state.events_tx).await,
                Err(error) => Err(error),
            };
            tracing::debug!(error = ?result.err(), "gateway event feed disconnected");
            let _ = state.events_tx.send(Event::Discontinuity);
            tokio::time::sleep(RECONNECT_INTERVAL).await;
            feed = open(&state).await;
        }
    })
}

async fn open(state: &AppState) -> Result<hyper::body::Incoming> {
    let mut request = http::Request::get(format!("http://localhost{PATH}")).body(axum::body::Body::empty())?;
    let mut authorization = http::HeaderValue::from_str(&format!("Bearer {}", state.token))?;
    authorization.set_sensitive(true);
    request.headers_mut().insert(header::AUTHORIZATION, authorization);
    let response = tokio::time::timeout(IO_TIMEOUT, state.service_client.request(request))
        .await
        .context("gateway event feed admission timed out")??;
    ensure!(
        response.status() == http::StatusCode::OK,
        "gateway event feed returned {}",
        response.status()
    );
    ensure!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| value == CONTENT_TYPE),
        "unexpected gateway event feed content type"
    );
    Ok(response.into_body())
}

async fn relay<B>(mut body: B, notifications: &broadcast::Sender<Event>) -> Result<()>
where
    B: hyper::body::Body<Data = bytes::Bytes> + Unpin,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let mut record = [0; FRAME_BYTES];
    let mut filled = 0;
    while let Some(frame) = body.frame().await {
        let mut data = frame?
            .into_data()
            .map_err(|_| anyhow::anyhow!("unexpected event feed trailer"))?;
        while !data.is_empty() {
            let chunk = data.split_to((FRAME_BYTES - filled).min(data.len()));
            record[filled..filled + chunk.len()].copy_from_slice(&chunk);
            filled += chunk.len();
            if filled == FRAME_BYTES {
                let event = SessionEvent::decode(record).context("invalid gateway event frame")?;
                if notifications.receiver_count() != 0 {
                    let (kind, id) = match event {
                        SessionEvent::ApprovalPending(id) => ("approval-pending", uuid::Uuid::from_bytes(id)),
                        SessionEvent::RequestDenied(id) => ("request-denied", uuid::Uuid::from_bytes(id)),
                        SessionEvent::EnvironmentRequestsChanged(id) => {
                            ("environment-requests-changed", uuid::Uuid::from_bytes(id))
                        }
                    };
                    let text = serde_json::json!({"type":kind,"payload":{"id":id.to_string()}}).to_string();
                    let _ = notifications.send(Event::Message(text));
                }
                filled = 0;
            }
        }
    }
    ensure!(filled == 0, "truncated gateway event frame");
    Ok(())
}

#[cfg(test)]
mod tests;

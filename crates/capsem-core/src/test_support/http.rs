use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedHttpRequest {
    pub method: Method,
    pub uri: Uri,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl RecordedHttpRequest {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingHttpState {
    requests: Arc<Mutex<Vec<RecordedHttpRequest>>>,
}

impl RecordingHttpState {
    pub(crate) fn requests(&self) -> Vec<RecordedHttpRequest> {
        self.requests.lock().expect("recorder poisoned").clone()
    }
}

pub(crate) struct LocalHttpRecorder {
    pub(crate) base_url: String,
    pub(crate) state: RecordingHttpState,
    shutdown: CancellationToken,
    handle: JoinHandle<()>,
}

impl Drop for LocalHttpRecorder {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.handle.abort();
    }
}

pub(crate) async fn spawn_http_recorder() -> anyhow::Result<LocalHttpRecorder> {
    let state = RecordingHttpState::default();
    let router = Router::new()
        .fallback(any(record_request))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
                .await;
        }
    });

    Ok(LocalHttpRecorder {
        base_url: format!("http://{addr}"),
        state,
        shutdown,
        handle,
    })
}

async fn record_request(
    State(state): State<RecordingHttpState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    state
        .requests
        .lock()
        .expect("recorder poisoned")
        .push(RecordedHttpRequest {
            method,
            uri,
            headers: lower_headers(&headers),
            body: body.to_vec(),
        });
    (StatusCode::OK, Body::from("ok"))
}

pub(crate) fn lower_headers(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_ascii_lowercase(), value.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_http_recorder_captures_request_shape() {
        let recorder = spawn_http_recorder().await.unwrap();
        let response = reqwest::Client::new()
            .post(format!("{}/credential/capture", recorder.base_url))
            .header("Authorization", "Bearer local-secret")
            .body("payload")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let requests = recorder.state.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, Method::POST);
        assert_eq!(requests[0].uri.path(), "/credential/capture");
        assert_eq!(
            requests[0].header("authorization"),
            Some("Bearer local-secret")
        );
        assert_eq!(requests[0].body, b"payload");
    }
}

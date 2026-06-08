use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
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
    responses: Arc<HashMap<String, RecordedHttpResponse>>,
    default_response: RecordedHttpResponse,
}

impl RecordingHttpState {
    pub(crate) fn requests(&self) -> Vec<RecordedHttpRequest> {
        self.requests.lock().expect("recorder poisoned").clone()
    }

    fn response_for(&self, path: &str) -> RecordedHttpResponse {
        self.responses
            .get(path)
            .cloned()
            .unwrap_or_else(|| self.default_response.clone())
    }
}

pub(crate) struct LocalHttpRecorder {
    pub(crate) base_url: String,
    pub(crate) state: RecordingHttpState,
    shutdown: CancellationToken,
    handle: JoinHandle<()>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedHttpResponse {
    pub status: StatusCode,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl RecordedHttpResponse {
    pub(crate) fn text(body: impl Into<String>) -> Self {
        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        );
        Self {
            status: StatusCode::OK,
            headers,
            body: body.into().into_bytes(),
        }
    }

    pub(crate) fn html(body: impl Into<String>) -> Self {
        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            "text/html; charset=utf-8".to_string(),
        );
        Self {
            status: StatusCode::OK,
            headers,
            body: body.into().into_bytes(),
        }
    }

    pub(crate) fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers
            .insert(key.to_ascii_lowercase(), value.to_string());
        self
    }
}

impl Default for RecordedHttpResponse {
    fn default() -> Self {
        Self::text("ok")
    }
}

impl Drop for LocalHttpRecorder {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.handle.abort();
    }
}

pub(crate) async fn spawn_http_recorder() -> anyhow::Result<LocalHttpRecorder> {
    spawn_static_http_recorder(std::iter::empty::<(String, RecordedHttpResponse)>()).await
}

pub(crate) async fn spawn_static_http_recorder<I, S>(routes: I) -> anyhow::Result<LocalHttpRecorder>
where
    I: IntoIterator<Item = (S, RecordedHttpResponse)>,
    S: Into<String>,
{
    let state = RecordingHttpState::default();
    let state = RecordingHttpState {
        responses: Arc::new(
            routes
                .into_iter()
                .map(|(path, response)| (path.into(), response))
                .collect(),
        ),
        ..state
    };
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
    let response = state.response_for(uri.path());
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

    let mut out = (response.status, Body::from(response.body)).into_response();
    for (key, value) in response.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            out.headers_mut().insert(name, value);
        }
    }
    out
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

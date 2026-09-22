use crate::transport::Transport;
use axum::body::{to_bytes, Body};
use axum::http::{Request as HttpRequest, Response};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub(crate) struct Server {
    pub url: String,
    pub received: mpsc::UnboundedReceiver<(axum::http::request::Parts, Vec<u8>)>,
    pub task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn reply(status: u16, body: &[u8], redirect: Option<String>) -> Self {
        let bytes = body.to_vec();
        Self::respond(move |_| {
            let mut response = Response::builder().status(status);
            if let Some(location) = &redirect {
                response = response.header("Location", location);
            }
            response.body(Body::from(bytes.clone())).unwrap()
        })
        .await
    }

    pub async fn respond(
        reply: impl Fn(&axum::http::request::Parts) -> Response<Body> + Send + Sync + 'static,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sent, received) = mpsc::unbounded_channel();
        let reply = Arc::new(reply);
        let app = axum::Router::new().fallback(move |incoming: HttpRequest<Body>| {
            let sent = sent.clone();
            let reply = Arc::clone(&reply);
            async move {
                let (parts, body) = incoming.into_parts();
                let response = reply(&parts);
                sent.send((parts, to_bytes(body, 1_000_000).await.unwrap().to_vec()))
                    .unwrap();
                response
            }
        });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, received, task }
    }

    pub fn client(&self) -> Transport {
        Transport::new(&self.url, "private-token", Duration::from_secs(2)).unwrap()
    }
}

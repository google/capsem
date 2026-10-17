use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper::{Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

pub struct Reply {
    pub status: u16,
    pub body: String,
    pub location: Option<String>,
    pub delay: Duration,
}

impl Reply {
    pub fn json(body: &str) -> Self {
        Self {
            status: 200,
            body: body.into(),
            location: None,
            delay: Duration::ZERO,
        }
    }
}

#[derive(Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub content_type: String,
    pub fields: BTreeMap<String, String>,
    pub field_count: usize,
}

pub struct Fixture {
    pub base: String,
    pub records: Arc<Mutex<Vec<Recorded>>>,
    pub received: Arc<Notify>,
    task: Option<JoinHandle<()>>,
}

impl Fixture {
    pub async fn new(replies: Vec<Reply>) -> Self {
        let listener = capsem_foundation::unix::tcp::bind_loopback("127.0.0.1".parse().unwrap()).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let listener = TcpListener::from_std(listener).unwrap();
        let records = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::new(Notify::new());
        let task_records = Arc::clone(&records);
        let task_received = Arc::clone(&received);
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let records = Arc::clone(&task_records);
                let received = Arc::clone(&task_received);
                let replies = Arc::clone(&replies);
                let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                    let records = Arc::clone(&records);
                    let received = Arc::clone(&received);
                    let replies = Arc::clone(&replies);
                    async move {
                        let method = request.method().to_string();
                        let path = request.uri().to_string();
                        let content_type = request
                            .headers()
                            .get("content-type")
                            .map_or("", |value| value.to_str().unwrap())
                            .to_string();
                        let bytes = request.into_body().collect().await.unwrap().to_bytes();
                        let pairs = url::form_urlencoded::parse(&bytes).into_owned().collect::<Vec<_>>();
                        records.lock().unwrap().push(Recorded {
                            method,
                            path,
                            content_type,
                            field_count: pairs.len(),
                            fields: pairs.into_iter().collect(),
                        });
                        received.notify_one();
                        let reply = replies.lock().unwrap().pop_front().unwrap_or(Reply {
                            status: 500,
                            body: "unexpected replay".into(),
                            location: None,
                            delay: Duration::ZERO,
                        });
                        tokio::time::sleep(reply.delay).await;
                        let mut response = Response::new(Full::new(Bytes::from(reply.body)));
                        *response.status_mut() = StatusCode::from_u16(reply.status).unwrap();
                        response
                            .headers_mut()
                            .insert("content-type", "application/json".parse().unwrap());
                        if let Some(location) = reply.location {
                            response.headers_mut().insert("location", location.parse().unwrap());
                        }
                        Ok::<_, Infallible>(response)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .keep_alive(false)
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            }
        });
        Self {
            base,
            records,
            received,
            task: Some(task),
        }
    }

    pub async fn close(mut self) {
        let task = self.task.take().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

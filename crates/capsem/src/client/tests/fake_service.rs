//! A scripted service on a Unix socket, for tests of commands that talk to
//! it. Every request is recorded; each is answered by the first unspent
//! route with its method and path (the query string aside).

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::Value;

use crate::client::UdsClient;

/// One recorded request.
#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| panic!("{} {}: {error}", self.method, self.path))
    }
}

struct Route {
    method: &'static str,
    path: String,
    status: u16,
    body: Vec<u8>,
    once: bool,
    spent: bool,
}

#[derive(Default)]
struct Script {
    routes: Vec<Route>,
    recorded: Vec<Recorded>,
}

pub(crate) struct FakeService {
    pub client: UdsClient,
    script: Arc<Mutex<Script>>,
    server: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}

impl FakeService {
    pub fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("service.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let script = Arc::new(Mutex::new(Script::default()));
        let shared = script.clone();
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let script = shared.clone();
                tokio::spawn(async move {
                    let answer = hyper::service::service_fn(move |request| answer(script.clone(), request));
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), answer)
                        .await;
                });
            }
        });
        Self {
            client: UdsClient::new(socket, false),
            script,
            server,
            _directory: directory,
        }
    }

    /// Answer every matching request with `status` and the JSON `body`.
    pub fn route(&self, method: &'static str, path: &str, status: u16, body: Value) -> &Self {
        self.add(method, path, status, serde_json::to_vec(&body).unwrap(), false)
    }

    /// Answer the next matching request only; later ones fall through.
    pub fn once(&self, method: &'static str, path: &str, status: u16, body: Value) -> &Self {
        self.add(method, path, status, serde_json::to_vec(&body).unwrap(), true)
    }

    fn add(&self, method: &'static str, path: &str, status: u16, body: Vec<u8>, once: bool) -> &Self {
        self.script.lock().unwrap().routes.push(Route {
            method,
            path: path.to_string(),
            status,
            body,
            once,
            spent: false,
        });
        self
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.script.lock().unwrap().recorded.clone()
    }

    /// `METHOD path` of every request, in order.
    pub fn calls(&self) -> Vec<String> {
        self.recorded()
            .iter()
            .map(|request| format!("{} {}", request.method, request.path))
            .collect()
    }

    pub fn find(&self, method: &str, path: &str) -> Vec<Recorded> {
        self.recorded()
            .into_iter()
            .filter(|request| request.method == method && request.path.starts_with(path))
            .collect()
    }
}

impl Drop for FakeService {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn answer(
    script: Arc<Mutex<Script>>,
    request: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = request.method().to_string();
    let path = request
        .uri()
        .path_and_query()
        .map_or_else(String::new, |path| path.to_string());
    let body = request
        .into_body()
        .collect()
        .await
        .map_or_else(|_| Vec::new(), |body| body.to_bytes().to_vec());
    let route_path = path.split('?').next().unwrap_or_default().to_string();
    let (status, body) = {
        let mut script = script.lock().unwrap();
        script.recorded.push(Recorded {
            method: method.clone(),
            path,
            body,
        });
        match script
            .routes
            .iter_mut()
            .find(|route| !route.spent && route.method == method && route.path == route_path)
        {
            Some(route) => {
                route.spent = route.once;
                (route.status, route.body.clone())
            }
            None => (404, br#"{"error":"no scripted route"}"#.to_vec()),
        }
    };
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap())
}

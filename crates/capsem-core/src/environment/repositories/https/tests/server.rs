//! Native TLS peer; the fake DNS name succeeds only with pinned admission.
use super::*;
use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
    routing::any,
    Router,
};
use base64::Engine;
use futures::{stream, StreamExt};
use std::convert::Infallible;
use tokio::task::{JoinHandle, JoinSet};

#[derive(Clone)]
pub(super) enum Reply {
    Bytes(Vec<u8>),
    WrongType(Vec<u8>),
    Stream(Vec<u8>),
    PendingBody(Vec<u8>, Arc<tokio::sync::Notify>),
    Status(u16),
    Redirect(String),
}
type Requests = Arc<Mutex<Vec<(http::Method, String, http::HeaderMap)>>>;

pub(super) struct Server {
    pub address: SocketAddr,
    pub pem: String,
    pub requests: Requests,
    task: JoinHandle<()>,
}
impl Server {
    pub async fn start(reply: Reply) -> Self {
        let (certificate, key) = capsem_crypto::tls::self_signed(vec!["git.fixture.invalid".into()]).unwrap();
        let pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            base64::engine::general_purpose::STANDARD.encode(certificate.as_ref())
        );
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(capsem_crypto::tls::provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)
            .unwrap();
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .fallback(any(handle))
            .with_state((reply, requests.clone()));
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap(); let tls = tls.clone(); let app = app.clone();
                        connections.spawn(async move {
                            let Ok(stream) = tls.accept(socket).await else { return; };
                            let _ = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), hyper_util::service::TowerToHyperService::new(app)).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Self {
            address,
            pem,
            requests,
            task,
        }
    }
    pub fn repository(&self) -> String {
        format!("https://git.fixture.invalid:{}/team/project.git", self.address.port())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle(State((reply, requests)): State<(Reply, Requests)>, request: Request) -> Response {
    requests.lock().unwrap().push((
        request.method().clone(),
        request.uri().to_string(),
        request.headers().clone(),
    ));
    let content_type = if matches!(&reply, Reply::WrongType(_)) {
        "text/plain"
    } else {
        "application/x-git-upload-pack-advertisement"
    };
    let mut response = Response::builder().header("content-type", content_type);
    let body = match reply {
        Reply::Bytes(bytes) => Body::from(bytes),
        Reply::WrongType(bytes) => Body::from(bytes),
        Reply::Stream(bytes) => Body::from_stream(stream::once(async { Ok::<_, Infallible>(bytes) })),
        Reply::PendingBody(bytes, ready) => Body::from_stream(
            stream::once(async move {
                ready.notify_one();
                Ok::<_, Infallible>(bytes)
            })
            .chain(stream::pending()),
        ),
        Reply::Status(status) => {
            response = response.status(status);
            Body::empty()
        }
        Reply::Redirect(location) => {
            response = response.status(302).header("location", location);
            Body::empty()
        }
    };
    response.body(body).unwrap()
}

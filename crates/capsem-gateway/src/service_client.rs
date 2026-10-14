use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use http::{Request, Response, Uri};
use hyper::body::Incoming;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::UnixStream;
use tower_service::Service;

use crate::service_grant::GatewayGrantClient;

const MAX_IDLE_CONNECTIONS: usize = 8;

#[derive(Clone)]
pub struct ServiceClient {
    inner: Client<UdsConnector, Body>,
    source: ConnectionSource,
}

impl ServiceClient {
    pub fn new(uds_path: &Path) -> Self {
        let source = ConnectionSource::Path(Arc::new(uds_path.to_path_buf()));
        let connector = UdsConnector { source: source.clone() };
        let inner = Client::builder(TokioExecutor::new())
            .set_host(false)
            .pool_max_idle_per_host(MAX_IDLE_CONNECTIONS)
            .build(connector);
        Self { inner, source }
    }

    pub(crate) fn granted(grants: GatewayGrantClient) -> Self {
        let source = ConnectionSource::Grant(grants);
        let inner = Client::builder(TokioExecutor::new())
            .set_host(false)
            .pool_max_idle_per_host(MAX_IDLE_CONNECTIONS)
            .build(UdsConnector { source: source.clone() });
        Self { inner, source }
    }

    pub async fn request(
        &self,
        request: Request<Body>,
    ) -> Result<Response<Incoming>, hyper_util::client::legacy::Error> {
        self.inner.request(request).await
    }

    pub(crate) async fn connect(&self) -> std::io::Result<UnixStream> {
        self.source.open_service().await
    }

    pub(crate) async fn connect_owner(&self, vm_id: &str, direct_path: &str) -> std::io::Result<UnixStream> {
        match &self.source {
            ConnectionSource::Path(_) => UnixStream::connect(direct_path).await,
            ConnectionSource::Grant(grants) => grants.open_owner(vm_id.to_owned()).await,
        }
    }
}

#[derive(Clone)]
struct UdsConnector {
    source: ConnectionSource,
}

#[derive(Clone)]
enum ConnectionSource {
    Path(Arc<PathBuf>),
    Grant(GatewayGrantClient),
}

impl ConnectionSource {
    async fn open_service(&self) -> std::io::Result<UnixStream> {
        match self {
            Self::Path(path) => UnixStream::connect(path.as_ref()).await,
            Self::Grant(grants) => grants.open_service().await,
        }
    }
}

impl Service<Uri> for UdsConnector {
    type Response = TokioIo<UnixStream>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: Uri) -> Self::Future {
        let source = self.source.clone();
        Box::pin(async move { source.open_service().await.map(TokioIo::new) })
    }
}

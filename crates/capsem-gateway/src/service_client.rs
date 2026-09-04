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

const MAX_IDLE_CONNECTIONS: usize = 8;

#[derive(Clone)]
pub struct ServiceClient {
    inner: Client<UdsConnector, Body>,
}

impl ServiceClient {
    pub fn new(uds_path: &Path) -> Self {
        let connector = UdsConnector {
            uds_path: Arc::new(uds_path.to_path_buf()),
        };
        let inner = Client::builder(TokioExecutor::new())
            .set_host(false)
            .pool_max_idle_per_host(MAX_IDLE_CONNECTIONS)
            .build(connector);
        Self { inner }
    }

    pub async fn request(
        &self,
        request: Request<Body>,
    ) -> Result<Response<Incoming>, hyper_util::client::legacy::Error> {
        self.inner.request(request).await
    }
}

#[derive(Clone)]
struct UdsConnector {
    uds_path: Arc<PathBuf>,
}

impl Service<Uri> for UdsConnector {
    type Response = TokioIo<UnixStream>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: Uri) -> Self::Future {
        let uds_path = self.uds_path.clone();
        Box::pin(async move { UnixStream::connect(uds_path.as_ref()).await.map(TokioIo::new) })
    }
}

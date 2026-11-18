//! A TLS OCI registry; production pullers verify its descriptors and cache bytes.
use super::*;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{Method, Response},
    routing::any,
    Router,
};
use base64::Engine;
use std::collections::HashMap;
use tokio::task::{JoinHandle, JoinSet};

type Blobs = Arc<Mutex<HashMap<String, Vec<u8>>>>;
pub(super) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", capsem_crypto::Sha256::digest(bytes))
}

pub(super) struct Registry {
    pub address: String,
    pub ca_pem: String,
    pub requests: Arc<Mutex<Vec<String>>>,
    blobs: Blobs,
    task: JoinHandle<()>,
}
impl Registry {
    pub async fn start() -> Self {
        let (cert, key) = capsem_crypto::tls::self_signed(vec!["localhost".into()]).unwrap();
        let ca_pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            base64::engine::general_purpose::STANDARD.encode(cert.as_ref())
        );
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(capsem_crypto::tls::provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap();
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("localhost:{}", listener.local_addr().unwrap().port());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let blobs = Arc::new(Mutex::new(HashMap::new()));
        let app = Router::new()
            .route("/v2/", any(|| async { "{}" }))
            .route("/v2/{*path}", any(blob))
            .with_state((blobs.clone(), requests.clone()));
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap(); let tls = tls.clone(); let app = app.clone();
                        connections.spawn(async move {
                            let Ok(stream) = tls.accept(socket).await else { return; };
                            let _ = hyper::server::conn::http1::Builder::new().serve_connection(
                                hyper_util::rt::TokioIo::new(stream), hyper_util::service::TowerToHyperService::new(app)
                            ).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Self {
            address,
            ca_pem,
            requests,
            blobs,
            task,
        }
    }
    pub fn reference(&self, kind: CatalogKind) -> String {
        format!("{}/team/{}:stable", self.address, kind.as_str())
    }
    pub fn publish(&self, kind: CatalogKind, document: serde_json::Value) -> String {
        let bytes = serde_json::to_vec(&document).unwrap();
        let descriptor =
            |media: &str, bytes: &[u8]| json!({"mediaType":media,"digest":digest(bytes),"size":bytes.len()});
        let manifest = serde_json::to_vec(
            &json!({"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
            "config":descriptor("application/vnd.oci.empty.v1+json", b"{}"),
            "layers":[descriptor(capsem_assets::catalogs::catalog_media_type(kind), &bytes)]}),
        )
        .unwrap();
        let pin = digest(&manifest);
        let root = format!("team/{}", kind.as_str());
        let mut blobs = self.blobs.lock().unwrap();
        blobs.insert(format!("{root}/blobs/{}", digest(b"{}")), b"{}".to_vec());
        blobs.insert(format!("{root}/blobs/{}", digest(&bytes)), bytes);
        blobs.insert(format!("{root}/manifests/stable"), manifest.clone());
        blobs.insert(format!("{root}/manifests/{pin}"), manifest);
        drop(blobs);
        pin
    }
    pub fn alias(&self, original: &str, replacement: &str) {
        let mut blobs = self.blobs.lock().unwrap();
        let copied: Vec<_> = blobs
            .iter()
            .filter_map(|(path, bytes)| {
                path.strip_prefix(original)
                    .map(|suffix| (format!("{replacement}{suffix}"), bytes.clone()))
            })
            .collect();
        blobs.extend(copied);
    }
    pub async fn stop(&mut self) {
        self.task.abort();
        assert!(matches!((&mut self.task).await, Err(error) if error.is_cancelled()));
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn blob(
    State((blobs, requests)): State<(Blobs, Arc<Mutex<Vec<String>>>)>,
    Path(path): Path<String>,
    method: Method,
) -> Response<Body> {
    requests.lock().unwrap().push(path.clone());
    let Some(bytes) = blobs.lock().unwrap().get(&path).cloned() else {
        return Response::builder().status(404).body(Body::empty()).unwrap();
    };
    Response::builder()
        .status(200)
        .header("docker-content-digest", digest(&bytes))
        .header(
            "content-type",
            if path.contains("/manifests/") {
                "application/vnd.oci.image.manifest.v1+json"
            } else {
                "application/octet-stream"
            },
        )
        .header("content-length", bytes.len())
        .body(if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(bytes)
        })
        .unwrap()
}

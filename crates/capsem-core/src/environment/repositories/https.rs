//! An admitted, pinned HTTPS request. No process, Git configuration, redirect,
//! ambient proxy, credential helper, or authentication header is consulted.
use super::{advertisement, RepositoryResolutionError as Error};
use reqwest::{Certificate, Url};
use std::{future::Future, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};

pub type RepositoryAdmission<'a> = Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, Error>> + Send + 'a>>;

/// Trusted host capability, not a request-body proof. The implementation owns
/// policy, address resolution and private-network/credential scope admission.
/// The client connects only to returned addresses and never resolves again.
pub trait RepositoryNetworkAuthority: Send + Sync {
    fn admit<'a>(&'a self, discovery: &'a Url) -> RepositoryAdmission<'a>;
}

pub struct RepositoryHttpsResolver {
    authority: Arc<dyn RepositoryNetworkAuthority>,
    root: Option<Certificate>,
}
impl RepositoryHttpsResolver {
    pub fn new(authority: Arc<dyn RepositoryNetworkAuthority>) -> Self {
        Self { authority, root: None }
    }
    /// Explicit host configuration, never taken from environment intent.
    pub fn with_root_certificate(mut self, pem: &[u8]) -> Result<Self, Error> {
        self.root = Some(Certificate::from_pem(pem).map_err(|_| Error::InvalidIntent)?);
        Ok(self)
    }
    pub async fn resolve(&self, repository: &str, reference: &str) -> Result<String, Error> {
        let budget = Duration::from_secs(capsem_proto::ENVIRONMENT_MAX_REPOSITORY_RESOLVE_SECONDS as u64);
        tokio::time::timeout(budget, self.resolve_inner(repository, reference, budget))
            .await
            .map_err(|_| Error::Deadline)?
    }
    async fn resolve_inner(&self, repository: &str, reference: &str, budget: Duration) -> Result<String, Error> {
        if repository.len() > capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES || !advertisement::valid_selection(reference) {
            return Err(Error::InvalidIntent);
        }
        let mut url = Url::parse(repository).map_err(|_| Error::InvalidIntent)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::InvalidIntent);
        }
        let path = format!("{}/info/refs", url.path().trim_end_matches('/'));
        url.set_path(&path);
        url.set_query(Some("service=git-upload-pack"));
        let addresses = self.authority.admit(&url).await?;
        let port = url.port_or_known_default().ok_or(Error::InvalidIntent)?;
        if addresses.is_empty() || addresses.iter().any(|address| address.port() != port) {
            return Err(Error::Refused);
        }
        let literal = url.host_str().and_then(|host| {
            host.trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .ok()
        });
        if literal.is_some_and(|ip| {
            addresses
                .iter()
                .any(|address| address.ip().to_canonical() != ip.to_canonical())
        }) {
            return Err(Error::Refused);
        }
        let mut builder = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(budget)
            .connect_timeout(budget)
            .pool_max_idle_per_host(0)
            .resolve_to_addrs(url.host_str().ok_or(Error::InvalidIntent)?, &addresses);
        if let Some(root) = &self.root {
            builder = builder.add_root_certificate(root.clone());
        }
        let client = builder.build().map_err(|_| Error::Unavailable)?;
        let mut response = client
            .get(url)
            .header("Accept", "application/x-git-upload-pack-advertisement")
            .header("Git-Protocol", "version=1")
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .map_err(http_error)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(
                if response.status().is_redirection() || matches!(response.status().as_u16(), 401 | 403) {
                    Error::Refused
                } else {
                    Error::Unavailable
                },
            );
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            != Some("application/x-git-upload-pack-advertisement")
        {
            return Err(Error::InvalidAdvertisement);
        }
        let maximum = capsem_proto::ENVIRONMENT_MAX_REPOSITORY_ADVERTISEMENT_BYTES;
        if response.content_length().is_some_and(|length| length > maximum as u64) {
            return Err(Error::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(http_error)? {
            if chunk.len() > maximum.saturating_sub(bytes.len()) {
                return Err(Error::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let reference = reference.to_owned();
        tokio::task::spawn_blocking(move || advertisement::resolve(&bytes, &reference))
            .await
            .map_err(|_| Error::Unavailable)?
    }
}
fn http_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Deadline
    } else {
        Error::Unavailable
    }
}

#[cfg(test)]
mod tests;

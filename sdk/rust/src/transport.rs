//! Shared HTTP connection pool. Dropping a request future cancels that request.

use std::time::Duration;

use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, Method, Url};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Default)]
pub enum MediaType {
    #[default]
    Json,
    Binary,
}

impl MediaType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Binary => "application/octet-stream",
        }
    }
}

/// An optional deadline for one request, including reading the response body.
#[derive(Debug, Clone, Copy, Default)]
pub struct CallOptions {
    pub timeout: Option<Duration>,
}

#[derive(Default)]
pub struct Request<'a> {
    pub parameters: &'a [(&'a str, &'a str)],
    pub query: &'a [(&'a str, String)],
    pub body: Option<Vec<u8>>,
    pub content_type: MediaType,
    pub accept: MediaType,
    pub options: CallOptions,
}

/// Clones share a connection pool; no local discovery, subprocesses or UDS access.
///
/// ```no_run
/// use capsem_sdk::transport::{Transport, Request};
/// use std::time::Duration;
/// # async fn example() -> capsem_sdk::Result<()> {
/// let client = Transport::new("http://127.0.0.1:19222", "token", Duration::from_secs(30))?;
/// let bytes = client.request(reqwest::Method::GET, "/status", Request::default()).await?;
/// let info: capsem_sdk::models::HypervisorInfo = serde_json::from_slice(&bytes)?;
/// assert_eq!(info.service, capsem_sdk::models::ServiceAvailability::Running);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Transport {
    base: Url,
    client: Client,
    authorization: HeaderValue,
}

impl Transport {
    pub fn new(url: &str, token: &str, timeout: Duration) -> Result<Self> {
        let base = Url::parse(url).map_err(|_| Error::InvalidInput("gateway URL is invalid"))?;
        if !matches!(base.scheme(), "http" | "https")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(Error::InvalidInput(
                "gateway URL must be HTTP(S) without credentials, query or fragment",
            ));
        }
        if token.is_empty() || timeout.is_zero() {
            return Err(Error::InvalidInput("bearer token and positive timeout are required"));
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| Error::InvalidInput("bearer token contains invalid header characters"))?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()?;
        Ok(Self {
            base,
            client,
            authorization,
        })
    }

    pub async fn request(&self, method: Method, path: &str, request: Request<'_>) -> Result<Vec<u8>> {
        if !path.starts_with('/') || path.contains(['?', '#']) {
            return Err(Error::InvalidInput(
                "operation path must be absolute without query or fragment",
            ));
        }
        let mut url = self.base.clone();
        {
            let mut segments = url.path_segments_mut().expect("HTTP URL supports path segments");
            segments.pop_if_empty();
            for segment in path[1..].split('/') {
                let value = if let Some(name) = segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                    request
                        .parameters
                        .iter()
                        .find(|(key, _)| *key == name)
                        .map(|(_, value)| *value)
                        .ok_or(Error::InvalidInput("missing path parameter"))?
                } else {
                    if segment.contains(['{', '}']) {
                        return Err(Error::InvalidInput("unresolved path parameter"));
                    }
                    segment
                };
                // URL libraries normalize dot-only segments, including escaped ones.
                if value.is_empty() || matches!(value, "." | "..") {
                    return Err(Error::InvalidInput("invalid path identifier"));
                }
                segments.push(value);
            }
        }
        if request.options.timeout.is_some_and(|timeout| timeout.is_zero()) {
            return Err(Error::InvalidInput("request timeout must be positive"));
        }
        let mut outgoing = self
            .client
            .request(method, url)
            .query(request.query)
            .header(AUTHORIZATION, self.authorization.clone())
            .header(ACCEPT, request.accept.as_str());
        if let Some(body) = request.body {
            outgoing = outgoing.header(CONTENT_TYPE, request.content_type.as_str()).body(body);
        }
        if let Some(timeout) = request.options.timeout {
            outgoing = outgoing.timeout(timeout);
        }
        let response = outgoing.send().await?;
        let status = response.status();
        let body = response.bytes().await?.to_vec();
        if !status.is_success() {
            return Err(Error::Http {
                status: status.as_u16(),
                body,
            });
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests;

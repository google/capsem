use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use bytes::Bytes;
use http::header::{CONTENT_TYPE, COOKIE, LOCATION, SET_COOKIE};
use http_body_util::Full;
use reqwest::{Client, Url};

use crate::{Error, Port, Result};

/// A cookie-scoped preview connection, separate from the authenticated gateway
/// pool: neither the gateway bearer token nor caller cookies reach the guest.
pub(super) struct Preview {
    client: Client,
    url: Url,
    cookie: http::HeaderValue,
}

impl Preview {
    pub async fn connect(port: &Port) -> Result<Self> {
        let invalid = || Error::InvalidResponse("invalid callback preview session");
        let url = Url::parse(port.url.as_deref().ok_or_else(invalid)?).map_err(|_| invalid())?;
        let host = format!("{}.localhost", port.id);
        if url.scheme() != "http"
            || url.host_str() != Some(host.as_str())
            || url.port().is_none()
            || url.path() != "/_capsem/bootstrap"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || !port.authenticate
            || port.host.is_some()
        {
            return Err(invalid());
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(20))
            .resolve(
                &host,
                SocketAddr::from((Ipv4Addr::LOCALHOST, url.port().expect("checked port"))),
            )
            .build()
            .map_err(|_| invalid())?;
        let response = client
            .post(url.clone())
            .form(&[("bootstrap_token", port.bootstrap_token.as_deref().ok_or_else(invalid)?)])
            .send()
            .await
            .map_err(|_| invalid())?;
        if response.status() != http::StatusCode::SEE_OTHER {
            return Err(invalid());
        }
        let raw = response
            .headers()
            .get(SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(invalid)?;
        let pair = raw.split(';').next().ok_or_else(invalid)?;
        if !pair.starts_with("capsem_preview=") || pair.len() > 4096 {
            return Err(invalid());
        }
        let mut cookie = http::HeaderValue::from_str(pair).map_err(|_| invalid())?;
        cookie.set_sensitive(true);
        Ok(Self { client, url, cookie })
    }

    pub async fn forward(&self, uri: &http::Uri) -> Result<http::Response<Full<Bytes>>> {
        let invalid = || Error::InvalidResponse("callback return failed");
        let mut url = self.url.clone();
        url.set_path(uri.path());
        url.set_query(uri.query());
        let mut response = self
            .client
            .get(url)
            .header(COOKIE, self.cookie.clone())
            .send()
            .await
            .map_err(|_| invalid())?;
        let mut result = http::Response::builder().status(response.status());
        if let Some(location) = response.headers().get(LOCATION) {
            let target = location
                .to_str()
                .ok()
                .and_then(|s| Url::parse(s).ok())
                .ok_or_else(invalid)?;
            if !matches!(target.scheme(), "http" | "https")
                || target.host_str().is_none()
                || !target.username().is_empty()
                || target.password().is_some()
            {
                return Err(invalid());
            }
            result = result.header(LOCATION, location);
        }
        if let Some(content_type) = response.headers().get(CONTENT_TYPE) {
            result = result.header(CONTENT_TYPE, content_type);
        }
        // OAuth callbacks are tiny HTTP responses, not a bulk-data path. Bound
        // each chunk before retaining it; never materialize an unbounded body.
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| invalid())? {
            if body.len() + chunk.len() > 65536 {
                return Err(invalid());
            }
            body.extend_from_slice(&chunk);
        }
        result
            .header("Cache-Control", "no-store")
            .header("Content-Security-Policy", "sandbox; default-src 'none'")
            .header("X-Content-Type-Options", "nosniff")
            .header("Connection", "close")
            .body(Full::new(Bytes::from(body)))
            .map_err(|_| invalid())
    }
}

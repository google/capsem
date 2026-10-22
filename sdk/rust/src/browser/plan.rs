use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use capsem_crypto::secrets::{constant_time_eq, Zeroizing};
use reqwest::Url;

use crate::{Error, Result};

pub(super) struct CallbackPlan {
    pub port: u16,
    path: String,
    host: String,
    state: Zeroizing<String>,
}

impl std::fmt::Debug for CallbackPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackPlan")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl CallbackPlan {
    pub fn parse(value: &str) -> Result<Option<Self>> {
        let invalid = || Error::InvalidInput("invalid browser callback request");
        if value.len() > 8192 || value.chars().any(char::is_control) {
            return Err(invalid());
        }
        let url = Url::parse(value).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid());
        }
        let redirects: Vec<_> = url.query_pairs().filter(|(key, _)| key == "redirect_uri").collect();
        if redirects.is_empty() {
            return Ok(None);
        }
        let states: Vec<_> = url.query_pairs().filter(|(key, _)| key == "state").collect();
        if url.scheme() != "https" || redirects.len() != 1 || states.len() != 1 {
            return Err(invalid());
        }
        let callback = Url::parse(&redirects[0].1).map_err(|_| invalid())?;
        let host = callback.host_str().unwrap_or_default();
        // A normal hosted OAuth redirect needs no local listener. Only
        // loopback redirects participate in this return bridge.
        if !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
            return Ok(None);
        }
        let port = callback.port().ok_or_else(invalid)?;
        if callback.scheme() != "http"
            || !matches!(host, "localhost" | "127.0.0.1" | "[::1]")
            || port < 1024
            || callback.query().is_some()
            || callback.fragment().is_some()
            || !callback.username().is_empty()
            || callback.password().is_some()
            || !(16..=1024).contains(&states[0].1.len())
        {
            return Err(invalid());
        }
        Ok(Some(Self {
            port,
            path: callback.path().to_owned(),
            host: host.to_owned(),
            state: Zeroizing::new(states[0].1.to_string()),
        }))
    }

    pub fn bind(&self) -> Result<Vec<tokio::net::TcpListener>> {
        let ips = match self.host.as_str() {
            "localhost" => vec![IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)],
            "127.0.0.1" => vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
            _ => vec![IpAddr::V6(Ipv6Addr::LOCALHOST)],
        };
        ips.into_iter()
            .map(|ip| {
                capsem_foundation::unix::tcp::bind_loopback_port(ip, self.port)
                    .and_then(tokio::net::TcpListener::from_std)
                    .map_err(|_| Error::InvalidInput("callback loopback address is unavailable"))
            })
            .collect()
    }

    pub fn admits<B>(&self, request: &http::Request<B>) -> bool {
        let uri = request.uri();
        let host = request.headers().get_all(http::header::HOST);
        let hosts: Vec<_> = host.iter().collect();
        if request.method() != http::Method::GET
            || uri.scheme().is_some()
            || uri.authority().is_some()
            || uri.path() != self.path
            || uri.to_string().len() > 8192
            || hosts.len() != 1
            || request.headers().contains_key(http::header::TRANSFER_ENCODING)
            || request
                .headers()
                .get(http::header::CONTENT_LENGTH)
                .is_some_and(|v| v != "0")
        {
            return false;
        }
        let allowed: Vec<_> = if self.host == "localhost" {
            ["localhost", "127.0.0.1", "[::1]"]
                .into_iter()
                .map(|h| format!("{h}:{}", self.port))
                .collect()
        } else {
            vec![format!("{}:{}", self.host, self.port)]
        };
        if !allowed.iter().any(|h| hosts[0].as_bytes() == h.as_bytes()) {
            return false;
        }
        let Ok(parsed) = Url::parse(&format!("http://localhost{uri}")) else {
            return false;
        };
        let states: Vec<_> = parsed.query_pairs().filter(|(key, _)| key == "state").collect();
        let results: Vec<_> = parsed
            .query_pairs()
            .filter(|(key, _)| key == "code" || key == "error")
            .collect();
        states.len() == 1
            && constant_time_eq(states[0].1.as_bytes(), self.state.as_bytes())
            && results.len() == 1
            && !results[0].1.is_empty()
    }
}

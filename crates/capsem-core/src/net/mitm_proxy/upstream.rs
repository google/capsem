//! Where a proxied request's upstream connection goes, fixed before policy.
//!
//! The rules judge `ip.value`, so the address has to be known before they
//! run, and the dial has to go to that address and nowhere else. Dialing the
//! name let a guest reach host loopback, private ranges and the metadata
//! server with any name that resolved there, and let a second DNS answer
//! swap the address between the check and the connect. TLS still verifies
//! the upstream certificate against the name; only the TCP target changes.
//!
//! One exception: `network.upstream_overrides` are trusted administrator
//! routing (hermetic replay, corp egress). They keep dialing their configured
//! target exactly as written, and the rules see what they always saw.

use std::fmt;
use std::net::{IpAddr, SocketAddr};

use hyper::client::conn::http1::SendRequest;
use tokio::net::TcpStream;

use super::body::ProxyBoxBody;
use super::protocol::Protocol;
use crate::net::policy::{NetworkMechanics, UpstreamOverrideProtocol};
use crate::net::upstream_address::{judged_address, UpstreamResolver};

/// The upstream connection one request will use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpstreamTarget {
    /// An administrator's `upstream_overrides` route, dialed as configured.
    Override { dial: String, protocol: Protocol },
    /// The addresses the guest's name resolved to, in the order to try.
    Resolved(Vec<SocketAddr>),
    /// The name did not resolve. Nothing is dialed; the reason is the error.
    Unresolved(String),
}

impl UpstreamTarget {
    /// Resolve one fresh target from trusted policy. The returned address set
    /// is what policy judges and what [`Self::connect`] later dials; the name
    /// is never resolved a second time.
    pub async fn resolve(resolver: &UpstreamResolver, policy: &NetworkMechanics, domain: &str, port: u16) -> Self {
        if let Some(target) = Self::override_target(policy, domain, port) {
            return target;
        }
        match resolver.resolve(domain, port).await {
            Ok(addresses) => Self::Resolved(addresses),
            Err(error) => Self::Unresolved(error),
        }
    }

    /// Fix the target for `domain:port` before the rules run. A keep-alive
    /// request for the same `domain:port` stays on the address its
    /// connection was judged and opened with instead of resolving again.
    pub(super) async fn select(
        resolver: &UpstreamResolver,
        policy: &NetworkMechanics,
        domain: &str,
        port: u16,
        cache: &UpstreamCache,
    ) -> Self {
        if let Some(target) = Self::override_target(policy, domain, port) {
            return target;
        }
        if let Some(pinned) = cache
            .lock()
            .await
            .as_ref()
            .and_then(|cached| cached.pinned_for(domain, port))
        {
            return pinned;
        }
        match resolver.resolve(domain, port).await {
            Ok(addresses) => Self::Resolved(addresses),
            Err(error) => Self::Unresolved(error),
        }
    }

    fn override_target(policy: &NetworkMechanics, domain: &str, port: u16) -> Option<Self> {
        let route = policy.find_upstream_override(domain, port)?;
        let protocol = match route.protocol {
            UpstreamOverrideProtocol::Http => Protocol::Http,
            UpstreamOverrideProtocol::Tls => Protocol::Tls,
        };
        Some(Self::Override {
            dial: route.dial.clone(),
            protocol,
        })
    }

    /// The address the rules see as `ip.value`. A resolved name fails
    /// closed: any local or private answer speaks for the whole set. An
    /// override reports only what the guest's literal host said, as before.
    pub fn judged_ip(&self, domain: &str) -> Option<IpAddr> {
        match self {
            Self::Resolved(addresses) => judged_address(addresses),
            Self::Override { .. } | Self::Unresolved(_) => domain.parse().ok(),
        }
    }

    /// The protocol spoken to the upstream: the override's, else the guest's.
    pub fn protocol(&self, guest: Protocol) -> Protocol {
        match self {
            Self::Override { protocol, .. } => *protocol,
            Self::Resolved(_) | Self::Unresolved(_) => guest,
        }
    }

    /// Connect to exactly this target. Returns the stream and the target
    /// pinned to the peer it reached, which is what a reused connection is
    /// keyed by.
    pub async fn connect(&self) -> std::io::Result<(TcpStream, Self)> {
        let stream = match self {
            Self::Override { dial, .. } => TcpStream::connect(dial.as_str()).await?,
            Self::Resolved(addresses) => TcpStream::connect(addresses.as_slice()).await?,
            Self::Unresolved(error) => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, error.clone())),
        };
        let _ = stream.set_nodelay(true);
        let pinned = match self {
            Self::Resolved(_) => Self::Resolved(vec![stream.peer_addr()?]),
            other => other.clone(),
        };
        Ok((stream, pinned))
    }
}

impl fmt::Display for UpstreamTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Override { dial, .. } => write!(f, "override {dial}"),
            Self::Resolved(addresses) => write!(f, "{addresses:?}"),
            Self::Unresolved(error) => write!(f, "unresolved ({error})"),
        }
    }
}

/// One guest connection's open upstream sender, keyed by the host it was
/// opened for and the target it reached. A plain-HTTP guest can name a
/// different host on every request; an unkeyed sender delivered a request
/// judged for one host to whichever host the connection was opened for.
pub(super) struct CachedUpstream {
    domain: String,
    port: u16,
    target: UpstreamTarget,
    pub(super) sender: SendRequest<ProxyBoxBody>,
}

pub(super) type UpstreamCache = tokio::sync::Mutex<Option<CachedUpstream>>;

impl CachedUpstream {
    pub(super) fn new(domain: &str, port: u16, target: UpstreamTarget, sender: SendRequest<ProxyBoxBody>) -> Self {
        Self {
            domain: domain.to_string(),
            port,
            target,
            sender,
        }
    }

    /// True when this sender reaches exactly what `domain:port` was judged for.
    pub(super) fn serves(&self, domain: &str, port: u16, target: &UpstreamTarget) -> bool {
        self.domain == domain && self.port == port && &self.target == target
    }

    fn pinned_for(&self, domain: &str, port: u16) -> Option<UpstreamTarget> {
        let pinned = self.domain == domain && self.port == port && matches!(self.target, UpstreamTarget::Resolved(_));
        pinned.then(|| self.target.clone())
    }
}

#[cfg(test)]
mod tests;

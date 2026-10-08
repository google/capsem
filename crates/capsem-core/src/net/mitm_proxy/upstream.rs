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
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use hyper::client::conn::http1::SendRequest;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use super::body::ProxyBoxBody;
use super::protocol::Protocol;
use crate::net::policy::{NetworkMechanics, UpstreamOverrideProtocol};
use crate::net::upstream_address::{judged_address, UpstreamResolver};

/// Future returned by a trusted TCP selection provider.
pub type TcpResolveGrantFuture<'a> = Pin<Box<dyn Future<Output = io::Result<TcpGrantSelection>> + Send + 'a>>;
/// Future returned by a trusted connected-stream provider.
pub type TcpConnectGrantFuture<'a> = Pin<Box<dyn Future<Output = io::Result<GrantedTcpStream>> + Send + 'a>>;

/// Selects and connects upstreams without exposing address choice to a worker.
pub trait TcpUpstreamGrants: Send + Sync {
    fn resolve(&self, protocol: Protocol, host: &str, port: u16) -> TcpResolveGrantFuture<'_>;
    fn connect(&self, selection_id: u64) -> TcpConnectGrantFuture<'_>;
}

struct SelectionLease {
    id: u64,
    claimed: AtomicBool,
    release: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Drop for SelectionLease {
    fn drop(&mut self) {
        if !self.claimed.load(Ordering::Acquire) {
            if let Some(release) = self.release.get_mut().unwrap().take() {
                release();
            }
        }
    }
}

/// One coordinator-selected target, released if policy rejects it before use.
#[derive(Clone)]
pub struct TcpGrantSelection {
    lease: Arc<SelectionLease>,
    pub protocol: Protocol,
    pub judged_ip: Option<IpAddr>,
}

impl TcpGrantSelection {
    pub fn new(
        selection_id: u64,
        protocol: Protocol,
        judged_ip: Option<IpAddr>,
        release: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self {
            lease: Arc::new(SelectionLease {
                id: selection_id,
                claimed: AtomicBool::new(false),
                release: Mutex::new(Some(Box::new(release))),
            }),
            protocol,
            judged_ip,
        }
    }

    fn claim(&self) -> io::Result<u64> {
        self.lease
            .claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| self.lease.id)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "upstream selection was already consumed"))
    }
}

impl fmt::Debug for TcpGrantSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TcpGrantSelection")
            .field("id", &self.lease.id)
            .field("protocol", &self.protocol)
            .field("judged_ip", &self.judged_ip)
            .finish()
    }
}

impl PartialEq for TcpGrantSelection {
    fn eq(&self, other: &Self) -> bool {
        self.lease.id == other.lease.id && self.protocol == other.protocol && self.judged_ip == other.judged_ip
    }
}

impl Eq for TcpGrantSelection {}

/// A connected upstream descriptor whose coordinator grant is released on drop.
pub struct GrantedTcpStream {
    stream: TcpStream,
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl GrantedTcpStream {
    pub fn new(stream: TcpStream, release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            stream,
            release: Some(Box::new(release)),
        }
    }
}

impl Drop for GrantedTcpStream {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

impl AsyncRead for GrantedTcpStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for GrantedTcpStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// The upstream connection one request will use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpstreamTarget {
    /// An administrator's `upstream_overrides` route, dialed as configured.
    Override { dial: String, protocol: Protocol },
    /// The addresses the guest's name resolved to, in the order to try.
    Resolved(Vec<SocketAddr>),
    /// A trusted coordinator's opaque selection. Connected targets retain
    /// their judged identity while a retry obtains a fresh one-shot token.
    Granted {
        host: String,
        port: u16,
        guest_protocol: Protocol,
        selection: Option<TcpGrantSelection>,
        protocol: Protocol,
        judged_ip: Option<IpAddr>,
    },
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
        guest_protocol: Protocol,
        domain: &str,
        port: u16,
        cache: &UpstreamCache,
        grants: Option<&dyn TcpUpstreamGrants>,
    ) -> Self {
        if grants.is_none() {
            if let Some(target) = Self::override_target(policy, domain, port) {
                return target;
            }
        }
        if let Some(pinned) = cache
            .lock()
            .await
            .as_ref()
            .and_then(|cached| cached.pinned_for(domain, port))
        {
            return pinned;
        }
        if let Some(grants) = grants {
            return Self::resolve_granted(grants, guest_protocol, domain, port).await;
        }
        match resolver.resolve(domain, port).await {
            Ok(addresses) => Self::Resolved(addresses),
            Err(error) => Self::Unresolved(error),
        }
    }

    async fn resolve_granted(grants: &dyn TcpUpstreamGrants, protocol: Protocol, host: &str, port: u16) -> Self {
        match grants.resolve(protocol, host, port).await {
            Ok(selection) => Self::Granted {
                host: host.to_owned(),
                port,
                guest_protocol: protocol,
                protocol: selection.protocol,
                judged_ip: selection.judged_ip,
                selection: Some(selection),
            },
            Err(error) => Self::Unresolved(error.to_string()),
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
            Self::Granted { judged_ip, .. } => *judged_ip,
            Self::Override { .. } | Self::Unresolved(_) => domain.parse().ok(),
        }
    }

    /// The protocol spoken to the upstream: the override's, else the guest's.
    pub fn protocol(&self, guest: Protocol) -> Protocol {
        match self {
            Self::Override { protocol, .. } => *protocol,
            Self::Granted { protocol, .. } => *protocol,
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
            Self::Granted { .. } => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "descriptor grant required for brokered upstream target",
                ));
            }
            Self::Unresolved(error) => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, error.clone())),
        };
        let _ = stream.set_nodelay(true);
        let pinned = match self {
            Self::Resolved(_) => Self::Resolved(vec![stream.peer_addr()?]),
            other => other.clone(),
        };
        Ok((stream, pinned))
    }

    /// Connect through the trusted grant provider. Direct targets are refused
    /// whenever a provider is installed, preventing an accidental fallback.
    pub async fn connect_with_grants(
        &self,
        grants: Option<&dyn TcpUpstreamGrants>,
    ) -> io::Result<(GrantedTcpStream, Self)> {
        let Some(grants) = grants else {
            let (stream, pinned) = self.connect().await?;
            return Ok((GrantedTcpStream::new(stream, || {}), pinned));
        };
        let Self::Granted {
            host,
            port,
            guest_protocol,
            selection,
            protocol,
            judged_ip,
        } = self
        else {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "direct upstream target refused while descriptor grants are required",
            ));
        };
        let refreshed;
        let selection = match selection {
            Some(selection) => selection,
            None => {
                refreshed = grants.resolve(*guest_protocol, host, *port).await?;
                if refreshed.protocol != *protocol || refreshed.judged_ip != *judged_ip {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "refreshed upstream selection changed the judged target",
                    ));
                }
                &refreshed
            }
        };
        let stream = grants.connect(selection.claim()?).await?;
        Ok((
            stream,
            Self::Granted {
                host: host.clone(),
                port: *port,
                guest_protocol: *guest_protocol,
                selection: None,
                protocol: *protocol,
                judged_ip: *judged_ip,
            },
        ))
    }
}

impl fmt::Display for UpstreamTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Override { dial, .. } => write!(f, "override {dial}"),
            Self::Resolved(addresses) => write!(f, "{addresses:?}"),
            Self::Granted {
                host,
                port,
                protocol,
                judged_ip,
                ..
            } => write!(f, "granted {host}:{port} ({protocol:?}, {judged_ip:?})"),
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
        let pinned = self.domain == domain
            && self.port == port
            && matches!(
                self.target,
                UpstreamTarget::Resolved(_) | UpstreamTarget::Granted { selection: None, .. }
            );
        pinned.then(|| self.target.clone())
    }
}

#[cfg(test)]
mod tests;

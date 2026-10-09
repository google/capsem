//! Upstream DNS forwarder over UDP.
//!
//! Sends raw query bytes verbatim to the first reachable upstream and
//! returns the raw response bytes. Transparent forwarding: no parsing,
//! no rewriting, so resolver-specific record shapes (CNAME chains,
//! TTLs, EDNS pseudo-records) survive untouched.
//!
//! The upstream list is iterated in order on every query -- failover is
//! per-query, not sticky, so a transient network blip on the primary
//! upstream doesn't poison the resolver. Default upstreams are
//! `1.1.1.1:53` (Cloudflare) and `8.8.8.8:53` (Google). The host
//! controls this list, not the guest, so the policy boundary stays
//! intact -- a compromised guest can't redirect its own DNS.
//!
//! Per-query timeout is 5s by default. DNS queries that don't return
//! in 5s are gone -- recursive resolution is interactive at human
//! scale (<200ms typical) and timing out the whole query rather than
//! the per-attempt is fine for an interactive sandbox.

use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tokio::net::UdpSocket;
use tracing::{debug, warn};

/// Default upstream nameservers. Cloudflare 1.1.1.1 and Google 8.8.8.8
/// chosen for global availability + DNSSEC validation by default. Host
/// owners override via [`DnsResolver::with_upstreams`].
pub const DEFAULT_UPSTREAMS: &[&str] = &["1.1.1.1:53", "8.8.8.8:53"];

/// Default per-attempt timeout. Two seconds covers a slow upstream, and
/// with two default upstreams the whole `resolve()` fits inside glibc's
/// five-second resolver timeout in the guest: the client gets our SERVFAIL
/// and moves on instead of retransmitting a second copy of the same query
/// into the backlog while we are still waiting on the first.
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(2000);

/// One connected upstream datagram plus the action that releases its grant.
pub struct DnsDatagram {
    socket: UdpSocket,
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl DnsDatagram {
    pub fn new(socket: UdpSocket, release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            socket,
            release: Some(Box::new(release)),
        }
    }

    fn socket(&self) -> &UdpSocket {
        &self.socket
    }
}

impl Drop for DnsDatagram {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

/// Future returned by a trusted connected-datagram grant provider.
pub type DnsGrantFuture<'a> = Pin<Box<dyn Future<Output = Result<DnsDatagram>> + Send + 'a>>;

/// Supplies a connected UDP socket for one configured upstream index.
pub trait DnsUpstreamGrants: Send + Sync {
    fn open(&self, upstream_index: u16, policy_digest: &str) -> DnsGrantFuture<'_>;
}

struct DirectDnsUpstreams(Arc<RwLock<Vec<SocketAddr>>>);

impl DnsUpstreamGrants for DirectDnsUpstreams {
    fn open(&self, upstream_index: u16, _policy_digest: &str) -> DnsGrantFuture<'_> {
        Box::pin(async move {
            let upstream = self
                .0
                .read()
                .unwrap()
                .get(usize::from(upstream_index))
                .copied()
                .ok_or_else(|| anyhow!("DNS upstream index {upstream_index} is not configured"))?;
            let bind_addr: SocketAddr = if upstream.is_ipv6() {
                "[::]:0".parse().unwrap()
            } else {
                "0.0.0.0:0".parse().unwrap()
            };
            let socket = UdpSocket::bind(bind_addr).await?;
            socket.connect(upstream).await?;
            Ok(DnsDatagram::new(socket, || {}))
        })
    }
}

/// UDP DNS forwarder. Iterates the upstream list per query; first
/// successful response wins.
#[derive(Clone)]
pub struct DnsResolver {
    upstreams: Arc<RwLock<Vec<SocketAddr>>>,
    grants: Arc<dyn DnsUpstreamGrants>,
    per_attempt_timeout: Duration,
}

impl fmt::Debug for DnsResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DnsResolver")
            .field("upstreams", &*self.upstreams.read().unwrap())
            .field("per_attempt_timeout", &self.per_attempt_timeout)
            .finish_non_exhaustive()
    }
}

impl DnsResolver {
    /// Build a resolver targeting the [`DEFAULT_UPSTREAMS`] nameservers
    /// with the default per-attempt timeout.
    pub fn new() -> Self {
        let upstreams = DEFAULT_UPSTREAMS.iter().filter_map(|s| s.parse().ok()).collect();
        Self::with_upstreams(upstreams)
    }

    /// Build a resolver targeting an explicit list of upstreams.
    /// Used by tests + future operator config (capsem.toml).
    pub fn with_upstreams(upstreams: Vec<SocketAddr>) -> Self {
        let upstreams = Arc::new(RwLock::new(upstreams));
        Self {
            grants: Arc::new(DirectDnsUpstreams(Arc::clone(&upstreams))),
            upstreams,
            per_attempt_timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Build a resolver whose connected sockets come from a trusted grant
    /// provider. The addresses are labels for ordering and telemetry only.
    pub fn with_grants(upstreams: Vec<SocketAddr>, grants: Arc<dyn DnsUpstreamGrants>) -> Self {
        Self {
            upstreams: Arc::new(RwLock::new(upstreams)),
            grants,
            per_attempt_timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Set the per-attempt timeout. The whole resolve() call still
    /// iterates every upstream, so worst-case wall time is
    /// `timeout * upstreams.len()`.
    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.per_attempt_timeout = t;
        self
    }

    /// Snapshot the configured upstream list (debugging / metrics only).
    pub fn upstreams(&self) -> Vec<SocketAddr> {
        self.upstreams.read().unwrap().clone()
    }

    /// Replace the configured order after the owning policy reload succeeds.
    /// An in-flight query keeps the one snapshot it began with.
    pub fn replace_upstreams(&self, upstreams: Vec<SocketAddr>) {
        *self.upstreams.write().unwrap() = upstreams;
    }

    /// Forward `query_bytes` upstream. Returns the raw response bytes
    /// from the first upstream that answers within
    /// `per_attempt_timeout`, plus the elapsed wall time of the
    /// successful attempt for telemetry.
    ///
    /// On total failure (every upstream timed out or errored) returns
    /// the cumulative error so the caller can synthesize a SERVFAIL.
    pub async fn resolve(&self, query_bytes: &[u8]) -> Result<(Vec<u8>, Duration)> {
        self.resolve_for_policy(query_bytes, "direct").await
    }

    /// Forward under the exact active-policy revision that admitted the query.
    pub async fn resolve_for_policy(&self, query_bytes: &[u8], policy_digest: &str) -> Result<(Vec<u8>, Duration)> {
        let upstreams = self.upstreams();
        if upstreams.is_empty() {
            return Err(anyhow!("no upstream nameservers configured"));
        }
        let mut last_err: Option<anyhow::Error> = None;
        for (index, upstream) in upstreams.iter().enumerate() {
            let t0 = Instant::now();
            let upstream_index = u16::try_from(index).map_err(|_| anyhow!("too many DNS upstreams configured"))?;
            match self
                .try_one(upstream_index, *upstream, query_bytes, policy_digest)
                .await
            {
                Ok(resp) => {
                    let elapsed = t0.elapsed();
                    debug!(
                        upstream = %upstream,
                        elapsed_ms = elapsed.as_millis() as u64,
                        bytes = resp.len(),
                        "dns upstream answered"
                    );
                    return Ok((resp, elapsed));
                }
                Err(e) => {
                    warn!(upstream = %upstream, error = %e, "dns upstream failed");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("all DNS upstreams failed")))
    }

    async fn try_one(
        &self,
        upstream_index: u16,
        upstream: SocketAddr,
        query_bytes: &[u8],
        policy_digest: &str,
    ) -> Result<Vec<u8>> {
        let expected = ExpectedAnswer::for_query(query_bytes)?;
        let grant = self.grants.open(upstream_index, policy_digest).await?;
        let sock = grant.socket();
        sock.send(query_bytes).await?;
        let deadline = Instant::now() + self.per_attempt_timeout;
        let mut buf = vec![0u8; 4096];
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "dns recv timeout").into());
            }
            let n = match tokio::time::timeout(remaining, sock.recv(&mut buf)).await {
                Ok(Ok(n)) => n,
                Ok(Err(e)) => return Err(e.into()),
                Err(_) => return Err(io::Error::new(io::ErrorKind::TimedOut, "dns recv timeout").into()),
            };
            match expected.check(&buf[..n]) {
                Ok(()) => {
                    buf.truncate(n);
                    return Ok(buf);
                }
                Err(reason) => {
                    // A connected UDP socket still accepts any datagram whose
                    // source is spoofed as the upstream, and the id is the
                    // guest's to choose. Whatever this was, it is not the
                    // answer to our question; keep waiting for that one.
                    warn!(upstream = %upstream, reason, bytes = n, "dns upstream: discarding datagram");
                }
            }
        }
    }
}

/// What a datagram must carry to be the answer to the query we sent: the
/// query's transaction id, the response bit, and the same question.
struct ExpectedAnswer {
    id: u16,
    qname: String,
    qtype: u16,
    qclass: u16,
}

impl ExpectedAnswer {
    fn for_query(query_bytes: &[u8]) -> Result<Self> {
        let query = crate::net::parsers::dns_parser::parse_query(query_bytes)?;
        if query.extra_questions != 0 || query_bytes[2] & 0xF8 != 0 {
            return Err(anyhow!("DNS forwarding requires one standard question"));
        }
        Ok(Self {
            id: query.id,
            qname: query.qname,
            qtype: query.qtype,
            qclass: query.qclass,
        })
    }

    fn check(&self, datagram: &[u8]) -> std::result::Result<(), &'static str> {
        if datagram.len() < 12 {
            return Err("shorter than a DNS header");
        }
        if u16::from_be_bytes([datagram[0], datagram[1]]) != self.id {
            return Err("transaction id does not match the query");
        }
        if datagram[2] & 0xF8 != 0x80 {
            return Err("not a standard query response");
        }
        let answered =
            crate::net::parsers::dns_parser::parse_query(datagram).map_err(|_| "question does not decode")?;
        if answered.extra_questions != 0
            || answered.qname != self.qname
            || answered.qtype != self.qtype
            || answered.qclass != self.qclass
        {
            return Err("question does not match the query");
        }
        Ok(())
    }
}

impl Default for DnsResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

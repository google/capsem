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

use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tokio::net::UdpSocket;
use tracing::{debug, warn};

/// Default upstream nameservers. Cloudflare 1.1.1.1 and Google 8.8.8.8
/// chosen for global availability + DNSSEC validation by default. Host
/// owners override via [`DnsResolver::with_upstreams`].
pub const DEFAULT_UPSTREAMS: &[&str] = &["1.1.1.1:53", "8.8.8.8:53"];

/// Default per-attempt timeout. Recursive resolution is interactive at
/// human scale; anything over a second is already a bad UX, so 5s is
/// generous-enough to cover one upstream that's slow without making the
/// guest hang on a dead upstream.
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(5000);

/// UDP DNS forwarder. Iterates the upstream list per query; first
/// successful response wins.
#[derive(Debug, Clone)]
pub struct DnsResolver {
    upstreams: Vec<SocketAddr>,
    per_attempt_timeout: Duration,
}

impl DnsResolver {
    /// Build a resolver targeting the [`DEFAULT_UPSTREAMS`] nameservers
    /// with the default per-attempt timeout.
    pub fn new() -> Self {
        let upstreams = DEFAULT_UPSTREAMS
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        Self {
            upstreams,
            per_attempt_timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Build a resolver targeting an explicit list of upstreams.
    /// Used by tests + future operator config (capsem.toml).
    pub fn with_upstreams(upstreams: Vec<SocketAddr>) -> Self {
        Self {
            upstreams,
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

    /// Borrow the configured upstream list (debugging / metrics only).
    pub fn upstreams(&self) -> &[SocketAddr] {
        &self.upstreams
    }

    /// Forward `query_bytes` upstream. Returns the raw response bytes
    /// from the first upstream that answers within
    /// `per_attempt_timeout`, plus the elapsed wall time of the
    /// successful attempt for telemetry.
    ///
    /// On total failure (every upstream timed out or errored) returns
    /// the cumulative error so the caller can synthesize a SERVFAIL.
    pub async fn resolve(&self, query_bytes: &[u8]) -> Result<(Vec<u8>, Duration)> {
        if self.upstreams.is_empty() {
            return Err(anyhow!("no upstream nameservers configured"));
        }
        let mut last_err: Option<anyhow::Error> = None;
        for upstream in &self.upstreams {
            let t0 = Instant::now();
            match self.try_one(*upstream, query_bytes).await {
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

    async fn try_one(&self, upstream: SocketAddr, query_bytes: &[u8]) -> Result<Vec<u8>> {
        let bind_addr: SocketAddr = if upstream.is_ipv6() {
            "[::]:0".parse().unwrap()
        } else {
            "0.0.0.0:0".parse().unwrap()
        };
        let sock = UdpSocket::bind(bind_addr).await?;
        sock.connect(upstream).await?;
        sock.send(query_bytes).await?;
        let mut buf = vec![0u8; 4096];
        let n = match tokio::time::timeout(self.per_attempt_timeout, sock.recv(&mut buf)).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "dns recv timeout").into())
            }
        };
        buf.truncate(n);
        Ok(buf)
    }
}

impl Default for DnsResolver {
    fn default() -> Self {
        Self::new()
    }
}

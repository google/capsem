//! The address a guest-named upstream really is.
//!
//! Security rules are written against names, but a socket reaches an
//! address, and the guest chooses the name: one that resolves to loopback,
//! link-local or a private range, or one whose answer changes between the
//! check and the dial (DNS rebinding). Every host-side dial on the guest's
//! behalf therefore resolves first, judges what it resolved, and connects to
//! exactly those addresses -- never to the name again.
//!
//! Names resolve through the host's system resolver, the one these dials
//! always went through: it honours `/etc/hosts`, split-horizon corp DNS and
//! the cloud metadata names a guest would aim for.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// True for an address on the public Internet; false for loopback, private,
/// link-local, shared, documentation, benchmark, multicast, broadcast and
/// unspecified space, and for an IPv6 address that maps or embeds one of
/// those.
pub fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            let segments = v6.segments();
            !(v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || (segments[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
                || (segments[0] == 0x2002 && !is_public_v4(embedded_6to4(segments))) // 6to4 of a private v4
                || (segments[0] == 0x0064 && segments[1] == 0xff9b && !is_public_v4(embedded_nat64(segments))))
        }
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [a, b, _, _] = address.octets();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_multicast()
        || address.is_documentation()
        || a == 0 // 0.0.0.0/8 "this network"
        || (a == 100 && (64..=127).contains(&b)) // 100.64.0.0/10 shared address space
        || (a == 198 && (b == 18 || b == 19)) // 198.18.0.0/15 benchmarking
        || a >= 240) // 240.0.0.0/4 reserved, incl. broadcast
}

fn embedded_6to4(segments: [u16; 8]) -> Ipv4Addr {
    Ipv4Addr::from((u32::from(segments[1]) << 16) | u32::from(segments[2]))
}

fn embedded_nat64(segments: [u16; 8]) -> Ipv4Addr {
    Ipv4Addr::from((u32::from(segments[6]) << 16) | u32::from(segments[7]))
}

/// Resolve `host:port` through the system resolver. An IP literal resolves
/// to itself. An IPv4-mapped IPv6 answer is reported as the IPv4 address it
/// reaches, so `::ffff:127.0.0.1` is judged as the loopback it is.
pub async fn resolve_upstream(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let canonical = |address: SocketAddr| SocketAddr::new(address.ip().to_canonical(), address.port());
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(vec![canonical(SocketAddr::new(address, port))]);
    }
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("could not resolve {host}: {error}"))?
        .map(canonical)
        .collect();
    if addresses.is_empty() {
        return Err(format!("could not resolve {host}: no addresses"));
    }
    Ok(addresses)
}

/// Where guest-named upstreams resolve. The system resolver unless a name
/// carries a fixed answer -- a deterministic stand-in for a DNS record, so a
/// test can point a public-looking name at loopback without touching DNS.
#[derive(Clone, Debug)]
pub struct UpstreamResolver {
    fixed: BTreeMap<String, Vec<IpAddr>>,
    system: bool,
}

impl Default for UpstreamResolver {
    fn default() -> Self {
        Self::system()
    }
}

impl UpstreamResolver {
    /// Resolve every name through the host's system resolver.
    pub fn system() -> Self {
        Self {
            fixed: BTreeMap::new(),
            system: true,
        }
    }

    /// Carry no host resolution authority.  Confined workers use connected
    /// descriptor grants instead, so even an accidental direct selection
    /// must fail closed before consulting DNS.
    pub fn disabled() -> Self {
        Self {
            fixed: BTreeMap::new(),
            system: false,
        }
    }

    /// Answer `name` with `addresses` instead of asking the system resolver.
    pub fn with_fixed_answer(mut self, name: &str, addresses: Vec<IpAddr>) -> Self {
        self.fixed.insert(name.to_ascii_lowercase(), addresses);
        self
    }

    /// The addresses `host:port` reaches, in the order to try them.
    pub async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
        match self.fixed.get(host) {
            Some(addresses) if addresses.is_empty() => Err(format!("could not resolve {host}: no addresses")),
            Some(addresses) => Ok(addresses
                .iter()
                .map(|address| SocketAddr::new(address.to_canonical(), port))
                .collect()),
            None if self.system => resolve_upstream(host, port).await,
            None => Err("upstream resolution is disabled in this process".to_owned()),
        }
    }
}

/// The one address that speaks for a resolved set when rules see a single
/// `ip.value`: the first non-public address if there is one, else the first.
/// It fails closed -- a name that resolves to anything local or private is
/// judged as local or private, whatever else it resolves to.
pub fn judged_address(addresses: &[SocketAddr]) -> Option<IpAddr> {
    addresses
        .iter()
        .map(SocketAddr::ip)
        .find(|address| !is_public_address(*address))
        .or_else(|| addresses.first().map(SocketAddr::ip))
}

#[cfg(test)]
mod tests;

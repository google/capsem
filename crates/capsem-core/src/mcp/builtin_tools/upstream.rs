//! Where a built-in HTTP tool is allowed to connect.
//!
//! The security rules judge `http.host`, but the socket goes to whatever
//! that name resolves to, and the guest can pick a name that resolves to
//! loopback, link-local or a private range -- or one whose answer changes
//! between the check and the dial (DNS rebinding). So the boundary judges
//! the resolved addresses too, refuses a non-public address that no rule
//! explicitly allows, and pins the connection to the exact addresses it
//! judged.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use reqwest::Client;

/// The HTTP settings the built-in tools connect with. Redirects are never
/// followed: the boundary evaluated the requested URL, and a 3xx to another
/// host would reach a name it never judged.
#[derive(Clone, Debug)]
pub struct BuiltinHttpClient {
    request_timeout: Duration,
    connect_timeout: Duration,
    user_agent: Option<String>,
}

impl BuiltinHttpClient {
    pub fn new(request_timeout: Duration, connect_timeout: Duration) -> Self {
        Self {
            request_timeout,
            connect_timeout,
            user_agent: None,
        }
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    fn builder(&self) -> reqwest::ClientBuilder {
        let builder = Client::builder()
            .timeout(self.request_timeout)
            .connect_timeout(self.connect_timeout)
            .redirect(reqwest::redirect::Policy::none());
        match &self.user_agent {
            Some(user_agent) => builder.user_agent(user_agent.clone()),
            None => builder,
        }
    }

    /// A client whose connections for `host` go only to `addresses`: the
    /// ones the boundary judged. A name that re-resolves elsewhere between
    /// the check and the dial reaches nothing new.
    pub fn pinned(&self, host: &str, addresses: &[SocketAddr]) -> reqwest::Result<Client> {
        self.builder().resolve_to_addrs(host, addresses).build()
    }
}

/// True for an address on the public Internet; false for loopback, private,
/// link-local, shared, documentation, benchmark, multicast, broadcast and
/// unspecified space, and for an IPv6 address that maps or embeds one of
/// those. Host rules are written against names; only an explicit allow rule
/// may send a built-in tool to an address in this set.
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

/// Resolve `host:port` the way the connector will. An IP literal resolves
/// to itself; a name goes through the system resolver off the runtime.
pub async fn resolve_upstream(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(address, port)]);
    }
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("could not resolve {host}: {error}"))?
        .collect();
    if addresses.is_empty() {
        return Err(format!("could not resolve {host}: no addresses"));
    }
    Ok(addresses)
}

/// The reason a non-public address is refused when no rule allowed it.
pub fn non_public_refusal(domain: &str, address: IpAddr) -> String {
    format!(
        "HTTP request blocked: {domain} resolves to {address}, a non-public address; \
         only an explicit allow rule may reach it"
    )
}

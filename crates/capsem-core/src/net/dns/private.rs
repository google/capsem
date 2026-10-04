//! The private zone: names of network members, answered on the host and
//! never asked upstream.
//!
//! `<vm>.<network>.capsem.internal` and `<vm>.capsem.internal` name a
//! member's lifetime address; the reverse of a pool address names the
//! member. Who may see which name is the service's decision (only current
//! members of a shared network), asked per query through [`PrivateNames`]
//! and answered with a zero TTL, so nothing is cached and nothing has to be
//! invalidated when a membership changes.
use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;

pub const PRIVATE_ZONE: &str = "capsem.internal";
const REVERSE_ZONE: &str = "in-addr.arpa";

/// Capsem's MCP gateway as a workload reaches it: streamable HTTP to this
/// name, carried by the VM's interception proxy to this VM's host-side MCP
/// endpoint. It is the zone's one name that is Capsem's own rather than a
/// member's, and it is answered before any member is asked, so a peer VM
/// named `mcp` can never draw the MCP traffic of another.
pub const MCP_HOST: &str = "mcp.capsem.internal";

/// What [`MCP_HOST`] resolves to. Nothing ever dials it: the VM's
/// interception rules hand every port-80 and port-443 connection to the
/// proxy, which routes on the name. TEST-NET-1 (RFC 5737) is routed nowhere,
/// so the answer cannot point a client at a real host, and it lies outside
/// the private pool, whose traffic leaves through a cable and skips the proxy.
pub const MCP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);

/// What a query under the zone asks: Capsem's MCP gateway, a member's name
/// (the labels before the zone, empty for the zone apex) or the owner of a
/// pool address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateQuestion {
    McpGateway,
    Name(String),
    Reverse(Ipv4Addr),
}

/// Whether `host` (any case, with or without the DNS root dot) is in the
/// zone. Nothing in it exists upstream, so nothing in it is ever dialed.
pub fn in_private_zone(host: &str) -> bool {
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    name == PRIVATE_ZONE || name.ends_with(&format!(".{PRIVATE_ZONE}"))
}

/// The private question a query is, if it is one. Reverse questions count
/// only inside the private pool: every other `in-addr.arpa` name is the
/// upstream's to answer.
pub fn private_question(qname: &str, _qtype: u16) -> Option<PrivateQuestion> {
    let name = qname.trim_end_matches('.').to_ascii_lowercase();
    if name == MCP_HOST {
        return Some(PrivateQuestion::McpGateway);
    }
    if name == PRIVATE_ZONE {
        return Some(PrivateQuestion::Name(String::new()));
    }
    if let Some(labels) = name.strip_suffix(&format!(".{PRIVATE_ZONE}")) {
        return Some(PrivateQuestion::Name(labels.to_string()));
    }
    let reversed = name.strip_suffix(&format!(".{REVERSE_ZONE}"))?;
    let octets: Vec<u8> = reversed.split('.').map(str::parse).collect::<Result<_, _>>().ok()?;
    let [d, c, b, a] = octets[..] else { return None };
    let address = Ipv4Addr::new(a, b, c, d);
    capsem_config::PrivatePool::DEFAULT
        .contains(address)
        .then_some(PrivateQuestion::Reverse(address))
}

pub type Lookup<'a, T> = Pin<Box<dyn Future<Output = Option<T>> + Send + 'a>>;

/// Who answers private questions for one VM: the service, through its
/// owner, with what that VM may see.
pub trait PrivateNames: Send + Sync {
    /// The address behind `name` (labels before the zone), if the asker
    /// may see it.
    fn address_of<'a>(&'a self, name: &'a str) -> Lookup<'a, Ipv4Addr>;
    /// The fully qualified name of the member at `address`, if the asker
    /// may see it.
    fn name_of(&self, address: Ipv4Addr) -> Lookup<'_, String>;
}

#[cfg(test)]
mod tests;

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

/// What a query under the zone asks: a name (the labels before the zone,
/// empty for the zone apex) or the owner of a pool address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateQuestion {
    Name(String),
    Reverse(Ipv4Addr),
}

/// The private question a query is, if it is one. Reverse questions count
/// only inside the private pool: every other `in-addr.arpa` name is the
/// upstream's to answer.
pub fn private_question(qname: &str, _qtype: u16) -> Option<PrivateQuestion> {
    let name = qname.trim_end_matches('.').to_ascii_lowercase();
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

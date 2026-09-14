//! The private IPv4 pool every VM draws its lifetime address from.
//!
//! One pool for the host: a VM's address is a host-wide fact, not a profile
//! setting, so the pool is not something two profiles could disagree on.
//! The pool must stay clear of the two links a guest already has -- its own
//! dummy interface on `10.0.0.0/24` and the container veth on `10.0.1.0/30`
//! -- or a private address would shadow a local one inside the guest.

use std::fmt;
use std::net::Ipv4Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrivatePool {
    network: Ipv4Addr,
    prefix_len: u8,
}

/// The guest's own link (`dummy0`), configured by capsem-init.
pub const GUEST_LINK: PrivatePool = PrivatePool::new_unchecked(Ipv4Addr::new(10, 0, 0, 0), 24);
/// The container veth link (`capsem0`/`eth0`), configured by the workload launcher.
pub const CONTAINER_LINK: PrivatePool = PrivatePool::new_unchecked(Ipv4Addr::new(10, 0, 1, 0), 30);

/// Wide enough that a pool never runs out on one host; narrow enough that a
/// typo cannot claim all of RFC 1918.
const MIN_PREFIX_LEN: u8 = 8;
/// A /29 leaves the gateway and five VMs; anything smaller cannot hold a VM.
const MAX_PREFIX_LEN: u8 = 29;

impl PrivatePool {
    /// `10.128.0.0/9`: the upper half of `10/8`, disjoint from both guest links.
    pub const DEFAULT: PrivatePool = PrivatePool::new_unchecked(Ipv4Addr::new(10, 128, 0, 0), 9);

    const fn new_unchecked(network: Ipv4Addr, prefix_len: u8) -> Self {
        Self { network, prefix_len }
    }

    /// Parse `a.b.c.d/p`. The address must be the network address, the
    /// prefix within bounds, the range private, and clear of the guest links.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (address, prefix) = text
            .split_once('/')
            .ok_or_else(|| format!("private pool {text:?} must be written as a.b.c.d/prefix"))?;
        let network: Ipv4Addr = address
            .parse()
            .map_err(|_| format!("private pool {text:?} has an invalid address"))?;
        let prefix_len: u8 = prefix
            .parse()
            .map_err(|_| format!("private pool {text:?} has an invalid prefix"))?;
        if !(MIN_PREFIX_LEN..=MAX_PREFIX_LEN).contains(&prefix_len) {
            return Err(format!(
                "private pool {text:?} prefix must be between /{MIN_PREFIX_LEN} and /{MAX_PREFIX_LEN}"
            ));
        }
        let pool = Self { network, prefix_len };
        if u32::from(network) & pool.mask() != u32::from(network) {
            return Err(format!("private pool {text:?} is not aligned to its prefix"));
        }
        if !network.is_private() {
            return Err(format!("private pool {text:?} must be an RFC 1918 range"));
        }
        for (name, link) in [("guest link", GUEST_LINK), ("container link", CONTAINER_LINK)] {
            if pool.overlaps(link) {
                return Err(format!("private pool {text:?} overlaps the {name} {link}"));
            }
        }
        Ok(pool)
    }

    pub const fn network(&self) -> Ipv4Addr {
        self.network
    }

    pub const fn prefix_len(&self) -> u8 {
        self.prefix_len
    }

    const fn mask(&self) -> u32 {
        u32::MAX << (32 - self.prefix_len)
    }

    const fn broadcast(&self) -> u32 {
        u32::from_be_bytes(self.network.octets()) | !self.mask()
    }

    /// The host's address on every link: the first host of the pool.
    pub const fn gateway(&self) -> Ipv4Addr {
        Ipv4Addr::from_bits(u32::from_be_bytes(self.network.octets()) + 1)
    }

    /// The first address a VM can be given.
    pub const fn first_host(&self) -> Ipv4Addr {
        Ipv4Addr::from_bits(u32::from_be_bytes(self.network.octets()) + 2)
    }

    /// The last address a VM can be given.
    pub const fn last_host(&self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.broadcast() - 1)
    }

    /// Addresses available to VMs: everything but network, gateway, broadcast.
    pub const fn capacity(&self) -> u32 {
        (1u32 << (32 - self.prefix_len)) - 3
    }

    pub const fn contains(&self, address: Ipv4Addr) -> bool {
        u32::from_be_bytes(address.octets()) & self.mask() == u32::from_be_bytes(self.network.octets())
    }

    pub const fn overlaps(&self, other: PrivatePool) -> bool {
        self.contains(other.network) || other.contains(self.network)
    }
}

impl fmt::Display for PrivatePool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix_len)
    }
}

#[cfg(test)]
mod tests;

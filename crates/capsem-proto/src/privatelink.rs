//! What the private link's two ends agree on: the connect header of a
//! private TCP connection on VSOCK `VSOCK_PORT_PRIVATE`, and the ethernet
//! facts of the frame link on VSOCK `VSOCK_PORT_NETWORK`.
//!
//! The link is a `tap0` in the guest whose frames cross the pump as
//! `u16`-length records and are switched by the network's confined process.
//! The MAC is a function of the pool address, so the switch derives every
//! member's from the registry and pins the source of every frame to it.
//!
//! The guest proxy intercepted a connect to a member's private address and
//! must say where it was going: iptables REDIRECT has rewritten the socket's
//! destination, so the original one travels here, before the process meta
//! line the MITM rail also sends and before any payload byte. The
//! workload's own port travels too: the destination's audit row names the
//! flow by both endpoints, and the ledger refuses a TCP endpoint without one.
use std::net::Ipv4Addr;

/// The ethernet header a frame on the link carries: two MACs, an ethertype.
pub const ETHERNET_HEADER_BYTES: usize = 14;
/// The largest IP packet the link carries: a `u16` frame less its header,
/// which is also the largest MTU Linux gives a tap device.
pub const LINK_MTU: usize = u16::MAX as usize - ETHERNET_HEADER_BYTES;

/// The MAC of the member at `address`: locally administered, unicast, and
/// nothing but the address, so it never has to be exchanged.
pub const fn mac_of(address: Ipv4Addr) -> [u8; 6] {
    let [a, b, c, d] = address.octets();
    [0x02, 0xca, a, b, c, d]
}

/// Fixed size on the wire: version, protocol, address, port, source port.
pub const HEADER_BYTES: usize = 10;
const VERSION: u8 = 2;
const PROTOCOL_TCP: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectHeader {
    pub destination: Ipv4Addr,
    pub port: u16,
    /// The workload's ephemeral port, as the REDIRECT target saw it.
    pub source_port: u16,
}

impl ConnectHeader {
    pub fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut bytes = [0u8; HEADER_BYTES];
        bytes[0] = VERSION;
        bytes[1] = PROTOCOL_TCP;
        bytes[2..6].copy_from_slice(&self.destination.octets());
        bytes[6..8].copy_from_slice(&self.port.to_be_bytes());
        bytes[8..10].copy_from_slice(&self.source_port.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8; HEADER_BYTES]) -> Result<Self, String> {
        if bytes[0] != VERSION {
            return Err(format!("private connect header version {} is not {VERSION}", bytes[0]));
        }
        if bytes[1] != PROTOCOL_TCP {
            return Err(format!("private connect protocol {} is not TCP", bytes[1]));
        }
        let port = u16::from_be_bytes([bytes[6], bytes[7]]);
        if port == 0 {
            return Err("private connect to port 0".into());
        }
        let source_port = u16::from_be_bytes([bytes[8], bytes[9]]);
        if source_port == 0 {
            return Err("private connect from port 0".into());
        }
        Ok(Self {
            destination: Ipv4Addr::new(bytes[2], bytes[3], bytes[4], bytes[5]),
            port,
            source_port,
        })
    }
}

#[cfg(test)]
mod tests;

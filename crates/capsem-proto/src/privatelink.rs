//! The first bytes of a private TCP connection on VSOCK `VSOCK_PORT_PRIVATE`.
//!
//! The guest proxy intercepted a connect to a member's private address and
//! must say where it was going: iptables REDIRECT has rewritten the socket's
//! destination, so the original one travels here, before the process meta
//! line the MITM rail also sends and before any payload byte.
use std::net::Ipv4Addr;

/// Fixed size on the wire: version, protocol, address, port.
pub const HEADER_BYTES: usize = 8;
const VERSION: u8 = 1;
const PROTOCOL_TCP: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectHeader {
    pub destination: Ipv4Addr,
    pub port: u16,
}

impl ConnectHeader {
    pub fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut bytes = [0u8; HEADER_BYTES];
        bytes[0] = VERSION;
        bytes[1] = PROTOCOL_TCP;
        bytes[2..6].copy_from_slice(&self.destination.octets());
        bytes[6..8].copy_from_slice(&self.port.to_be_bytes());
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
        Ok(Self {
            destination: Ipv4Addr::new(bytes[2], bytes[3], bytes[4], bytes[5]),
            port,
        })
    }
}

#[cfg(test)]
mod tests;

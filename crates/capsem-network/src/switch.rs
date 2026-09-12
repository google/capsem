//! The verdict on one frame from one member of a network.
//!
//! A member is known by its pool address; its MAC is `mac_of(address)`, so
//! the switch needs no learning table and no broadcast domain. A frame is
//! forwarded to exactly one other member, answered (an ARP request for a
//! member), or dropped for a reason that is counted. The checks are the
//! whole security of the data plane past membership: the source is pinned
//! to the member the stream belongs to, the destination must be a linked
//! member, and TCP never crosses because it has its own admitted path.
//!
//! Only the bytes the verdict needs are read: two MACs, an ethertype, the
//! IPv4 fixed header's version, length, protocol and addresses. Fragments
//! carry all of those and pass like any packet; nothing is reassembled.
use capsem_proto::privatelink::{mac_of, ETHERNET_HEADER_BYTES};
use std::net::Ipv4Addr;

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
const IPV4_HEADER_BYTES: usize = 20;
const ARP_BYTES: usize = 28;
const PROTOCOL_TCP: u8 = 6;
const ARP_REQUEST: u16 = 1;
const ARP_REPLY: u16 = 2;
const BROADCAST: [u8; 6] = [0xff; 6];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Deliver the frame as is to the member at this address.
    Forward(Ipv4Addr),
    /// Write this frame back to the member that sent the request.
    Reply(Vec<u8>),
    Drop(DropReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum DropReason {
    /// Shorter than the headers the verdict needs.
    Short,
    /// Neither IPv4 nor ARP.
    EtherType,
    /// The source MAC is not the member's.
    SourceMac,
    /// The source address is not the member's.
    SourceAddress,
    /// An IPv4 header the verdict cannot trust.
    Header,
    /// TCP has its own admitted path and never crosses the link.
    Tcp,
    /// The destination MAC does not name the destination address.
    DestinationMac,
    /// The destination is not a linked member, or is the sender itself.
    Unknown,
    /// An ARP frame that is not a member's request for a member.
    Arp,
}

impl DropReason {
    pub const ALL: [DropReason; 9] = [
        Self::Short,
        Self::EtherType,
        Self::SourceMac,
        Self::SourceAddress,
        Self::Header,
        Self::Tcp,
        Self::DestinationMac,
        Self::Unknown,
        Self::Arp,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::EtherType => "ethertype",
            Self::SourceMac => "source_mac",
            Self::SourceAddress => "source_address",
            Self::Header => "header",
            Self::Tcp => "tcp",
            Self::DestinationMac => "destination_mac",
            Self::Unknown => "unknown",
            Self::Arp => "arp",
        }
    }
}

fn address(bytes: &[u8]) -> Ipv4Addr {
    Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])
}

/// The fate of `frame`, sent by the member at `own`, on a network where
/// `member` says which addresses are linked.
pub fn classify(own: Ipv4Addr, member: impl Fn(Ipv4Addr) -> bool, frame: &[u8]) -> Verdict {
    if frame.len() < ETHERNET_HEADER_BYTES {
        return Verdict::Drop(DropReason::Short);
    }
    let (header, payload) = frame.split_at(ETHERNET_HEADER_BYTES);
    if header[6..12] != mac_of(own) {
        return Verdict::Drop(DropReason::SourceMac);
    }
    match u16::from_be_bytes([header[12], header[13]]) {
        ETHERTYPE_IPV4 => ipv4(own, member, header, payload),
        ETHERTYPE_ARP => arp(own, member, header, payload),
        _ => Verdict::Drop(DropReason::EtherType),
    }
}

fn ipv4(own: Ipv4Addr, member: impl Fn(Ipv4Addr) -> bool, header: &[u8], packet: &[u8]) -> Verdict {
    if packet.len() < IPV4_HEADER_BYTES {
        return Verdict::Drop(DropReason::Short);
    }
    let total_length = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    if packet[0] >> 4 != 4 || packet[0] & 0x0f < 5 || total_length < IPV4_HEADER_BYTES || total_length > packet.len() {
        return Verdict::Drop(DropReason::Header);
    }
    if address(&packet[12..16]) != own {
        return Verdict::Drop(DropReason::SourceAddress);
    }
    if packet[9] == PROTOCOL_TCP {
        return Verdict::Drop(DropReason::Tcp);
    }
    let destination = address(&packet[16..20]);
    if destination == own || !member(destination) {
        return Verdict::Drop(DropReason::Unknown);
    }
    if header[..6] != mac_of(destination) {
        return Verdict::Drop(DropReason::DestinationMac);
    }
    Verdict::Forward(destination)
}

/// A member's ARP request for another member is answered here, with the
/// derived MAC; the request never reaches anyone else.
fn arp(own: Ipv4Addr, member: impl Fn(Ipv4Addr) -> bool, header: &[u8], body: &[u8]) -> Verdict {
    let own_mac = mac_of(own);
    let request = body.len() >= ARP_BYTES
        && body[..8] == [0, 1, 0x08, 0, 6, 4, 0, ARP_REQUEST as u8]
        && body[8..14] == own_mac
        && address(&body[14..18]) == own;
    if !request {
        return Verdict::Drop(DropReason::Arp);
    }
    let target = address(&body[24..28]);
    let target_mac = mac_of(target);
    let addressed = header[..6] == BROADCAST || header[..6] == target_mac;
    if !addressed || target == own || !member(target) {
        return Verdict::Drop(DropReason::Arp);
    }
    let mut reply = Vec::with_capacity(ETHERNET_HEADER_BYTES + ARP_BYTES);
    reply.extend_from_slice(&own_mac);
    reply.extend_from_slice(&target_mac);
    reply.extend_from_slice(&ETHERTYPE_ARP.to_be_bytes());
    reply.extend_from_slice(&[0, 1, 0x08, 0, 6, 4, 0, ARP_REPLY as u8]);
    reply.extend_from_slice(&target_mac);
    reply.extend_from_slice(&target.octets());
    reply.extend_from_slice(&own_mac);
    reply.extend_from_slice(&own.octets());
    Verdict::Reply(reply)
}

#[cfg(test)]
mod tests;

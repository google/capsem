//! Where one frame goes on a network's switch.
//!
//! A switch is plugged ports, each known by the MAC of the cable plugged
//! into it. It reads the two MACs at the front of a frame and nothing
//! else: every protocol crosses, and the kernels at either end do ARP, IP
//! and everything above. A frame to a port's MAC goes to that port; a frame
//! to a group address (broadcast, multicast) floods to every other port,
//! which is how guests find each other with ordinary ARP. The service
//! programs the table on plug and unplug, so nothing is learned and an
//! unknown destination is dropped rather than flooded.
//!
//! Every port is one station: the cable's MAC and the one address the
//! service leased it. A frame must come from that MAC, and an IPv4 packet
//! or an ARP message from that address -- port security, as a managed
//! switch has it -- so no member speaks as another, no ARP reply steals
//! another's traffic, and every counter names the port that sent the frame.
//! Other ethertypes cross unchecked: members have IPv4 addresses and names
//! only.
//!
//! The switch is a network, not a security boundary: whoever is plugged into
//! it can reach everyone else plugged into it.
use std::collections::HashMap;

pub type Mac = [u8; 6];

/// The two MACs and the ethertype.
pub const ETHERNET_HEADER_BYTES: usize = 14;
const ETHERTYPE_IPV4: [u8; 2] = [0x08, 0x00];
const ETHERTYPE_ARP: [u8; 2] = [0x08, 0x06];
/// Where a frame names its sender's address: an IPv4 header's source, and
/// the sender protocol address of an ARP message over ethernet.
const IPV4_SOURCE: std::ops::Range<usize> = ETHERNET_HEADER_BYTES + 12..ETHERNET_HEADER_BYTES + 16;
const ARP_SENDER: std::ops::Range<usize> = ETHERNET_HEADER_BYTES + 14..ETHERNET_HEADER_BYTES + 18;

/// Who is plugged into a port: its cable's MAC and the address it leased.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Station {
    pub mac: Mac,
    pub address: [u8; 4],
}

#[derive(Debug, PartialEq, Eq)]
pub enum Route<'a, P> {
    /// Deliver the frame as is to this port.
    Unicast(&'a P),
    /// Deliver the frame to every port but the sender's.
    Flood,
    Drop(DropReason),
}

/// Why a frame was not delivered. Each reason indexes one counter slot, so
/// the I/O side counts its own losses with the same array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum DropReason {
    /// Shorter than an ethernet header, or an IPv4 or ARP frame that ends
    /// before naming its sender.
    Short,
    /// The source MAC is not the sending port's.
    SourceMac,
    /// An IPv4 or ARP sender address is not the sending port's.
    SourceAddress,
    /// No port owns the destination MAC, or the sender does.
    Unknown,
    /// The destination port's queue was full.
    QueueFull,
    /// The sender flooded past its broadcast cap.
    Storm,
}

impl DropReason {
    pub const ALL: [DropReason; 6] = [
        Self::Short,
        Self::SourceMac,
        Self::SourceAddress,
        Self::Unknown,
        Self::QueueFull,
        Self::Storm,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::SourceMac => "source_mac",
            Self::SourceAddress => "source_address",
            Self::Unknown => "unknown",
            Self::QueueFull => "queue_full",
            Self::Storm => "storm",
        }
    }
}

/// The ports plugged into one switch, by MAC.
#[derive(Debug, Clone)]
pub struct Table<P> {
    ports: HashMap<Mac, P>,
}

impl<P> Default for Table<P> {
    fn default() -> Self {
        Self { ports: HashMap::new() }
    }
}

impl<P> Table<P> {
    /// Plug `port` in for `mac`, returning the port it replaces.
    pub fn plug(&mut self, mac: Mac, port: P) -> Option<P> {
        self.ports.insert(mac, port)
    }

    pub fn unplug(&mut self, mac: &Mac) -> Option<P> {
        self.ports.remove(mac)
    }

    pub fn len(&self) -> usize {
        self.ports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }

    /// Where `frame`, sent by the port `own` is plugged into, goes.
    pub fn route(&self, own: &Station, frame: &[u8]) -> Route<'_, P> {
        if frame.len() < ETHERNET_HEADER_BYTES {
            return Route::Drop(DropReason::Short);
        }
        if frame[6..12] != own.mac[..] {
            return Route::Drop(DropReason::SourceMac);
        }
        let sender = match [frame[12], frame[13]] {
            ETHERTYPE_IPV4 => Some(IPV4_SOURCE),
            ETHERTYPE_ARP => Some(ARP_SENDER),
            _ => None,
        };
        if let Some(sender) = sender {
            match frame.get(sender) {
                None => return Route::Drop(DropReason::Short),
                Some(address) if address != own.address => return Route::Drop(DropReason::SourceAddress),
                Some(_) => {}
            }
        }
        let destination: &Mac = frame[..6].try_into().expect("six bytes");
        if destination[0] & 1 == 1 {
            return Route::Flood;
        }
        match self.ports.get(destination) {
            Some(port) if *destination != own.mac => Route::Unicast(port),
            _ => Route::Drop(DropReason::Unknown),
        }
    }

    /// Every port a flood from `own` reaches.
    pub fn others<'a>(&'a self, own: &'a Mac) -> impl Iterator<Item = &'a P> {
        self.ports
            .iter()
            .filter(move |(mac, _)| *mac != own)
            .map(|(_, port)| port)
    }
}

#[cfg(test)]
mod tests;

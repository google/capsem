//! Where one frame goes on a network's switch.
//!
//! A switch is plugged ports, each known by the MAC of the cable plugged
//! into it. It reads the two MACs at the front of a frame and nothing
//! else: every protocol crosses, and the kernels at either end do ARP, IP
//! and everything above. A frame to a port's MAC goes to that port; a frame
//! to a group address (broadcast, multicast) floods to every other port,
//! which is how guests find each other with ordinary ARP. The service
//! programs the table on plug and unplug, so nothing is learned and an
//! unknown destination is dropped rather than flooded. A frame whose source
//! is not its port's MAC is dropped too, so every counter names the port
//! that sent the frame.
//!
//! The switch is a network, not a security boundary: whoever is plugged into
//! it can reach everyone else plugged into it.
use std::collections::HashMap;

pub type Mac = [u8; 6];

/// The two MACs and the ethertype.
pub const ETHERNET_HEADER_BYTES: usize = 14;

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
    /// Shorter than an ethernet header.
    Short,
    /// The source MAC is not the sending port's.
    SourceMac,
    /// No port owns the destination MAC, or the sender does.
    Unknown,
    /// The destination port's queue was full.
    QueueFull,
    /// The sender flooded past its broadcast cap.
    Storm,
}

impl DropReason {
    pub const ALL: [DropReason; 5] = [
        Self::Short,
        Self::SourceMac,
        Self::Unknown,
        Self::QueueFull,
        Self::Storm,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::SourceMac => "source_mac",
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

    /// Where `frame`, sent by the port plugged in for `own`, goes.
    pub fn route(&self, own: &Mac, frame: &[u8]) -> Route<'_, P> {
        if frame.len() < ETHERNET_HEADER_BYTES {
            return Route::Drop(DropReason::Short);
        }
        if frame[6..12] != own[..] {
            return Route::Drop(DropReason::SourceMac);
        }
        let destination: &Mac = frame[..6].try_into().expect("six bytes");
        if destination[0] & 1 == 1 {
            return Route::Flood;
        }
        match self.ports.get(destination) {
            Some(port) if destination != own => Route::Unicast(port),
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

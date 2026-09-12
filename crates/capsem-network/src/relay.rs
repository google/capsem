//! The datagram relay: what an owner does with a guest's tun0 packets.
//!
//! Headers only. A packet is parsed as far as the IPv4 header plus UDP ports
//! or the ICMP echo identifier, and that is all the host ever reads of it.
//! Flows are keyed by the peer and what the peer sees (its port, or the echo
//! identifier); the same engine serves both seats, because a destination's
//! replies leave its tun0 exactly like a source's requests do. Everything
//! past a bound is dropped and counted, never buffered.
use smoltcp::wire::{Icmpv4Message, Icmpv4Packet, IpProtocol, Ipv4Packet, UdpPacket};
use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

/// Flows one VM may hold at once, admitted or waiting.
pub const MAX_FLOWS: usize = 64;
/// Packets kept per flow while the service is asked; UDP is lossy and the
/// guest will resend, so a short queue only covers the ask itself.
pub const HELD_PACKETS: usize = 8;
/// Idle time after which a flow ends and a new packet asks again.
pub const UDP_IDLE: Duration = Duration::from_secs(60);
pub const ICMP_IDLE: Duration = Duration::from_secs(30);
/// How long a refusal is remembered, so a flood at a refused destination
/// does not ask the service once per packet.
pub const REFUSAL_MEMORY: Duration = Duration::from_secs(10);
/// A tun0 frame carries a `u16` length; nothing larger exists on the link.
pub const MAX_PACKET_BYTES: usize = u16::MAX as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// The peer's port: the destination port a source dials, the source
    /// port a destination answers.
    Udp {
        peer_port: u16,
    },
    Icmp {
        identifier: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlowKey {
    pub peer: Ipv4Addr,
    pub protocol: Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Udp { source_port: u16, destination_port: u16 },
    EchoRequest { identifier: u16 },
    EchoReply { identifier: u16 },
}

/// A packet's headers, all the relay knows of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Datagram {
    pub source: Ipv4Addr,
    pub destination: Ipv4Addr,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropReason {
    Oversize,
    Malformed,
    Fragment,
    Tcp,
    Unsupported,
    ForgedSource,
    WrongDestination,
    TableFull,
    HeldFull,
    Unadmitted,
    WrongFlow,
}

/// What to do with a packet the guest sent.
#[derive(Debug, PartialEq, Eq)]
pub enum Outbound {
    /// The flow is admitted: send it to the peer.
    Forward(FlowKey),
    /// A new flow: ask the service; the packet is held meanwhile.
    Ask(FlowKey),
    /// Held behind a pending ask.
    Held(FlowKey),
    Dropped(DropReason),
}

/// What a flow moved, reported when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlowReport {
    pub packets_out: u64,
    pub packets_in: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    pub forwarded: u64,
    pub delivered: u64,
    pub dropped: u64,
    pub dropped_tcp: u64,
    pub dropped_forged: u64,
}

enum State {
    Pending,
    Admitted,
    Refused(Instant),
}

struct Flow {
    state: State,
    last: Instant,
    held: VecDeque<Vec<u8>>,
    report: FlowReport,
}

impl Flow {
    fn idle(&self, key: &FlowKey) -> Duration {
        match key.protocol {
            Protocol::Udp { .. } => UDP_IDLE,
            Protocol::Icmp { .. } => ICMP_IDLE,
        }
    }
}

pub struct Relay {
    own: Ipv4Addr,
    flows: HashMap<FlowKey, Flow>,
    counters: Counters,
}

/// Parse the headers of one packet, refusing everything the relay does not
/// carry: fragments, TCP (which has its own path), ICMP other than echo.
pub fn parse(bytes: &[u8]) -> Result<Datagram, DropReason> {
    if bytes.len() > MAX_PACKET_BYTES {
        return Err(DropReason::Oversize);
    }
    let packet = Ipv4Packet::new_checked(bytes).map_err(|_| DropReason::Malformed)?;
    if packet.version() != 4 {
        return Err(DropReason::Malformed);
    }
    if packet.more_frags() || packet.frag_offset() != 0 {
        return Err(DropReason::Fragment);
    }
    let source: Ipv4Addr = packet.src_addr();
    let destination: Ipv4Addr = packet.dst_addr();
    let kind = match packet.next_header() {
        IpProtocol::Udp => {
            let udp = UdpPacket::new_checked(packet.payload()).map_err(|_| DropReason::Malformed)?;
            Kind::Udp {
                source_port: udp.src_port(),
                destination_port: udp.dst_port(),
            }
        }
        IpProtocol::Icmp => {
            let icmp = Icmpv4Packet::new_checked(packet.payload()).map_err(|_| DropReason::Malformed)?;
            match icmp.msg_type() {
                Icmpv4Message::EchoRequest => Kind::EchoRequest {
                    identifier: icmp.echo_ident(),
                },
                Icmpv4Message::EchoReply => Kind::EchoReply {
                    identifier: icmp.echo_ident(),
                },
                _ => return Err(DropReason::Unsupported),
            }
        }
        IpProtocol::Tcp => return Err(DropReason::Tcp),
        _ => return Err(DropReason::Unsupported),
    };
    Ok(Datagram {
        source,
        destination,
        kind,
    })
}

impl Relay {
    pub fn new(own: Ipv4Addr) -> Self {
        Self {
            own,
            flows: HashMap::new(),
            counters: Counters::default(),
        }
    }

    pub fn own(&self) -> Ipv4Addr {
        self.own
    }

    pub fn counters(&self) -> Counters {
        self.counters
    }

    pub fn flows(&self) -> usize {
        self.flows.len()
    }

    fn drop(&mut self, reason: DropReason) -> Outbound {
        self.counters.dropped += 1;
        match reason {
            DropReason::Tcp => self.counters.dropped_tcp += 1,
            DropReason::ForgedSource => self.counters.dropped_forged += 1,
            _ => {}
        }
        Outbound::Dropped(reason)
    }

    /// The flow a packet leaving this VM belongs to.
    fn outbound_key(&self, datagram: &Datagram) -> Result<FlowKey, DropReason> {
        if datagram.source != self.own {
            return Err(DropReason::ForgedSource);
        }
        if datagram.destination == self.own {
            return Err(DropReason::WrongDestination);
        }
        let protocol = match datagram.kind {
            Kind::Udp { destination_port, .. } => Protocol::Udp {
                peer_port: destination_port,
            },
            Kind::EchoRequest { identifier } | Kind::EchoReply { identifier } => Protocol::Icmp { identifier },
        };
        Ok(FlowKey {
            peer: datagram.destination,
            protocol,
        })
    }

    /// A packet the guest sent on tun0.
    pub fn outbound(&mut self, now: Instant, bytes: &[u8]) -> Outbound {
        let datagram = match parse(bytes) {
            Ok(datagram) => datagram,
            Err(reason) => return self.drop(reason),
        };
        let key = match self.outbound_key(&datagram) {
            Ok(key) => key,
            Err(reason) => return self.drop(reason),
        };
        if let Some(flow) = self.flows.get_mut(&key) {
            match flow.state {
                State::Admitted => {
                    flow.last = now;
                    flow.report.packets_out += 1;
                    self.counters.forwarded += 1;
                    return Outbound::Forward(key);
                }
                State::Pending => {
                    if flow.held.len() >= HELD_PACKETS {
                        return self.drop(DropReason::HeldFull);
                    }
                    flow.held.push_back(bytes.to_vec());
                    return Outbound::Held(key);
                }
                State::Refused(until) if now < until => return self.drop(DropReason::Unadmitted),
                State::Refused(_) => {
                    self.flows.remove(&key);
                }
            }
        }
        if self.flows.len() >= MAX_FLOWS {
            return self.drop(DropReason::TableFull);
        }
        let mut held = VecDeque::with_capacity(HELD_PACKETS);
        held.push_back(bytes.to_vec());
        self.flows.insert(
            key,
            Flow {
                state: State::Pending,
                last: now,
                held,
                report: FlowReport::default(),
            },
        );
        Outbound::Ask(key)
    }

    /// The service admitted the flow: what was held goes out now.
    pub fn admitted(&mut self, key: FlowKey, now: Instant) -> Vec<Vec<u8>> {
        let Some(flow) = self.flows.get_mut(&key) else {
            return Vec::new();
        };
        flow.state = State::Admitted;
        flow.last = now;
        let held: Vec<Vec<u8>> = flow.held.drain(..).collect();
        flow.report.packets_out += held.len() as u64;
        self.counters.forwarded += held.len() as u64;
        held
    }

    /// The service refused the flow: what was held is dropped, and so is
    /// everything else for it for a while.
    pub fn refused(&mut self, key: FlowKey, now: Instant) {
        if let Some(flow) = self.flows.get_mut(&key) {
            let dropped = flow.held.len() as u64;
            flow.held.clear();
            flow.state = State::Refused(now + REFUSAL_MEMORY);
            flow.last = now;
            self.counters.dropped += dropped;
        }
    }

    /// The destination seat: a flow the service admitted towards this VM.
    pub fn expect(&mut self, key: FlowKey, now: Instant) -> Result<(), DropReason> {
        if !self.flows.contains_key(&key) && self.flows.len() >= MAX_FLOWS {
            self.counters.dropped += 1;
            return Err(DropReason::TableFull);
        }
        self.flows.insert(
            key,
            Flow {
                state: State::Admitted,
                last: now,
                held: VecDeque::new(),
                report: FlowReport::default(),
            },
        );
        Ok(())
    }

    /// A frame a peer sent for `key`: only the admitted flow's packets pass,
    /// addressed from the peer to this VM.
    pub fn inbound(&mut self, now: Instant, key: FlowKey, bytes: &[u8]) -> Result<(), DropReason> {
        let verdict = parse(bytes).and_then(|datagram| {
            if datagram.source != key.peer {
                return Err(DropReason::ForgedSource);
            }
            if datagram.destination != self.own {
                return Err(DropReason::WrongDestination);
            }
            let matches = match (key.protocol, datagram.kind) {
                (Protocol::Udp { peer_port }, Kind::Udp { source_port, .. }) => source_port == peer_port,
                (Protocol::Icmp { identifier }, Kind::EchoRequest { identifier: seen })
                | (Protocol::Icmp { identifier }, Kind::EchoReply { identifier: seen }) => identifier == seen,
                _ => false,
            };
            if !matches {
                return Err(DropReason::WrongFlow);
            }
            Ok(())
        });
        let verdict = verdict.and_then(|()| match self.flows.get_mut(&key) {
            Some(flow) if matches!(flow.state, State::Admitted) => {
                flow.last = now;
                flow.report.packets_in += 1;
                Ok(())
            }
            _ => Err(DropReason::Unadmitted),
        });
        match verdict {
            Ok(()) => {
                self.counters.delivered += 1;
                Ok(())
            }
            Err(reason) => {
                self.drop(reason);
                Err(reason)
            }
        }
    }

    /// End a flow, returning what it moved.
    pub fn close(&mut self, key: &FlowKey) -> Option<FlowReport> {
        self.flows.remove(key).map(|flow| {
            self.counters.dropped += flow.held.len() as u64;
            flow.report
        })
    }

    /// Flows idle past their protocol's limit, ended; forgotten refusals go
    /// silently.
    pub fn expire(&mut self, now: Instant) -> Vec<(FlowKey, FlowReport)> {
        let mut ended = Vec::new();
        self.flows.retain(|key, flow| match flow.state {
            State::Refused(until) => now < until,
            _ => {
                if now.duration_since(flow.last) >= flow.idle(key) {
                    ended.push((*key, flow.report));
                    false
                } else {
                    true
                }
            }
        });
        ended
    }
}

#[cfg(test)]
mod tests;

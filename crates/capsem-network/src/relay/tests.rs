use super::*;
use smoltcp::wire::{Icmpv4Message, Icmpv4Packet, IpProtocol, Ipv4Packet, UdpPacket};

const A: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 2);
const B: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 3);
const STRANGER: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 9);

fn ipv4(source: Ipv4Addr, destination: Ipv4Addr, protocol: IpProtocol, payload_len: usize) -> Vec<u8> {
    let total = 20 + payload_len;
    let mut bytes = vec![0u8; total];
    let mut packet = Ipv4Packet::new_unchecked(&mut bytes);
    packet.set_version(4);
    packet.set_header_len(20);
    packet.set_dscp(0);
    packet.set_ecn(0);
    packet.set_total_len(total as u16);
    packet.set_ident(7);
    packet.clear_flags();
    packet.set_frag_offset(0);
    packet.set_hop_limit(64);
    packet.set_next_header(protocol);
    packet.set_src_addr(source);
    packet.set_dst_addr(destination);
    packet.fill_checksum();
    bytes
}

fn udp(source: Ipv4Addr, sport: u16, destination: Ipv4Addr, dport: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = ipv4(source, destination, IpProtocol::Udp, 8 + payload.len());
    let mut packet = Ipv4Packet::new_unchecked(&mut bytes);
    let mut udp = UdpPacket::new_unchecked(packet.payload_mut());
    udp.set_src_port(sport);
    udp.set_dst_port(dport);
    udp.set_len((8 + payload.len()) as u16);
    udp.set_checksum(0);
    udp.payload_mut().copy_from_slice(payload);
    bytes
}

fn echo(source: Ipv4Addr, destination: Ipv4Addr, message: Icmpv4Message, identifier: u16) -> Vec<u8> {
    let mut bytes = ipv4(source, destination, IpProtocol::Icmp, 8 + 4);
    let mut packet = Ipv4Packet::new_unchecked(&mut bytes);
    let mut icmp = Icmpv4Packet::new_unchecked(packet.payload_mut());
    icmp.set_msg_type(message);
    icmp.set_msg_code(0);
    icmp.set_echo_ident(identifier);
    icmp.set_echo_seq_no(1);
    icmp.fill_checksum();
    bytes
}

fn tcp(source: Ipv4Addr, destination: Ipv4Addr) -> Vec<u8> {
    ipv4(source, destination, IpProtocol::Tcp, 20)
}

#[test]
fn parse_reads_only_the_headers_and_names_what_it_refuses() {
    let packet = udp(A, 40000, B, 5353, b"query");
    assert_eq!(
        parse(&packet).unwrap(),
        Datagram {
            source: A,
            destination: B,
            kind: Kind::Udp {
                source_port: 40000,
                destination_port: 5353
            }
        }
    );
    let request = echo(A, B, Icmpv4Message::EchoRequest, 0x1234);
    assert_eq!(parse(&request).unwrap().kind, Kind::EchoRequest { identifier: 0x1234 });
    assert_eq!(parse(&tcp(A, B)), Err(DropReason::Tcp));
    assert_eq!(parse(&[0x45, 0x00]), Err(DropReason::Malformed));
    assert_eq!(parse(&vec![0u8; MAX_PACKET_BYTES + 1]), Err(DropReason::Oversize));
    let mut fragment = udp(A, 40000, B, 5353, b"x");
    Ipv4Packet::new_unchecked(&mut fragment).set_more_frags(true);
    assert_eq!(parse(&fragment), Err(DropReason::Fragment));
    let unreachable = echo(A, B, Icmpv4Message::DstUnreachable, 0);
    assert_eq!(parse(&unreachable), Err(DropReason::Unsupported));
    let mut v6 = udp(A, 40000, B, 5353, b"x");
    v6[0] = 0x65;
    assert!(parse(&v6).is_err());
}

#[test]
fn a_new_flow_asks_once_holds_a_few_packets_and_forwards_once_admitted() {
    let now = Instant::now();
    let mut relay = Relay::new(A);
    let key = FlowKey {
        peer: B,
        protocol: Protocol::Udp { peer_port: 5353 },
    };
    assert_eq!(relay.outbound(now, &udp(A, 40000, B, 5353, b"1")), Outbound::Ask(key));
    for n in 0..HELD_PACKETS - 1 {
        assert_eq!(
            relay.outbound(now, &udp(A, 40000, B, 5353, &[n as u8])),
            Outbound::Held(key)
        );
    }
    assert_eq!(
        relay.outbound(now, &udp(A, 40000, B, 5353, b"overflow")),
        Outbound::Dropped(DropReason::HeldFull)
    );
    let held = relay.admitted(key, now);
    assert_eq!(held.len(), HELD_PACKETS);
    assert_eq!(
        relay.outbound(now, &udp(A, 40001, B, 5353, b"more")),
        Outbound::Forward(key)
    );
    assert_eq!(relay.counters().forwarded, HELD_PACKETS as u64 + 1);
    assert_eq!(relay.counters().dropped, 1);
    assert_eq!(relay.close(&key).unwrap().packets_out, HELD_PACKETS as u64 + 1);
    assert_eq!(relay.flows(), 0);
}

#[test]
fn a_refusal_is_remembered_and_forgotten() {
    let now = Instant::now();
    let mut relay = Relay::new(A);
    let key = FlowKey {
        peer: STRANGER,
        protocol: Protocol::Udp { peer_port: 53 },
    };
    assert_eq!(
        relay.outbound(now, &udp(A, 40000, STRANGER, 53, b"1")),
        Outbound::Ask(key)
    );
    relay.refused(key, now);
    assert_eq!(
        relay.outbound(now, &udp(A, 40000, STRANGER, 53, b"2")),
        Outbound::Dropped(DropReason::Unadmitted)
    );
    assert_eq!(relay.counters().dropped, 2, "the held packet and the retry");
    let later = now + REFUSAL_MEMORY;
    assert_eq!(
        relay.outbound(later, &udp(A, 40000, STRANGER, 53, b"3")),
        Outbound::Ask(key)
    );
}

#[test]
fn forged_sources_tcp_and_the_own_address_never_leave() {
    let now = Instant::now();
    let mut relay = Relay::new(A);
    assert_eq!(
        relay.outbound(now, &udp(STRANGER, 1, B, 5353, b"x")),
        Outbound::Dropped(DropReason::ForgedSource)
    );
    assert_eq!(relay.outbound(now, &tcp(A, B)), Outbound::Dropped(DropReason::Tcp));
    assert_eq!(
        relay.outbound(now, &udp(A, 1, A, 5353, b"x")),
        Outbound::Dropped(DropReason::WrongDestination)
    );
    let counters = relay.counters();
    assert_eq!(
        (counters.dropped, counters.dropped_forged, counters.dropped_tcp),
        (3, 1, 1)
    );
    assert_eq!(relay.flows(), 0);
}

#[test]
fn the_flow_table_is_bounded_under_a_flood() {
    let now = Instant::now();
    let mut relay = Relay::new(A);
    for port in 0..MAX_FLOWS as u16 {
        assert!(matches!(
            relay.outbound(now, &udp(A, 40000, B, 1000 + port, b"x")),
            Outbound::Ask(_)
        ));
    }
    assert_eq!(
        relay.outbound(now, &udp(A, 40000, B, 9999, b"x")),
        Outbound::Dropped(DropReason::TableFull)
    );
    let key = FlowKey {
        peer: STRANGER,
        protocol: Protocol::Icmp { identifier: 1 },
    };
    assert_eq!(relay.expect(key, now), Err(DropReason::TableFull));
}

#[test]
fn the_destination_seat_delivers_only_the_admitted_peer_and_answers_on_the_same_flow() {
    let now = Instant::now();
    let mut relay = Relay::new(B);
    let key = FlowKey {
        peer: A,
        protocol: Protocol::Udp { peer_port: 40000 },
    };
    assert_eq!(
        relay.inbound(now, key, &udp(A, 40000, B, 5353, b"early")),
        Err(DropReason::Unadmitted)
    );
    relay.expect(key, now).unwrap();
    relay.inbound(now, key, &udp(A, 40000, B, 5353, b"query")).unwrap();
    assert_eq!(
        relay.inbound(now, key, &udp(STRANGER, 40000, B, 5353, b"forged")),
        Err(DropReason::ForgedSource)
    );
    assert_eq!(
        relay.inbound(now, key, &udp(A, 40001, B, 5353, b"other socket")),
        Err(DropReason::WrongFlow)
    );
    assert_eq!(
        relay.inbound(now, key, &udp(A, 40000, STRANGER, 5353, b"not for me")),
        Err(DropReason::WrongDestination)
    );
    assert_eq!(relay.inbound(now, key, &tcp(A, B)), Err(DropReason::Tcp));
    // The guest's answer leaves on the same flow, no ask.
    assert_eq!(
        relay.outbound(now, &udp(B, 5353, A, 40000, b"answer")),
        Outbound::Forward(key)
    );
    let report = relay.close(&key).unwrap();
    assert_eq!((report.packets_in, report.packets_out), (1, 1));
    assert_eq!(relay.counters().delivered, 1);
    assert_eq!(relay.counters().dropped, 5);
}

#[test]
fn echo_is_one_flow_per_identifier_in_both_directions() {
    let now = Instant::now();
    let mut source = Relay::new(A);
    let mut destination = Relay::new(B);
    let key_at_source = FlowKey {
        peer: B,
        protocol: Protocol::Icmp { identifier: 0x4242 },
    };
    let key_at_destination = FlowKey {
        peer: A,
        protocol: Protocol::Icmp { identifier: 0x4242 },
    };
    let request = echo(A, B, Icmpv4Message::EchoRequest, 0x4242);
    assert_eq!(source.outbound(now, &request), Outbound::Ask(key_at_source));
    destination.expect(key_at_destination, now).unwrap();
    destination.inbound(now, key_at_destination, &request).unwrap();
    let reply = echo(B, A, Icmpv4Message::EchoReply, 0x4242);
    assert_eq!(destination.outbound(now, &reply), Outbound::Forward(key_at_destination));
    source.admitted(key_at_source, now);
    source.inbound(now, key_at_source, &reply).unwrap();
    let other = echo(B, A, Icmpv4Message::EchoReply, 0x4343);
    assert_eq!(source.inbound(now, key_at_source, &other), Err(DropReason::WrongFlow));
}

#[test]
fn idle_flows_end_with_their_counts_and_refusals_fade() {
    let now = Instant::now();
    let mut relay = Relay::new(A);
    let udp_key = FlowKey {
        peer: B,
        protocol: Protocol::Udp { peer_port: 5353 },
    };
    let icmp_key = FlowKey {
        peer: B,
        protocol: Protocol::Icmp { identifier: 1 },
    };
    relay.outbound(now, &udp(A, 40000, B, 5353, b"x"));
    relay.admitted(udp_key, now);
    relay.outbound(now, &echo(A, B, Icmpv4Message::EchoRequest, 1));
    relay.admitted(icmp_key, now);
    let refused = FlowKey {
        peer: STRANGER,
        protocol: Protocol::Udp { peer_port: 1 },
    };
    relay.outbound(now, &udp(A, 1, STRANGER, 1, b"x"));
    relay.refused(refused, now);
    assert_eq!(relay.flows(), 3);
    assert!(relay.expire(now + ICMP_IDLE - Duration::from_secs(1)).is_empty());
    let ended = relay.expire(now + ICMP_IDLE);
    assert_eq!(
        ended,
        vec![(
            icmp_key,
            FlowReport {
                packets_out: 1,
                packets_in: 0
            }
        )]
    );
    assert_eq!(relay.flows(), 1, "the refusal faded with it");
    let ended = relay.expire(now + UDP_IDLE);
    assert_eq!(ended.len(), 1);
    assert_eq!(relay.flows(), 0);
}

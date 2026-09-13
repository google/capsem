use super::*;

const A: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 2);
const B: Ipv4Addr = Ipv4Addr::new(10, 129, 7, 200);
const STRANGER: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 9);
const BROADCAST: [u8; 6] = [0xff; 6];
const UDP: u8 = 17;
const ICMP: u8 = 1;
const TCP: u8 = 6;

fn member(address: Ipv4Addr) -> bool {
    address == A || address == B
}

fn ethernet(destination: [u8; 6], source: [u8; 6], ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(ETHERNET_HEADER_BYTES + payload.len());
    frame.extend_from_slice(&destination);
    frame.extend_from_slice(&source);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn ipv4(source: Ipv4Addr, destination: Ipv4Addr, protocol: u8, payload: &[u8]) -> Vec<u8> {
    let mut packet = vec![0u8; 20];
    packet[0] = 0x45;
    let total = (20 + payload.len()) as u16;
    packet[2..4].copy_from_slice(&total.to_be_bytes());
    packet[8] = 64;
    packet[9] = protocol;
    packet[12..16].copy_from_slice(&source.octets());
    packet[16..20].copy_from_slice(&destination.octets());
    packet.extend_from_slice(payload);
    packet
}

fn from_a_to_b(protocol: u8, payload: &[u8]) -> Vec<u8> {
    ethernet(mac_of(B), mac_of(A), ETHERTYPE_IPV4, &ipv4(A, B, protocol, payload))
}

fn arp(operation: u16, sender: ([u8; 6], Ipv4Addr), target: ([u8; 6], Ipv4Addr)) -> Vec<u8> {
    let mut body = Vec::with_capacity(28);
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
    body.push(6);
    body.push(4);
    body.extend_from_slice(&operation.to_be_bytes());
    body.extend_from_slice(&sender.0);
    body.extend_from_slice(&sender.1.octets());
    body.extend_from_slice(&target.0);
    body.extend_from_slice(&target.1.octets());
    body
}

#[test]
fn a_members_datagram_to_another_member_is_forwarded() {
    let frame = from_a_to_b(UDP, &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(classify(A, member, &frame), Verdict::Forward(B));
    // Echo, and the errors path MTU discovery and traceroute depend on.
    for icmp_type in [0u8, 3, 8, 11] {
        let frame = from_a_to_b(ICMP, &[icmp_type, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            classify(A, member, &frame),
            Verdict::Forward(B),
            "icmp type {icmp_type}"
        );
    }
}

#[test]
fn only_udp_and_icmp_cross_the_link() {
    // TCP has its own admitted path; a tunnel (IPIP, 6in4, GRE, ESP) would
    // carry TCP past that admission; SCTP and the rest are unaudited streams.
    for protocol in [0u8, 2, 4, TCP, 33, 41, 47, 50, 51, 132, 255] {
        assert_eq!(
            classify(A, member, &from_a_to_b(protocol, &[8, 0, 0, 0, 0, 0, 0, 0])),
            Verdict::Drop(DropReason::Protocol),
            "protocol {protocol}"
        );
    }
}

#[test]
fn icmp_crosses_only_as_echo_or_a_delivery_error() {
    // Redirect, router advertisement, timestamp, address mask and the rest.
    for icmp_type in [1u8, 4, 5, 9, 10, 13, 17, 255] {
        assert_eq!(
            classify(A, member, &from_a_to_b(ICMP, &[icmp_type, 0, 0, 0, 0, 0, 0, 0])),
            Verdict::Drop(DropReason::IcmpType),
            "icmp type {icmp_type}"
        );
    }
    let typeless = from_a_to_b(ICMP, &[]);
    assert_eq!(classify(A, member, &typeless), Verdict::Drop(DropReason::Short));
    // A later fragment carries no type and cannot rewrite the first one's:
    // its offset is at least eight bytes past the type.
    let mut later = ipv4(A, B, ICMP, &[5; 64]);
    later[6..8].copy_from_slice(&1u16.to_be_bytes());
    let frame = ethernet(mac_of(B), mac_of(A), ETHERTYPE_IPV4, &later);
    assert_eq!(classify(A, member, &frame), Verdict::Forward(B));
    // The type is read past the options, not at byte twenty.
    let mut options = ipv4(A, B, ICMP, &[0, 0, 0, 0, 5, 0, 0, 0]);
    options[0] = 0x46;
    let frame = ethernet(mac_of(B), mac_of(A), ETHERTYPE_IPV4, &options);
    assert_eq!(classify(A, member, &frame), Verdict::Drop(DropReason::IcmpType));
}

#[test]
fn fragments_are_forwarded_untouched() {
    let mut packet = ipv4(A, B, UDP, &[0; 1400]);
    packet[6..8].copy_from_slice(&(0x2000u16 | 100).to_be_bytes()); // MF, offset 800
    let frame = ethernet(mac_of(B), mac_of(A), ETHERTYPE_IPV4, &packet);
    assert_eq!(classify(A, member, &frame), Verdict::Forward(B));
}

#[test]
fn a_forged_source_is_dropped_before_any_lookup() {
    let wrong_mac = ethernet(mac_of(B), mac_of(STRANGER), ETHERTYPE_IPV4, &ipv4(A, B, UDP, &[0; 8]));
    assert_eq!(classify(A, member, &wrong_mac), Verdict::Drop(DropReason::SourceMac));
    let wrong_address = ethernet(mac_of(B), mac_of(A), ETHERTYPE_IPV4, &ipv4(B, A, UDP, &[0; 8]));
    assert_eq!(
        classify(A, member, &wrong_address),
        Verdict::Drop(DropReason::SourceAddress)
    );
}

#[test]
fn only_a_linked_member_can_be_reached_and_never_oneself() {
    let stranger = ethernet(
        mac_of(STRANGER),
        mac_of(A),
        ETHERTYPE_IPV4,
        &ipv4(A, STRANGER, UDP, &[0; 8]),
    );
    assert_eq!(classify(A, member, &stranger), Verdict::Drop(DropReason::Unknown));
    let hairpin = ethernet(mac_of(A), mac_of(A), ETHERTYPE_IPV4, &ipv4(A, A, UDP, &[0; 8]));
    assert_eq!(classify(A, member, &hairpin), Verdict::Drop(DropReason::Unknown));
}

#[test]
fn the_destination_mac_must_name_the_destination_address() {
    let mismatch = ethernet(mac_of(STRANGER), mac_of(A), ETHERTYPE_IPV4, &ipv4(A, B, UDP, &[0; 8]));
    assert_eq!(
        classify(A, member, &mismatch),
        Verdict::Drop(DropReason::DestinationMac)
    );
    let broadcast = ethernet(BROADCAST, mac_of(A), ETHERTYPE_IPV4, &ipv4(A, B, UDP, &[0; 8]));
    assert_eq!(
        classify(A, member, &broadcast),
        Verdict::Drop(DropReason::DestinationMac)
    );
}

#[test]
fn other_ethertypes_short_and_malformed_frames_are_dropped() {
    let ipv6 = ethernet(mac_of(B), mac_of(A), 0x86dd, &[0x60; 40]);
    assert_eq!(classify(A, member, &ipv6), Verdict::Drop(DropReason::EtherType));
    assert_eq!(classify(A, member, &[0; 13]), Verdict::Drop(DropReason::Short));
    let headerless = ethernet(mac_of(B), mac_of(A), ETHERTYPE_IPV4, &[0x45; 19]);
    assert_eq!(classify(A, member, &headerless), Verdict::Drop(DropReason::Short));
    let mut wrong_version = ipv4(A, B, UDP, &[0; 8]);
    wrong_version[0] = 0x65;
    let frame = ethernet(mac_of(B), mac_of(A), ETHERTYPE_IPV4, &wrong_version);
    assert_eq!(classify(A, member, &frame), Verdict::Drop(DropReason::Header));
    let mut short_ihl = ipv4(A, B, UDP, &[0; 8]);
    short_ihl[0] = 0x44;
    let frame = ethernet(mac_of(B), mac_of(A), ETHERTYPE_IPV4, &short_ihl);
    assert_eq!(classify(A, member, &frame), Verdict::Drop(DropReason::Header));
    let mut overlong = ipv4(A, B, UDP, &[0; 8]);
    overlong[2..4].copy_from_slice(&500u16.to_be_bytes());
    let frame = ethernet(mac_of(B), mac_of(A), ETHERTYPE_IPV4, &overlong);
    assert_eq!(classify(A, member, &frame), Verdict::Drop(DropReason::Header));
}

#[test]
fn a_padded_frame_is_forwarded() {
    let mut frame = from_a_to_b(UDP, &[0; 8]);
    frame.extend_from_slice(&[0; 18]);
    assert_eq!(classify(A, member, &frame), Verdict::Forward(B));
}

#[test]
fn the_switch_answers_arp_for_a_member_itself() {
    let request = ethernet(
        BROADCAST,
        mac_of(A),
        ETHERTYPE_ARP,
        &arp(1, (mac_of(A), A), ([0; 6], B)),
    );
    let expected = ethernet(
        mac_of(A),
        mac_of(B),
        ETHERTYPE_ARP,
        &arp(2, (mac_of(B), B), (mac_of(A), A)),
    );
    assert_eq!(classify(A, member, &request), Verdict::Reply(expected));
    // A unicast re-validation request gets the same answer.
    let unicast = ethernet(
        mac_of(B),
        mac_of(A),
        ETHERTYPE_ARP,
        &arp(1, (mac_of(A), A), (mac_of(B), B)),
    );
    assert!(matches!(classify(A, member, &unicast), Verdict::Reply(_)));
}

#[test]
fn arp_for_a_stranger_from_a_forger_or_not_a_request_is_dropped() {
    let stranger = ethernet(
        BROADCAST,
        mac_of(A),
        ETHERTYPE_ARP,
        &arp(1, (mac_of(A), A), ([0; 6], STRANGER)),
    );
    assert_eq!(classify(A, member, &stranger), Verdict::Drop(DropReason::Arp));
    let forged = ethernet(
        BROADCAST,
        mac_of(A),
        ETHERTYPE_ARP,
        &arp(1, (mac_of(A), B), ([0; 6], B)),
    );
    assert_eq!(classify(A, member, &forged), Verdict::Drop(DropReason::Arp));
    let reply = ethernet(
        mac_of(B),
        mac_of(A),
        ETHERTYPE_ARP,
        &arp(2, (mac_of(A), A), (mac_of(B), B)),
    );
    assert_eq!(classify(A, member, &reply), Verdict::Drop(DropReason::Arp));
    let short = ethernet(BROADCAST, mac_of(A), ETHERTYPE_ARP, &[0; 27]);
    assert_eq!(classify(A, member, &short), Verdict::Drop(DropReason::Arp));
}

#[test]
fn every_drop_reason_has_a_name_and_a_slot() {
    let names: std::collections::HashSet<&str> = DropReason::ALL.iter().map(|reason| reason.name()).collect();
    assert_eq!(names.len(), DropReason::ALL.len());
    for (slot, reason) in DropReason::ALL.iter().enumerate() {
        assert_eq!(*reason as usize, slot);
    }
}

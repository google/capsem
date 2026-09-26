use super::*;
use capsem_proto::privatelink::mac_of;
use std::net::Ipv4Addr;

const A: Station = station(Ipv4Addr::new(10, 128, 0, 2));
const B: Station = station(Ipv4Addr::new(10, 128, 0, 3));
const C: Station = station(Ipv4Addr::new(10, 128, 0, 4));
const STRANGER: Station = station(Ipv4Addr::new(10, 128, 0, 9));
const BROADCAST: Mac = [0xff; 6];
const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_ARP: u16 = 0x0806;

const fn station(address: Ipv4Addr) -> Station {
    Station {
        mac: mac_of(address),
        address: address.octets(),
    }
}

fn ethernet(destination: Mac, source: Mac, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + payload.len());
    frame.extend_from_slice(&destination);
    frame.extend_from_slice(&source);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// A 20-byte IPv4 header for `protocol` from `source` to `destination`.
fn ipv4(source: [u8; 4], destination: [u8; 4], protocol: u8) -> Vec<u8> {
    let mut packet = vec![0x45, 0, 0, 20, 0, 0, 0, 0, 64, protocol, 0, 0];
    packet.extend_from_slice(&source);
    packet.extend_from_slice(&destination);
    packet
}

/// An ARP request over ethernet for `target`, sent by `sender`.
fn arp(sender: &Station, sender_address: [u8; 4], target: [u8; 4]) -> Vec<u8> {
    let mut packet = vec![0, 1, 8, 0, 6, 4, 0, 1];
    packet.extend_from_slice(&sender.mac);
    packet.extend_from_slice(&sender_address);
    packet.extend_from_slice(&[0; 6]);
    packet.extend_from_slice(&target);
    packet
}

/// A switch with A, B and C plugged into ports named after them.
fn switch() -> Table<&'static str> {
    let mut table = Table::default();
    table.plug(A.mac, "a");
    table.plug(B.mac, "b");
    table.plug(C.mac, "c");
    table
}

#[test]
fn a_tcp_segment_goes_to_the_port_owning_its_destination_mac() {
    let mut segment = ipv4(A.address, B.address, 6);
    segment.resize(40, 0);
    let frame = ethernet(B.mac, A.mac, ETHERTYPE_IPV4, &segment);
    assert_eq!(switch().route(&A, &frame), Route::Unicast(&"b"));
}

#[test]
fn every_ethertype_crosses() {
    for (ethertype, payload) in [
        (ETHERTYPE_IPV4, ipv4(B.address, C.address, 47)),
        (ETHERTYPE_ARP, arp(&B, B.address, C.address)),
        (0x86dd, vec![7; 40]),
        (0x88cc, vec![7; 32]),
        (0x0000, vec![]),
    ] {
        let frame = ethernet(C.mac, B.mac, ethertype, &payload);
        assert_eq!(
            switch().route(&B, &frame),
            Route::Unicast(&"c"),
            "ethertype {ethertype:#06x}"
        );
    }
}

#[test]
fn a_frame_with_only_a_header_still_crosses() {
    let frame = ethernet(B.mac, A.mac, 0x88b5, &[]);
    assert_eq!(switch().route(&A, &frame), Route::Unicast(&"b"));
}

#[test]
fn an_arp_request_to_broadcast_floods() {
    let frame = ethernet(BROADCAST, A.mac, ETHERTYPE_ARP, &arp(&A, A.address, B.address));
    assert_eq!(switch().route(&A, &frame), Route::Flood);
}

#[test]
fn any_group_address_floods() {
    // The group bit is the low bit of the first octet: IPv4 multicast and
    // anything else addressed to a group, not just all-ones.
    for destination in [
        [0x01, 0x00, 0x5e, 0, 0, 1],
        [0x33, 0x33, 0, 0, 0, 1],
        [0x03, 0, 0, 0, 0, 0],
    ] {
        let frame = ethernet(destination, B.mac, ETHERTYPE_IPV4, &ipv4(B.address, [224, 0, 0, 1], 2));
        assert_eq!(switch().route(&B, &frame), Route::Flood, "{destination:02x?}");
    }
}

#[test]
fn a_flood_reaches_every_port_but_the_sender() {
    let table = switch();
    let mut reached: Vec<_> = table.others(&B.mac).copied().collect();
    reached.sort_unstable();
    assert_eq!(reached, ["a", "c"]);
}

#[test]
fn an_unknown_destination_is_dropped_not_flooded() {
    let frame = ethernet(
        STRANGER.mac,
        A.mac,
        ETHERTYPE_IPV4,
        &ipv4(A.address, STRANGER.address, 17),
    );
    assert_eq!(switch().route(&A, &frame), Route::Drop(DropReason::Unknown));
}

#[test]
fn a_frame_back_to_its_own_port_is_dropped() {
    let frame = ethernet(A.mac, A.mac, ETHERTYPE_IPV4, &ipv4(A.address, A.address, 17));
    assert_eq!(switch().route(&A, &frame), Route::Drop(DropReason::Unknown));
}

#[test]
fn a_frame_whose_source_is_not_its_ports_mac_is_dropped() {
    // Unicast and broadcast alike: a forged source never floods either.
    for destination in [B.mac, BROADCAST] {
        for source in [B.mac, STRANGER.mac, [0; 6]] {
            let frame = ethernet(destination, source, ETHERTYPE_ARP, &arp(&A, A.address, B.address));
            assert_eq!(
                switch().route(&A, &frame),
                Route::Drop(DropReason::SourceMac),
                "{source:02x?}"
            );
        }
    }
}

#[test]
fn an_ipv4_packet_from_another_members_address_is_dropped() {
    // The port's own MAC, someone else's address: to a member or to all.
    for destination in [B.mac, BROADCAST] {
        for forged in [B.address, STRANGER.address, [0; 4], [255; 4]] {
            let frame = ethernet(destination, A.mac, ETHERTYPE_IPV4, &ipv4(forged, B.address, 6));
            assert_eq!(
                switch().route(&A, &frame),
                Route::Drop(DropReason::SourceAddress),
                "{forged:?}"
            );
        }
    }
}

#[test]
fn an_arp_message_claiming_another_members_address_is_dropped() {
    // A gratuitous reply for B's address from A would steal B's traffic.
    for destination in [C.mac, BROADCAST] {
        for claimed in [B.address, STRANGER.address, [0; 4]] {
            let frame = ethernet(destination, A.mac, ETHERTYPE_ARP, &arp(&A, claimed, C.address));
            assert_eq!(
                switch().route(&A, &frame),
                Route::Drop(DropReason::SourceAddress),
                "{claimed:?}"
            );
        }
    }
}

#[test]
fn an_ipv4_or_arp_frame_too_short_to_name_its_sender_is_dropped() {
    // The sender field ends 16 bytes into an IPv4 header and 18 into ARP.
    for (ethertype, payload, named) in [
        (ETHERTYPE_IPV4, ipv4(A.address, B.address, 6), 16),
        (ETHERTYPE_ARP, arp(&A, A.address, B.address), 18),
    ] {
        let frame = ethernet(B.mac, A.mac, ethertype, &payload);
        let named = ETHERNET_HEADER_BYTES + named;
        assert_eq!(switch().route(&A, &frame[..named]), Route::Unicast(&"b"));
        for length in ETHERNET_HEADER_BYTES..named {
            assert_eq!(
                switch().route(&A, &frame[..length]),
                Route::Drop(DropReason::Short),
                "ethertype {ethertype:#06x}, {length} bytes"
            );
        }
    }
}

#[test]
fn a_frame_shorter_than_an_ethernet_header_is_dropped() {
    let frame = ethernet(B.mac, A.mac, 0x88b5, &[]);
    for length in 0..frame.len() {
        assert_eq!(
            switch().route(&A, &frame[..length]),
            Route::Drop(DropReason::Short),
            "{length} bytes"
        );
    }
}

#[test]
fn plugging_a_mac_again_replaces_its_port() {
    let mut table = switch();
    assert_eq!(table.plug(B.mac, "b2"), Some("b"));
    let frame = ethernet(B.mac, A.mac, ETHERTYPE_IPV4, &ipv4(A.address, B.address, 17));
    assert_eq!(table.route(&A, &frame), Route::Unicast(&"b2"));
    assert_eq!(table.len(), 3);
}

#[test]
fn an_unplugged_mac_is_unknown() {
    let mut table = switch();
    assert_eq!(table.unplug(&B.mac), Some("b"));
    assert_eq!(table.unplug(&B.mac), None);
    let frame = ethernet(B.mac, A.mac, ETHERTYPE_IPV4, &ipv4(A.address, B.address, 17));
    assert_eq!(table.route(&A, &frame), Route::Drop(DropReason::Unknown));
    assert_eq!(table.others(&A.mac).count(), 1);
    assert_eq!(table.len(), 2);
}

#[test]
fn drop_reasons_index_their_own_counter_slot() {
    for (slot, reason) in DropReason::ALL.iter().enumerate() {
        assert_eq!(*reason as usize, slot);
    }
    let mut names: Vec<_> = DropReason::ALL.iter().map(|reason| reason.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), DropReason::ALL.len());
}

#[test]
fn a_table_is_empty_until_a_port_is_plugged_and_after_the_last_leaves() {
    let mut table = Table::default();
    assert!(table.is_empty());
    table.plug(A.mac, "a");
    assert!(!table.is_empty());
    table.unplug(&A.mac);
    assert!(table.is_empty());
}

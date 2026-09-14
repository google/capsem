use super::*;
use capsem_proto::privatelink::mac_of;
use std::net::Ipv4Addr;

const A: Mac = mac_of(Ipv4Addr::new(10, 128, 0, 2));
const B: Mac = mac_of(Ipv4Addr::new(10, 128, 0, 3));
const C: Mac = mac_of(Ipv4Addr::new(10, 128, 0, 4));
const STRANGER: Mac = mac_of(Ipv4Addr::new(10, 128, 0, 9));
const BROADCAST: Mac = [0xff; 6];
const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_ARP: u16 = 0x0806;

fn ethernet(destination: Mac, source: Mac, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + payload.len());
    frame.extend_from_slice(&destination);
    frame.extend_from_slice(&source);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// A switch with A, B and C plugged into ports named after them.
fn switch() -> Table<&'static str> {
    let mut table = Table::default();
    table.plug(A, "a");
    table.plug(B, "b");
    table.plug(C, "c");
    table
}

#[test]
fn a_tcp_segment_goes_to_the_port_owning_its_destination_mac() {
    // An IPv4 header with protocol 6: the switch never looks past the MACs.
    let mut packet = vec![0x45, 0, 0, 40, 0, 0, 0, 0, 64, 6];
    packet.resize(40, 0);
    let frame = ethernet(B, A, ETHERTYPE_IPV4, &packet);
    assert_eq!(switch().route(&A, &frame), Route::Unicast(&"b"));
}

#[test]
fn every_ethertype_crosses() {
    for ethertype in [ETHERTYPE_IPV4, ETHERTYPE_ARP, 0x86dd, 0x88cc, 0x0000] {
        let frame = ethernet(C, B, ethertype, &[7; 32]);
        assert_eq!(
            switch().route(&B, &frame),
            Route::Unicast(&"c"),
            "ethertype {ethertype:#06x}"
        );
    }
}

#[test]
fn a_frame_with_only_a_header_still_crosses() {
    let frame = ethernet(B, A, ETHERTYPE_IPV4, &[]);
    assert_eq!(switch().route(&A, &frame), Route::Unicast(&"b"));
}

#[test]
fn an_arp_request_to_broadcast_floods() {
    let frame = ethernet(BROADCAST, A, ETHERTYPE_ARP, &[0; 28]);
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
        let frame = ethernet(destination, B, ETHERTYPE_IPV4, &[0; 20]);
        assert_eq!(switch().route(&B, &frame), Route::Flood, "{destination:02x?}");
    }
}

#[test]
fn a_flood_reaches_every_port_but_the_sender() {
    let table = switch();
    let mut reached: Vec<_> = table.others(&B).copied().collect();
    reached.sort_unstable();
    assert_eq!(reached, ["a", "c"]);
}

#[test]
fn an_unknown_destination_is_dropped_not_flooded() {
    let frame = ethernet(STRANGER, A, ETHERTYPE_IPV4, &[0; 20]);
    assert_eq!(switch().route(&A, &frame), Route::Drop(DropReason::Unknown));
}

#[test]
fn a_frame_back_to_its_own_port_is_dropped() {
    let frame = ethernet(A, A, ETHERTYPE_IPV4, &[0; 20]);
    assert_eq!(switch().route(&A, &frame), Route::Drop(DropReason::Unknown));
}

#[test]
fn a_frame_whose_source_is_not_its_ports_mac_is_dropped() {
    // Unicast and broadcast alike: a forged source never floods either.
    for destination in [B, BROADCAST] {
        for source in [B, STRANGER, [0; 6]] {
            let frame = ethernet(destination, source, ETHERTYPE_ARP, &[0; 28]);
            assert_eq!(
                switch().route(&A, &frame),
                Route::Drop(DropReason::SourceMac),
                "{source:02x?}"
            );
        }
    }
}

#[test]
fn a_frame_shorter_than_an_ethernet_header_is_dropped() {
    let frame = ethernet(B, A, ETHERTYPE_IPV4, &[]);
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
    assert_eq!(table.plug(B, "b2"), Some("b"));
    let frame = ethernet(B, A, ETHERTYPE_IPV4, &[0; 20]);
    assert_eq!(table.route(&A, &frame), Route::Unicast(&"b2"));
    assert_eq!(table.len(), 3);
}

#[test]
fn an_unplugged_mac_is_unknown() {
    let mut table = switch();
    assert_eq!(table.unplug(&B), Some("b"));
    assert_eq!(table.unplug(&B), None);
    let frame = ethernet(B, A, ETHERTYPE_IPV4, &[0; 20]);
    assert_eq!(table.route(&A, &frame), Route::Drop(DropReason::Unknown));
    assert_eq!(table.others(&A).count(), 1);
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

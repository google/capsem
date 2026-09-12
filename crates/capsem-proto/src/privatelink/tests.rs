use super::*;

#[test]
fn header_round_trips_and_is_ten_bytes() {
    let header = ConnectHeader {
        destination: Ipv4Addr::new(10, 129, 7, 200),
        port: 6379,
        source_port: 40001,
    };
    let bytes = header.encode();
    assert_eq!(bytes.len(), HEADER_BYTES);
    assert_eq!(bytes, [2, 6, 10, 129, 7, 200, 0x18, 0xeb, 0x9c, 0x41]);
    assert_eq!(ConnectHeader::decode(&bytes).unwrap(), header);
}

#[test]
fn a_wrong_version_protocol_or_port_is_refused() {
    let good = ConnectHeader {
        destination: Ipv4Addr::new(10, 128, 0, 2),
        port: 80,
        source_port: 40001,
    }
    .encode();
    assert!(ConnectHeader::decode(&good).is_ok());
    for (index, value, what) in [
        (0, 1, "version"),
        (1, 17, "protocol"),
        (6, 0, "port"),
        (8, 0, "source port"),
    ] {
        let mut bad = good;
        bad[index] = value;
        if what == "port" {
            bad[7] = 0;
        }
        if what == "source port" {
            bad[9] = 0;
        }
        assert!(ConnectHeader::decode(&bad).is_err(), "{what}: {bad:?}");
    }
}

#[test]
fn a_member_mac_is_locally_administered_unicast_and_names_its_address() {
    let mac = mac_of(Ipv4Addr::new(10, 129, 7, 200));
    assert_eq!(mac, [0x02, 0xca, 10, 129, 7, 200]);
    assert_eq!(mac[0] & 0x01, 0, "unicast");
    assert_eq!(mac[0] & 0x02, 0x02, "locally administered");
    assert_ne!(mac_of(Ipv4Addr::new(10, 129, 7, 201)), mac);
}

#[test]
fn the_link_mtu_fills_a_u16_frame_with_its_ethernet_header() {
    assert_eq!(LINK_MTU + ETHERNET_HEADER_BYTES, usize::from(u16::MAX));
}

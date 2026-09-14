use super::*;

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

#[test]
fn a_seat_frame_names_its_kind_and_token_and_refuses_others() {
    let frame = seat_frame(SEAT_LINK, 0x00ff_00ff_00ff_00ff);
    assert_eq!(frame, [1, 5, 0, 0xff, 0, 0xff, 0, 0xff, 0, 0xff]);
    assert_eq!(decode_seat_frame(&frame).unwrap(), (SEAT_LINK, 0x00ff_00ff_00ff_00ff));
    assert!(
        decode_seat_frame(&seat_frame(4, 7)).is_err(),
        "the retired private TCP handoff kind is not a seat frame"
    );
    assert!(
        decode_seat_frame(&seat_frame(1, 7)).is_err(),
        "a router grant is not a seat frame"
    );
    let mut wrong_version = frame;
    wrong_version[0] = 3;
    assert!(decode_seat_frame(&wrong_version).is_err());
}

#[test]
fn a_cable_opens_with_its_id_and_id_zero_is_no_cable() {
    assert_eq!(cable_header(7), [0, 0, 0, 7]);
    assert_eq!(decode_cable_header(&cable_header(7)), Ok(7));
    assert_eq!(decode_cable_header(&cable_header(u32::MAX)), Ok(u32::MAX));
    assert!(decode_cable_header(&[0, 0, 0, 0]).is_err());
    assert_eq!(CABLE_HEADER_BYTES, 4);
}

#[test]
fn a_cables_guest_device_is_named_after_it_within_the_interface_name_limit() {
    assert_eq!(cable_device(3), "cable3");
    // IFNAMSIZ is 16 including the terminator.
    assert!(cable_device(u32::MAX).len() < 16);
}

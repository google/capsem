use super::*;

#[test]
fn an_echo_request_has_a_valid_checksum_and_its_identity() {
    let request = echo_request(0x1234, 5, 56);
    assert_eq!(request.len(), 8 + 56);
    assert_eq!(request[0], 8, "type: echo request");
    assert_eq!(request[1], 0);
    assert_eq!(checksum(&request), 0, "a packet with its checksum sums to zero");
    assert_eq!(&request[4..6], &0x1234u16.to_be_bytes());
    assert_eq!(&request[6..8], &5u16.to_be_bytes());
}

#[test]
fn the_reply_is_found_behind_its_ip_header_and_only_for_our_identifier() {
    let mut reply = echo_request(0x1234, 9, 16);
    reply[0] = 0;
    reply[2..4].copy_from_slice(&[0, 0]);
    let sum = checksum(&reply);
    reply[2..4].copy_from_slice(&sum.to_be_bytes());
    let mut packet = vec![0u8; 20];
    packet[0] = 0x45;
    packet.extend_from_slice(&reply);
    assert_eq!(echo_reply(&packet, 0x1234), Some(9));
    assert_eq!(echo_reply(&packet, 0x4321), None, "someone else's echo");
    assert_eq!(echo_reply(&reply, 0x1234), Some(9), "without an IP header too");
    let mut request_not_reply = packet.clone();
    request_not_reply[20] = 8;
    assert_eq!(echo_reply(&request_not_reply, 0x1234), None);
    assert_eq!(echo_reply(&packet[..25], 0x1234), None, "truncated");
}

#[test]
fn the_rfc_1071_checksum_matches_a_known_vector() {
    // From RFC 1071 section 3: 00 01 f2 03 f4 f5 f6 f7 sums to 0xddf2 complemented.
    assert_eq!(checksum(&[0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7]), 0x220d);
    assert_eq!(checksum(&[0x00, 0x01, 0xf2]), !0xf201u16, "odd length pads with zero");
}

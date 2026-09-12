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

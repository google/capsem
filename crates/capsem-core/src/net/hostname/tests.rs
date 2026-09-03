use super::*;

#[test]
fn normalize_host_lowercases_and_strips_root_dots() {
    assert_eq!(normalize_host("Example.COM."), "example.com");
    assert_eq!(normalize_host("api.evil.com.."), "api.evil.com");
    assert_eq!(normalize_host("..."), "");
    assert_eq!(normalize_host(""), "");
}

#[test]
fn every_resolver_spelling_of_loopback_is_one_host() {
    for spelling in [
        "127.0.0.1",
        "127.0.0.1.",
        "127.1",
        "127.0.1",
        "0x7f000001",
        "0X7F000001",
        "0x7f.0.0.1",
        "0x7f.1",
        "0177.0.0.1",
        "0177.0.0.01",
        "2130706433",
        "0x7f.0x0.0x0.0x1",
        "0177.0.0.0x1",
    ] {
        assert_eq!(normalize_host(spelling), "127.0.0.1", "{spelling:?}");
    }
    assert_eq!(
        normalize_host("0xa9fea9fe"),
        "169.254.169.254",
        "the metadata endpoint in hex"
    );
    assert_eq!(normalize_host("1"), "0.0.0.1");
    assert_eq!(normalize_host("256"), "0.0.1.0", "a single part spans all four bytes");
    assert_eq!(normalize_host("0"), "0.0.0.0");
    assert_eq!(normalize_host("4294967295"), "255.255.255.255");
}

#[test]
fn strings_the_resolver_would_not_read_as_an_address_stay_names() {
    for spelling in [
        "example.com",
        "1e3",
        "0x",
        "0x7g000001",
        "300.1.1.1",
        "1.2.3.4.5",
        "08.1.1.1",
        "09",
        "127..1",
        ".127.0.0.1",
        "127.0.0.256",
        "127.0.65536",
        "127.16777216",
        "4294967296",
        "0x100000000",
        "99999999999999999999",
        "-1",
        "+1",
        " 127.0.0.1",
        "127.0.0.1 ",
        "١٢٧.0.0.1",
        "127.0.0.1:80",
        "[::1]",
        "::1",
    ] {
        assert_eq!(legacy_ipv4(spelling), None, "{spelling:?} is not an inet_aton address");
        assert_eq!(
            normalize_host(spelling),
            spelling.to_ascii_lowercase(),
            "{spelling:?} is left as a name"
        );
    }
}

#[test]
fn part_widths_follow_inet_aton() {
    assert_eq!(legacy_ipv4("1.2.3.4"), Some(Ipv4Addr::new(1, 2, 3, 4)));
    assert_eq!(legacy_ipv4("1.2.772"), Some(Ipv4Addr::new(1, 2, 3, 4)));
    assert_eq!(legacy_ipv4("1.131844"), Some(Ipv4Addr::new(1, 2, 3, 4)));
    assert_eq!(legacy_ipv4("16909060"), Some(Ipv4Addr::new(1, 2, 3, 4)));
    assert_eq!(legacy_ipv4("1.2.65536"), None, "two-byte tail overflows");
    assert_eq!(legacy_ipv4("1.16777216"), None, "three-byte tail overflows");
    assert_eq!(legacy_ipv4("255.255.255.255"), Some(Ipv4Addr::BROADCAST));
}

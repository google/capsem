use super::*;

#[test]
fn the_default_pool_is_the_upper_half_of_ten_slash_eight() {
    let pool = PrivatePool::parse("10.128.0.0/9").unwrap();
    assert_eq!(pool, PrivatePool::DEFAULT);
    assert_eq!(pool.to_string(), "10.128.0.0/9");
    assert_eq!(pool.gateway(), Ipv4Addr::new(10, 128, 0, 1));
    assert_eq!(pool.first_host(), Ipv4Addr::new(10, 128, 0, 2));
    assert_eq!(pool.last_host(), Ipv4Addr::new(10, 255, 255, 254));
    assert_eq!(pool.capacity(), (1 << 23) - 3);
    assert!(pool.contains(Ipv4Addr::new(10, 200, 1, 1)));
    assert!(!pool.contains(Ipv4Addr::new(10, 0, 0, 1)));
}

#[test]
fn the_default_pool_is_clear_of_both_guest_links() {
    assert!(!PrivatePool::DEFAULT.overlaps(GUEST_LINK));
    assert!(!PrivatePool::DEFAULT.overlaps(CONTAINER_LINK));
    assert!(GUEST_LINK.contains(Ipv4Addr::new(10, 0, 0, 1)));
    assert!(CONTAINER_LINK.contains(Ipv4Addr::new(10, 0, 1, 2)));
}

#[test]
fn pools_that_would_shadow_a_guest_link_are_refused() {
    for (text, reason) in [
        ("10.0.0.0/8", "guest link"),
        ("10.0.0.0/24", "guest link"),
        ("10.0.1.0/29", "container link"),
    ] {
        let error = PrivatePool::parse(text).unwrap_err();
        assert!(error.contains(reason), "{text}: {error}");
    }
}

#[test]
fn malformed_and_public_pools_are_refused() {
    for (text, reason) in [
        ("10.128.0.0", "a.b.c.d/prefix"),
        ("10.128.0.x/9", "invalid address"),
        ("10.128.0.0/nine", "invalid prefix"),
        ("10.128.0.0/7", "between /8 and /29"),
        ("10.128.0.0/30", "between /8 and /29"),
        ("10.128.1.0/9", "not aligned"),
        ("8.8.0.0/16", "RFC 1918"),
    ] {
        let error = PrivatePool::parse(text).unwrap_err();
        assert!(error.contains(reason), "{text}: {error}");
    }
}

#[test]
fn a_small_pool_still_carries_gateway_and_hosts() {
    let pool = PrivatePool::parse("192.168.7.8/29").unwrap();
    assert_eq!(pool.gateway(), Ipv4Addr::new(192, 168, 7, 9));
    assert_eq!(pool.first_host(), Ipv4Addr::new(192, 168, 7, 10));
    assert_eq!(pool.last_host(), Ipv4Addr::new(192, 168, 7, 14));
    assert_eq!(pool.capacity(), 5);
}

#[test]
fn a_pool_is_carved_into_network_subnets_in_order() {
    let pool = PrivatePool::DEFAULT;
    assert_eq!(pool.subnet_count(NETWORK_PREFIX_LEN), 1 << 15);
    assert_eq!(pool.subnet(0, NETWORK_PREFIX_LEN).unwrap().to_string(), "10.128.0.0/24");
    assert_eq!(pool.subnet(1, NETWORK_PREFIX_LEN).unwrap().to_string(), "10.128.1.0/24");
    assert_eq!(
        pool.subnet((1 << 15) - 1, NETWORK_PREFIX_LEN).unwrap().to_string(),
        "10.255.255.0/24"
    );
    assert_eq!(pool.subnet(1 << 15, NETWORK_PREFIX_LEN), None);
    let subnet = pool.subnet(3, NETWORK_PREFIX_LEN).unwrap();
    assert!(pool.contains(subnet.network()) && pool.contains(subnet.last_host()));
    assert_eq!(subnet.first_host(), Ipv4Addr::new(10, 128, 3, 2));
    assert_eq!(subnet.capacity(), 253);
}

#[test]
fn subnets_never_overlap_and_never_leave_their_pool() {
    let pool = PrivatePool::parse("172.16.0.0/22").unwrap();
    let subnets: Vec<_> = (0..pool.subnet_count(24))
        .map(|index| pool.subnet(index, 24).unwrap())
        .collect();
    assert_eq!(subnets.len(), 4);
    for (index, subnet) in subnets.iter().enumerate() {
        assert!(pool.contains(subnet.network()) && pool.contains(subnet.last_host()));
        for other in &subnets[index + 1..] {
            assert!(!subnet.overlaps(*other), "{subnet} overlaps {other}");
        }
    }
}

#[test]
fn a_prefix_the_pool_cannot_be_carved_into_has_no_subnets() {
    let pool = PrivatePool::parse("192.168.7.8/29").unwrap();
    for prefix_len in [24, 29, 30, 31, 32, 33] {
        assert_eq!(pool.subnet(0, prefix_len), None, "/{prefix_len}");
    }
    assert_eq!(pool.subnet_count(24), 0);
    assert_eq!(pool.subnet_count(33), 0);
}

#[test]
fn a_subnet_parses_back_to_itself() {
    let subnet = PrivatePool::DEFAULT.subnet(42, NETWORK_PREFIX_LEN).unwrap();
    assert_eq!(PrivatePool::parse(&subnet.to_string()).unwrap(), subnet);
}

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

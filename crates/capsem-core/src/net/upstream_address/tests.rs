use super::*;

fn addresses(list: &[&str], port: u16) -> Vec<SocketAddr> {
    list.iter()
        .map(|address| SocketAddr::new(address.parse().unwrap(), port))
        .collect()
}

#[test]
fn public_addresses_are_told_apart_from_everything_else() {
    for public in [
        "8.8.8.8",
        "1.1.1.1",
        "93.184.216.34",
        "2606:4700::1111",
        "2a00:1450:4001::1",
        "64:ff9b::808:808",
    ] {
        assert!(is_public_address(public.parse().unwrap()), "{public} is public");
    }
    for private in [
        "127.0.0.1",
        "127.255.255.254",
        "10.0.0.1",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "100.127.255.255",
        "0.0.0.0",
        "0.1.2.3",
        "255.255.255.255",
        "224.0.0.1",
        "240.0.0.1",
        "198.18.0.1",
        "198.51.100.7",
        "::1",
        "::",
        "fe80::1",
        "fc00::1",
        "fd12:3456::1",
        "ff02::1",
        "2001:db8::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "::ffff:169.254.169.254",
        "2002:7f00:1::",
        "2002:a9fe:a9fe::",
        "64:ff9b::7f00:1",
        "64:ff9b::a9fe:a9fe",
    ] {
        assert!(!is_public_address(private.parse().unwrap()), "{private} is not public");
    }
}

#[tokio::test]
async fn ip_literals_resolve_to_themselves_and_names_through_the_resolver() {
    assert_eq!(
        resolve_upstream("127.0.0.1", 80).await.unwrap(),
        vec![SocketAddr::from(([127, 0, 0, 1], 80))]
    );
    assert_eq!(
        resolve_upstream("::1", 8080).await.unwrap(),
        vec![SocketAddr::new(IpAddr::V6("::1".parse().unwrap()), 8080)]
    );
    let localhost = resolve_upstream("localhost", 1).await.expect("localhost resolves");
    assert!(!localhost.is_empty());
    assert!(
        localhost.iter().all(|address| address.ip().is_loopback()),
        "{localhost:?}"
    );
    assert!(resolve_upstream("no-such-host.invalid", 80).await.is_err());
}

/// `::ffff:127.0.0.1` reaches loopback over a dual-stack socket, but its text
/// does not start with `127.`: the rules would miss it unless it is judged as
/// the IPv4 address it is.
#[tokio::test]
async fn an_ipv4_mapped_answer_is_judged_as_its_ipv4_address() {
    assert_eq!(
        resolve_upstream("::ffff:127.0.0.1", 80).await.unwrap(),
        vec![SocketAddr::from(([127, 0, 0, 1], 80))]
    );
    let resolver =
        UpstreamResolver::system().with_fixed_answer("mapped.invalid", vec!["::ffff:10.0.0.7".parse().unwrap()]);
    assert_eq!(
        resolver.resolve("mapped.invalid", 443).await.unwrap(),
        vec![SocketAddr::from(([10, 0, 0, 7], 443))]
    );
}

#[tokio::test]
async fn a_fixed_answer_replaces_the_system_resolver_for_that_name_only() {
    let resolver = UpstreamResolver::system()
        .with_fixed_answer("Rebind.Invalid", vec!["127.0.0.1".parse().unwrap()])
        .with_fixed_answer("empty.invalid", Vec::new());
    assert_eq!(
        resolver.resolve("rebind.invalid", 8080).await.unwrap(),
        vec![SocketAddr::from(([127, 0, 0, 1], 8080))]
    );
    assert!(resolver.resolve("empty.invalid", 80).await.is_err());
    assert!(
        resolver.resolve("other.invalid", 80).await.is_err(),
        "other names still go to DNS"
    );
    assert_eq!(
        resolver.resolve("10.1.2.3", 80).await.unwrap(),
        vec![SocketAddr::from(([10, 1, 2, 3], 80))]
    );
}

#[tokio::test]
async fn a_disabled_resolver_has_no_system_or_literal_authority() {
    let resolver = UpstreamResolver::disabled();
    for host in ["localhost", "127.0.0.1", "::1"] {
        assert_eq!(
            resolver.resolve(host, 443).await.unwrap_err(),
            "upstream resolution is disabled in this process"
        );
    }
}

#[test]
fn the_judged_address_fails_closed_on_any_local_answer() {
    assert_eq!(judged_address(&[]), None);
    assert_eq!(
        judged_address(&addresses(&["93.184.216.34", "2606:4700::1111"], 443)),
        Some("93.184.216.34".parse().unwrap()),
        "an all-public answer is judged by the first address"
    );
    for (answer, judged) in [
        (vec!["93.184.216.34", "127.0.0.1"], "127.0.0.1"),
        (vec!["2606:4700::1111", "::1", "10.0.0.1"], "::1"),
        (vec!["8.8.8.8", "fe80::1"], "fe80::1"),
        (vec!["1.1.1.1", "169.254.169.254"], "169.254.169.254"),
    ] {
        assert_eq!(
            judged_address(&addresses(&answer, 80)),
            Some(judged.parse().unwrap()),
            "{answer:?} must be judged as its non-public member"
        );
    }
}

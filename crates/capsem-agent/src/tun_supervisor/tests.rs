use super::*;

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn a_host_named_address_and_pool_make_the_pump_arguments() {
    let link = PrivateLink::from_env(&env(&[
        ("CAPSEM_VM_ID", "x"),
        ("CAPSEM_PRIVATE_ADDRESS", "10.129.3.4"),
        ("CAPSEM_PRIVATE_POOL", "10.128.0.0/9"),
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(link.address, Ipv4Addr::new(10, 129, 3, 4));
    assert_eq!(link.prefix, 9);
    assert_eq!(link.arguments(), ["--address", "10.129.3.4", "--prefix", "9"]);
}

#[test]
fn no_address_means_no_pump_and_a_bad_pair_is_refused() {
    assert_eq!(PrivateLink::from_env(&env(&[("CAPSEM_VM_ID", "x")])).unwrap(), None);
    for (address, pool) in [
        ("10.128.0.2", ""),
        ("10.128.0.2", "10.128.0.0"),
        ("10.128.0.2", "10.128.0.0/0"),
        ("10.128.0.2", "10.128.0.0/31"),
        ("nope", "10.128.0.0/9"),
        ("10.0.0.2", "10.128.0.0/9"),
    ] {
        let mut pairs = vec![("CAPSEM_PRIVATE_ADDRESS", address)];
        if !pool.is_empty() {
            pairs.push(("CAPSEM_PRIVATE_POOL", pool));
        }
        assert!(PrivateLink::from_env(&env(&pairs)).is_err(), "{address} {pool}");
    }
}

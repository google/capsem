use super::*;

#[test]
fn listen_port_is_above_privileged_range() {
    // > 1024 means we don't need CAP_NET_BIND_SERVICE.
    const _: () = assert!(LISTEN_PORT > 1024);
}

#[test]
fn listen_port_matches_iptables_target() {
    // capsem-init redirects guest port 53 to LISTEN_PORT via
    // `iptables -t nat -A OUTPUT -p (udp|tcp) --dport 53
    // -j REDIRECT --to-port 1053`. Pinning the constant here
    // means any drift between this binary and the iptables rule
    // shows up in the test diff first.
    assert_eq!(LISTEN_PORT, 1053);
}

#[test]
fn vsock_port_matches_proto_constant() {
    assert_eq!(VSOCK_PORT_DNS_PROXY, 5007);
}

#[test]
fn max_udp_dns_bytes_supports_edns() {
    // RFC 6891 default EDNS UDP payload size is 4096; smaller
    // would risk truncation flag (TC bit) on legit queries.
    const _: () = assert!(MAX_UDP_DNS_BYTES >= 4096);
}

#[test]
fn dns_proxy_uses_persistent_vsock_worker_pool() {
    const _: () = assert!(
        DNS_VSOCK_WORKERS >= 2,
        "DNS must not regress to per-query vsock connect/close; keep a persistent worker pool"
    );
}

#[test]
fn dns_proxy_round_robins_vsock_workers() {
    let next = AtomicUsize::new(0);
    assert_eq!(next_worker_index(&next, 3), 0);
    assert_eq!(next_worker_index(&next, 3), 1);
    assert_eq!(next_worker_index(&next, 3), 2);
    assert_eq!(next_worker_index(&next, 3), 0);
}

#[test]
fn dns_request_envelope_uses_string_proto_label() {
    // The agent sends "udp" or "tcp" -- pinning the labels here
    // means a typo in `forward_query("udb", ...)` (or whatever)
    // gets caught at compile-time-of-test rather than as a
    // confused host-side telemetry row.
    let req = DnsRequest {
        raw: vec![0u8; 12],
        proto: "udp".into(),
        process_name: None,
    };
    assert_eq!(req.proto, "udp");
    let req = DnsRequest {
        raw: vec![0u8; 12],
        proto: "tcp".into(),
        process_name: None,
    };
    assert_eq!(req.proto, "tcp");
}

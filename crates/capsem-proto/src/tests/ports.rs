use super::*;

#[test]
fn vsock_dns_proxy_port_constant() {
    // Pinned so an accidental renumbering shows up in the diff: T3
    // wires both host (capsem-process dispatch) and guest
    // (capsem-agent listener) on this exact port.
    assert_eq!(VSOCK_PORT_DNS_PROXY, 5007);
}

#[test]
fn vsock_network_port_constant() {
    // Pinned like the DNS port: the guest `capsem-tun` pump and the host
    // dispatch both name this exact port for the tun0 packet stream.
    assert_eq!(VSOCK_PORT_NETWORK, 5009);
    assert_eq!(HostVsockService::from_port(5009), Some(HostVsockService::Network));
    assert_eq!(HostVsockService::Network.as_str(), "network");
}

#[test]
fn vsock_private_port_constant() {
    // The guest proxy's private listener and the owner's dispatch both name
    // this exact port for intercepted private TCP.
    assert_eq!(VSOCK_PORT_PRIVATE, 5010);
    assert_eq!(HostVsockService::from_port(5010), Some(HostVsockService::Private));
    assert_eq!(HostVsockService::Private.as_str(), "private");
}

#[test]
fn vsock_port_constants_are_distinct() {
    let ports = [
        VSOCK_PORT_CONTROL,
        VSOCK_PORT_TERMINAL,
        VSOCK_PORT_SNI_PROXY,
        VSOCK_PORT_LIFECYCLE,
        VSOCK_PORT_EXEC,
        VSOCK_PORT_AUDIT,
        VSOCK_PORT_DNS_PROXY,
        VSOCK_PORT_PUBLICATION,
        VSOCK_PORT_NETWORK,
        VSOCK_PORT_PRIVATE,
    ];
    let unique: std::collections::HashSet<_> = ports.iter().collect();
    assert_eq!(unique.len(), ports.len(), "vsock port collision");
}

#[test]
fn host_vsock_registry_is_the_only_boot_listener_contract() {
    let ports: Vec<u32> = host_vsock_services().iter().map(|service| service.port()).collect();
    assert_eq!(
        ports,
        vec![
            VSOCK_PORT_CONTROL,
            VSOCK_PORT_TERMINAL,
            VSOCK_PORT_SNI_PROXY,
            VSOCK_PORT_LIFECYCLE,
            VSOCK_PORT_EXEC,
            VSOCK_PORT_AUDIT,
            VSOCK_PORT_DNS_PROXY,
            VSOCK_PORT_PUBLICATION,
            VSOCK_PORT_NETWORK,
            VSOCK_PORT_PRIVATE,
        ],
        "boot must use the typed host VSOCK service registry, not an inline array"
    );

    assert!(
        HostVsockService::from_port(5003).is_none(),
        "retired raw MCP VSOCK port must stay closed"
    );
    assert!(
        HostVsockService::from_port(11434).is_none(),
        "guest TCP ports must be redirected through the MITM rail, not exposed as raw VSOCK"
    );
}

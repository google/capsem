use super::*;

#[test]
fn vsock_dns_proxy_port_constant() {
    // Pinned so an accidental renumbering shows up in the diff: T3
    // wires both host (capsem-process dispatch) and guest
    // (capsem-agent listener) on this exact port.
    assert_eq!(VSOCK_PORT_DNS_PROXY, 5007);
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

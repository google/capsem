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
    // Pinned like the DNS port: every guest `capsem-tun` pump and the host
    // dispatch name this exact port for network cables.
    assert_eq!(VSOCK_PORT_NETWORK, 5009);
    assert_eq!(HostVsockService::from_port(5009), Some(HostVsockService::Network));
    assert_eq!(HostVsockService::Network.as_str(), "network");
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
        ],
        "boot must use the typed host VSOCK service registry, not an inline array"
    );

    assert!(
        HostVsockService::from_port(5003).is_none(),
        "retired raw MCP VSOCK port must stay closed"
    );
    assert!(
        HostVsockService::from_port(5010).is_none(),
        "retired private TCP VSOCK port must stay closed"
    );
    assert!(
        HostVsockService::from_port(11434).is_none(),
        "guest TCP ports must be redirected through the MITM rail, not exposed as raw VSOCK"
    );
}

/// Publications name the namespace they reach; a missing field from an older
/// host is the container, never the VM.
#[test]
fn publication_target_defaults_to_the_container_on_the_guest_wire() {
    #[derive(serde::Serialize)]
    struct OldConnectPort {
        flow: router::FlowKey,
        port: u16,
    }
    #[derive(serde::Serialize)]
    #[serde(tag = "t", content = "d", rename_all = "lowercase")]
    enum OldHostToGuest {
        ConnectPort(OldConnectPort),
    }
    let flow = router::FlowKey { id: 1, generation: 7 };
    let old = rmp_serde::to_vec_named(&OldHostToGuest::ConnectPort(OldConnectPort { flow, port: 8080 })).unwrap();
    match rmp_serde::from_slice::<HostToGuest>(&old).unwrap() {
        HostToGuest::ConnectPort { target, port, .. } => {
            assert_eq!((target, port), (PublicationTarget::Container, 8080));
        }
        other => panic!("decoded {other:?}"),
    }
    let vm = HostToGuest::ConnectPort {
        flow,
        port: 8080,
        target: PublicationTarget::Vm,
    };
    let bytes = rmp_serde::to_vec_named(&vm).unwrap();
    assert!(matches!(
        rmp_serde::from_slice::<HostToGuest>(&bytes).unwrap(),
        HostToGuest::ConnectPort {
            target: PublicationTarget::Vm,
            ..
        }
    ));
}

#[test]
fn publication_target_vm_refuses_capsem_service_ports() {
    for port in CAPSEM_GUEST_LOOPBACK_PORTS {
        assert!(
            !PublicationTarget::Vm.admits(port),
            "VM target admitted Capsem port {port}"
        );
        assert!(
            PublicationTarget::Container.admits(port),
            "a container namespace has no Capsem listener on {port}"
        );
    }
    assert!(PublicationTarget::Vm.admits(8080));
    assert!(!PublicationTarget::Vm.admits(0) && !PublicationTarget::Container.admits(0));
}

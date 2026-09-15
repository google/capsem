//! Every host VSOCK port classifies to its typed service, and nothing else does.
use super::*;

#[test]
fn classify_terminal_port() {
    assert_eq!(
        HostVsockService::from_port(capsem_proto::VSOCK_PORT_TERMINAL),
        Some(HostVsockService::Terminal)
    );
}

#[test]
fn classify_control_port() {
    assert_eq!(
        HostVsockService::from_port(capsem_proto::VSOCK_PORT_CONTROL),
        Some(HostVsockService::Control)
    );
}

#[test]
fn classify_sni_proxy_port() {
    assert_eq!(
        HostVsockService::from_port(capsem_proto::VSOCK_PORT_SNI_PROXY),
        Some(HostVsockService::SniProxy)
    );
}

#[test]
fn classify_exec_port() {
    assert_eq!(
        HostVsockService::from_port(capsem_proto::VSOCK_PORT_EXEC),
        Some(HostVsockService::Exec)
    );
}

#[test]
fn classify_lifecycle_port() {
    assert_eq!(
        HostVsockService::from_port(capsem_proto::VSOCK_PORT_LIFECYCLE),
        Some(HostVsockService::Lifecycle)
    );
}

#[test]
fn classify_audit_port() {
    assert_eq!(
        HostVsockService::from_port(capsem_proto::VSOCK_PORT_AUDIT),
        Some(HostVsockService::Audit)
    );
}

#[test]
fn classify_dns_proxy_port() {
    assert_eq!(
        HostVsockService::from_port(capsem_proto::VSOCK_PORT_DNS_PROXY),
        Some(HostVsockService::DnsProxy)
    );
}

#[test]
fn classify_network_port() {
    assert_eq!(
        HostVsockService::from_port(capsem_proto::VSOCK_PORT_NETWORK),
        Some(HostVsockService::Network)
    );
}

#[test]
fn classify_unknown_port() {
    assert_eq!(HostVsockService::from_port(99999), None);
}

#[test]
fn classify_port_zero_unknown() {
    assert_eq!(HostVsockService::from_port(0), None);
}

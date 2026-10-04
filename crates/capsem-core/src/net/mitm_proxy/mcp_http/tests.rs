use super::*;

#[test]
fn the_exact_name_on_its_default_port_is_the_gateway() {
    assert_eq!(
        internal_route("mcp.capsem.internal", 80, Protocol::Http),
        Some(InternalRoute::McpGateway)
    );
    assert_eq!(
        internal_route("mcp.capsem.internal", 443, Protocol::Tls),
        Some(InternalRoute::McpGateway)
    );
    // The proxy hands this function the normalized host, but a raw spelling
    // must not change the answer either way.
    assert_eq!(
        internal_route("MCP.Capsem.Internal.", 80, Protocol::Http),
        Some(InternalRoute::McpGateway)
    );
}

#[test]
fn another_port_on_the_gateway_name_is_refused_not_dialed() {
    for (port, protocol) in [
        (8080, Protocol::Http),
        (443, Protocol::Http),
        (80, Protocol::Tls),
        (11434, Protocol::Http),
        (0, Protocol::Http),
    ] {
        assert_eq!(
            internal_route("mcp.capsem.internal", port, protocol),
            Some(InternalRoute::Refused),
            "{protocol:?} :{port}"
        );
    }
    assert_eq!(
        internal_route("mcp.capsem.internal", 80, Protocol::Unknown),
        Some(InternalRoute::Refused)
    );
}

#[test]
fn every_other_name_in_the_zone_is_refused() {
    for host in [
        "capsem.internal",
        "xmcp.capsem.internal",
        "mcp.team.capsem.internal",
        "beta.capsem.internal",
        "a.mcp.capsem.internal",
    ] {
        assert_eq!(
            internal_route(host, 80, Protocol::Http),
            Some(InternalRoute::Refused),
            "{host}"
        );
    }
}

#[test]
fn lookalikes_outside_the_zone_are_ordinary_hosts() {
    for host in [
        "mcp.capsem.internal.example.com",
        "mcp-capsem.internal",
        "mcpcapsem.internal",
        "notcapsem.internal",
        "mcp.capsem.internals",
        "example.com",
    ] {
        assert_eq!(internal_route(host, 80, Protocol::Http), None, "{host}");
    }
}

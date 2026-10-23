//! Public diagnostic contracts included in a support bundle.

pub(super) fn runtime_boundary_debug_contract() -> serde_json::Value {
    let host_vsock_services: Vec<_> = capsem_proto::host_vsock_services()
        .iter()
        .map(|service| {
            serde_json::json!({
                "service": service.as_str(),
                "port": service.port(),
            })
        })
        .collect();

    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "host_vsock_services": host_vsock_services,
        "closed_raw_vsock_ports": [
            {
                "port": 5003,
                "reason": "retired_mcp_raw_port",
            },
            {
                "port": 11434,
                "reason": "guest_tcp_ollama_must_use_mitm_redirect",
            },
            {
                "port": 3128,
                "reason": "guest_tcp_proxy_must_use_mitm_redirect",
            },
            {
                "port": 8080,
                "reason": "guest_tcp_proxy_must_use_mitm_redirect",
            }
        ],
        "debug_routes": [
            "/version",
            "/status",
            "/triage",
            "/panics",
            "/host-logs/{name}",
            "/vms/{id}/status",
            "/vms/{id}/info",
            "/vms/{id}/logs",
            "/vms/{id}/history",
            "/vms/{id}/security/latest",
            "/vms/{id}/security/status",
            "/vms/{id}/detection/latest",
            "/vms/{id}/detection/status",
            "/vms/{id}/enforcement/latest",
            "/vms/{id}/enforcement/status",
            "/assets/status",
            "/settings/info",
            "/corp/info",
            "/plugins/list",
            "/plugins/{plugin_id}/info",
            "/plugins/credential_broker/credentials/info",
            "/mcp/info",
            "/mcp/default/info",
            "/mcp/servers/list"
        ],
    })
}

pub(super) fn supply_chain_debug_references() -> serde_json::Value {
    serde_json::json!({
        "host_sbom": {
            "format": "spdx_json_2_3",
            "scope": "host_binaries",
            "generator": "cargo-sbom",
            "release_artifact": "capsem-sbom.spdx.json",
            "attestation": "github_attestations",
            "workflow": ".github/workflows/release.yaml",
        },
        "vm_obom": {
            "format": "cyclonedx-obom.v1",
            "scope": "base_image",
            "generator": "cdxgen",
            "descriptor_source": "release manifest",
        },
        "manifest": {
            "hash": "blake3",
            "runtime_status": "/status",
            "runtime_update_status": "/update/status",
            "runtime_update_status_field": "supply_chain",
            "support_bundle_paths": [
                "assets/manifest.json",
                "assets/manifest-metadata.json",
            ],
        },
    })
}

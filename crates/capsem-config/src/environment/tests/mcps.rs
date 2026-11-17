use super::*;

const MCPS: &str = r#"
[mcps.servers.remote]
kind = 'remote'
url = 'https://mcp.example.org/rpc'
[mcps.permissions.remote]
default = 'ask'
[mcps.permissions.remote.tools]
read = 'allow'
write = 'block'
[mcps.servers.worker]
kind = 'sandbox'
executable = 'uvx'
args = ['--offline', 'example-server']
workdir = '/workspace'
[mcps.servers.worker.variables]
PROJECT = 'launch-variable-canary'
[mcps.servers.worker.secrets.api]
binding_id = '00000000-0000-4000-8000-000000000001'
env_name = 'API_KEY'
"#;

#[test]
fn environment_mcps_preserve_remote_and_sandbox_definitions_without_raw_auth() {
    let source = format!("{}\n{MCPS}", with_image().to_toml().unwrap());
    let doc = EnvironmentDocument::from_toml(&source).unwrap();
    assert!(EnvironmentDocument::from_toml(&doc.to_toml().unwrap()).unwrap() == doc);
    assert!(!format!("{doc:?}").contains("launch-variable-canary"));
    for bad in [
        source.replace("kind = 'remote'", "kind = 'local'"),
        source.replace("mcp.example.org", "user:secret@mcp.example.org"),
        source.replace("https://mcp", "file://mcp"),
        source.replace("mcps.servers.remote", "mcps.servers.local"),
        source.replace("default = 'ask'", "default = 'rewrite'"),
        source.replace("executable = 'uvx'", "executable = ''"),
        source.replace("00000000-0000-4000-8000-000000000001", "raw-secret-canary"),
        source.replace("API_KEY", "PROJECT"),
        source.replace(
            "url = 'https://mcp.example.org/rpc'",
            "url = 'https://mcp.example.org/rpc'\nheaders={Authorization='raw-secret-canary'}",
        ),
    ] {
        let error = EnvironmentDocument::from_toml(&bad).unwrap_err();
        assert!(!error.contains("raw-secret-canary"));
    }
}

#[test]
fn environment_mcps_catalog_drafts_refuse_pins_and_unselected_permissions() {
    let source = "name='development'\n[[mcps.catalog.selected]]\nid='catalog-server'\nversion='1.0.0'\n[mcps.permissions.catalog-server]\ndefault='block'\n[mcps.permissions.catalog-server.tools]\nread='allow'\n";
    let draft: EnvironmentDraft = toml::from_str(source).unwrap();
    draft.validate().unwrap();
    let bad: EnvironmentDraft =
        toml::from_str(&source.replace("permissions.catalog-server", "permissions.missing")).unwrap();
    assert!(bad.validate().is_err());
    assert!(toml::from_str::<EnvironmentDraft>(
        &source.replace("version='1.0.0'", "version='1.0.0'\nresolved_digest='forged'")
    )
    .is_err());
}

#[test]
fn environment_mcps_default_to_block_and_enforce_launch_and_override_bounds() {
    let mut doc = EnvironmentDocument::from_toml(&format!("{}\n{MCPS}", with_image().to_toml().unwrap())).unwrap();
    doc.specification
        .mcps
        .permissions
        .insert("worker".into(), Default::default());
    assert_eq!(
        doc.specification.mcps.permissions["worker"].default,
        EnvironmentCapabilityDecision::Block
    );
    let permission = doc.specification.mcps.permissions.get_mut("worker").unwrap();
    for index in 0..capsem_proto::ENVIRONMENT_MAX_MCP_TOOL_OVERRIDES {
        permission
            .tools
            .insert(format!("tool-{index}"), EnvironmentCapabilityDecision::Ask);
    }
    doc.to_toml().unwrap();
    doc.specification
        .mcps
        .permissions
        .get_mut("worker")
        .unwrap()
        .tools
        .insert("extra".into(), EnvironmentCapabilityDecision::Allow);
    assert!(doc.to_toml().is_err());
    doc.specification
        .mcps
        .permissions
        .get_mut("worker")
        .unwrap()
        .tools
        .clear();
    if let EnvironmentMcpDefinition::Sandbox { args, .. } = doc.specification.mcps.servers.get_mut("worker").unwrap() {
        *args = vec!["approved\nmultiline\targument".into(); capsem_proto::ENVIRONMENT_MAX_MCP_ARGUMENTS];
    }
    doc.to_toml().unwrap();
    if let EnvironmentMcpDefinition::Sandbox { args, .. } = doc.specification.mcps.servers.get_mut("worker").unwrap() {
        args.push("extra".into());
    }
    assert!(doc.to_toml().is_err());
    if let EnvironmentMcpDefinition::Sandbox { args, .. } = doc.specification.mcps.servers.get_mut("worker").unwrap() {
        *args = vec!["nul\0argument".into()];
    }
    assert!(doc.to_toml().is_err());
}

#[test]
fn environment_mcps_bound_combined_catalog_and_inline_server_counts() {
    let mut doc = with_image();
    for index in 0..capsem_proto::ENVIRONMENT_MAX_MCP_SERVERS {
        doc.specification.mcps.servers.insert(
            format!("server-{index}"),
            EnvironmentMcpDefinition::Remote {
                url: "https://mcp.example.org/rpc".into(),
                headers: Default::default(),
                binding_id: None,
            },
        );
    }
    doc.to_toml().unwrap();
    doc.specification.mcps.catalog.selected.push(EnvironmentCatalogPin {
        id: "extra".into(),
        version: "1.0.0".into(),
        catalog_reference: "ghcr.io/google/capsem/mcps:stable".into(),
        resolved_digest: format!("sha256:{}", "a".repeat(64)),
    });
    assert!(doc.to_toml().is_err());
    doc.specification.mcps.servers.remove("server-0");
    doc.to_toml().unwrap();
    doc.specification.mcps.catalog.selected[0].id = "server-1".into();
    assert!(doc.to_toml().is_err());
}

#[test]
fn environment_mcps_bound_headers_and_preserve_transport_authority() {
    let mut doc = with_image();
    let mut headers = BTreeMap::new();
    for index in 0..capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_ENTRIES {
        headers.insert(format!("x-metadata-{index}"), "value".into());
    }
    doc.specification.mcps.servers.insert(
        "remote".into(),
        EnvironmentMcpDefinition::Remote {
            url: "https://mcp.example.org/rpc".into(),
            headers,
            binding_id: None,
        },
    );
    doc.to_toml().unwrap();
    let original = doc.clone();
    for (name, value) in [
        ("Host", "attacker.example"),
        ("Transfer-Encoding", "chunked"),
        ("Bad Header", "value"),
        ("x-api-key", "private-canary"),
        ("x-metadata", "bad\r\nvalue"),
    ] {
        let mut bad = original.clone();
        if let EnvironmentMcpDefinition::Remote { headers, .. } =
            bad.specification.mcps.servers.get_mut("remote").unwrap()
        {
            headers.clear();
            headers.insert(name.into(), value.into());
        }
        assert!(bad.to_toml().is_err());
    }
    if let EnvironmentMcpDefinition::Remote { headers, .. } = doc.specification.mcps.servers.get_mut("remote").unwrap()
    {
        headers.insert("x-extra".into(), "value".into());
    }
    assert!(doc.to_toml().is_err());
}

#[test]
fn environment_mcps_bound_launch_text_before_serializing_and_require_guest_workdirs() {
    let original = EnvironmentDocument::from_toml(&format!("{}\n{MCPS}", with_image().to_toml().unwrap())).unwrap();
    let mut doc = original.clone();
    if let EnvironmentMcpDefinition::Sandbox { args, .. } = doc.specification.mcps.servers.get_mut("worker").unwrap() {
        *args = vec!["x".repeat(capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES)];
    }
    doc.to_toml().unwrap();
    if let EnvironmentMcpDefinition::Sandbox { args, .. } = doc.specification.mcps.servers.get_mut("worker").unwrap() {
        args[0].push('x');
    }
    assert!(doc.to_toml().is_err());
    for workdir in [
        "relative",
        "/workspace/../outside",
        "/workspace/./project",
        "/workspace/\0bad",
    ] {
        let mut bad = original.clone();
        if let EnvironmentMcpDefinition::Sandbox { workdir: target, .. } =
            bad.specification.mcps.servers.get_mut("worker").unwrap()
        {
            *target = Some(workdir.into());
        }
        assert!(bad.to_toml().is_err());
    }
    let mut bad = original;
    bad.specification.image = None;
    assert!(bad.to_toml().is_err());
}

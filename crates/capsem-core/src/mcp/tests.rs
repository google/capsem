use super::*;
use std::io::Write;
use crate::mcp::policy::{McpManualServer, McpUserConfig};

fn make_tool(ns_name: &str, orig_name: &str, server: &str, desc: Option<&str>) -> McpToolDef {
    McpToolDef {
        namespaced_name: ns_name.into(),
        original_name: orig_name.into(),
        description: desc.map(String::from),
        input_schema: serde_json::json!({"type": "object"}),
        server_name: server.into(),
        annotations: None,
    }
}

// ── compute_tool_hash tests ─────────────────────────────────────

#[test]
fn compute_tool_hash_deterministic() {
    let tool = make_tool("github__search", "search", "github", Some("Search repos"));
    let h1 = compute_tool_hash(&tool);
    let h2 = compute_tool_hash(&tool);
    assert_eq!(h1, h2);
}

#[test]
fn compute_tool_hash_changes_on_description() {
    let mut tool = make_tool("github__search", "search", "github", Some("Search repos"));
    let h1 = compute_tool_hash(&tool);
    tool.description = Some("Search all repos".into());
    let h2 = compute_tool_hash(&tool);
    assert_ne!(h1, h2);
}

#[test]
fn compute_tool_hash_changes_on_schema() {
    let mut tool = make_tool("github__search", "search", "github", Some("Search"));
    let h1 = compute_tool_hash(&tool);
    tool.input_schema = serde_json::json!({"type": "object", "properties": {"q": {"type": "string"}}});
    let h2 = compute_tool_hash(&tool);
    assert_ne!(h1, h2);
}

#[test]
fn compute_tool_hash_changes_on_annotations() {
    let mut tool = make_tool("github__search", "search", "github", Some("Search"));
    tool.annotations = Some(ToolAnnotations { read_only_hint: true, ..Default::default() });
    let h1 = compute_tool_hash(&tool);
    tool.annotations = Some(ToolAnnotations { read_only_hint: false, ..Default::default() });
    let h2 = compute_tool_hash(&tool);
    assert_ne!(h1, h2);
}

// ── detect_pin_changes tests ────────────────────────────────────

#[test]
fn detect_pin_changes_no_change() {
    let tool = make_tool("github__search", "search", "github", Some("Search"));
    let hash = compute_tool_hash(&tool);
    let cache = vec![ToolCacheEntry {
        namespaced_name: "github__search".into(),
        original_name: "search".into(),
        description: Some("Search".into()),
        server_name: "github".into(),
        annotations: None,
        pin_hash: hash,
        first_seen: "2025-01-01".into(),
        last_seen: "2025-01-01".into(),
        approved: true,
    }];
    let changes = detect_pin_changes(&[tool], &cache);
    assert!(changes.is_empty());
}

#[test]
fn detect_pin_changes_description_changed() {
    let tool = make_tool("github__search", "search", "github", Some("New description"));
    let cache = vec![ToolCacheEntry {
        namespaced_name: "github__search".into(),
        original_name: "search".into(),
        description: Some("Old description".into()),
        server_name: "github".into(),
        annotations: None,
        pin_hash: "oldhash".into(),
        first_seen: "2025-01-01".into(),
        last_seen: "2025-01-01".into(),
        approved: true,
    }];
    let changes = detect_pin_changes(&[tool], &cache);
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], PinChange::Changed { .. }));
}

#[test]
fn detect_pin_changes_new_tool() {
    let tool = make_tool("github__new_tool", "new_tool", "github", None);
    let changes = detect_pin_changes(&[tool], &[]);
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], PinChange::New { .. }));
}

#[test]
fn detect_pin_changes_tool_removed() {
    let cache = vec![ToolCacheEntry {
        namespaced_name: "github__removed".into(),
        original_name: "removed".into(),
        description: None,
        server_name: "github".into(),
        annotations: None,
        pin_hash: "hash".into(),
        first_seen: "2025-01-01".into(),
        last_seen: "2025-01-01".into(),
        approved: true,
    }];
    let changes = detect_pin_changes(&[], &cache);
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], PinChange::Removed { .. }));
}

#[test]
fn rug_pull_subtle_description_change() {
    // Single character change must be detected
    let tool = make_tool("github__search", "search", "github", Some("Search repo"));
    let cache = vec![ToolCacheEntry {
        namespaced_name: "github__search".into(),
        original_name: "search".into(),
        description: Some("Search repos".into()),
        server_name: "github".into(),
        annotations: None,
        pin_hash: compute_tool_hash(&make_tool("github__search", "search", "github", Some("Search repos"))),
        first_seen: "2025-01-01".into(),
        last_seen: "2025-01-01".into(),
        approved: true,
    }];
    let changes = detect_pin_changes(&[tool], &cache);
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], PinChange::Changed { .. }));
}

#[test]
fn rug_pull_schema_injection() {
    let mut tool = make_tool("github__search", "search", "github", Some("Search"));
    // Add a hidden parameter
    tool.input_schema = serde_json::json!({"type": "object", "properties": {"q": {"type": "string"}, "hidden": {"type": "string"}}});
    let original = make_tool("github__search", "search", "github", Some("Search"));
    let cache = vec![ToolCacheEntry {
        namespaced_name: "github__search".into(),
        original_name: "search".into(),
        description: Some("Search".into()),
        server_name: "github".into(),
        annotations: None,
        pin_hash: compute_tool_hash(&original),
        first_seen: "2025-01-01".into(),
        last_seen: "2025-01-01".into(),
        approved: true,
    }];
    let changes = detect_pin_changes(&[tool], &cache);
    assert_eq!(changes.len(), 1);
}

#[test]
fn rug_pull_annotation_flip() {
    let mut tool = make_tool("github__delete", "delete", "github", Some("Delete"));
    tool.annotations = Some(ToolAnnotations { read_only_hint: false, ..Default::default() });
    let mut original = make_tool("github__delete", "delete", "github", Some("Delete"));
    original.annotations = Some(ToolAnnotations { read_only_hint: true, ..Default::default() });
    let cache = vec![ToolCacheEntry {
        namespaced_name: "github__delete".into(),
        original_name: "delete".into(),
        description: Some("Delete".into()),
        server_name: "github".into(),
        annotations: Some(ToolAnnotations { read_only_hint: true, ..Default::default() }),
        pin_hash: compute_tool_hash(&original),
        first_seen: "2025-01-01".into(),
        last_seen: "2025-01-01".into(),
        approved: true,
    }];
    let changes = detect_pin_changes(&[tool], &cache);
    assert_eq!(changes.len(), 1);
}

#[test]
fn cross_server_name_collision() {
    let tools = vec![
        make_tool("a__search", "search", "a", None),
        make_tool("b__search", "search", "b", None),
    ];
    let collisions = detect_name_collisions(&tools);
    assert_eq!(collisions.len(), 1);
    assert_eq!(collisions[0].0, "search");
    assert_eq!(collisions[0].1.len(), 2);
}

// ── tool cache I/O tests ────────────────────────────────────────

#[test]
fn tool_cache_roundtrip() {
    let entries = vec![ToolCacheEntry {
        namespaced_name: "github__search".into(),
        original_name: "search".into(),
        description: Some("Search".into()),
        server_name: "github".into(),
        annotations: None,
        pin_hash: "abc123".into(),
        first_seen: "2025-01-01".into(),
        last_seen: "2025-01-01".into(),
        approved: true,
    }];
    let json = serde_json::to_string(&entries).unwrap();
    let decoded: Vec<ToolCacheEntry> = serde_json::from_str(&json).unwrap();
    assert_eq!(entries, decoded);
}

#[test]
fn tool_cache_missing_file_returns_empty() {
    // load_tool_cache with nonexistent HOME
    std::env::set_var("HOME", "/nonexistent_test_dir_xyz");
    let cache = load_tool_cache();
    assert!(cache.is_empty());
}

// ── build_server_list tests ─────────────────────────────────────

#[test]
fn build_server_list_empty() {
    let user = McpUserConfig::default();
    let corp = McpUserConfig::default();
    // No auto-detected servers in test env, no manual, no corp
    let list = build_server_list(&user, &corp);
    // May have auto-detected servers from local dev env, but at least no crash
    assert!(list.iter().all(|s| s.name != "builtin"));
}

#[test]
fn build_server_list_manual_servers() {
    let user = McpUserConfig {
        servers: vec![McpManualServer {
            name: "myserver".into(),
            url: "https://mcp.example.com/v1".into(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
        }],
        ..Default::default()
    };
    let corp = McpUserConfig::default();
    let list = build_server_list(&user, &corp);
    assert!(list.iter().any(|s| s.name == "myserver" && s.source == "manual"));
}

#[test]
fn build_server_list_corp_servers_added() {
    let user = McpUserConfig::default();
    let corp = McpUserConfig {
        servers: vec![McpManualServer {
            name: "corp-server".into(),
            url: "https://corp.internal/mcp".into(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
        }],
        ..Default::default()
    };
    let list = build_server_list(&user, &corp);
    assert!(list.iter().any(|s| s.name == "corp-server" && s.source == "corp"));
}

#[test]
fn build_server_list_reject_builtin_name() {
    let user = McpUserConfig {
        servers: vec![McpManualServer {
            name: "builtin".into(),
            url: "https://evil.com/mcp".into(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
        }],
        ..Default::default()
    };
    let corp = McpUserConfig::default();
    let list = build_server_list(&user, &corp);
    assert!(!list.iter().any(|s| s.name == "builtin"));
}

#[test]
fn build_server_list_empty_name_rejected() {
    let user = McpUserConfig {
        servers: vec![McpManualServer {
            name: "".into(),
            url: "https://test.com/mcp".into(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
        }],
        ..Default::default()
    };
    let corp = McpUserConfig::default();
    let list = build_server_list(&user, &corp);
    assert!(!list.iter().any(|s| s.name.is_empty()));
}

#[test]
fn build_server_list_enabled_override() {
    let user = McpUserConfig {
        servers: vec![McpManualServer {
            name: "myserver".into(),
            url: "https://mcp.example.com/v1".into(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
        }],
        server_enabled: {
            let mut m = HashMap::new();
            m.insert("myserver".into(), false);
            m
        },
        ..Default::default()
    };
    let corp = McpUserConfig::default();
    let list = build_server_list(&user, &corp);
    let s = list.iter().find(|s| s.name == "myserver").unwrap();
    assert!(!s.enabled);
}

// ── original parse tests ────────────────────────────────────────

#[test]
fn parse_claude_settings_stdio_flagged_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let mut f = std::fs::File::create(&path).unwrap();
    write!(
        f,
        r#"{{
        "mcpServers": {{
            "github": {{
                "command": "npx",
                "args": ["-y", "@github/mcp-server"],
                "env": {{"GITHUB_TOKEN": "ghp_secret"}}
            }},
            "capsem": {{
                "command": "/run/capsem-mcp-server"
            }}
        }}
    }}"#
    )
    .unwrap();

    let defs = parse_mcp_servers_from_file(&path, "claude").unwrap();
    assert_eq!(defs.len(), 1); // capsem filtered out
    assert_eq!(defs[0].name, "github");
    assert!(defs[0].is_stdio());
    assert_eq!(defs[0].command.as_deref(), Some("npx"));
    assert_eq!(defs[0].source, "claude");
}

#[test]
fn parse_http_server_from_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"mcpServers": {"api": {"url": "https://mcp.example.com/v1", "bearerToken": "tok_123"}}}"#,
    )
    .unwrap();

    let defs = parse_mcp_servers_from_file(&path, "claude").unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "api");
    assert_eq!(defs[0].url, "https://mcp.example.com/v1");
    assert_eq!(defs[0].bearer_token.as_deref(), Some("tok_123"));
    assert!(!defs[0].is_stdio());
}

#[test]
fn parse_mixed_stdio_and_http_servers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"mcpServers": {
            "http-server": {"url": "https://mcp.example.com/v1"},
            "stdio-server": {"command": "npx", "args": ["-y", "@test/server"]}
        }}"#,
    )
    .unwrap();

    let defs = parse_mcp_servers_from_file(&path, "test").unwrap();
    assert_eq!(defs.len(), 2);
    let http = defs.iter().find(|d| d.name == "http-server").unwrap();
    let stdio = defs.iter().find(|d| d.name == "stdio-server").unwrap();
    assert!(!http.is_stdio());
    assert!(stdio.is_stdio());
}

#[test]
fn parse_missing_file_returns_none() {
    let result = parse_mcp_servers_from_file(Path::new("/nonexistent/settings.json"), "test");
    assert!(result.is_none());
}

#[test]
fn parse_no_mcp_servers_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, r#"{"other": "stuff"}"#).unwrap();
    let result = parse_mcp_servers_from_file(&path, "test");
    assert!(result.is_none());
}

#[test]
fn parse_server_without_url_or_command_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"mcpServers": {"bad": {"name": "bad"}}}"#,
    )
    .unwrap();
    let defs = parse_mcp_servers_from_file(&path, "test").unwrap();
    assert_eq!(defs.len(), 0);
}

#[test]
fn build_server_list_rejects_names_with_separator() {
    let mut user = McpUserConfig::default();
    user.servers.push(crate::mcp::policy::McpManualServer {
        name: "bad__name".to_string(),
        url: "http://localhost".to_string(),
        headers: HashMap::new(),
        bearer_token: None,
        enabled: true,
    });
    user.servers.push(crate::mcp::policy::McpManualServer {
        name: "goodname".to_string(),
        url: "http://localhost".to_string(),
        headers: HashMap::new(),
        bearer_token: None,
        enabled: true,
    });

    let mut corp = McpUserConfig::default();
    corp.servers.push(crate::mcp::policy::McpManualServer {
        name: "corp__bad".to_string(),
        url: "http://localhost".to_string(),
        headers: HashMap::new(),
        bearer_token: None,
        enabled: true,
    });

    let servers = build_server_list(&user, &corp);
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "goodname");
}

// ------------------------------------------------------------------
// Binary coverage: ensure every [[bin]] in capsem-agent/Cargo.toml
// appears in Dockerfile.rootfs and justfile _pack-initrd.
// ------------------------------------------------------------------

/// Parse [[bin]] name entries from a Cargo.toml file.
fn parse_cargo_bin_names(path: &std::path::Path) -> Vec<String> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let doc: toml::Value = toml::from_str(&text)
        .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
    doc.get("bin")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    entry.get("name").and_then(|n| n.as_str()).map(String::from)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn repo_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR is crates/capsem-core
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn all_guest_binaries_in_dockerfile_rootfs() {
    let root = repo_root();
    let bins = parse_cargo_bin_names(&root.join("crates/capsem-agent/Cargo.toml"));
    assert!(!bins.is_empty(), "no [[bin]] entries found in capsem-agent");

    let template = std::fs::read_to_string(
        root.join("src/capsem/builder/templates/Dockerfile.rootfs.j2"),
    )
    .expect("cannot read Dockerfile.rootfs.j2");

    // The Jinja template uses a loop over guest_binaries to COPY each binary.
    // Verify the loop pattern exists -- the Python build context test
    // (test_docker.py) verifies the actual binary list matches.
    assert!(
        template.contains("{% for binary in guest_binaries %}"),
        "Dockerfile.rootfs.j2 missing guest_binaries loop"
    );
    assert!(
        template.contains("COPY {{ binary }} /usr/local/bin/{{ binary }}"),
        "Dockerfile.rootfs.j2 missing COPY template for guest binaries"
    );

    // Also verify that prepare_build_context includes all agent binaries
    // by checking the Python build context function lists them.
    let docker_py = std::fs::read_to_string(root.join("src/capsem/builder/docker.py"))
        .expect("cannot read docker.py");
    for bin in &bins {
        assert!(
            docker_py.contains(bin),
            "docker.py missing guest binary '{bin}' in build context"
        );
    }
}

#[test]
fn all_guest_binaries_in_pack_initrd() {
    let root = repo_root();
    let bins = parse_cargo_bin_names(&root.join("crates/capsem-agent/Cargo.toml"));
    assert!(!bins.is_empty(), "no [[bin]] entries found in capsem-agent");

    let justfile = std::fs::read_to_string(root.join("justfile"))
        .expect("cannot read justfile");

    // Extract the _pack-initrd recipe section (from "_pack-initrd:" to next recipe)
    let start = justfile
        .find("_pack-initrd:")
        .expect("_pack-initrd recipe not found in justfile");
    let section = &justfile[start..];
    let end = section[1..]
        .find("\n\n")
        .map(|i| i + 1)
        .unwrap_or(section.len());
    let recipe = &section[..end];

    for bin in &bins {
        assert!(
            recipe.contains(bin),
            "justfile _pack-initrd missing guest binary '{bin}'. \
             Add cp + chmod lines for {bin}."
        );
    }
}

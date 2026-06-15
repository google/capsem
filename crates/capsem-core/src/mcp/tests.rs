use super::*;
use crate::mcp::policy::{McpManualServer, McpUserConfig};

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn make_tool(ns_name: &str, orig_name: &str, server: &str, desc: Option<&str>) -> McpToolDef {
    McpToolDef {
        namespaced_name: ns_name.into(),
        original_name: orig_name.into(),
        description: desc.map(String::from),
        input_schema: serde_json::json!({"type": "object"}),
        server_name: server.into(),
        annotations: None,
        timeout_secs: None,
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
    tool.input_schema =
        serde_json::json!({"type": "object", "properties": {"q": {"type": "string"}}});
    let h2 = compute_tool_hash(&tool);
    assert_ne!(h1, h2);
}

#[test]
fn compute_tool_hash_changes_on_annotations() {
    let mut tool = make_tool("github__search", "search", "github", Some("Search"));
    tool.annotations = Some(ToolAnnotations {
        read_only_hint: true,
        ..Default::default()
    });
    let h1 = compute_tool_hash(&tool);
    tool.annotations = Some(ToolAnnotations {
        read_only_hint: false,
        ..Default::default()
    });
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
    let tool = make_tool(
        "github__search",
        "search",
        "github",
        Some("New description"),
    );
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
        pin_hash: compute_tool_hash(&make_tool(
            "github__search",
            "search",
            "github",
            Some("Search repos"),
        )),
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
    tool.annotations = Some(ToolAnnotations {
        read_only_hint: false,
        ..Default::default()
    });
    let mut original = make_tool("github__delete", "delete", "github", Some("Delete"));
    original.annotations = Some(ToolAnnotations {
        read_only_hint: true,
        ..Default::default()
    });
    let cache = vec![ToolCacheEntry {
        namespaced_name: "github__delete".into(),
        original_name: "delete".into(),
        description: Some("Delete".into()),
        server_name: "github".into(),
        annotations: Some(ToolAnnotations {
            read_only_hint: true,
            ..Default::default()
        }),
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
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    // load_tool_cache with nonexistent HOME
    let _home_guard = EnvVarGuard::set("HOME", "/nonexistent_test_dir_xyz");
    let cache = load_tool_cache();
    assert!(cache.is_empty());
}

#[test]
fn mcp_config_rejects_raw_bearer_token_field() {
    let err = toml::from_str::<McpUserConfig>(
        r#"
[[servers]]
name = "remote"
url = "https://mcp.example.com/v1"
bearer_token = "tok_raw"
"#,
    )
    .expect_err("raw bearer_token must not be accepted in MCP config");
    assert!(err.to_string().contains("bearer_token"), "{err}");
}

#[test]
fn mcp_config_rejects_secret_bearing_headers() {
    let cfg: McpUserConfig = toml::from_str(
        r#"
[[servers]]
name = "remote"
url = "https://mcp.example.com/v1"
[servers.headers]
Authorization = "Bearer raw"
"#,
    )
    .unwrap();
    let err = cfg
        .validate("profile")
        .expect_err("Authorization headers must be brokered, not stored in TOML");
    assert!(err.contains("credential broker"), "{err}");
}

#[test]
fn mcp_config_accepts_oauth_broker_reference() {
    let cfg: McpUserConfig = toml::from_str(&format!(
        r#"
[[servers]]
name = "remote"
url = "https://mcp.example.com/v1"

[servers.auth]
kind = "oauth"
credential_ref = "credential:blake3:{}"
"#,
        "a".repeat(64)
    ))
    .unwrap();
    cfg.validate("profile")
        .expect("brokered OAuth auth must validate");
    assert_eq!(
        cfg.servers[0].auth.as_ref().unwrap().kind,
        crate::mcp::types::McpAuthKind::OAuth
    );
}

#[test]
fn credential_broker_resolves_mcp_oauth_material_by_reference() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let _store_guard = EnvVarGuard::set(
        crate::credential_broker::TEST_STORE_ENV,
        dir.path().join("store.json"),
    );
    let observation = crate::credential_broker::CredentialObservation {
        provider: crate::credential_broker::CredentialProvider::Mcp,
        raw_value: "oauth-access-token".to_string(),
        source: "mcp.auth.remote".to_string(),
        event_type: None,
        trace_id: None,
        context_json: None,
    };
    let brokered = crate::credential_broker::broker_observed_credential(&observation).unwrap();
    let resolved = crate::credential_broker::resolve_broker_reference_for_provider(
        crate::credential_broker::CredentialProvider::Mcp,
        &brokered.credential_ref,
    )
    .unwrap();
    assert_eq!(resolved.as_deref(), Some("oauth-access-token"));
}

#[test]
fn build_profile_server_list_uses_profile_manual_servers_only() {
    let profile = McpUserConfig {
        servers: vec![McpManualServer {
            name: "profile-api".into(),
            url: "https://profile.example/mcp".into(),
            headers: HashMap::new(),
            auth: None,
            enabled: true,
        }],
        ..Default::default()
    };

    let list = build_profile_server_list(&profile, None, HashMap::new());

    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "profile-api");
    assert_eq!(list[0].source, "profile");
}

#[test]
fn build_profile_server_list_respects_local_builtin_enablement() {
    let dir = tempfile::tempdir().unwrap();
    let builtin = dir.path().join("capsem-mcp-builtin");
    std::fs::write(&builtin, "#!/bin/sh\n").unwrap();

    let mut enabled = HashMap::new();
    enabled.insert("local".to_string(), false);
    let profile = McpUserConfig {
        server_enabled: enabled,
        ..Default::default()
    };

    let list = build_profile_server_list(&profile, Some(&builtin), HashMap::new());

    let local = list.iter().find(|server| server.name == "local").unwrap();
    assert_eq!(local.source, "builtin");
    assert!(!local.enabled);
}

#[test]
fn build_profile_server_list_rejects_names_with_separator() {
    let mut profile = McpUserConfig::default();
    profile.servers.push(crate::mcp::policy::McpManualServer {
        name: "bad__name".to_string(),
        url: "http://localhost".to_string(),
        headers: HashMap::new(),
        auth: None,
        enabled: true,
    });
    profile.servers.push(crate::mcp::policy::McpManualServer {
        name: "goodname".to_string(),
        url: "http://localhost".to_string(),
        headers: HashMap::new(),
        auth: None,
        enabled: true,
    });

    let servers = build_profile_server_list(&profile, None, HashMap::new());
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
    let doc: toml::Value =
        toml::from_str(&text).unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
    doc.get("bin")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.get("name").and_then(|n| n.as_str()).map(String::from))
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

    let template = std::fs::read_to_string(root.join("config/docker/Dockerfile.rootfs.j2"))
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

    let justfile = std::fs::read_to_string(root.join("justfile")).expect("cannot read justfile");

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

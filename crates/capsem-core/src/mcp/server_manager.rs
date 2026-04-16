//! Manages host-side MCP server connections via rmcp.
//!
//! Supports two transport types:
//! - HTTP: Streamable HTTP endpoint via `StreamableHttpClientTransport`
//! - Stdio: Subprocess via `TokioChildProcess` (for local/builtin servers)

use std::collections::HashMap;

use anyhow::{Context, Result};
use rmcp::model::{
    CallToolRequestParams, GetPromptRequestParams, ReadResourceRequestParams,
};
use rmcp::service::{Peer, RunningService};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::{RoleClient, ServiceExt};
use tracing::{debug, info, warn};

use super::types::*;

/// A connected host-side MCP server backed by rmcp.
struct RunningServer {
    client: RunningService<RoleClient, ()>,
}

/// Manages host-side MCP server connections and provides a unified tool catalog.
pub struct McpServerManager {
    definitions: Vec<McpServerDef>,
    running: HashMap<String, RunningServer>,
    http_client: reqwest::Client,
    // Unified, namespaced catalogs
    tool_catalog: Vec<McpToolDef>,
    resource_catalog: Vec<McpResourceDef>,
    prompt_catalog: Vec<McpPromptDef>,
    // Routing maps
    tool_routing: HashMap<String, String>,     // namespaced_name -> server_name
    resource_routing: HashMap<String, String>,  // namespaced_uri -> server_name
    prompt_routing: HashMap<String, String>,    // namespaced_name -> server_name
}

impl McpServerManager {
    pub fn new(defs: Vec<McpServerDef>, http_client: reqwest::Client) -> Self {
        Self {
            definitions: defs,
            running: HashMap::new(),
            http_client,
            tool_catalog: Vec::new(),
            resource_catalog: Vec::new(),
            prompt_catalog: Vec::new(),
            tool_routing: HashMap::new(),
            resource_routing: HashMap::new(),
            prompt_routing: HashMap::new(),
        }
    }

    /// Connect to all enabled servers (HTTP and stdio), run MCP handshake,
    /// then query each to build the unified catalog.
    pub async fn initialize_all(&mut self) -> Result<()> {
        let defs: Vec<McpServerDef> = self
            .definitions
            .iter()
            .filter(|d| d.enabled)
            .cloned()
            .collect();

        for def in &defs {
            match self.connect_and_initialize(def).await {
                Ok(()) => {
                    let transport = if def.is_stdio() { "stdio" } else { "http" };
                    info!(server = %def.name, transport, "MCP server initialized");
                }
                Err(e) => {
                    warn!(server = %def.name, error = %e, "failed to initialize MCP server");
                }
            }
        }

        info!(
            tools = self.tool_catalog.len(),
            resources = self.resource_catalog.len(),
            prompts = self.prompt_catalog.len(),
            servers = self.running.len(),
            "MCP gateway catalog built"
        );
        Ok(())
    }

    /// Connect to a single server, run MCP handshake, populate catalogs.
    /// Public within the crate for testing (errors propagate, unlike initialize_all
    /// which warns and continues).
    pub(crate) async fn connect_and_initialize(&mut self, def: &McpServerDef) -> Result<()> {
        let client = if def.is_stdio() {
            self.connect_stdio(def).await?
        } else {
            self.connect_http(def).await?
        };

        // Fetch tools with automatic pagination
        match client.list_all_tools().await {
            Ok(tools) => {
                for tool in tools {
                    let name = tool.name.as_ref();
                    if name.is_empty() {
                        continue;
                    }
                    let ns_name = namespace_name(&def.name, name);

                    let annotations = tool.annotations.as_ref().map(|a| ToolAnnotations {
                        title: a.title.clone(),
                        read_only_hint: a.read_only_hint.unwrap_or(false),
                        destructive_hint: a.destructive_hint.unwrap_or(true),
                        idempotent_hint: a.idempotent_hint.unwrap_or(false),
                        open_world_hint: a.open_world_hint.unwrap_or(true),
                    });

                    let input_schema = serde_json::to_value(&*tool.input_schema)
                        .unwrap_or(serde_json::json!({}));

                    self.tool_catalog.push(McpToolDef {
                        namespaced_name: ns_name.clone(),
                        original_name: name.to_string(),
                        description: tool.description.as_ref().map(|d| d.to_string()),
                        input_schema,
                        server_name: def.name.clone(),
                        annotations,
                    });
                    self.tool_routing.insert(ns_name, def.name.clone());
                }
            }
            Err(e) => {
                warn!(server = %def.name, error = %e, "failed to list tools");
            }
        }

        // Fetch resources (optional)
        match client.list_all_resources().await {
            Ok(resources) => {
                for resource in resources {
                    let uri = resource.raw.uri.as_str();
                    if uri.is_empty() {
                        continue;
                    }
                    let ns_uri = namespace_resource_uri(&def.name, uri);
                    self.resource_catalog.push(McpResourceDef {
                        namespaced_uri: ns_uri.clone(),
                        original_uri: uri.to_string(),
                        name: Some(resource.raw.name.clone()),
                        description: resource.raw.description.clone(),
                        mime_type: resource.raw.mime_type.clone(),
                        server_name: def.name.clone(),
                    });
                    self.resource_routing.insert(ns_uri, def.name.clone());
                }
            }
            Err(e) => {
                debug!(server = %def.name, error = %e, "failed to list resources (may not be supported)");
            }
        }

        // Fetch prompts (optional)
        match client.list_all_prompts().await {
            Ok(prompts) => {
                for prompt in prompts {
                    let name = prompt.name.as_str();
                    if name.is_empty() {
                        continue;
                    }
                    let ns_name = namespace_name(&def.name, name);
                    let arguments: Vec<serde_json::Value> = prompt
                        .arguments
                        .as_ref()
                        .map(|args| args.iter().filter_map(|a| serde_json::to_value(a).ok()).collect())
                        .unwrap_or_default();
                    self.prompt_catalog.push(McpPromptDef {
                        namespaced_name: ns_name.clone(),
                        original_name: name.to_string(),
                        description: prompt.description.clone(),
                        arguments,
                        server_name: def.name.clone(),
                    });
                    self.prompt_routing.insert(ns_name, def.name.clone());
                }
            }
            Err(e) => {
                debug!(server = %def.name, error = %e, "failed to list prompts (may not be supported)");
            }
        }

        self.running.insert(def.name.clone(), RunningServer { client });
        Ok(())
    }

    /// Connect to an HTTP MCP server.
    async fn connect_http(&self, def: &McpServerDef) -> Result<RunningService<RoleClient, ()>> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(def.url.as_str());
        if let Some(ref token) = def.bearer_token {
            config = config.auth_header(token.clone());
        }
        if !def.headers.is_empty() {
            let mut headers = HashMap::new();
            for (key, val) in &def.headers {
                let name: http::header::HeaderName = key.parse()
                    .with_context(|| format!("invalid header name: {key}"))?;
                let value: http::header::HeaderValue = val.parse()
                    .with_context(|| format!("invalid header value for {key}"))?;
                headers.insert(name, value);
            }
            config = config.custom_headers(headers);
        }
        let transport = StreamableHttpClientTransport::with_client(
            self.http_client.clone(),
            config,
        );
        ().serve(transport)
            .await
            .with_context(|| format!("failed to connect to HTTP MCP server '{}'", def.name))
    }

    /// Spawn and connect to a stdio MCP server subprocess.
    async fn connect_stdio(&self, def: &McpServerDef) -> Result<RunningService<RoleClient, ()>> {
        let command = def.command.as_deref()
            .ok_or_else(|| anyhow::anyhow!("stdio server '{}' has no command", def.name))?;

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(&def.args);
        for (k, v) in &def.env {
            cmd.env(k, v);
        }

        let transport = TokioChildProcess::new(cmd)
            .with_context(|| format!("failed to spawn stdio MCP server '{}'", def.name))?;

        ().serve(transport)
            .await
            .with_context(|| format!("failed to initialize stdio MCP server '{}'", def.name))
    }

    /// Get the aggregated, namespaced tool catalog.
    pub fn tool_catalog(&self) -> &[McpToolDef] {
        &self.tool_catalog
    }

    /// Get the aggregated, namespaced resource catalog.
    pub fn resource_catalog(&self) -> &[McpResourceDef] {
        &self.resource_catalog
    }

    /// Get the aggregated, namespaced prompt catalog.
    pub fn prompt_catalog(&self) -> &[McpPromptDef] {
        &self.prompt_catalog
    }

    /// Get the server definitions.
    pub fn definitions(&self) -> &[McpServerDef] {
        &self.definitions
    }

    /// Count tools provided by a named server.
    pub fn tool_count_for_server(&self, name: &str) -> usize {
        self.tool_catalog.iter().filter(|t| t.server_name == name).count()
    }

    /// Check if a server is currently connected.
    pub fn is_running(&self, name: &str) -> bool {
        self.running.contains_key(name)
    }

    /// Look up a tool's peer and original name. Clone the peer so the caller
    /// can drop the manager lock before making the (potentially slow) RPC call.
    pub fn lookup_tool_peer(&self, namespaced_name: &str) -> Result<(Peer<RoleClient>, String)> {
        let (server_name, original_name) = parse_namespaced(namespaced_name)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced tool name: {namespaced_name}"))?;
        let server = self
            .running
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;
        Ok((server.client.peer().clone(), original_name.to_string()))
    }

    /// Look up a resource's peer and original URI.
    pub fn lookup_resource_peer(&self, namespaced_uri: &str) -> Result<(Peer<RoleClient>, String)> {
        let (server_name, original_uri) = parse_resource_uri(namespaced_uri)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced resource URI: {namespaced_uri}"))?;
        let server = self
            .running
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;
        Ok((server.client.peer().clone(), original_uri.to_string()))
    }

    /// Look up a prompt's peer and original name.
    pub fn lookup_prompt_peer(&self, namespaced_name: &str) -> Result<(Peer<RoleClient>, String)> {
        let (server_name, original_name) = parse_namespaced(namespaced_name)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced prompt name: {namespaced_name}"))?;
        let server = self
            .running
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;
        Ok((server.client.peer().clone(), original_name.to_string()))
    }

    /// Route a tools/call: parse namespace, strip prefix, forward to server.
    pub async fn call_tool(
        &self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<JsonRpcResponse> {
        let (peer, original_name) = self.lookup_tool_peer(namespaced_name)?;

        let args: Option<serde_json::Map<String, serde_json::Value>> = match arguments {
            serde_json::Value::Object(map) if !map.is_empty() => Some(map),
            _ => None,
        };

        let mut params = CallToolRequestParams::new(original_name.clone());
        if let Some(args) = args {
            params = params.with_arguments(args);
        }

        let result = peer.call_tool(params).await
            .with_context(|| format!("tool call '{}' failed", original_name))?;

        let result_json = serde_json::to_value(&result)
            .context("failed to serialize tool result")?;
        Ok(JsonRpcResponse::ok(None, result_json))
    }

    /// Route a resources/read: parse namespaced URI, forward to server.
    pub async fn read_resource(&self, namespaced_uri: &str) -> Result<JsonRpcResponse> {
        let (peer, original_uri) = self.lookup_resource_peer(namespaced_uri)?;

        let params = ReadResourceRequestParams::new(original_uri.clone());
        let result = peer.read_resource(params).await
            .with_context(|| format!("resource read '{}' failed", original_uri))?;

        let result_json = serde_json::to_value(&result)
            .context("failed to serialize resource result")?;
        Ok(JsonRpcResponse::ok(None, result_json))
    }

    /// Route a prompts/get: parse namespace, forward to server.
    pub async fn get_prompt(
        &self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<JsonRpcResponse> {
        let (peer, original_name) = self.lookup_prompt_peer(namespaced_name)?;

        let mut params = GetPromptRequestParams::new(original_name.clone());
        if let serde_json::Value::Object(map) = arguments {
            if !map.is_empty() {
                params = params.with_arguments(map);
            }
        }

        let result = peer.get_prompt(params).await
            .with_context(|| format!("prompt get '{}' failed", original_name))?;

        let result_json = serde_json::to_value(&result)
            .context("failed to serialize prompt result")?;
        Ok(JsonRpcResponse::ok(None, result_json))
    }

    /// Shut down all server connections.
    pub async fn shutdown_all(&mut self) {
        for (name, server) in self.running.drain() {
            debug!(server = %name, "disconnecting MCP server");
            if let Err(e) = server.client.cancel().await {
                warn!(server = %name, error = %e, "error cancelling MCP server");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_server_def() -> McpServerDef {
        McpServerDef {
            name: "test".to_string(),
            url: "https://mcp.example.com/v1".to_string(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
            source: "test".to_string(),
            command: None,
            args: vec![],
            env: HashMap::new(),
        }
    }

    #[test]
    fn new_manager_has_empty_catalogs() {
        let mgr = McpServerManager::new(vec![test_server_def()], reqwest::Client::new());
        assert!(mgr.tool_catalog().is_empty());
        assert!(mgr.resource_catalog().is_empty());
        assert!(mgr.prompt_catalog().is_empty());
        assert_eq!(mgr.definitions().len(), 1);
    }

    #[test]
    fn disabled_server_definition_stored() {
        let mut def = test_server_def();
        def.enabled = false;
        let mgr = McpServerManager::new(vec![def], reqwest::Client::new());
        assert_eq!(mgr.definitions().len(), 1);
        assert!(!mgr.definitions()[0].enabled);
    }

    #[test]
    fn stdio_server_stored() {
        let mut def = test_server_def();
        def.command = Some("/usr/bin/my-mcp-server".to_string());
        let mgr = McpServerManager::new(vec![def], reqwest::Client::new());
        assert_eq!(mgr.definitions().len(), 1);
        assert!(mgr.definitions()[0].is_stdio());
    }

    #[test]
    fn tool_count_for_server_empty() {
        let mgr = McpServerManager::new(vec![test_server_def()], reqwest::Client::new());
        assert_eq!(mgr.tool_count_for_server("test"), 0);
    }

    #[test]
    fn tool_count_for_server_nonexistent() {
        let mgr = McpServerManager::new(vec![], reqwest::Client::new());
        assert_eq!(mgr.tool_count_for_server("nonexistent"), 0);
    }

    #[tokio::test]
    async fn call_tool_unknown_server_errors() {
        let mgr = McpServerManager::new(vec![], reqwest::Client::new());
        let result = mgr.call_tool("unknown__tool", serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not running"));
    }

    #[tokio::test]
    async fn call_tool_no_separator_errors() {
        let mgr = McpServerManager::new(vec![], reqwest::Client::new());
        let result = mgr.call_tool("noseparator", serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid namespaced"));
    }

    #[tokio::test]
    async fn read_resource_invalid_uri_errors() {
        let mgr = McpServerManager::new(vec![], reqwest::Client::new());
        let result = mgr.read_resource("http://invalid").await;
        assert!(result.is_err());
    }

    #[test]
    fn lookup_tool_peer_unknown_server_errors() {
        let mgr = McpServerManager::new(vec![], reqwest::Client::new());
        let result = mgr.lookup_tool_peer("unknown__tool");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not running"));
    }

    #[test]
    fn lookup_tool_peer_no_separator_errors() {
        let mgr = McpServerManager::new(vec![], reqwest::Client::new());
        let result = mgr.lookup_tool_peer("noseparator");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid namespaced"));
    }

    /// Live integration test against DeepWiki's public MCP server (no auth).
    /// Uses connect_and_initialize directly so errors propagate instead of
    /// being silently swallowed by initialize_all's warn-and-continue logic.
    #[tokio::test]
    async fn integration_live_mcp_server() {
        let def = McpServerDef {
            name: "deepwiki".to_string(),
            url: "https://mcp.deepwiki.com/mcp".to_string(),
            headers: HashMap::new(),
            bearer_token: None,
            enabled: true,
            source: "test".to_string(),
            command: None,
            args: vec![],
            env: HashMap::new(),
        };
        let mut mgr = McpServerManager::new(vec![def.clone()], reqwest::Client::new());
        // Call connect_and_initialize directly -- errors surface immediately
        // instead of being silently logged by initialize_all.
        mgr.connect_and_initialize(&def)
            .await
            .expect("failed to connect to DeepWiki MCP server");

        assert!(mgr.is_running("deepwiki"), "server should be running after successful init");
        assert!(
            mgr.tool_count_for_server("deepwiki") > 0,
            "DeepWiki should expose at least one tool, got catalog: {:?}",
            mgr.tool_catalog()
        );
    }

    /// Live integration test that connects to all HTTP MCP servers from the
    /// developer's config (user.toml manual servers + auto-detected from
    /// ~/.claude/settings.json and ~/.gemini/settings.json). Skips if none found.
    /// Covers bearer_token auth, custom headers, and multi-server catalog building.
    #[tokio::test]
    async fn integration_live_configured_mcp_servers() {
        use crate::net::policy_config::{load_settings_file, user_config_path};
        use crate::mcp::build_server_list;
        use crate::mcp::policy::McpUserConfig;

        let user_mcp = user_config_path()
            .and_then(|p| load_settings_file(&p).ok())
            .and_then(|f| f.mcp)
            .unwrap_or_default();
        let corp_mcp = McpUserConfig::default();

        let servers = build_server_list(&user_mcp, &corp_mcp);
        let http_servers: Vec<_> = servers
            .iter()
            .filter(|s| s.enabled && !s.is_stdio())
            .collect();

        if http_servers.is_empty() {
            eprintln!("no HTTP MCP servers configured, skipping");
            return;
        }

        let mut mgr = McpServerManager::new(
            http_servers.iter().map(|s| (*s).clone()).collect(),
            reqwest::Client::new(),
        );

        for def in &http_servers {
            match mgr.connect_and_initialize(def).await {
                Ok(()) => {
                    assert!(
                        mgr.is_running(&def.name),
                        "server '{}' should be running after init",
                        def.name,
                    );
                    assert!(
                        mgr.tool_count_for_server(&def.name) > 0,
                        "server '{}' should expose at least one tool, got catalog: {:?}",
                        def.name,
                        mgr.tool_catalog(),
                    );
                }
                Err(e) => {
                    panic!(
                        "failed to connect to configured MCP server '{}' (url={}): {e:#}",
                        def.name, def.url,
                    );
                }
            }
        }
    }
}

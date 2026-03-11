//! Manages host-side MCP server connections via rmcp.
//!
//! Each server is a Streamable HTTP endpoint. The manager connects via rmcp's
//! `StreamableHttpClientTransport`, which handles JSON-RPC, SSE, and session
//! lifecycle internally. We just connect, query catalogs, and route calls.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rmcp::model::{
    CallToolRequestParams, GetPromptRequestParams, ReadResourceRequestParams,
};
use rmcp::service::RunningService;
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
    pub fn new(defs: Vec<McpServerDef>) -> Self {
        Self {
            definitions: defs,
            running: HashMap::new(),
            tool_catalog: Vec::new(),
            resource_catalog: Vec::new(),
            prompt_catalog: Vec::new(),
            tool_routing: HashMap::new(),
            resource_routing: HashMap::new(),
            prompt_routing: HashMap::new(),
        }
    }

    /// Connect to all enabled HTTP servers, run MCP initialize handshake,
    /// then query each to build the unified catalog.
    pub async fn initialize_all(&mut self) -> Result<()> {
        let defs: Vec<McpServerDef> = self
            .definitions
            .iter()
            .filter(|d| d.enabled && !d.unsupported_stdio)
            .cloned()
            .collect();

        for def in &defs {
            match self.connect_and_initialize(def).await {
                Ok(()) => {
                    info!(server = %def.name, "MCP server initialized");
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
        // Build transport config
        let mut config = StreamableHttpClientTransportConfig::with_uri(def.url.as_str());
        if let Some(ref token) = def.bearer_token {
            config = config.auth_header(format!("Bearer {token}"));
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
            reqwest::Client::new(),
            config,
        );

        // rmcp's serve() does the full MCP handshake (initialize + notifications/initialized)
        let client = ().serve(transport)
            .await
            .with_context(|| format!("failed to connect to MCP server '{}'", def.name))?;

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

    /// Route a tools/call: parse namespace, strip prefix, forward to server.
    pub async fn call_tool(
        &mut self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<JsonRpcResponse> {
        let (server_name, original_name) = parse_namespaced(namespaced_name)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced tool name: {namespaced_name}"))?;

        let server = self
            .running
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;

        let args: Option<serde_json::Map<String, serde_json::Value>> = match arguments {
            serde_json::Value::Object(map) if !map.is_empty() => Some(map),
            _ => None,
        };

        let mut params = CallToolRequestParams::new(original_name.to_string());
        if let Some(args) = args {
            params = params.with_arguments(args);
        }

        let result = server.client.call_tool(params).await
            .with_context(|| format!("tool call '{}' on server '{}' failed", original_name, server_name))?;

        let result_json = serde_json::to_value(&result)
            .context("failed to serialize tool result")?;
        Ok(JsonRpcResponse::ok(None, result_json))
    }

    /// Route a resources/read: parse namespaced URI, forward to server.
    pub async fn read_resource(&mut self, namespaced_uri: &str) -> Result<JsonRpcResponse> {
        let (server_name, original_uri) = parse_resource_uri(namespaced_uri)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced resource URI: {namespaced_uri}"))?;

        let server = self
            .running
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;

        let params = ReadResourceRequestParams::new(original_uri);
        let result = server.client.read_resource(params).await
            .with_context(|| format!("resource read '{}' on server '{}' failed", original_uri, server_name))?;

        let result_json = serde_json::to_value(&result)
            .context("failed to serialize resource result")?;
        Ok(JsonRpcResponse::ok(None, result_json))
    }

    /// Route a prompts/get: parse namespace, forward to server.
    pub async fn get_prompt(
        &mut self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<JsonRpcResponse> {
        let (server_name, original_name) = parse_namespaced(namespaced_name)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced prompt name: {namespaced_name}"))?;

        let server = self
            .running
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;

        let mut params = GetPromptRequestParams::new(original_name.to_string());
        if let serde_json::Value::Object(map) = arguments {
            if !map.is_empty() {
                params = params.with_arguments(map);
            }
        }

        let result = server.client.get_prompt(params).await
            .with_context(|| format!("prompt get '{}' on server '{}' failed", original_name, server_name))?;

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
            unsupported_stdio: false,
        }
    }

    #[test]
    fn new_manager_has_empty_catalogs() {
        let mgr = McpServerManager::new(vec![test_server_def()]);
        assert!(mgr.tool_catalog().is_empty());
        assert!(mgr.resource_catalog().is_empty());
        assert!(mgr.prompt_catalog().is_empty());
        assert_eq!(mgr.definitions().len(), 1);
    }

    #[test]
    fn disabled_server_definition_stored() {
        let mut def = test_server_def();
        def.enabled = false;
        let mgr = McpServerManager::new(vec![def]);
        assert_eq!(mgr.definitions().len(), 1);
        assert!(!mgr.definitions()[0].enabled);
    }

    #[test]
    fn unsupported_stdio_server_stored() {
        let mut def = test_server_def();
        def.unsupported_stdio = true;
        let mgr = McpServerManager::new(vec![def]);
        assert_eq!(mgr.definitions().len(), 1);
        assert!(mgr.definitions()[0].unsupported_stdio);
    }

    #[test]
    fn tool_count_for_server_empty() {
        let mgr = McpServerManager::new(vec![test_server_def()]);
        assert_eq!(mgr.tool_count_for_server("test"), 0);
    }

    #[test]
    fn tool_count_for_server_nonexistent() {
        let mgr = McpServerManager::new(vec![]);
        assert_eq!(mgr.tool_count_for_server("nonexistent"), 0);
    }

    #[tokio::test]
    async fn call_tool_unknown_server_errors() {
        let mut mgr = McpServerManager::new(vec![]);
        let result = mgr.call_tool("unknown__tool", serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not running"));
    }

    #[tokio::test]
    async fn call_tool_no_separator_errors() {
        let mut mgr = McpServerManager::new(vec![]);
        let result = mgr.call_tool("noseparator", serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid namespaced"));
    }

    #[tokio::test]
    async fn read_resource_invalid_uri_errors() {
        let mut mgr = McpServerManager::new(vec![]);
        let result = mgr.read_resource("http://invalid").await;
        assert!(result.is_err());
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
            unsupported_stdio: false,
        };
        let mut mgr = McpServerManager::new(vec![def.clone()]);
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
}

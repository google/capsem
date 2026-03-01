//! Manages host-side MCP server processes.
//!
//! Each server is spawned on demand and kept alive for reuse. The manager
//! maintains a unified, namespaced tool/resource/prompt catalog aggregated
//! from all servers.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use tokio::io::BufReader;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::{debug, info, warn};

use super::stdio_bridge;
use super::types::*;

/// A running host-side MCP server process.
struct RunningServer {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

impl RunningServer {
    /// Send a JSON-RPC request and read the response.
    async fn call(&mut self, method: &str, params: Option<serde_json::Value>) -> Result<JsonRpcResponse> {
        let id = self.next_id;
        self.next_id += 1;

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(id)),
            method: method.into(),
            params,
        };

        stdio_bridge::write_request(&mut self.stdin, &req)
            .await
            .context("failed to write to MCP server stdin")?;

        // Read response lines until we get one with a matching id.
        // Skip notifications (no id) sent by the server.
        loop {
            let resp = stdio_bridge::read_response(&mut self.reader)
                .await
                .context("failed to read from MCP server stdout")?;

            match resp {
                Some(r) => {
                    // Check if this is a response (has matching id) vs a notification
                    if r.id.is_some() {
                        return Ok(r);
                    }
                    // Otherwise it's a server-initiated notification, skip it
                    debug!(method, "skipping server notification while waiting for response");
                }
                None => bail!("MCP server closed stdout unexpectedly"),
            }
        }
    }

    /// Send a notification (no id, no response expected).
    async fn notify(&mut self, method: &str, params: Option<serde_json::Value>) -> Result<()> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: method.into(),
            params,
        };
        stdio_bridge::write_request(&mut self.stdin, &req)
            .await
            .context("failed to write notification to MCP server")
    }
}

/// Manages host-side MCP server processes and provides a unified tool catalog.
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

    /// Spawn all enabled servers, run MCP initialize handshake,
    /// then call tools/list on each to build the unified catalog.
    pub async fn initialize_all(&mut self) -> Result<()> {
        let defs: Vec<McpServerDef> = self
            .definitions
            .iter()
            .filter(|d| d.enabled)
            .cloned()
            .collect();

        for def in &defs {
            match self.spawn_and_initialize(def).await {
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

    async fn spawn_and_initialize(&mut self, def: &McpServerDef) -> Result<()> {
        let mut cmd = Command::new(&def.command);
        cmd.args(&def.args);
        cmd.envs(&def.env);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().context("failed to spawn MCP server")?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let mut server = RunningServer {
            child,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
        };

        // MCP initialize handshake
        let init_params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "capsem",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        let init_resp = server.call("initialize", Some(init_params)).await?;
        if init_resp.error.is_some() {
            bail!(
                "MCP server {} rejected initialize: {:?}",
                def.name,
                init_resp.error
            );
        }

        // Send initialized notification
        server.notify("notifications/initialized", None).await?;

        // Fetch tools
        let tools_resp = server.call("tools/list", None).await?;
        if let Some(result) = &tools_resp.result {
            if let Some(tools) = result.get("tools").and_then(|t| t.as_array()) {
                for tool in tools {
                    let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    let ns_name = namespace_name(&def.name, name);
                    self.tool_catalog.push(McpToolDef {
                        namespaced_name: ns_name.clone(),
                        original_name: name.to_string(),
                        description: tool
                            .get("description")
                            .and_then(|d| d.as_str())
                            .map(String::from),
                        input_schema: tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or(serde_json::json!({})),
                        server_name: def.name.clone(),
                    });
                    self.tool_routing.insert(ns_name, def.name.clone());
                }
            }
        }

        // Fetch resources (optional, server may not support)
        if let Ok(res_resp) = server.call("resources/list", None).await {
            if let Some(result) = &res_resp.result {
                if let Some(resources) = result.get("resources").and_then(|r| r.as_array()) {
                    for resource in resources {
                        let uri = resource.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                        if uri.is_empty() {
                            continue;
                        }
                        let ns_uri = namespace_resource_uri(&def.name, uri);
                        self.resource_catalog.push(McpResourceDef {
                            namespaced_uri: ns_uri.clone(),
                            original_uri: uri.to_string(),
                            name: resource
                                .get("name")
                                .and_then(|n| n.as_str())
                                .map(String::from),
                            description: resource
                                .get("description")
                                .and_then(|d| d.as_str())
                                .map(String::from),
                            mime_type: resource
                                .get("mimeType")
                                .and_then(|m| m.as_str())
                                .map(String::from),
                            server_name: def.name.clone(),
                        });
                        self.resource_routing.insert(ns_uri, def.name.clone());
                    }
                }
            }
        }

        // Fetch prompts (optional, server may not support)
        if let Ok(prompt_resp) = server.call("prompts/list", None).await {
            if let Some(result) = &prompt_resp.result {
                if let Some(prompts) = result.get("prompts").and_then(|p| p.as_array()) {
                    for prompt in prompts {
                        let name = prompt.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        if name.is_empty() {
                            continue;
                        }
                        let ns_name = namespace_name(&def.name, name);
                        self.prompt_catalog.push(McpPromptDef {
                            namespaced_name: ns_name.clone(),
                            original_name: name.to_string(),
                            description: prompt
                                .get("description")
                                .and_then(|d| d.as_str())
                                .map(String::from),
                            arguments: prompt
                                .get("arguments")
                                .and_then(|a| a.as_array())
                                .cloned()
                                .unwrap_or_default(),
                            server_name: def.name.clone(),
                        });
                        self.prompt_routing.insert(ns_name, def.name.clone());
                    }
                }
            }
        }

        self.running.insert(def.name.clone(), server);
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

    /// Check if a server is currently running.
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
            .get_mut(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;

        let params = serde_json::json!({
            "name": original_name,
            "arguments": arguments,
        });
        server.call("tools/call", Some(params)).await
    }

    /// Route a resources/read: parse namespaced URI, forward to server.
    pub async fn read_resource(&mut self, namespaced_uri: &str) -> Result<JsonRpcResponse> {
        let (server_name, original_uri) = parse_resource_uri(namespaced_uri)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced resource URI: {namespaced_uri}"))?;

        let server = self
            .running
            .get_mut(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;

        let params = serde_json::json!({"uri": original_uri});
        server.call("resources/read", Some(params)).await
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
            .get_mut(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;

        let params = serde_json::json!({
            "name": original_name,
            "arguments": arguments,
        });
        server.call("prompts/get", Some(params)).await
    }

    /// Forward an arbitrary JSON-RPC request to a named server.
    pub async fn forward(
        &mut self,
        server_name: &str,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse> {
        let server = self
            .running
            .get_mut(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;
        server.call(method, params).await
    }

    /// Shut down all running servers.
    pub async fn shutdown_all(&mut self) {
        for (name, mut server) in self.running.drain() {
            debug!(server = %name, "shutting down MCP server");
            let _ = server.child.kill().await;
        }
    }
}

impl Drop for McpServerManager {
    fn drop(&mut self) {
        // Best-effort kill all child processes synchronously
        for (name, mut server) in self.running.drain() {
            debug!(server = %name, "killing MCP server on drop");
            let _ = server.child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_server_def() -> McpServerDef {
        // We can't easily test with a real MCP server in unit tests,
        // but we can test the catalog/routing logic.
        McpServerDef {
            name: "test".to_string(),
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
            enabled: true,
            source: "test".to_string(),
        }
    }

    #[test]
    fn new_manager_has_empty_catalogs() {
        let mgr = McpServerManager::new(vec![echo_server_def()]);
        assert!(mgr.tool_catalog().is_empty());
        assert!(mgr.resource_catalog().is_empty());
        assert!(mgr.prompt_catalog().is_empty());
        assert_eq!(mgr.definitions().len(), 1);
    }

    #[test]
    fn disabled_server_definition_stored() {
        let mut def = echo_server_def();
        def.enabled = false;
        let mgr = McpServerManager::new(vec![def]);
        assert_eq!(mgr.definitions().len(), 1);
        assert!(!mgr.definitions()[0].enabled);
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
}

//! Manages host-side MCP server connections via rmcp.
//!
//! Supports two transport types:
//! - HTTP: Streamable HTTP endpoint via `StreamableHttpClientTransport`
//! - Stdio: Subprocess via `TokioChildProcess` (for local/builtin servers)

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use rmcp::model::{CallToolRequestParams, GetPromptRequestParams, ReadResourceRequestParams};
use rmcp::service::{Peer, RunningService};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::{RoleClient, ServiceExt};
use tracing::{debug, info, warn};

use super::types::*;

/// One rmcp client connection. For stdio-pool servers, the manager keeps
/// several of these in a `ServerPool`.
struct RunningServer {
    client: RunningService<RoleClient, ()>,
}

/// A pool of one-or-more rmcp connections to a single MCP server. Stdio
/// servers may run with `peers.len() > 1` to bypass rmcp's
/// per-`RunningService` mpsc → driver-task → stdin funnel; HTTP servers
/// always use a single peer (HTTP/2 multiplexes natively).
///
/// `next` round-robins across `peers`, but only for tools whose original
/// (post-namespace-strip) name appears in `pool_safe_tools`. Tools NOT in
/// that allowlist pin to `peers[0]` so per-process state (e.g. the
/// builtin's `Arc<Mutex<AutoSnapshotScheduler>>`) stays consistent.
struct ServerPool {
    peers: Vec<RunningServer>,
    next: AtomicUsize,
    pool_safe_tools: HashSet<String>,
}

impl ServerPool {
    /// Pick a peer for a call to `original_tool_or_uri`. If the name is
    /// in the safe list, round-robin; otherwise pin to `peers[0]`.
    fn pick(&self, original: &str) -> &RunningServer {
        let is_safe = self.pool_safe_tools.contains(original);
        let idx = next_peer_index(self.peers.len(), is_safe, &self.next);
        &self.peers[idx]
    }
}

/// Round-robin picker, separated for unit-testability without rmcp.
/// Returns 0 unless the pool has > 1 peer AND the tool is pool-safe.
fn next_peer_index(peer_count: usize, is_pool_safe: bool, counter: &AtomicUsize) -> usize {
    if peer_count <= 1 || !is_pool_safe {
        return 0;
    }
    counter.fetch_add(1, Ordering::Relaxed) % peer_count
}

/// Manages host-side MCP server connections and provides a unified tool catalog.
pub struct McpServerManager {
    definitions: Vec<McpServerDef>,
    running: HashMap<String, ServerPool>,
    http_client: reqwest::Client,
    // Unified, namespaced catalogs
    tool_catalog: Vec<McpToolDef>,
    resource_catalog: Vec<McpResourceDef>,
    prompt_catalog: Vec<McpPromptDef>,
    // Routing maps
    tool_routing: HashMap<String, String>, // namespaced_name -> server_name
    resource_routing: HashMap<String, String>, // namespaced_uri -> server_name
    prompt_routing: HashMap<String, String>, // namespaced_name -> server_name
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
            "MCP aggregator catalog built"
        );
        Ok(())
    }

    /// Connect to a single server, run MCP handshake, populate catalogs.
    /// Public within the crate for testing (errors propagate, unlike initialize_all
    /// which warns and continues).
    ///
    /// For stdio servers with `pool_size > 1`, spawns N independent
    /// subprocess clients. Catalog discovery happens against `peers[0]`
    /// only; subsequent peers are pure dispatch backends so we don't pay
    /// the `tools/list`/`resources/list`/`prompts/list` cost N times.
    pub(crate) async fn connect_and_initialize(&mut self, def: &McpServerDef) -> Result<()> {
        // Stdio servers can be pooled; HTTP transports already multiplex
        // via HTTP/2 so additional peers buy nothing at the transport
        // level. Clamp `pool_size` to ≥ 1 (and to 1 for HTTP).
        let requested_pool = def.pool_size.unwrap_or(1).max(1) as usize;
        let pool_size = if def.is_stdio() { requested_pool } else { 1 };

        let client = if def.is_stdio() {
            self.connect_stdio(def, 0).await?
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

                    let input_schema =
                        serde_json::to_value(&*tool.input_schema).unwrap_or(serde_json::json!({}));

                    self.tool_catalog.push(McpToolDef {
                        namespaced_name: ns_name.clone(),
                        original_name: name.to_string(),
                        description: tool.description.as_ref().map(|d| d.to_string()),
                        input_schema,
                        server_name: def.name.clone(),
                        annotations,
                        timeout_secs: None,
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
                        .map(|args| {
                            args.iter()
                                .filter_map(|a| serde_json::to_value(a).ok())
                                .collect()
                        })
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

        // Catalog discovery is done. Build the pool: peers[0] is the
        // already-connected `client`; peers[1..pool_size] are spawned now
        // for stdio servers. We DON'T list-tools again on the additional
        // peers — they expose the same catalog by construction (same
        // binary, same env), so re-querying just costs latency.
        let mut peers = Vec::with_capacity(pool_size);
        peers.push(RunningServer { client });
        for i in 1..pool_size {
            match self.connect_stdio(def, i as u32).await {
                Ok(extra) => peers.push(RunningServer { client: extra }),
                Err(e) => {
                    warn!(
                        server = %def.name,
                        peer_index = i,
                        error = %e,
                        "failed to spawn additional pool peer; continuing with smaller pool"
                    );
                    break;
                }
            }
        }

        if pool_size > 1 {
            info!(
                server = %def.name,
                pool_size = peers.len(),
                pool_safe_tools = def.pool_safe_tools.len(),
                "MCP server pool initialized"
            );
        }

        let pool = ServerPool {
            peers,
            next: AtomicUsize::new(0),
            pool_safe_tools: def.pool_safe_tools.iter().cloned().collect(),
        };
        self.running.insert(def.name.clone(), pool);
        Ok(())
    }

    /// Connect to an HTTP MCP server.
    async fn connect_http(&self, def: &McpServerDef) -> Result<RunningService<RoleClient, ()>> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(def.url.as_str());
        if let Some(auth) = &def.auth {
            let token = crate::credential_broker::resolve_broker_reference_for_provider(
                crate::credential_broker::CredentialProvider::Mcp,
                &auth.credential_ref,
            )
            .map_err(|error| anyhow::anyhow!(error))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "MCP auth credential reference could not be resolved for server '{}'",
                    def.name
                )
            })?;
            config = config.auth_header(token);
        }
        if !def.headers.is_empty() {
            let mut headers = HashMap::new();
            for (key, val) in &def.headers {
                let name: http::header::HeaderName = key
                    .parse()
                    .with_context(|| format!("invalid header name: {key}"))?;
                let value: http::header::HeaderValue = val
                    .parse()
                    .with_context(|| format!("invalid header value for {key}"))?;
                headers.insert(name, value);
            }
            config = config.custom_headers(headers);
        }
        let transport =
            StreamableHttpClientTransport::with_client(self.http_client.clone(), config);
        ().serve(transport)
            .await
            .with_context(|| format!("failed to connect to HTTP MCP server '{}'", def.name))
    }

    /// Spawn and connect to a stdio MCP server subprocess.
    ///
    /// `peer_index` distinguishes pool members (0 = primary, 1..N =
    /// secondaries). Forwarded as `CAPSEM_BUILTIN_PEER_INDEX` so the
    /// builtin can pick a per-peer lockfile and avoid the singleton
    /// guard's "another instance holds the lock; exiting 0" path.
    async fn connect_stdio(
        &self,
        def: &McpServerDef,
        peer_index: u32,
    ) -> Result<RunningService<RoleClient, ()>> {
        let command = def
            .command
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("stdio server '{}' has no command", def.name))?;

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(&def.args);
        for (k, v) in &def.env {
            cmd.env(k, v);
        }
        cmd.env("CAPSEM_PARENT_PID", std::process::id().to_string());
        cmd.env("CAPSEM_BUILTIN_PEER_INDEX", peer_index.to_string());

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
        self.tool_catalog
            .iter()
            .filter(|t| t.server_name == name)
            .count()
    }

    /// Check if a server is currently connected.
    pub fn is_running(&self, name: &str) -> bool {
        self.running.contains_key(name)
    }

    /// Look up a tool's peer and original name. Clone the peer so the caller
    /// can drop the manager lock before making the (potentially slow) RPC call.
    ///
    /// For pooled servers, the peer is round-robin-picked across the pool
    /// when the original name is in `pool_safe_tools`; otherwise it pins
    /// to `peers[0]`.
    pub fn lookup_tool_peer(&self, namespaced_name: &str) -> Result<(Peer<RoleClient>, String)> {
        let (server_name, original_name) = parse_namespaced(namespaced_name)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced tool name: {namespaced_name}"))?;
        let pool = self
            .running
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;
        let server = pool.pick(original_name);
        Ok((server.client.peer().clone(), original_name.to_string()))
    }

    /// Look up a resource's peer and original URI. Resource URIs are not
    /// pool-routed (pool_safe_tools applies to tool names only); they pin
    /// to `peers[0]`.
    pub fn lookup_resource_peer(&self, namespaced_uri: &str) -> Result<(Peer<RoleClient>, String)> {
        let (server_name, original_uri) = parse_resource_uri(namespaced_uri)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced resource URI: {namespaced_uri}"))?;
        let pool = self
            .running
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;
        let server = &pool.peers[0];
        Ok((server.client.peer().clone(), original_uri.to_string()))
    }

    /// Look up a prompt's peer and original name. Prompts pin to
    /// `peers[0]` (pool routing applies to tool calls only).
    pub fn lookup_prompt_peer(&self, namespaced_name: &str) -> Result<(Peer<RoleClient>, String)> {
        let (server_name, original_name) = parse_namespaced(namespaced_name)
            .ok_or_else(|| anyhow::anyhow!("invalid namespaced prompt name: {namespaced_name}"))?;
        let pool = self
            .running
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server not running: {server_name}"))?;
        let server = &pool.peers[0];
        Ok((server.client.peer().clone(), original_name.to_string()))
    }

    /// Route a tools/call: parse namespace, strip prefix, forward to server.
    pub async fn call_tool(
        &self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<JsonRpcResponse> {
        self.dispatch_call_tool(namespaced_name, arguments)?.await
    }

    /// Route a resources/read: parse namespaced URI, forward to server.
    pub async fn read_resource(&self, namespaced_uri: &str) -> Result<JsonRpcResponse> {
        self.dispatch_read_resource(namespaced_uri)?.await
    }

    /// Route a prompts/get: parse namespace, forward to server.
    pub async fn get_prompt(
        &self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<JsonRpcResponse> {
        self.dispatch_get_prompt(namespaced_name, arguments)?.await
    }

    /// Resolve a tools/call to an owned future. The lookup runs synchronously
    /// against `&self`, then returns a `'static + Send` future that owns the
    /// cloned `Peer`. Lets callers drop a sync RwLock guard before awaiting
    /// the (potentially slow) RPC, so concurrent dispatches don't serialize
    /// on the manager.
    pub fn dispatch_call_tool(
        &self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<impl Future<Output = Result<JsonRpcResponse>> + Send + 'static> {
        let (peer, original_name) = self.lookup_tool_peer(namespaced_name)?;
        let args: Option<serde_json::Map<String, serde_json::Value>> = match arguments {
            serde_json::Value::Object(map) if !map.is_empty() => Some(map),
            _ => None,
        };
        let mut params = CallToolRequestParams::new(original_name.clone());
        if let Some(args) = args {
            params = params.with_arguments(args);
        }
        Ok(async move {
            let result = peer
                .call_tool(params)
                .await
                .with_context(|| format!("tool call '{}' failed", original_name))?;
            let result_json =
                serde_json::to_value(&result).context("failed to serialize tool result")?;
            Ok(JsonRpcResponse::ok(None, result_json))
        })
    }

    /// Resolve a resources/read to an owned future. See `dispatch_call_tool`.
    pub fn dispatch_read_resource(
        &self,
        namespaced_uri: &str,
    ) -> Result<impl Future<Output = Result<JsonRpcResponse>> + Send + 'static> {
        let (peer, original_uri) = self.lookup_resource_peer(namespaced_uri)?;
        let params = ReadResourceRequestParams::new(original_uri.clone());
        Ok(async move {
            let result = peer
                .read_resource(params)
                .await
                .with_context(|| format!("resource read '{}' failed", original_uri))?;
            let result_json =
                serde_json::to_value(&result).context("failed to serialize resource result")?;
            Ok(JsonRpcResponse::ok(None, result_json))
        })
    }

    /// Resolve a prompts/get to an owned future. See `dispatch_call_tool`.
    pub fn dispatch_get_prompt(
        &self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<impl Future<Output = Result<JsonRpcResponse>> + Send + 'static> {
        let (peer, original_name) = self.lookup_prompt_peer(namespaced_name)?;
        let mut params = GetPromptRequestParams::new(original_name.clone());
        if let serde_json::Value::Object(map) = arguments {
            if !map.is_empty() {
                params = params.with_arguments(map);
            }
        }
        Ok(async move {
            let result = peer
                .get_prompt(params)
                .await
                .with_context(|| format!("prompt get '{}' failed", original_name))?;
            let result_json =
                serde_json::to_value(&result).context("failed to serialize prompt result")?;
            Ok(JsonRpcResponse::ok(None, result_json))
        })
    }

    /// Shut down all server connections.
    pub async fn shutdown_all(&mut self) {
        self.drain_running().await
    }

    /// Take ownership of all running server connections and return a future
    /// that cancels them. Caller must drop any manager guard before awaiting.
    /// Drains every peer in every pool.
    pub fn drain_running(&mut self) -> impl Future<Output = ()> + Send + 'static {
        let running = std::mem::take(&mut self.running);
        async move {
            for (name, pool) in running {
                let peer_count = pool.peers.len();
                if peer_count > 1 {
                    debug!(server = %name, pool_size = peer_count, "disconnecting MCP server pool");
                } else {
                    debug!(server = %name, "disconnecting MCP server");
                }
                for (i, server) in pool.peers.into_iter().enumerate() {
                    if let Err(e) = server.client.cancel().await {
                        warn!(
                            server = %name,
                            peer_index = i,
                            error = %e,
                            "error cancelling MCP server peer"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvVarGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::path::Path>) -> Self {
            let old = std::env::var(key).ok();
            std::env::set_var(key, value.as_ref());
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

    fn test_server_def() -> McpServerDef {
        McpServerDef {
            name: "test".to_string(),
            url: "https://mcp.example.com/v1".to_string(),
            headers: HashMap::new(),
            auth: None,
            enabled: true,
            source: "test".to_string(),
            command: None,
            args: vec![],
            env: HashMap::new(),
            pool_size: None,
            pool_safe_tools: Vec::new(),
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
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid namespaced"));
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
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid namespaced"));
    }

    // ── ServerPool round-robin tests (T3 angle 2) ───────────────────

    #[test]
    fn next_peer_index_single_peer_always_zero() {
        let counter = AtomicUsize::new(0);
        // Pool of 1 peer ⇒ always idx 0 regardless of pool-safe flag.
        for _ in 0..10 {
            assert_eq!(next_peer_index(1, true, &counter), 0);
            assert_eq!(next_peer_index(1, false, &counter), 0);
        }
        // Counter never bumped (we early-return).
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn next_peer_index_zero_peers_returns_zero() {
        // Defensive: 0 peers shouldn't crash; returns 0 (caller should
        // never invoke pick with empty pool but mod-by-zero would panic).
        let counter = AtomicUsize::new(0);
        assert_eq!(next_peer_index(0, true, &counter), 0);
        assert_eq!(next_peer_index(0, false, &counter), 0);
    }

    #[test]
    fn next_peer_index_unsafe_tool_pins_to_zero() {
        let counter = AtomicUsize::new(0);
        // Pool of 4 peers, but tool is NOT pool-safe ⇒ always idx 0.
        for _ in 0..20 {
            assert_eq!(next_peer_index(4, false, &counter), 0);
        }
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn next_peer_index_safe_tool_round_robins() {
        let counter = AtomicUsize::new(0);
        let peer_count = 4;
        // First peer_count calls cover every index exactly once.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..peer_count {
            seen.insert(next_peer_index(peer_count, true, &counter));
        }
        assert_eq!(seen, (0..peer_count).collect::<HashSet<_>>());
    }

    #[test]
    fn next_peer_index_safe_tool_balanced_over_many_calls() {
        let counter = AtomicUsize::new(0);
        let peer_count = 4;
        let n = 4_000;
        let mut hits = vec![0usize; peer_count];
        for _ in 0..n {
            hits[next_peer_index(peer_count, true, &counter)] += 1;
        }
        // Each peer hit exactly n / peer_count times (round-robin is
        // deterministic, not random).
        for h in &hits {
            assert_eq!(*h, n / peer_count);
        }
    }

    #[test]
    fn next_peer_index_counter_wraps_cleanly_at_usize_overflow() {
        // Round-robin uses fetch_add + modulo; if peer_count doesn't
        // divide usize::MAX evenly the wraparound produces a non-uniform
        // step at the wrap point. We accept that — the cost is one
        // imbalanced bucket every 2^63 calls, which is irrelevant in
        // practice. This test just asserts no panic at the boundary.
        let counter = AtomicUsize::new(usize::MAX - 1);
        assert!(next_peer_index(4, true, &counter) < 4);
        assert!(next_peer_index(4, true, &counter) < 4); // wraps
        assert!(next_peer_index(4, true, &counter) < 4); // post-wrap
    }

    #[test]
    fn server_pool_pick_routes_pool_safe_via_round_robin() {
        // Build a ServerPool by hand (no real RunningServer needed —
        // pick() returns &RunningServer but the test only inspects the
        // index it would have picked via next_peer_index).
        // We can't synthesize RunningServer (no public ctor), so this
        // test exercises the helper directly. Coverage of the
        // ServerPool::pick branching arrives via the live integration
        // test once a pool_size > 1 def is wired (see live integration
        // tests below).
        let counter = AtomicUsize::new(0);
        let safe: HashSet<String> = ["echo".into()].iter().cloned().collect();
        // Mimic the ServerPool::pick guard.
        for tool in &["echo", "echo", "echo", "fetch_http"] {
            let is_safe = safe.contains(*tool);
            let idx = next_peer_index(3, is_safe, &counter);
            if *tool == "echo" {
                assert!(idx < 3, "echo should round-robin");
            } else {
                assert_eq!(idx, 0, "fetch_http (not in safe set) pins to 0");
            }
        }
    }

    fn local_http_mcp_def(url: String, auth: Option<McpAuthConfig>) -> McpServerDef {
        let def = McpServerDef {
            name: "localtest".to_string(),
            url,
            headers: HashMap::new(),
            auth,
            enabled: true,
            source: "test".to_string(),
            command: None,
            args: vec![],
            env: HashMap::new(),
            pool_size: None,
            pool_safe_tools: Vec::new(),
        };
        assert!(!def.is_stdio());
        def
    }

    #[tokio::test]
    async fn local_http_mcp_e2e_uses_brokered_oauth_and_records_tool_call() {
        let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let _store_guard = EnvVarGuard::set(
            crate::credential_broker::TEST_STORE_ENV,
            dir.path().join("store.json"),
        );
        let harness = crate::test_support::mcp::spawn_recording_mcp_server()
            .await
            .unwrap();
        let observation = crate::credential_broker::CredentialObservation {
            provider: crate::credential_broker::CredentialProvider::Mcp,
            raw_value: "local-mcp-oauth-token".to_string(),
            source: "mcp.auth.local_e2e".to_string(),
            event_type: Some("mcp.server.auth".to_string()),
            confidence: 1.0,
            trace_id: Some("trace-local-mcp".to_string()),
            context_json: None,
        };
        let brokered = crate::credential_broker::broker_observed_credential(&observation)
            .expect("test credential should broker");
        let def = local_http_mcp_def(
            harness.url.clone(),
            Some(McpAuthConfig {
                kind: McpAuthKind::OAuth,
                credential_ref: brokered.credential_ref.clone(),
            }),
        );
        let mut mgr = McpServerManager::new(vec![def.clone()], reqwest::Client::new());

        mgr.connect_and_initialize(&def)
            .await
            .expect("local MCP server should initialize");

        assert!(
            mgr.is_running("localtest"),
            "local server should be running after successful init"
        );
        assert!(
            mgr.tool_catalog()
                .iter()
                .any(|tool| tool.namespaced_name == "localtest__echo"),
            "local MCP should expose echo, got catalog: {:?}",
            mgr.tool_catalog()
        );

        let result = mgr
            .call_tool(
                "localtest__echo",
                serde_json::json!({ "message": "winter" }),
            )
            .await
            .expect("local echo tool should dispatch");
        let result_json = serde_json::to_string(&result).unwrap();
        assert!(
            result_json.contains("echo:winter"),
            "tool result should include echo output: {result_json}"
        );

        let tool_calls = harness.state.tool_calls();
        assert_eq!(
            tool_calls,
            vec![crate::test_support::mcp::RecordedMcpToolCall {
                tool: "echo".to_string(),
                arguments: serde_json::json!({ "message": "winter" }),
            }]
        );

        let requests = harness.state.http_requests();
        assert!(
            requests.iter().any(|request| request
                .header("authorization")
                .is_some_and(|value| value == "Bearer local-mcp-oauth-token")),
            "local MCP server should receive the broker-resolved bearer token: {requests:?}"
        );
        assert!(
            requests.iter().all(|request| !request
                .header("authorization")
                .unwrap_or_default()
                .contains("credential:blake3:")),
            "broker references must not be sent as auth material: {requests:?}"
        );
    }

    #[tokio::test]
    async fn local_http_mcp_unresolved_broker_ref_fails_before_network_dispatch() {
        let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let _store_guard = EnvVarGuard::set(
            crate::credential_broker::TEST_STORE_ENV,
            dir.path().join("store.json"),
        );
        let harness = crate::test_support::mcp::spawn_recording_mcp_server()
            .await
            .unwrap();
        let def = local_http_mcp_def(
            harness.url.clone(),
            Some(McpAuthConfig {
                kind: McpAuthKind::Bearer,
                credential_ref: "credential:blake3:missing-local-mcp-token".to_string(),
            }),
        );
        let mut mgr = McpServerManager::new(vec![def.clone()], reqwest::Client::new());

        let err = mgr
            .connect_and_initialize(&def)
            .await
            .expect_err("unresolved broker ref must fail closed");

        assert!(
            err.to_string().contains("could not be resolved"),
            "unexpected error: {err:#}"
        );
        assert!(
            harness.state.http_requests().is_empty(),
            "unresolved broker refs must fail before any remote MCP request"
        );
    }
}

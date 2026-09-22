use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::warn;

use crate::net::policy_config::{SecurityRuleSet, SharedPluginPolicy};
use capsem_logger::DbWriter;
use capsem_proto::mcp_aggregator::AggregatorClient;
use capsem_proto::mcp_contracts::{builtin_ledger, parse_namespaced, JsonRpcRequest, JsonRpcResponse, McpToolDef};

const DEFAULT_MCP_TIMEOUT_SECS: u64 = 60;
const DEFAULT_MCP_TOOL_CALL_TIMEOUT_SECS: u64 = 300;
const DEFAULT_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTimeouts {
    pub default_timeout: Duration,
    pub tool_call_default: Duration,
    pub tool_call_ceiling: Duration,
}

impl Default for McpTimeouts {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(DEFAULT_MCP_TIMEOUT_SECS),
            tool_call_default: Duration::from_secs(DEFAULT_MCP_TOOL_CALL_TIMEOUT_SECS),
            tool_call_ceiling: Duration::from_secs(DEFAULT_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS),
        }
    }
}

impl McpTimeouts {
    pub fn from_env() -> Self {
        let default_timeout = env_duration_secs("CAPSEM_MCP_DEFAULT_TIMEOUT_SECS", DEFAULT_MCP_TIMEOUT_SECS);
        let tool_call_ceiling = env_duration_secs(
            "CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS",
            DEFAULT_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS,
        );
        let tool_call_default =
            env_duration_secs("CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS", DEFAULT_MCP_TOOL_CALL_TIMEOUT_SECS)
                .min(tool_call_ceiling);

        Self {
            default_timeout,
            tool_call_default,
            tool_call_ceiling,
        }
    }
}

fn env_duration_secs(key: &str, default_secs: u64) -> Duration {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default_secs))
}

pub struct McpEndpointState {
    pub aggregator: AggregatorClient,
    builtin_ledger: Option<Arc<DbWriter>>,
    builtin_servers: std::sync::RwLock<BTreeSet<String>>,
    pub security_rules: Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
    pub plugin_policy: SharedPluginPolicy,
    pub inflight: Arc<tokio::sync::Semaphore>,
    pub timeouts: McpTimeouts,
    scoped_tools: Option<Arc<dyn ScopedMcpTools>>,
    tool_timeout_overrides: RwLock<HashMap<String, Duration>>,
}

/// Tools implemented by the current VM owner rather than an MCP subprocess.
///
/// The endpoint still applies the normal MCP admission and logging around
/// these calls. Implementations receive only the tool arguments and whatever
/// owner-scoped capabilities they were constructed with.
pub trait ScopedMcpTools: Send + Sync {
    fn definitions(&self) -> Vec<McpToolDef>;

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        arguments: serde_json::Value,
    ) -> futures::future::BoxFuture<'a, Result<serde_json::Value, String>>;
}

impl McpEndpointState {
    pub fn new(
        aggregator: AggregatorClient,
        security_rules: Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
        plugin_policy: SharedPluginPolicy,
        inflight: Arc<tokio::sync::Semaphore>,
        timeouts: McpTimeouts,
    ) -> Self {
        Self {
            aggregator,
            builtin_ledger: None,
            builtin_servers: std::sync::RwLock::new(BTreeSet::new()),
            security_rules,
            plugin_policy,
            inflight,
            timeouts,
            scoped_tools: None,
            tool_timeout_overrides: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_scoped_tools(mut self, scoped_tools: Arc<dyn ScopedMcpTools>) -> Self {
        self.scoped_tools = Some(scoped_tools);
        self
    }

    /// Admit ledger records carried by tools from the configured builtin MCP
    /// servers. Other endpoint users have no authority to write this ledger.
    pub fn with_builtin_ledger(mut self, ledger: Arc<DbWriter>, servers: BTreeSet<String>) -> Self {
        self.builtin_ledger = Some(ledger);
        *self.builtin_servers.get_mut().expect("builtin server set poisoned") = servers;
        self
    }

    /// Replace the builtin server names after a refresh rebuilt the server list.
    pub fn set_builtin_servers(&self, names: BTreeSet<String>) {
        *self.builtin_servers.write().expect("builtin server set poisoned") = names;
    }

    fn is_builtin_tool(&self, namespaced_tool: &str) -> bool {
        parse_namespaced(namespaced_tool).is_some_and(|(server, _)| {
            self.builtin_servers
                .read()
                .expect("builtin server set poisoned")
                .contains(server)
        })
    }

    /// Strip the reserved metadata from every tool result and record it only
    /// for a tool owned by a configured builtin server.
    async fn take_builtin_ledger(&self, namespaced_tool: &str, result: &mut serde_json::Value) {
        let Some(value) = builtin_ledger::take(result) else {
            return;
        };
        if !self.is_builtin_tool(namespaced_tool) {
            warn!(
                tool = namespaced_tool,
                "dropped builtin ledger records from a server that is not the builtin"
            );
            return;
        }
        let Some(ledger) = &self.builtin_ledger else {
            warn!(
                tool = namespaced_tool,
                "dropped builtin ledger records because the endpoint has no session ledger"
            );
            return;
        };
        let records = match builtin_ledger::decode(value) {
            Ok(records) => records,
            Err(error) => {
                warn!(tool = namespaced_tool, %error, "builtin ledger records did not parse; nothing recorded");
                return;
            }
        };
        let rules = Arc::clone(&*self.security_rules.read().expect("security rules poisoned"));
        crate::mcp::builtin_ledger::record_builtin_ledger(ledger, &rules, records).await;
    }

    pub async fn record_tool_catalog_timeouts(&self, tools: &[McpToolDef]) {
        let mut overrides = self.tool_timeout_overrides.write().await;
        overrides.clear();
        for tool in tools {
            let Some(timeout_secs) = tool.timeout_secs else {
                continue;
            };
            let timeout = Duration::from_secs(timeout_secs).min(self.timeouts.tool_call_ceiling);
            overrides.insert(tool.namespaced_name.clone(), timeout);
        }
    }

    pub async fn timeout_for_request(&self, method: &str, tool_name: Option<&str>) -> Duration {
        if method != "tools/call" {
            return self.timeouts.default_timeout;
        }
        let Some(tool_name) = tool_name else {
            return self.timeouts.tool_call_default;
        };
        self.tool_timeout_overrides
            .read()
            .await
            .get(tool_name)
            .copied()
            .unwrap_or(self.timeouts.tool_call_default)
            .min(self.timeouts.tool_call_ceiling)
    }

    pub(crate) async fn handle_request(&self, req: &JsonRpcRequest) -> Option<JsonRpcResponse> {
        if req.method == "notifications/initialized" {
            return None;
        }

        let timeout = self.timeout_for_request(&req.method, param_str(req, "name")).await;
        match tokio::time::timeout(timeout, self.dispatch(req, timeout)).await {
            Ok(response) => Some(response),
            Err(_) => Some(JsonRpcResponse::err(
                req.id.clone(),
                -32000,
                format!("MCP request timed out after {} ms", timeout.as_millis()),
            )),
        }
    }

    /// `timeout` is forwarded to the aggregator so it cancels the upstream
    /// request when the endpoint stops waiting; the `tokio::time::timeout`
    /// around this call is the backstop for a dead aggregator.
    async fn dispatch(&self, req: &JsonRpcRequest, timeout: Duration) -> JsonRpcResponse {
        match req.method.as_str() {
            "initialize" => JsonRpcResponse::ok(
                req.id.clone(),
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {},
                        "resources": {},
                        "prompts": {}
                    },
                    "serverInfo": {
                        "name": "capsem-mcp-mitm-endpoint",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ),

            "tools/list" => match self.aggregator.list_tools().await {
                Ok(mut tools) => {
                    if let Some(scoped) = &self.scoped_tools {
                        let scoped = scoped.definitions();
                        if let Some(collision) = scoped
                            .iter()
                            .find(|local| tools.iter().any(|tool| tool.namespaced_name == local.namespaced_name))
                        {
                            return JsonRpcResponse::err(
                                req.id.clone(),
                                -32603,
                                format!("reserved scoped tool name collision: {}", collision.namespaced_name),
                            );
                        }
                        tools.extend(scoped);
                    }
                    self.record_tool_catalog_timeouts(&tools).await;
                    let tools: Vec<serde_json::Value> = tools
                        .iter()
                        .map(|tool| {
                            let mut value = serde_json::json!({
                                "name": tool.namespaced_name,
                                "description": tool.description,
                                "inputSchema": tool.input_schema,
                            });
                            if let Some(annotations) = &tool.annotations {
                                value["annotations"] = annotations.to_mcp_json();
                            }
                            value
                        })
                        .collect();
                    JsonRpcResponse::ok(req.id.clone(), serde_json::json!({"tools": tools}))
                }
                Err(e) => JsonRpcResponse::err(req.id.clone(), -32603, format!("tools list failed: {e}")),
            },

            "tools/call" => {
                let tool_name = param_str(req, "name").unwrap_or("");
                if tool_name.is_empty() {
                    return JsonRpcResponse::err(req.id.clone(), -32602, "missing tool name");
                }

                let arguments = req
                    .params
                    .as_ref()
                    .and_then(|params| params.get("arguments"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                if let Some(scoped) = &self.scoped_tools {
                    if scoped
                        .definitions()
                        .iter()
                        .any(|tool| tool.namespaced_name == tool_name)
                    {
                        return match scoped.call_tool(tool_name, arguments).await {
                            Ok(result) => JsonRpcResponse::ok(req.id.clone(), result),
                            Err(error) => JsonRpcResponse::err(
                                req.id.clone(),
                                -32603,
                                format!("scoped tool call failed: {error}"),
                            ),
                        };
                    }
                }
                match self.aggregator.call_tool(tool_name, arguments, Some(timeout)).await {
                    Ok(mut result) => {
                        self.take_builtin_ledger(tool_name, &mut result).await;
                        JsonRpcResponse::ok(req.id.clone(), result)
                    }
                    Err(e) => JsonRpcResponse::err(req.id.clone(), -32603, format!("tool call failed: {e}")),
                }
            }

            "resources/list" => match self.aggregator.list_resources().await {
                Ok(resources) => {
                    let resources: Vec<serde_json::Value> = resources
                        .iter()
                        .map(|resource| {
                            serde_json::json!({
                                "uri": resource.namespaced_uri,
                                "name": resource.name,
                                "description": resource.description,
                                "mimeType": resource.mime_type,
                            })
                        })
                        .collect();
                    JsonRpcResponse::ok(req.id.clone(), serde_json::json!({"resources": resources}))
                }
                Err(e) => JsonRpcResponse::err(req.id.clone(), -32603, format!("resources list failed: {e}")),
            },

            "resources/read" => {
                let uri = param_str(req, "uri").unwrap_or("");
                if uri.is_empty() {
                    return JsonRpcResponse::err(req.id.clone(), -32602, "missing resource URI");
                }

                match self.aggregator.read_resource(uri, Some(timeout)).await {
                    Ok(mut result) => {
                        strip_builtin_ledger("resources/read", &mut result);
                        JsonRpcResponse::ok(req.id.clone(), result)
                    }
                    Err(e) => JsonRpcResponse::err(req.id.clone(), -32603, format!("resource read failed: {e}")),
                }
            }

            "prompts/list" => match self.aggregator.list_prompts().await {
                Ok(prompts) => {
                    let prompts: Vec<serde_json::Value> = prompts
                        .iter()
                        .map(|prompt| {
                            serde_json::json!({
                                "name": prompt.namespaced_name,
                                "description": prompt.description,
                                "arguments": prompt.arguments,
                            })
                        })
                        .collect();
                    JsonRpcResponse::ok(req.id.clone(), serde_json::json!({"prompts": prompts}))
                }
                Err(e) => JsonRpcResponse::err(req.id.clone(), -32603, format!("prompts list failed: {e}")),
            },

            "prompts/get" => {
                let prompt_name = param_str(req, "name").unwrap_or("");
                if prompt_name.is_empty() {
                    return JsonRpcResponse::err(req.id.clone(), -32602, "missing prompt name");
                }

                let arguments = req
                    .params
                    .as_ref()
                    .and_then(|params| params.get("arguments"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                match self.aggregator.get_prompt(prompt_name, arguments, Some(timeout)).await {
                    Ok(mut result) => {
                        strip_builtin_ledger("prompts/get", &mut result);
                        JsonRpcResponse::ok(req.id.clone(), result)
                    }
                    Err(e) => JsonRpcResponse::err(req.id.clone(), -32603, format!("prompt get failed: {e}")),
                }
            }

            _ => JsonRpcResponse::err(req.id.clone(), -32601, format!("method not found: {}", req.method)),
        }
    }
}

fn strip_builtin_ledger(method: &str, result: &mut serde_json::Value) {
    if builtin_ledger::take(result).is_some() {
        warn!(
            method,
            "dropped builtin ledger records from a result that cannot carry them"
        );
    }
}

fn param_str<'a>(req: &'a JsonRpcRequest, key: &str) -> Option<&'a str> {
    req.params
        .as_ref()
        .and_then(|params| params.get(key))
        .and_then(|value| value.as_str())
}

#[cfg(test)]
mod tests;

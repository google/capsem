use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::mcp::aggregator::AggregatorClient;
use crate::mcp::policy::McpPolicy;
use crate::mcp::types::{JsonRpcRequest, JsonRpcResponse, McpToolDef};

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
        let default_timeout =
            env_duration_secs("CAPSEM_MCP_DEFAULT_TIMEOUT_SECS", DEFAULT_MCP_TIMEOUT_SECS);
        let tool_call_ceiling = env_duration_secs(
            "CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS",
            DEFAULT_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS,
        );
        let tool_call_default = env_duration_secs(
            "CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS",
            DEFAULT_MCP_TOOL_CALL_TIMEOUT_SECS,
        )
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
    pub policy: Arc<RwLock<Arc<McpPolicy>>>,
    pub security_engine: Arc<super::RuntimeSecurityEngineSlot>,
    pub inflight: Arc<tokio::sync::Semaphore>,
    pub timeouts: McpTimeouts,
    tool_timeout_overrides: RwLock<HashMap<String, Duration>>,
}

impl McpEndpointState {
    pub fn new(
        aggregator: AggregatorClient,
        policy: Arc<RwLock<Arc<McpPolicy>>>,
        security_engine: Arc<super::RuntimeSecurityEngineSlot>,
        inflight: Arc<tokio::sync::Semaphore>,
        timeouts: McpTimeouts,
    ) -> Self {
        Self {
            aggregator,
            policy,
            security_engine,
            inflight,
            timeouts,
            tool_timeout_overrides: RwLock::new(HashMap::new()),
        }
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

        let started = Instant::now();
        let method_kind = mcp_method_kind_label(&req.method);
        let tool_kind = mcp_tool_kind_label(param_str(req, "name"));
        let timeout = self
            .timeout_for_request(&req.method, param_str(req, "name"))
            .await;
        let response = match tokio::time::timeout(timeout, self.dispatch(req)).await {
            Ok(response) => response,
            Err(_) => JsonRpcResponse::err(
                req.id.clone(),
                -32000,
                format!("MCP request timed out after {} ms", timeout.as_millis()),
            ),
        };
        record_endpoint_dispatch(started, method_kind, tool_kind, &response);
        Some(response)
    }

    async fn dispatch(&self, req: &JsonRpcRequest) -> JsonRpcResponse {
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
                Ok(tools) => {
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
                Err(e) => {
                    JsonRpcResponse::err(req.id.clone(), -32603, format!("tools list failed: {e}"))
                }
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
                match self.aggregator.call_tool(tool_name, arguments).await {
                    Ok(result) => JsonRpcResponse::ok(req.id.clone(), result),
                    Err(e) => JsonRpcResponse::err(
                        req.id.clone(),
                        -32603,
                        format!("tool call failed: {e}"),
                    ),
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
                Err(e) => JsonRpcResponse::err(
                    req.id.clone(),
                    -32603,
                    format!("resources list failed: {e}"),
                ),
            },

            "resources/read" => {
                let uri = param_str(req, "uri").unwrap_or("");
                if uri.is_empty() {
                    return JsonRpcResponse::err(req.id.clone(), -32602, "missing resource URI");
                }

                match self.aggregator.read_resource(uri).await {
                    Ok(result) => JsonRpcResponse::ok(req.id.clone(), result),
                    Err(e) => JsonRpcResponse::err(
                        req.id.clone(),
                        -32603,
                        format!("resource read failed: {e}"),
                    ),
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
                Err(e) => JsonRpcResponse::err(
                    req.id.clone(),
                    -32603,
                    format!("prompts list failed: {e}"),
                ),
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
                match self.aggregator.get_prompt(prompt_name, arguments).await {
                    Ok(result) => JsonRpcResponse::ok(req.id.clone(), result),
                    Err(e) => JsonRpcResponse::err(
                        req.id.clone(),
                        -32603,
                        format!("prompt get failed: {e}"),
                    ),
                }
            }

            _ => JsonRpcResponse::err(
                req.id.clone(),
                -32601,
                format!("method not found: {}", req.method),
            ),
        }
    }
}

fn param_str<'a>(req: &'a JsonRpcRequest, key: &str) -> Option<&'a str> {
    req.params
        .as_ref()
        .and_then(|params| params.get(key))
        .and_then(|value| value.as_str())
}

fn record_endpoint_dispatch(
    started: Instant,
    method_kind: &'static str,
    tool_kind: &'static str,
    response: &JsonRpcResponse,
) {
    let result = if response.error.is_some() {
        "error"
    } else {
        "ok"
    };
    ::metrics::histogram!(
        super::metrics::MCP_ENDPOINT_DISPATCH_MS,
        "method_kind" => method_kind,
        "tool_kind" => tool_kind,
        "result" => result,
    )
    .record(started.elapsed().as_secs_f64() * 1000.0);
}

fn mcp_method_kind_label(method: &str) -> &'static str {
    match method {
        "initialize" => "initialize",
        "tools/list" => "tools/list",
        "tools/call" => "tools/call",
        "resources/list" => "resources/list",
        "resources/read" => "resources/read",
        "prompts/list" => "prompts/list",
        "prompts/get" => "prompts/get",
        _ => "unknown",
    }
}

fn mcp_tool_kind_label(tool_name: Option<&str>) -> &'static str {
    match tool_name {
        Some("local__echo") => "local_echo",
        Some(name) if name.starts_with("local__snapshots_") => "local_snapshot",
        Some("local__fetch_http" | "local__grep_http" | "local__http_headers") => "local_http",
        Some(name) if name.starts_with("local__") => "local_other",
        Some(_) => "external",
        None => "none",
    }
}

#[cfg(test)]
mod tests;

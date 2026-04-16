//! Protocol types for the MCP aggregator subprocess.
//!
//! The aggregator is a low-privilege subprocess that manages connections to
//! external MCP servers. It communicates with capsem-process via NDJSON over
//! stdin/stdout (one JSON message per line).
//!
//! Separation boundary: the aggregator handles external server connections
//! (rmcp HTTP, bearer tokens). It does NOT have access to the VM, session DB,
//! filesystem, or service IPC.

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use super::types::{McpPromptDef, McpResourceDef, McpServerDef, McpToolDef};

// ── Length-prefixed MessagePack framing ────────────────────────────
//
// Wire format: [4 bytes big-endian payload length] [N bytes msgpack]
// Max frame size: 16 MB.

const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// Write a length-prefixed msgpack frame.
pub async fn write_frame<W, T>(writer: &mut W, msg: &T) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let payload = rmp_serde::to_vec_named(msg).context("msgpack serialize")?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes()).await.context("write frame length")?;
    writer.write_all(&payload).await.context("write frame payload")?;
    writer.flush().await.context("flush frame")?;
    Ok(())
}

/// Read a length-prefixed msgpack frame. Returns None on EOF.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<Option<T>>
where
    R: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e).context("read frame length"),
    }
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_SIZE {
        anyhow::bail!("frame too large: {len} bytes (max {MAX_FRAME_SIZE})");
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await.context("read frame payload")?;
    let msg: T = rmp_serde::from_slice(&buf).context("msgpack deserialize")?;
    Ok(Some(msg))
}

// ── Request (process -> aggregator) ─────────────────────────────────

/// A request from capsem-process to the aggregator subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorRequest {
    /// Correlation ID for matching responses to requests.
    pub id: u64,
    #[serde(flatten)]
    pub method: AggregatorMethod,
}

/// The set of operations the aggregator supports.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum AggregatorMethod {
    /// List all server definitions with connection status.
    #[serde(rename = "list_servers")]
    ListServers,

    /// List all discovered tools across connected servers.
    #[serde(rename = "list_tools")]
    ListTools,

    /// List all discovered resources across connected servers.
    #[serde(rename = "list_resources")]
    ListResources,

    /// List all discovered prompts across connected servers.
    #[serde(rename = "list_prompts")]
    ListPrompts,

    /// Call a tool on an external MCP server.
    #[serde(rename = "call_tool")]
    CallTool {
        /// Namespaced tool name (e.g. "github__search_repos").
        name: String,
        /// Tool arguments as a JSON object.
        arguments: serde_json::Value,
    },

    /// Read a resource from an external MCP server.
    #[serde(rename = "read_resource")]
    ReadResource {
        /// Namespaced resource URI.
        uri: String,
    },

    /// Get a prompt from an external MCP server.
    #[serde(rename = "get_prompt")]
    GetPrompt {
        /// Namespaced prompt name.
        name: String,
        /// Prompt arguments as a JSON object.
        arguments: serde_json::Value,
    },

    /// Disconnect from all servers, reload definitions, and reconnect.
    #[serde(rename = "refresh")]
    Refresh {
        /// New server definitions to use (replaces the current set).
        servers: Vec<McpServerDef>,
    },

    /// Shut down all connections and exit.
    #[serde(rename = "shutdown")]
    Shutdown,
}

// ── Response (aggregator -> process) ────────────────────────────────

/// A response from the aggregator subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorResponse {
    /// Correlation ID matching the request.
    pub id: u64,
    #[serde(flatten)]
    pub body: AggregatorResult,
}

/// The result of an aggregator operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AggregatorResult {
    Error {
        error: String,
    },
    Servers {
        servers: Vec<AggregatorServerStatus>,
    },
    Tools {
        tools: Vec<McpToolDef>,
    },
    Resources {
        resources: Vec<McpResourceDef>,
    },
    Prompts {
        prompts: Vec<McpPromptDef>,
    },
    /// Result of a tool call, resource read, or prompt get.
    CallResult {
        result: serde_json::Value,
    },
    /// Acknowledgement (refresh, shutdown).
    Ok {
        ok: bool,
    },
}

/// Status of a single MCP server as reported by the aggregator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorServerStatus {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub source: String,
    /// True if this server uses stdio transport (subprocess).
    #[serde(default)]
    pub is_stdio: bool,
    pub connected: bool,
    pub tool_count: usize,
    pub resource_count: usize,
    pub prompt_count: usize,
}

// ── Client (used by capsem-process gateway) ────────────────────────

/// Internal message sent through the client's mpsc channel.
type ClientMessage = (AggregatorRequest, oneshot::Sender<AggregatorResponse>);

static NEXT_REQ_ID: AtomicU64 = AtomicU64::new(1);

/// Client handle for communicating with the aggregator subprocess.
///
/// Multiple gateway sessions share one `AggregatorClient` via `Arc`. Each call
/// sends a request through an mpsc channel to a background driver task, which
/// serializes requests to the subprocess stdin and routes responses back.
#[derive(Clone)]
pub struct AggregatorClient {
    tx: mpsc::Sender<ClientMessage>,
}

impl AggregatorClient {
    /// Create a new client backed by the given channel.
    ///
    /// The caller must spawn a driver task that reads from `rx` and forwards
    /// requests to the subprocess (see `capsem-process` for the driver).
    pub fn new(tx: mpsc::Sender<ClientMessage>) -> Self {
        Self { tx }
    }

    /// Create a client/receiver pair for wiring up the driver.
    pub fn channel(buffer: usize) -> (Self, mpsc::Receiver<ClientMessage>) {
        let (tx, rx) = mpsc::channel(buffer);
        (Self { tx }, rx)
    }

    /// Send a request and wait for the response.
    pub async fn request(&self, method: AggregatorMethod) -> Result<AggregatorResult> {
        let id = NEXT_REQ_ID.fetch_add(1, Ordering::Relaxed);
        let req = AggregatorRequest { id, method };
        let (resp_tx, resp_rx) = oneshot::channel();

        self.tx
            .send((req, resp_tx))
            .await
            .map_err(|_| anyhow::anyhow!("aggregator channel closed"))?;

        let resp = resp_rx
            .await
            .context("aggregator response channel dropped")?;

        Ok(resp.body)
    }

    /// List servers with connection status.
    pub async fn list_servers(&self) -> Result<Vec<AggregatorServerStatus>> {
        match self.request(AggregatorMethod::ListServers).await? {
            AggregatorResult::Servers { servers } => Ok(servers),
            AggregatorResult::Error { error } => Err(anyhow::anyhow!(error)),
            other => Err(anyhow::anyhow!("unexpected response: {:?}", other)),
        }
    }

    /// List all discovered tools.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>> {
        match self.request(AggregatorMethod::ListTools).await? {
            AggregatorResult::Tools { tools } => Ok(tools),
            AggregatorResult::Error { error } => Err(anyhow::anyhow!(error)),
            other => Err(anyhow::anyhow!("unexpected response: {:?}", other)),
        }
    }

    /// List all discovered resources.
    pub async fn list_resources(&self) -> Result<Vec<McpResourceDef>> {
        match self.request(AggregatorMethod::ListResources).await? {
            AggregatorResult::Resources { resources } => Ok(resources),
            AggregatorResult::Error { error } => Err(anyhow::anyhow!(error)),
            other => Err(anyhow::anyhow!("unexpected response: {:?}", other)),
        }
    }

    /// List all discovered prompts.
    pub async fn list_prompts(&self) -> Result<Vec<McpPromptDef>> {
        match self.request(AggregatorMethod::ListPrompts).await? {
            AggregatorResult::Prompts { prompts } => Ok(prompts),
            AggregatorResult::Error { error } => Err(anyhow::anyhow!(error)),
            other => Err(anyhow::anyhow!("unexpected response: {:?}", other)),
        }
    }

    /// Call a tool on an external MCP server.
    pub async fn call_tool(
        &self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        match self
            .request(AggregatorMethod::CallTool {
                name: namespaced_name.to_string(),
                arguments,
            })
            .await?
        {
            AggregatorResult::CallResult { result } => Ok(result),
            AggregatorResult::Error { error } => Err(anyhow::anyhow!(error)),
            other => Err(anyhow::anyhow!("unexpected response: {:?}", other)),
        }
    }

    /// Read a resource from an external MCP server.
    pub async fn read_resource(&self, namespaced_uri: &str) -> Result<serde_json::Value> {
        match self
            .request(AggregatorMethod::ReadResource {
                uri: namespaced_uri.to_string(),
            })
            .await?
        {
            AggregatorResult::CallResult { result } => Ok(result),
            AggregatorResult::Error { error } => Err(anyhow::anyhow!(error)),
            other => Err(anyhow::anyhow!("unexpected response: {:?}", other)),
        }
    }

    /// Get a prompt from an external MCP server.
    pub async fn get_prompt(
        &self,
        namespaced_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        match self
            .request(AggregatorMethod::GetPrompt {
                name: namespaced_name.to_string(),
                arguments,
            })
            .await?
        {
            AggregatorResult::CallResult { result } => Ok(result),
            AggregatorResult::Error { error } => Err(anyhow::anyhow!(error)),
            other => Err(anyhow::anyhow!("unexpected response: {:?}", other)),
        }
    }

    /// Refresh: disconnect from all servers and reconnect with new definitions.
    pub async fn refresh(&self, servers: Vec<McpServerDef>) -> Result<()> {
        match self.request(AggregatorMethod::Refresh { servers }).await? {
            AggregatorResult::Ok { .. } => Ok(()),
            AggregatorResult::Error { error } => Err(anyhow::anyhow!(error)),
            other => Err(anyhow::anyhow!("unexpected response: {:?}", other)),
        }
    }

    /// Shut down the aggregator subprocess.
    pub async fn shutdown(&self) -> Result<()> {
        match self.request(AggregatorMethod::Shutdown).await? {
            AggregatorResult::Ok { .. } => Ok(()),
            AggregatorResult::Error { error } => Err(anyhow::anyhow!(error)),
            other => Err(anyhow::anyhow!("unexpected response: {:?}", other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roundtrip helper: serialize to msgpack and back.
    fn msgpack_roundtrip<T: Serialize + for<'de> Deserialize<'de>>(val: &T) -> T {
        let bytes = rmp_serde::to_vec_named(val).unwrap();
        rmp_serde::from_slice(&bytes).unwrap()
    }

    #[test]
    fn request_list_servers_roundtrip() {
        let req = AggregatorRequest {
            id: 1,
            method: AggregatorMethod::ListServers,
        };
        let decoded = msgpack_roundtrip(&req);
        assert_eq!(decoded.id, 1);
        assert!(matches!(decoded.method, AggregatorMethod::ListServers));
    }

    #[test]
    fn request_call_tool_roundtrip() {
        let req = AggregatorRequest {
            id: 42,
            method: AggregatorMethod::CallTool {
                name: "github__search_repos".into(),
                arguments: serde_json::json!({"query": "rust"}),
            },
        };
        let decoded = msgpack_roundtrip(&req);
        assert_eq!(decoded.id, 42);
        if let AggregatorMethod::CallTool { name, arguments } = decoded.method {
            assert_eq!(name, "github__search_repos");
            assert_eq!(arguments["query"], "rust");
        } else {
            panic!("expected CallTool");
        }
    }

    #[test]
    fn request_shutdown_roundtrip() {
        let req = AggregatorRequest {
            id: 99,
            method: AggregatorMethod::Shutdown,
        };
        let decoded = msgpack_roundtrip(&req);
        assert!(matches!(decoded.method, AggregatorMethod::Shutdown));
    }

    #[test]
    fn request_refresh_roundtrip() {
        let req = AggregatorRequest {
            id: 10,
            method: AggregatorMethod::Refresh {
                servers: vec![McpServerDef {
                    name: "test".into(),
                    url: "https://mcp.example.com".into(),
                    command: None,
                    args: vec![],
                    env: Default::default(),
                    headers: Default::default(),
                    bearer_token: None,
                    enabled: true,
                    source: "manual".into(),
                }],
            },
        };
        let decoded = msgpack_roundtrip(&req);
        if let AggregatorMethod::Refresh { servers } = decoded.method {
            assert_eq!(servers.len(), 1);
            assert_eq!(servers[0].name, "test");
        } else {
            panic!("expected Refresh");
        }
    }

    #[test]
    fn response_servers_roundtrip() {
        let resp = AggregatorResponse {
            id: 1,
            body: AggregatorResult::Servers {
                servers: vec![AggregatorServerStatus {
                    name: "github".into(),
                    url: "https://mcp.github.com".into(),
                    enabled: true,
                    source: "claude".into(),
                    is_stdio: false,
                    connected: true,
                    tool_count: 5,
                    resource_count: 0,
                    prompt_count: 0,
                }],
            },
        };
        let decoded = msgpack_roundtrip(&resp);
        assert_eq!(decoded.id, 1);
        if let AggregatorResult::Servers { servers } = decoded.body {
            assert_eq!(servers[0].name, "github");
            assert!(servers[0].connected);
        } else {
            panic!("expected Servers");
        }
    }

    #[test]
    fn response_error_roundtrip() {
        let resp = AggregatorResponse {
            id: 2,
            body: AggregatorResult::Error {
                error: "server not found".into(),
            },
        };
        let decoded = msgpack_roundtrip(&resp);
        if let AggregatorResult::Error { error } = decoded.body {
            assert_eq!(error, "server not found");
        } else {
            panic!("expected Error");
        }
    }

    #[test]
    fn response_ok_roundtrip() {
        let resp = AggregatorResponse {
            id: 3,
            body: AggregatorResult::Ok { ok: true },
        };
        let decoded = msgpack_roundtrip(&resp);
        if let AggregatorResult::Ok { ok } = decoded.body {
            assert!(ok);
        } else {
            panic!("expected Ok");
        }
    }

    #[test]
    fn response_call_result_roundtrip() {
        let resp = AggregatorResponse {
            id: 4,
            body: AggregatorResult::CallResult {
                result: serde_json::json!({"content": [{"type": "text", "text": "hello"}]}),
            },
        };
        let decoded = msgpack_roundtrip(&resp);
        if let AggregatorResult::CallResult { result } = decoded.body {
            assert_eq!(result["content"][0]["text"], "hello");
        } else {
            panic!("expected CallResult");
        }
    }
}

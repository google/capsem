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

use crate::mcp_contracts::{McpPromptDef, McpResourceDef, McpServerDef, McpToolDef};

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
    writer
        .write_all(&len.to_be_bytes())
        .await
        .context("write frame length")?;
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

// ── Client (used by capsem-process and MITM MCP endpoint) ───────────

/// Internal message sent through the client's mpsc channel.
type ClientMessage = (AggregatorRequest, oneshot::Sender<AggregatorResponse>);

static NEXT_REQ_ID: AtomicU64 = AtomicU64::new(1);

/// Client handle for communicating with the aggregator subprocess.
///
/// Multiple callers share one `AggregatorClient` via `Arc`. Each call
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

        let resp = resp_rx.await.context("aggregator response channel dropped")?;

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
    pub async fn call_tool(&self, namespaced_name: &str, arguments: serde_json::Value) -> Result<serde_json::Value> {
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
    pub async fn get_prompt(&self, namespaced_name: &str, arguments: serde_json::Value) -> Result<serde_json::Value> {
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
mod tests;

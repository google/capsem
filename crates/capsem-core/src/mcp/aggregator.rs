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
use std::time::Instant;

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
    let payload = encode_frame_payload(msg)?;
    write_frame_payload(writer, &payload).await
}

/// Read a length-prefixed msgpack frame. Returns None on EOF.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<Option<T>>
where
    R: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let Some(buf) = read_frame_payload(reader).await? else {
        return Ok(None);
    };
    Ok(Some(decode_frame_payload(&buf)?))
}

/// Serialize a frame payload without writing it. Exposed so hot paths can
/// time MessagePack encode separately from pipe write latency.
pub fn encode_frame_payload<T>(msg: &T) -> Result<Vec<u8>>
where
    T: Serialize,
{
    rmp_serde::to_vec_named(msg).context("msgpack serialize")
}

/// Deserialize a frame payload without reading it. Exposed so hot paths can
/// time MessagePack decode separately from pipe read latency.
pub fn decode_frame_payload<T>(buf: &[u8]) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    rmp_serde::from_slice(buf).context("msgpack deserialize")
}

/// Write a pre-serialized length-prefixed MessagePack frame.
pub async fn write_frame_payload<W>(writer: &mut W, payload: &[u8]) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let len = payload.len() as u32;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .context("write frame length")?;
    writer
        .write_all(payload)
        .await
        .context("write frame payload")?;
    writer.flush().await.context("flush frame")?;
    Ok(())
}

/// Read a raw length-prefixed MessagePack payload.
pub async fn read_frame_payload<R>(reader: &mut R) -> Result<Option<Vec<u8>>>
where
    R: AsyncReadExt + Unpin,
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
    reader
        .read_exact(&mut buf)
        .await
        .context("read frame payload")?;
    Ok(Some(buf))
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

impl AggregatorMethod {
    pub fn metric_label(&self) -> &'static str {
        match self {
            Self::ListServers => "list_servers",
            Self::ListTools => "list_tools",
            Self::ListResources => "list_resources",
            Self::ListPrompts => "list_prompts",
            Self::CallTool { .. } => "call_tool",
            Self::ReadResource { .. } => "read_resource",
            Self::GetPrompt { .. } => "get_prompt",
            Self::Refresh { .. } => "refresh",
            Self::Shutdown => "shutdown",
        }
    }

    pub fn tool_kind_label(&self) -> &'static str {
        match self {
            Self::CallTool { name, .. } | Self::GetPrompt { name, .. } => {
                namespaced_metric_kind(name)
            }
            Self::ReadResource { uri } => namespaced_metric_kind(uri),
            _ => "none",
        }
    }
}

pub fn namespaced_metric_kind(name: &str) -> &'static str {
    match name {
        "local__echo" => "local_echo",
        n if n.starts_with("local__snapshots_") => "local_snapshot",
        "local__fetch_http" | "local__grep_http" | "local__http_headers" => "local_http",
        n if n.starts_with("local__") => "local_other",
        n if n.contains("__") || n.starts_with("capsem://") => "external",
        _ => "unknown",
    }
}

pub const MCP_AGGREGATOR_CLIENT_STAGE_MS: &str = "mcp.aggregator_client_stage_duration_ms";
pub const MCP_AGGREGATOR_STAGE_MS: &str = "mcp.aggregator_stage_duration_ms";
pub const MCP_BUILTIN_TOOL_DURATION_MS: &str = "mcp.builtin_tool_duration_ms";

pub fn record_aggregator_client_stage_metric(
    started: Instant,
    stage: &'static str,
    method_kind: &'static str,
    tool_kind: &'static str,
    result: &'static str,
) {
    ::metrics::histogram!(
        MCP_AGGREGATOR_CLIENT_STAGE_MS,
        "stage" => stage,
        "method_kind" => method_kind,
        "tool_kind" => tool_kind,
        "result" => result,
    )
    .record(started.elapsed().as_secs_f64() * 1000.0);
}

pub fn record_aggregator_stage_metric(
    started: Instant,
    stage: &'static str,
    method_kind: &'static str,
    tool_kind: &'static str,
    result: &'static str,
) {
    ::metrics::histogram!(
        MCP_AGGREGATOR_STAGE_MS,
        "stage" => stage,
        "method_kind" => method_kind,
        "tool_kind" => tool_kind,
        "result" => result,
    )
    .record(started.elapsed().as_secs_f64() * 1000.0);
}

pub fn record_builtin_tool_metric(started: Instant, tool_kind: &'static str, result: &'static str) {
    ::metrics::histogram!(
        MCP_BUILTIN_TOOL_DURATION_MS,
        "tool_kind" => tool_kind,
        "result" => result,
    )
    .record(started.elapsed().as_secs_f64() * 1000.0);
}

fn record_aggregator_request_metric(
    started: Instant,
    method_kind: &'static str,
    tool_kind: &'static str,
    result: &'static str,
) {
    ::metrics::histogram!(
        crate::net::mitm_proxy::metrics::MCP_AGGREGATOR_REQUEST_MS,
        "method_kind" => method_kind,
        "tool_kind" => tool_kind,
        "result" => result,
    )
    .record(started.elapsed().as_secs_f64() * 1000.0);
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
type ClientMessage = (
    AggregatorRequest,
    Instant,
    oneshot::Sender<AggregatorResponse>,
);

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
        let started = Instant::now();
        let method_kind = method.metric_label();
        let tool_kind = method.tool_kind_label();
        let id = NEXT_REQ_ID.fetch_add(1, Ordering::Relaxed);
        let req = AggregatorRequest { id, method };
        let (resp_tx, resp_rx) = oneshot::channel();

        let send_started = Instant::now();
        let enqueued_at = Instant::now();
        if self.tx.send((req, enqueued_at, resp_tx)).await.is_err() {
            record_aggregator_client_stage_metric(
                send_started,
                "channel_send",
                method_kind,
                tool_kind,
                "channel_closed",
            );
            record_aggregator_request_metric(started, method_kind, tool_kind, "channel_closed");
            return Err(anyhow::anyhow!("aggregator channel closed"));
        }
        record_aggregator_client_stage_metric(
            send_started,
            "channel_send",
            method_kind,
            tool_kind,
            "ok",
        );

        let resp = match resp_rx.await {
            Ok(resp) => resp,
            Err(error) => {
                record_aggregator_request_metric(
                    started,
                    method_kind,
                    tool_kind,
                    "channel_dropped",
                );
                return Err(error).context("aggregator response channel dropped");
            }
        };
        let result = match &resp.body {
            AggregatorResult::Error { .. } => "error",
            _ => "ok",
        };
        record_aggregator_request_metric(started, method_kind, tool_kind, result);

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
                    pool_size: None,
                    pool_safe_tools: Vec::new(),
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
    fn aggregator_metric_labels_are_bounded() {
        assert_eq!(namespaced_metric_kind("local__echo"), "local_echo");
        assert_eq!(
            namespaced_metric_kind("local__snapshots_list"),
            "local_snapshot"
        );
        assert_eq!(namespaced_metric_kind("local__http_headers"), "local_http");
        assert_eq!(namespaced_metric_kind("github__issue"), "external");
        assert_eq!(namespaced_metric_kind("capsem://local/readme"), "external");
        assert_eq!(namespaced_metric_kind("malformed"), "unknown");
    }

    #[tokio::test]
    async fn aggregator_client_records_request_duration_metric() {
        use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

        let recorder = DebuggingRecorder::new();
        let snapshotter: Snapshotter = recorder.snapshotter();
        let _guard = ::metrics::set_default_local_recorder(&recorder);

        let (client, mut rx) = AggregatorClient::channel(4);
        tokio::spawn(async move {
            while let Some((req, _enqueued_at, resp_tx)) = rx.recv().await {
                let _ = resp_tx.send(AggregatorResponse {
                    id: req.id,
                    body: AggregatorResult::CallResult {
                        result: serde_json::json!({"ok": true}),
                    },
                });
            }
        });

        let result = client
            .call_tool("local__echo", serde_json::json!({"text": "hi"}))
            .await
            .unwrap();
        assert_eq!(result["ok"], true);

        let present =
            snapshotter
                .snapshot()
                .into_vec()
                .into_iter()
                .any(|(key, _, _, value)| {
                    key.key().name() == crate::net::mitm_proxy::metrics::MCP_AGGREGATOR_REQUEST_MS
                        && key.key().labels().any(|label| {
                            label.key() == "method_kind" && label.value() == "call_tool"
                        })
                        && key.key().labels().any(|label| {
                            label.key() == "tool_kind" && label.value() == "local_echo"
                        })
                        && key
                            .key()
                            .labels()
                            .any(|label| label.key() == "result" && label.value() == "ok")
                        && matches!(value, DebugValue::Histogram(_))
                });
        assert!(present, "aggregator request histogram should be recorded");
    }

    #[test]
    fn aggregator_subprocess_remains_session_db_free() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(std::path::Path::parent)
            .expect("capsem-core should live under crates/");
        let files = [
            repo_root.join("crates/capsem-mcp-aggregator/Cargo.toml"),
            repo_root.join("crates/capsem-mcp-aggregator/src/main.rs"),
        ];
        let forbidden = [
            "capsem-logger",
            "capsem_logger",
            "rusqlite",
            "DbWriter",
            "DbReader",
            "WriteOp",
            "McpCall",
            "session.db",
        ];

        for file in files {
            let text = std::fs::read_to_string(&file).unwrap_or_else(|err| {
                panic!("failed to read {}: {err}", file.display());
            });
            for needle in forbidden {
                assert!(
                    !text.contains(needle),
                    "{} must not reference {needle}; MCP auditing belongs in the MITM endpoint/process, not the low-privilege aggregator subprocess",
                    file.display()
                );
            }
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

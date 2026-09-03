//! MCP frame wire layer: reading, parsing, validating, and writing framed JSON-RPC.

use super::*;

#[derive(Debug, Clone)]
pub(super) struct JsonRpcPayloadError {
    pub(super) code: i64,
    pub(super) message: String,
    pub(super) id: Option<serde_json::Value>,
}

impl fmt::Display for JsonRpcPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for JsonRpcPayloadError {}

pub(super) struct OutboundFrame {
    pub(super) stream_id: u32,
    pub(super) process_name: String,
    pub(super) payload: Vec<u8>,
}

pub(super) async fn send_response(
    tx: &tokio::sync::mpsc::Sender<OutboundFrame>,
    stream_id: u32,
    process_name: &str,
    response: &JsonRpcResponse,
) -> Result<()> {
    let payload = serde_json::to_vec(response).context("serialize framed MCP response")?;
    tx.send(OutboundFrame {
        stream_id,
        process_name: process_name.to_string(),
        payload,
    })
    .await
    .context("framed MCP writer channel closed")
}

pub(super) async fn read_next_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<FrameRead> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(FrameRead::Eof),
        Err(e) => return Err(e).context("read MCP frame length"),
    }

    let total_len = u32::from_be_bytes(len_buf) as usize;
    if !(capsem_proto::MCP_FRAME_HEADER_LEN as usize..=capsem_proto::MCP_FRAME_MAX_SIZE).contains(&total_len) {
        bail!("invalid MCP frame length: {total_len}");
    }

    // The length prefix is committed, so the body must arrive promptly. A guest
    // that announces a length and then stalls would otherwise hold this read
    // loop (and the connection) forever. The idle wait between requests happens
    // on the length read above and is deliberately not bounded here.
    let mut body = vec![0u8; total_len];
    match tokio::time::timeout(FRAME_BODY_TIMEOUT, reader.read_exact(&mut body)).await {
        Ok(result) => result.context("read MCP frame body")?,
        Err(_) => bail!("MCP frame body did not arrive within {:?}", FRAME_BODY_TIMEOUT),
    };
    match capsem_proto::decode_mcp_frame_body(&body) {
        Ok(frame) => Ok(FrameRead::Frame(frame)),
        Err(e) => Ok(FrameRead::InvalidFrame {
            stream_id: recover_stream_id(&body),
            error: e.to_string(),
        }),
    }
}

pub(super) fn recover_stream_id(body: &[u8]) -> Option<u32> {
    if body.len() < 8 {
        return None;
    }
    Some(u32::from_be_bytes([body[4], body[5], body[6], body[7]]))
}

pub(super) fn parse_json_rpc_payload(payload: &[u8]) -> std::result::Result<JsonRpcRequest, JsonRpcPayloadError> {
    if payload.len() > MCP_JSON_RPC_MAX_BYTES {
        return Err(JsonRpcPayloadError {
            code: -32600,
            message: format!("JSON-RPC payload too large: {} bytes", payload.len()),
            id: None,
        });
    }

    let value = serde_json::from_slice::<serde_json::Value>(payload).map_err(|e| JsonRpcPayloadError {
        code: -32700,
        message: format!("parse error: {e}"),
        id: None,
    })?;

    let id = value.get("id").cloned();
    if value.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
        return Err(JsonRpcPayloadError {
            code: -32600,
            message: "unsupported JSON-RPC version".to_string(),
            id,
        });
    }
    let missing_method = value
        .get("method")
        .and_then(|v| v.as_str())
        .map(|method| method.is_empty())
        .unwrap_or(true);
    if missing_method {
        return Err(JsonRpcPayloadError {
            code: -32600,
            message: "missing JSON-RPC method".to_string(),
            id,
        });
    }

    serde_json::from_value(value).map_err(|e| JsonRpcPayloadError {
        code: -32600,
        message: format!("invalid JSON-RPC request: {e}"),
        id: None,
    })
}

pub(super) fn validate_frame_request_pair(frame: &capsem_proto::McpFrame, req: &JsonRpcRequest) -> Result<()> {
    match (frame.is_notification(), req.id.is_some()) {
        (true, false) => Ok(()),
        (true, true) => bail!("notification stream carried a JSON-RPC id"),
        (false, true) => Ok(()),
        (false, false) => bail!("request stream is missing a JSON-RPC id"),
    }
}

pub(super) fn interpret_mcp_method(req: &JsonRpcRequest) -> McpMethodSummary {
    let mut server_name = None;
    let mut tool_name = None;

    let kind = match req.method.as_str() {
        "initialize" => McpMethodKind::Initialize,
        "notifications/initialized" => McpMethodKind::InitializedNotification,
        "tools/list" => {
            server_name = Some("*".to_string());
            McpMethodKind::ToolsList
        }
        "tools/call" => {
            if let Some(name) = param_str(req, "name") {
                server_name = parse_namespaced(name)
                    .map(|(server, _)| server.to_string())
                    .or_else(|| Some(String::new()));
                tool_name = Some(name.to_string());
            }
            McpMethodKind::ToolsCall
        }
        "resources/list" => {
            server_name = Some("*".to_string());
            McpMethodKind::ResourcesList
        }
        "resources/read" => {
            if let Some(uri) = param_str(req, "uri") {
                server_name = parse_resource_uri(uri)
                    .map(|(server, _)| server.to_string())
                    .or_else(|| Some(String::new()));
            }
            McpMethodKind::ResourcesRead
        }
        "prompts/list" => {
            server_name = Some("*".to_string());
            McpMethodKind::PromptsList
        }
        "prompts/get" => {
            if let Some(name) = param_str(req, "name") {
                server_name = parse_namespaced(name)
                    .map(|(server, _)| server.to_string())
                    .or_else(|| Some(String::new()));
            }
            McpMethodKind::PromptsGet
        }
        _ => McpMethodKind::Unknown,
    };

    let request_preview = req
        .params
        .as_ref()
        .and_then(|params| serde_json::to_string(params).ok())
        .map(|preview| truncate_preview(&preview, MCP_REQUEST_PREVIEW_BYTES));

    McpMethodSummary {
        kind,
        method: req.method.clone(),
        request_id: req.id.as_ref().and_then(json_rpc_id_to_log_string),
        server_name,
        tool_name,
        request_preview,
    }
}

pub(super) fn param_str<'a>(req: &'a JsonRpcRequest, key: &str) -> Option<&'a str> {
    req.params
        .as_ref()
        .and_then(|params| params.get(key))
        .and_then(|value| value.as_str())
}

pub(super) fn mcp_log_attribution(req: &JsonRpcRequest) -> (String, Option<String>) {
    match req.method.as_str() {
        "tools/call" => {
            let tool_name = param_str(req, "name").map(String::from);
            let server_name = tool_name
                .as_deref()
                .and_then(parse_namespaced)
                .map(|(server, _)| server.to_string())
                .unwrap_or_else(|| "gateway".to_string());
            (server_name, tool_name)
        }
        "resources/read" => {
            let server_name = param_str(req, "uri")
                .and_then(parse_resource_uri)
                .map(|(server, _)| server.to_string())
                .unwrap_or_else(|| "gateway".to_string());
            (server_name, None)
        }
        "prompts/get" => {
            let server_name = param_str(req, "name")
                .and_then(parse_namespaced)
                .map(|(server, _)| server.to_string())
                .unwrap_or_else(|| "gateway".to_string());
            (server_name, None)
        }
        "tools/list" | "resources/list" | "prompts/list" => ("*".to_string(), None),
        _ => ("gateway".to_string(), None),
    }
}

pub(in crate::net::mitm_proxy) fn truncate_preview(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = max_bytes;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

pub(super) fn record_method_metric(summary: &McpMethodSummary) {
    ::metrics::counter!(
        metrics::MCP_METHODS_TOTAL,
        "method" => summary.method.clone(),
        "kind" => summary.kind.label(),
    )
    .increment(1);
}

pub(super) async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, out: &OutboundFrame) -> Result<()> {
    let bytes = capsem_proto::encode_mcp_frame(out.stream_id, 0, &out.process_name, &out.payload)?;
    writer.write_all(&bytes).await.context("write MCP frame")?;
    writer.flush().await.context("flush MCP frame")
}

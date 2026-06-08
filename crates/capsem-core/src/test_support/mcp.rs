use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::Router;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler};
use serde::Deserialize;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::http::lower_headers;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedMcpHttpRequest {
    pub method: String,
    pub uri: String,
    pub headers: HashMap<String, String>,
}

impl RecordedMcpHttpRequest {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedMcpToolCall {
    pub tool: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RecordingMcpState {
    http_requests: Arc<Mutex<Vec<RecordedMcpHttpRequest>>>,
    tool_calls: Arc<Mutex<Vec<RecordedMcpToolCall>>>,
}

impl RecordingMcpState {
    pub(crate) fn http_requests(&self) -> Vec<RecordedMcpHttpRequest> {
        self.http_requests
            .lock()
            .expect("MCP HTTP recorder poisoned")
            .clone()
    }

    pub(crate) fn tool_calls(&self) -> Vec<RecordedMcpToolCall> {
        self.tool_calls
            .lock()
            .expect("MCP tool recorder poisoned")
            .clone()
    }
}

pub(crate) struct LocalMcpServer {
    pub(crate) url: String,
    pub(crate) state: RecordingMcpState,
    shutdown: CancellationToken,
    handle: JoinHandle<()>,
}

impl Drop for LocalMcpServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.handle.abort();
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EchoRequest {
    message: String,
}

#[derive(Debug, Clone)]
struct RecordingMcpHandler {
    tool_router: ToolRouter<Self>,
    state: RecordingMcpState,
}

impl RecordingMcpHandler {
    fn new(state: RecordingMcpState) -> Self {
        Self {
            tool_router: Self::tool_router(),
            state,
        }
    }
}

#[tool_router]
impl RecordingMcpHandler {
    #[tool(description = "Echo one message and record the received arguments")]
    fn echo(&self, Parameters(EchoRequest { message }): Parameters<EchoRequest>) -> String {
        self.state
            .tool_calls
            .lock()
            .expect("MCP tool recorder poisoned")
            .push(RecordedMcpToolCall {
                tool: "echo".to_string(),
                arguments: serde_json::json!({ "message": message.clone() }),
            });
        format!("echo:{message}")
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RecordingMcpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Local recording MCP server for Capsem tests")
    }
}

pub(crate) async fn spawn_recording_mcp_server() -> anyhow::Result<LocalMcpServer> {
    let state = RecordingMcpState::default();
    let handler_state = state.clone();
    let shutdown = CancellationToken::new();
    let service: StreamableHttpService<RecordingMcpHandler, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(RecordingMcpHandler::new(handler_state.clone())),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(None)
                .with_cancellation_token(shutdown.child_token()),
        );

    let router =
        Router::new()
            .nest_service("/mcp", service)
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                record_mcp_http_request,
            ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
                .await;
        }
    });

    Ok(LocalMcpServer {
        url: format!("http://{addr}/mcp"),
        state,
        shutdown,
        handle,
    })
}

async fn record_mcp_http_request(
    State(state): State<RecordingMcpState>,
    req: Request,
    next: Next,
) -> axum::response::Response {
    state
        .http_requests
        .lock()
        .expect("MCP HTTP recorder poisoned")
        .push(RecordedMcpHttpRequest {
            method: req.method().to_string(),
            uri: req.uri().to_string(),
            headers: lower_headers(req.headers()),
        });
    next.run(req).await
}

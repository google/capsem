//! One owned frame: human wait cannot stop its connection reading cancellation.
use super::*;

pub(super) struct Dispatcher {
    pub endpoint: Arc<McpEndpointState>,
    pub db: Arc<DbWriter>,
    pub streams: Arc<Mutex<StreamTracker>>,
    pub tx: tokio::sync::mpsc::Sender<OutboundFrame>,
}

pub(super) struct Call {
    pub stream_id: u32,
    pub process_name: String,
    pub request: JsonRpcRequest,
    pub disposition: StreamDisposition,
}

impl Dispatcher {
    pub(super) async fn run(&self, call: Call, held_bytes: usize) -> Result<()> {
        if call.disposition == StreamDisposition::Notification {
            let summary = interpret_mcp_method(&call.request);
            let decision = evaluate_mcp_security_event(
                &self.endpoint,
                &self.db,
                mcp_security_event_from_summary(
                    runtime_mcp_event_type(&summary.method),
                    &summary,
                    &call.process_name,
                    None,
                ),
                held_bytes,
                None,
            )
            .await;
            if decision.is_allowed() {
                let _permit = self
                    .endpoint
                    .inflight
                    .acquire()
                    .await
                    .context("framed MCP inflight semaphore closed")?;
                let _ = self.endpoint.handle_request(&call.request).await;
            }
            let response = JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: None,
                result: None,
                error: None,
                meta: None,
            };
            log_mcp_call_with_policy(
                Arc::clone(&self.db),
                &self.endpoint,
                &call.request,
                &response,
                McpCaller::frame(&call.process_name),
                0,
                McpCallPolicyFields::from(&decision),
            )
            .await;
            return Ok(());
        }
        let response = dispatch_logged_mcp_request(
            Arc::clone(&self.endpoint),
            Arc::clone(&self.db),
            call.request,
            call.process_name.clone(),
            McpTransport::VsockFrame,
        )
        .await;
        self.streams
            .lock()
            .expect("framed MCP stream tracker poisoned")
            .complete(call.stream_id);
        if let Some(response) = response {
            send_response(&self.tx, call.stream_id, &call.process_name, &response.response).await?;
        }
        Ok(())
    }
}

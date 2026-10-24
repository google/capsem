//! Gate one already-executed remote MCP response before delivering its bytes.
use super::*;
use crate::net::proxy_engine::ProxyPolicySnapshot;

pub(super) struct Context {
    pub config: Arc<MitmProxyConfig>,
    pub policy: Arc<ProxyPolicySnapshot>,
    pub observed: mcp_observe::ObservedMcpHttpRequest,
    pub http: HttpSecurityEvent,
    pub upstream_ip: Option<std::net::IpAddr>,
    pub upstream_port: u16,
    pub process_name: Option<String>,
    pub credential_ref: Option<String>,
    pub duration_ms: u64,
    pub decision: SecurityBoundaryDecisionFields,
}

pub(super) struct Outcome {
    pub decision: SecurityBoundaryDecisionFields,
    pub denied: bool,
}

#[derive(Default)]
pub(super) struct Tasks {
    jobs: std::sync::Mutex<tokio::task::JoinSet<()>>,
    stopped: tokio_util::sync::CancellationToken,
}

impl Tasks {
    pub(super) fn until_stopped(stopped: tokio_util::sync::CancellationToken) -> Self {
        Self {
            jobs: Default::default(),
            stopped,
        }
    }
    pub(super) async fn enforce(
        &self,
        context: Context,
        body: Bytes,
    ) -> Result<(Bytes, Outcome), crate::security_engine::SecurityActionError> {
        self.run(move |cancellation| async move {
            let mut body = body;
            enforce(context, &mut body, &cancellation)
                .await
                .map(|outcome| (body, outcome))
        })
        .await
    }

    pub(super) async fn run<T, F, Fut>(&self, work: F) -> Result<T, crate::security_engine::SecurityActionError>
    where
        T: Send + 'static,
        F: FnOnce(tokio_util::sync::CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T, crate::security_engine::SecurityActionError>> + Send + 'static,
    {
        let cancellation = self.stopped.child_token();
        let _cancel_on_disconnect = cancellation.clone().drop_guard();
        let (sender, response) = tokio::sync::oneshot::channel();
        {
            let mut jobs = self.jobs.lock().expect("MCP response tasks poisoned");
            // HTTP/1 drives one response at a time. Reap prior completions so
            // a long-lived connection never retains their task slots.
            while jobs.try_join_next().is_some() {}
            jobs.spawn(async move {
                let result = work(cancellation).await;
                // The connection owns this task until its audit is drained,
                // even when its original response receiver disconnects.
                let _ = sender.send(result);
            });
        }
        response
            .await
            .map_err(|_| crate::security_engine::SecurityActionError::new("HTTP completion worker stopped"))?
    }

    pub(super) async fn run_http<F, Fut>(
        self: &Arc<Self>,
        work: F,
    ) -> Result<hyper::Response<ProxyBoxBody>, anyhow::Error>
    where
        F: FnOnce(tokio_util::sync::CancellationToken) -> Fut,
        Fut: std::future::Future<Output = Result<hyper::Response<ProxyBoxBody>, anyhow::Error>> + Send + 'static,
    {
        let cancellation = self.stopped.child_token();
        // Ordinary requests stay on hyper's task: only an interrupted
        // original is transferred to the connection-owned completion set.
        HeldHttpRequest {
            future: Some(Box::pin(work(cancellation.clone()))),
            tasks: Arc::clone(self),
            cancellation,
        }
        .await
    }

    pub(super) fn spawn_end(&self, future: super::hooks::ChunkEndFuture) {
        self.jobs.lock().expect("HTTP completion tasks poisoned").spawn(future);
    }

    pub(super) async fn drain(&self) {
        self.stopped.cancel();
        loop {
            let mut jobs = {
                let mut owner = self.jobs.lock().expect("MCP response tasks poisoned");
                std::mem::take(&mut *owner)
            };
            if jobs.is_empty() {
                break;
            }
            while let Some(result) = jobs.join_next().await {
                if let Err(error) = result {
                    tracing::warn!(%error, "MCP response worker failed during connection drain");
                }
            }
        }
    }
}

struct HeldHttpRequest<F>
where
    F: std::future::Future<Output = Result<hyper::Response<ProxyBoxBody>, anyhow::Error>> + Send + 'static,
{
    future: Option<std::pin::Pin<Box<F>>>,
    tasks: Arc<Tasks>,
    cancellation: tokio_util::sync::CancellationToken,
}

impl<F> std::future::Future for HeldHttpRequest<F>
where
    F: std::future::Future<Output = Result<hyper::Response<ProxyBoxBody>, anyhow::Error>> + Send + 'static,
{
    type Output = F::Output;
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        let result = this
            .future
            .as_mut()
            .expect("HTTP request polled after completion")
            .as_mut()
            .poll(cx);
        if result.is_ready() {
            this.future = None;
        }
        result
    }
}

impl<F> Drop for HeldHttpRequest<F>
where
    F: std::future::Future<Output = Result<hyper::Response<ProxyBoxBody>, anyhow::Error>> + Send + 'static,
{
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(future) = self.future.take() {
            // Dropping the completed response runs its normal end hooks;
            // those futures join this same owner's finite drain.
            self.tasks
                .jobs
                .lock()
                .expect("HTTP completion tasks poisoned")
                .spawn(async move {
                    drop(future.await);
                });
        }
    }
}

async fn enforce(
    context: Context,
    body: &mut Bytes,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<Outcome, crate::security_engine::SecurityActionError> {
    let preview = String::from_utf8_lossy(body).to_string();
    let tool_list = (context.observed.method == "tools/list").then(|| preview.clone());
    let event = security_event_with_transport(
        context
            .observed
            .security_event(tool_list, Some(&preview))
            .with_http(context.http),
        context.upstream_ip,
        context.upstream_port,
    );
    // The collected body is already bounded; account its original bytes while
    // held. No upstream request is issued or replayed by human resolution.
    let evaluation = context
        .config
        .engine
        .evaluate_cancellable(&context.policy, event, body.len(), Some(cancellation))
        .await?;
    let denied = !evaluation.enforcement.is_allowed();
    let decision = if denied || evaluation.enforcement.ask_id.is_some() {
        context.decision.with_evaluation(&evaluation)
    } else {
        // A later unmatched/allowed response must not erase the original
        // request's reviewed policy provenance.
        context.decision
    };
    if denied {
        *body = Bytes::from(format!(
            "capsem: MCP response blocked by security rule: {}\n",
            evaluation.enforcement.rule_id.as_deref().unwrap_or("unknown")
        ));
    }
    let call = McpCall {
        event_id: decision.approval_event_id.clone(),
        timestamp: SystemTime::now(),
        server_name: context.observed.server_name.clone(),
        method: context.observed.method.clone(),
        tool_name: context.observed.tool_name.clone(),
        request_id: context.observed.request_id.clone(),
        request_preview: context.observed.request_preview.clone(),
        response_preview: Some(String::from_utf8_lossy(body).to_string()),
        decision: if denied { "denied" } else { "allowed" }.to_string(),
        duration_ms: context.duration_ms,
        error_message: denied.then(|| "MCP response blocked by security policy".into()),
        process_name: context.process_name,
        bytes_sent: context.observed.bytes_sent,
        bytes_received: body.len() as u64,
        transport: "http".into(),
        policy_mode: decision.policy_mode.clone(),
        policy_action: decision.policy_action.clone(),
        policy_rule: decision.policy_rule.clone(),
        policy_reason: decision.policy_reason.clone(),
        trace_id: capsem_foundation::telemetry::ambient_capsem_trace_id(),
        credential_ref: context.credential_ref,
    };
    if let Some(event_id) = emit_security_write(&context.config.db, WriteOp::McpCall(call)).await {
        emit_evaluated_security_rules(
            Arc::clone(&context.config.db),
            event_id,
            context.observed.event_type(),
            Arc::clone(context.policy.security_rules()),
            Arc::clone(context.policy.plugins()),
            evaluation.event,
            current_unix_ms(),
            "mcp_over_http_response",
        )
        .await;
    }
    Ok(Outcome { decision, denied })
}

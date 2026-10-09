//! Bounded Responses message association, enforcement and shared hook dispatch.
use super::*;
use crate::net::parsers::sse_parser::SseEvent;
use crate::security_engine::ProcessSecurityEvent;
use hooks::HookState;
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};

const MAX_PENDING: usize = 64;
const MAX_CAPTURED: usize = AI_BODY_CAPTURE_LIMIT * 2;

#[derive(Deserialize)]
struct Wire {
    #[serde(rename = "type")]
    kind: String,
    stream_id: Option<String>,
    generate: Option<bool>,
    response: Option<Head>,
}

#[derive(Deserialize)]
struct Head {
    id: String,
}

struct Turn {
    lane: Option<String>,
    id: Option<String>,
    request: String,
    response: Vec<String>,
    response_bytes: usize,
    policy_snapshot: Arc<crate::net::proxy_engine::ProxyPolicySnapshot>,
    state: HookState,
}

#[derive(Default)]
pub(super) struct Turns {
    pending: VecDeque<Turn>,
    active: HashMap<Option<String>, Turn>,
}

pub(super) enum Request {
    Forward(String),
    Refuse(String),
}

impl Turns {
    pub(super) async fn request(&mut self, text: &str, ctx: &Context) -> anyhow::Result<Request> {
        let wire: Wire = serde_json::from_str(text)?;
        anyhow::ensure!(
            self.pending.len() + self.active.len() < MAX_PENDING,
            "Too many pending Responses turns"
        );
        self.bound(text.len())?;
        let mut turn = Turn::new(
            text,
            wire.stream_id,
            wire.generate != Some(false) && wire.kind == "response.create",
            ctx,
        );
        let result = turn.evaluate(None, ctx)?;
        if !result.enforcement.is_allowed() {
            let error = refusal(&turn.lane, &result.enforcement);
            turn.deny(&result.enforcement);
            turn.response.push(error.clone());
            turn.complete(ctx).await;
            return Ok(Request::Refuse(error));
        }
        turn.decision(&result.enforcement);
        let runtime = result
            .event
            .model
            .and_then(|model| model.request_body)
            .unwrap_or_else(|| text.to_owned());
        anyhow::ensure!(
            runtime.len() <= AI_BODY_CAPTURE_LIMIT,
            "Rewritten Responses request exceeds limit"
        );
        self.bound(runtime.len())?;
        let rewritten: Wire = serde_json::from_str(&runtime)?;
        anyhow::ensure!(
            rewritten.kind == wire.kind && rewritten.stream_id == turn.lane,
            "Security action changed Responses routing"
        );
        turn.request = runtime.clone();
        turn.capture_request();
        if wire.kind == "response.create" {
            self.pending.push_back(turn);
        } else {
            // Control messages still cross the model boundary, but do not
            // invent an inference or consume an upcoming response.created.
            turn.complete(ctx).await;
        }
        Ok(Request::Forward(runtime))
    }

    pub(super) async fn response(&mut self, text: &str, ctx: &Context) -> anyhow::Result<Vec<String>> {
        let wire: Wire = serde_json::from_str(text)?;
        self.bound(text.len() + 1)?;
        if wire.kind == "response.created" {
            anyhow::ensure!(
                !self.active.contains_key(&wire.stream_id),
                "Overlapping Responses on one stream"
            );
            let position = self
                .pending
                .iter()
                .position(|turn| turn.lane == wire.stream_id)
                .ok_or_else(|| anyhow::anyhow!("Responses creation has no matching request"))?;
            let id = wire
                .response
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing Responses ID"))?
                .id
                .clone();
            let mut turn = self.pending.remove(position).unwrap();
            turn.id = Some(id);
            self.active.insert(wire.stream_id.clone(), turn);
        } else if wire.kind == "error" && !self.active.contains_key(&wire.stream_id) {
            if let Some(position) = self.pending.iter().position(|turn| turn.lane == wire.stream_id) {
                let turn = self.pending.remove(position).unwrap();
                self.active.insert(wire.stream_id.clone(), turn);
            }
        }
        let turn = self
            .active
            .get_mut(&wire.stream_id)
            .ok_or_else(|| anyhow::anyhow!("Responses event has no matching turn"))?;
        if let Some(head) = wire.response.as_ref() {
            anyhow::ensure!(
                turn.id.as_ref().is_none_or(|id| id == &head.id),
                "Responses ID changed during turn"
            );
        }
        turn.response_bytes += text.len() + 1;
        anyhow::ensure!(
            turn.response_bytes <= AI_BODY_CAPTURE_LIMIT,
            "Responses turn exceeds capture limit"
        );
        turn.response.push(text.to_owned());
        if !matches!(
            wire.kind.as_str(),
            "response.completed" | "response.failed" | "response.incomplete" | "error"
        ) {
            return Ok(Vec::new());
        }
        let turn = self.active.get(&wire.stream_id).unwrap();
        let response = turn.response.join("\n") + "\n";
        let evaluation = turn.evaluate(Some(&response), ctx)?;
        let output = if evaluation.enforcement.is_allowed() {
            let runtime = evaluation
                .event
                .model
                .and_then(|model| model.response_body)
                .unwrap_or(response);
            anyhow::ensure!(
                runtime.len() <= AI_BODY_CAPTURE_LIMIT,
                "Rewritten Responses output exceeds limit"
            );
            let frames: Vec<_> = runtime.lines().map(str::to_owned).collect();
            for frame in &frames {
                let event: Wire = serde_json::from_str(frame)?;
                anyhow::ensure!(event.stream_id == turn.lane, "Security action changed Responses stream");
                if let Some(head) = event.response {
                    anyhow::ensure!(
                        turn.id.as_ref().is_none_or(|id| id == &head.id),
                        "Security action changed Responses ID"
                    );
                }
            }
            frames
        } else {
            let error = refusal(&turn.lane, &evaluation.enforcement);
            vec![error]
        };
        let mut turn = self.active.remove(&wire.stream_id).unwrap();
        if evaluation.enforcement.is_allowed() {
            turn.decision(&evaluation.enforcement);
        } else {
            turn.deny(&evaluation.enforcement);
        }
        turn.response = output.clone();
        turn.complete(ctx).await;
        Ok(output)
    }

    fn bound(&self, incoming: usize) -> anyhow::Result<()> {
        let total: usize = self
            .pending
            .iter()
            .chain(self.active.values())
            .map(|turn| turn.request.len() + turn.response_bytes)
            .sum();
        anyhow::ensure!(
            incoming <= MAX_CAPTURED.saturating_sub(total),
            "Responses connection capture exceeds limit"
        );
        Ok(())
    }

    pub(super) async fn finish(&mut self, ctx: &Context) {
        for turn in self.pending.drain(..).chain(self.active.drain().map(|(_, turn)| turn)) {
            turn.complete(ctx).await;
        }
    }
}

impl Turn {
    fn new(text: &str, lane: Option<String>, inference: bool, ctx: &Context) -> Self {
        let policy_snapshot = ctx.engine.policy().snapshot();
        let mut state = HookState::default();
        state.set::<Option<TelemetryRequestContext>>(Some(TelemetryRequestContext {
            policy_snapshot: Arc::clone(&policy_snapshot),
            domain: ctx.conn.domain.clone(),
            process_name: ctx.conn.process_name.clone(),
            ai_provider: ctx.conn.ai_provider,
            ai_protocol: ctx.conn.ai_protocol,
            model_traffic: inference,
            method: ctx.method.clone(),
            path: ctx.path.clone(),
            query: ctx.query.clone(),
            status_code: Some(101),
            decision: Decision::Allowed,
            matched_rule: None,
            request_headers: Some(ctx.request_headers.clone()),
            response_headers: Some(ctx.response_headers.clone()),
            start_time: Instant::now(),
            request_body_stats: Arc::new(Mutex::new(BodyStats {
                bytes: text.len() as u64,
                preview: text.as_bytes().to_vec(),
                max_body_capture: AI_BODY_CAPTURE_LIMIT,
            })),
            max_response_body_capture: AI_BODY_CAPTURE_LIMIT,
            port: ctx.conn.port,
            conn_type: if ctx.conn.protocol == Protocol::Tls {
                "wss-mitm"
            } else {
                "ws-mitm"
            },
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
            credential_ref: ctx.credential_ref.clone(),
            credential_observations: ctx.credential_observations.clone(),
            credential_injections: Vec::new(),
        }));
        state.set(decompression_hook::DecompressionConfig { gzip: false });
        Self {
            lane,
            id: None,
            request: text.to_owned(),
            response: vec![],
            response_bytes: 0,
            policy_snapshot,
            state,
        }
    }

    fn evaluate(
        &self,
        response: Option<&str>,
        ctx: &Context,
    ) -> anyhow::Result<crate::security_engine::SecurityBoundaryEvaluation> {
        let provider = ctx
            .conn
            .ai_provider
            .ok_or_else(|| anyhow::anyhow!("Missing Responses provider"))?;
        let model =
            crate::net::ai_traffic::request_parser::parse_request(ModelProtocol::OpenAi, self.request.as_bytes()).model;
        let event = model_security_event(
            RuntimeSecurityEventType::ModelCall,
            provider,
            model,
            Some(self.request.as_bytes()),
            response.map(str::as_bytes),
        )
        .with_http(HttpSecurityEvent {
            host: Some(ctx.conn.domain.clone()),
            method: Some(ctx.method.clone()),
            path: Some(ctx.path.clone()),
            query: ctx.query.clone(),
            status: Some("101".into()),
            body: Some(response.unwrap_or(&self.request).to_owned()),
        })
        .with_process(ProcessSecurityEvent {
            name: ctx.conn.process_name.clone(),
            ..Default::default()
        });
        let event = security_event_with_transport(event, ctx.ip, ctx.conn.port);
        Ok(ctx.engine.evaluate(&self.policy_snapshot, event)?)
    }

    fn decision(&mut self, decision: &crate::security_engine::SecurityEnforcementDecision) {
        if let Some(request) = self
            .state
            .peek_mut::<Option<TelemetryRequestContext>>()
            .and_then(Option::as_mut)
        {
            let fields = SecurityBoundaryDecisionFields::from_enforcement(decision);
            request.policy_mode = fields.policy_mode;
            request.policy_action = fields.policy_action;
            request.policy_rule = fields.policy_rule;
            request.policy_reason = fields.policy_reason;
            request.matched_rule = decision.rule_id.clone();
        }
    }

    fn deny(&mut self, decision: &crate::security_engine::SecurityEnforcementDecision) {
        self.decision(decision);
        if let Some(request) = self
            .state
            .peek_mut::<Option<TelemetryRequestContext>>()
            .and_then(Option::as_mut)
        {
            request.status_code = Some(403);
            request.decision = Decision::Denied;
        }
    }

    fn capture_request(&mut self) {
        if let Some(request) = self
            .state
            .peek::<Option<TelemetryRequestContext>>()
            .and_then(Option::as_ref)
        {
            let mut stats = request.request_body_stats.lock().unwrap();
            stats.bytes = self.request.len() as u64;
            stats.preview = self.request.as_bytes().to_vec();
        }
    }

    async fn complete(mut self, ctx: &Context) {
        for text in self.response {
            let event = serde_json::from_str::<Wire>(&text).ok();
            if let Some(event) = event {
                if self.state.peek::<sse_parser_hook::SseEventStream>().is_none() {
                    self.state.set(sse_parser_hook::SseEventStream::default());
                }
                let stream = self.state.peek_mut::<sse_parser_hook::SseEventStream>().unwrap();
                // Responses JSON event payloads share the existing interpreter.
                // Only this semantic envelope is adapted; archive bytes remain JSON.
                stream.events.push(SseEvent {
                    event_type: Some(event.kind),
                    data: text.clone(),
                });
            }
            let mut bytes = Bytes::from(text + "\n");
            ctx.pipeline
                .dispatch_response_chunk(&mut bytes, &mut self.state, &ctx.conn, None);
        }
        let end = ctx.pipeline.dispatch_response_end(&mut self.state, &ctx.conn, None);
        for pending in end.pending {
            pending.await;
        }
    }
}

fn refusal(lane: &Option<String>, decision: &crate::security_engine::SecurityEnforcementDecision) -> String {
    serde_json::json!({"type": "error", "stream_id": lane, "status": 403, "error": {
        "type": "capsem_policy_error", "code": "policy_denied", "message": format!("Capsem model traffic blocked by {}", decision.rule_id.as_deref().unwrap_or("security policy"))
    }}).to_string()
}

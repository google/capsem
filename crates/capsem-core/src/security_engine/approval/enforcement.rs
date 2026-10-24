//! Only the connected coordinator's decision can release the original call.
use super::client::ApprovalCapability;
use crate::security_engine::*;
use capsem_proto::approval::{ApprovalOutcome, ApprovalRequest};
use std::future::Future;

pub fn resolve_emission<'a>(
    db: &'a DbWriter,
    capability: Option<&'a ApprovalCapability>,
    emission: &'a mut SecurityRuleEmission,
    held_bytes: usize,
) -> impl Future<Output = Result<(), String>> + Send + 'a {
    resolve_emission_registered(db, capability, emission, held_bytes, None)
}

pub async fn resolve_emission_registered(
    db: &DbWriter,
    capability: Option<&ApprovalCapability>,
    emission: &mut SecurityRuleEmission,
    held_bytes: usize,
    registered: Option<tokio::sync::oneshot::Sender<capsem_proto::approval::ApprovalRegistration>>,
) -> Result<(), String> {
    if emission.enforcement.action != SecurityEnforcementAction::Ask {
        return Ok(());
    }
    let outcome = if let Some(capability) = capability {
        db.flush_checked().await?;
        capability
            .decide_registered(
                request(&emission.enforcement, &emission.event, &emission.event_id, held_bytes)?,
                registered,
            )
            .await
    } else {
        ApprovalOutcome::Unavailable
    };
    apply(&mut emission.enforcement, &mut emission.event, outcome)
}

pub(crate) async fn resolve_proxy(
    ledger: &dyn crate::net::proxy_engine::ProxyLedger,
    capability: Option<&ApprovalCapability>,
    rules: &SecurityRuleSet,
    evaluation: &mut SecurityBoundaryEvaluation,
    held_bytes: usize,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
    registered: Option<tokio::sync::oneshot::Sender<capsem_proto::approval::ApprovalRegistration>>,
) -> Result<(), String> {
    if evaluation.enforcement.action != SecurityEnforcementAction::Ask {
        return Ok(());
    }
    let event_id = SecurityEventId::new_uuid4();
    evaluation.approval_event_id = Some(event_id.clone());
    let ask_id = SecurityEventId::new_uuid4();
    let rule = rules
        .rules()
        .iter()
        .find(|rule| evaluation.enforcement.rule_id.as_ref() == Some(&rule.rule_id))
        .ok_or("approval does not identify a matched rule")?;
    let pending = security_ask_pending_event(
        ask_id.clone(),
        event_id.clone(),
        evaluation.event.event_type,
        rule,
        &evaluation.event,
        super::super::current_unix_ms(),
    )?;
    evaluation.enforcement.ask_id = Some(ask_id);
    // No upstream descriptor or side effect exists here. Persist the pending
    // event before the coordinator can offer a decision to a client.
    super::super::emission::emit_security_write_with(WriteOp::SecurityAskEvent(pending), |op| ledger.write(op)).await?;
    ledger.flush().await?;
    let outcome = if let Some(capability) = capability {
        let decision = capability.decide_registered(
            request(&evaluation.enforcement, &evaluation.event, &event_id, held_bytes)?,
            registered,
        );
        if let Some(cancellation) = cancellation {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => ApprovalOutcome::Cancelled,
                outcome = decision => outcome,
            }
        } else {
            decision.await
        }
    } else {
        ApprovalOutcome::Unavailable
    };
    apply(&mut evaluation.enforcement, &mut evaluation.event, outcome)
}

fn request(
    enforcement: &SecurityEnforcementDecision,
    event: &SecurityEvent,
    event_id: &SecurityEventId,
    held_bytes: usize,
) -> Result<ApprovalRequest, String> {
    let request = ApprovalRequest {
        ask_id: enforcement
            .ask_id
            .as_ref()
            .ok_or("missing original approval id")?
            .as_str()
            .into(),
        event_id: event_id.as_str().into(),
        event_type: event.event_type.as_str().into(),
        rule_id: enforcement.rule_id.clone().ok_or("missing approval rule")?,
        rule_name: enforcement.rule_name.clone().ok_or("missing approval rule name")?,
        summary: summary(event),
        held_bytes: held_bytes as u64,
    };
    request.validate().map_err(str::to_string)?;
    Ok(request)
}

fn summary(event: &SecurityEvent) -> String {
    if let Some(http) = &event.http {
        return format!(
            "HTTP {} {}",
            http.method.as_deref().unwrap_or("request"),
            http.host.as_deref().unwrap_or("destination")
        );
    }
    if let Some(dns) = &event.dns {
        return format!("DNS {}", dns.qname.as_deref().unwrap_or("lookup"));
    }
    if let Some(mcp) = &event.mcp {
        return format!(
            "MCP {} {}",
            mcp.server_name.as_deref().unwrap_or("server"),
            mcp.tool_call_name
                .as_deref()
                .or(mcp.method.as_deref())
                .unwrap_or("operation")
        );
    }
    format!("{} operation", event.event_type.as_str())
}

/// Conservative retained copies for proxy semantic bodies and request heads.
/// Oversized inputs fail before human wait; previews never determine admission.
pub(crate) fn held_bytes(event: &SecurityEvent) -> usize {
    let mut bytes = event
        .http
        .as_ref()
        .and_then(|http| http.body.as_ref())
        .map_or(0, String::len);
    if let Some(model) = &event.model {
        bytes = bytes
            .saturating_add(model.request_body.as_ref().map_or(0, String::len))
            .saturating_add(model.response_body.as_ref().map_or(0, String::len));
    }
    if let Some(request) = &event.http_request {
        bytes = bytes.saturating_add(request.query.as_ref().map_or(0, String::len));
        for (name, value) in &request.headers {
            bytes = bytes.saturating_add(name.as_str().len()).saturating_add(value.len());
        }
    }
    bytes.saturating_mul(2)
}

fn apply(
    enforcement: &mut SecurityEnforcementDecision,
    event: &mut SecurityEvent,
    outcome: ApprovalOutcome,
) -> Result<(), String> {
    let ask_id = enforcement
        .ask_id
        .as_ref()
        .ok_or("missing original approval id")?
        .as_str();
    let allowed = outcome.is_allowed() && event.decision.effective == SecurityDecisionKind::Ask;
    let status = if allowed {
        SecurityAskStatus::Approved
    } else {
        SecurityAskStatus::Denied
    };
    *enforcement = enforcement
        .with_ask_resolution(
            ask_id,
            status,
            Some(if allowed {
                "human approved original operation"
            } else {
                "approval denied or unavailable"
            }),
        )
        .map_err(|error| error.to_string())?;
    // This is the narrow, capability-backed Ask transition; a Block never
    // passes through this path and cannot be relaxed by an approval.
    event.decision.effective = if allowed {
        SecurityDecisionKind::Allow
    } else {
        SecurityDecisionKind::Block
    };
    Ok(())
}

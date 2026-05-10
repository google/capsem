//! `PolicyHook`: domain + method allow/deny enforcement, expressed as
//! a `Hook`. Subscribes to `Event::RawRequestHead` (L1) so it runs
//! before any upstream dial. On deny it returns
//! `Stop(Reject(403))`.
//!
//! T1 slice 2b. Slice 2c will replace the inline call to
//! `NetworkPolicy::evaluate` in `handle_request` with a dispatch
//! through this hook.

#![allow(dead_code)]

use std::pin::Pin;
use std::sync::{Arc, RwLock};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use tracing::{debug, instrument, warn};

use super::events::{Event, EventKind, EventMask};
use super::hooks::{Hook, HookCtx, HookOutcome, StopAction};
use super::metrics as m;
use crate::net::policy::{NetworkPolicy, PolicyDecision};

/// Live-swappable network policy reference. Same shape as
/// `MitmProxyConfig::policy` so the hook + the inline call site share
/// the same source of truth during the slice-2c transition.
pub type LivePolicy = Arc<RwLock<Arc<NetworkPolicy>>>;

/// Per-connection scratch slot the hook stashes its evaluation in,
/// so `handle_request` can read it back after `pipeline.dispatch`
/// returns and use the matched-rule + reason for telemetry context.
#[derive(Clone, Default)]
pub struct LastPolicyDecision {
    pub allowed: bool,
    pub matched_rule: String,
    pub reason: String,
}

/// Policy enforcement hook. Returns `Stop(Reject)` for denied
/// requests so the dispatcher short-circuits before the upstream
/// dial. Decision is logged at `target = "mitm.policy"`.
pub struct PolicyHook {
    policy: LivePolicy,
}

impl PolicyHook {
    pub fn new(policy: LivePolicy) -> Self {
        Self { policy }
    }
}

impl Hook for PolicyHook {
    fn name(&self) -> &'static str {
        "policy"
    }

    fn interest(&self) -> EventMask {
        EventMask::single(EventKind::RawRequestHead)
    }

    fn priority(&self) -> i32 {
        // Run before any other RawRequestHead consumer (decompression
        // setup, telemetry init) so a denied request short-circuits
        // cleanly without touching downstream state.
        -1000
    }

    fn on_event<'a, 'b>(
        &'a self,
        ev: &'b mut Event<'_>,
        ctx: &'b mut HookCtx<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
    where
        'a: 'b,
    {
        let policy = self.policy.clone();
        Box::pin(async move {
            let parts = match ev {
                Event::RawRequestHead(parts) => parts,
                // EventMask should make this unreachable in practice;
                // be defensive in case the dispatcher is misconfigured.
                _ => return HookOutcome::Continue,
            };

            let domain = ctx.conn().domain.clone();
            let method = parts.method.to_string();
            let snapshot: Arc<NetworkPolicy> = policy.read().expect("policy lock poisoned").clone();
            let decision = snapshot.evaluate(&domain, &method);

            // Stash the evaluation so handle_request can use it for
            // telemetry context after dispatch returns.
            let slot = ctx.state::<LastPolicyDecision>(LastPolicyDecision::default);
            slot.allowed = decision.allowed;
            slot.matched_rule = decision.matched_rule.clone();
            slot.reason = decision.reason.clone();

            evaluate_decision(&decision, &domain, &method)
        })
    }
}

/// Map a `PolicyDecision` to a `HookOutcome` + emit the matching
/// tracing + counter signals. Pulled out so the slice-2c rewire can
/// call this from `handle_request` in parallel-deploy mode without
/// duplicating the rendering.
#[instrument(skip_all, target = "mitm.policy", fields(domain, method, decision = tracing::field::Empty, rule = %decision.matched_rule))]
pub(super) fn evaluate_decision(
    decision: &PolicyDecision,
    domain: &str,
    method: &str,
) -> HookOutcome {
    if decision.allowed {
        metrics::counter!(m::POLICY_DECISIONS_TOTAL, "decision" => "allow").increment(1);
        tracing::Span::current().record("decision", "allow");
        debug!(target: "mitm.policy", domain, method, rule = %decision.matched_rule, "allow");
        HookOutcome::Continue
    } else {
        metrics::counter!(m::POLICY_DECISIONS_TOTAL, "decision" => "deny").increment(1);
        tracing::Span::current().record("decision", "deny");
        warn!(target: "mitm.policy", domain, method, rule = %decision.matched_rule, reason = %decision.reason, "deny");
        let body = Full::new(Bytes::from_static(b"forbidden"))
            .map_err(|never| match never {})
            .boxed();
        let resp = http::Response::builder()
            .status(http::StatusCode::FORBIDDEN)
            .header("content-type", "text/plain; charset=utf-8")
            .body(body)
            .expect("static response build");
        HookOutcome::Stop(StopAction::Reject(resp))
    }
}

#[cfg(test)]
mod tests;

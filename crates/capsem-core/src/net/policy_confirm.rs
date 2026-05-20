//! Shared `ask -> confirm()` plumbing for Policy.
//!
//! Every callback site that matches a rule with action `ask` calls a
//! `Confirmer` to resolve the ask into a final `Accept | Deny` outcome.
//! The placeholder implementation always accepts so existing flows keep
//! working until real decision authorities (UI prompter, remote policy
//! plugin, automated resolver) plug in behind the same trait.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use capsem_proto::poll::RetryOpts;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::net::policy::PolicyCallback;
use crate::poll::poll_until;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Accept,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmerKind {
    Placeholder,
    UserUi,
    RemotePlugin,
    Automated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmArgs {
    pub callback: PolicyCallback,
    pub rule_id: String,
    pub args_snapshot: Value,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub reason: Option<String>,
}

#[async_trait]
pub trait Confirmer: Send + Sync {
    async fn confirm(&self, args: ConfirmArgs) -> Decision;

    fn kind(&self) -> ConfirmerKind;
}

/// Default backoff configuration for confirm calls: 30 s overall
/// budget, initial per-attempt delay 50 ms growing to 500 ms. Sourced
/// from the shared `capsem_proto::poll::RetryOpts` infrastructure so
/// the timing behavior is configurable and consistent with the rest of
/// the codebase's "wait with backoff" primitives.
pub fn default_confirm_backoff(label: &'static str) -> RetryOpts {
    RetryOpts::new(label, Duration::from_secs(30))
}

/// Resolve a `Confirmer::confirm` call with exponential-backoff
/// fail-closed semantics. Uses the shared `poll_until` retry primitive:
/// each attempt wraps `confirmer.confirm` in a per-attempt budget of
/// `opts.max_delay`; if the attempt exceeds the budget the future is
/// dropped (so cancel-safe authorities are required) and `poll_until`
/// sleeps with exponential backoff (starting at `opts.initial_delay`,
/// capped at `opts.max_delay`) before retrying. On overall
/// `opts.timeout` elapse the wrapper returns `Decision::Deny` so the
/// runtime fails closed.
///
/// Panic safety: if `confirm()` panics, the panic propagates out
/// through `.await`. Production callsites all sit inside a spawned
/// task, so a panicking confirmer is contained at the runtime
/// task boundary (see the `..._confirmer_panic_isolated_to_task`
/// regression locks).
pub async fn confirm_with_backoff(
    confirmer: &Arc<dyn Confirmer>,
    args: ConfirmArgs,
    opts: &RetryOpts,
) -> Decision {
    let per_attempt = opts.max_delay;
    let confirmer = Arc::clone(confirmer);
    let f = || {
        let confirmer = Arc::clone(&confirmer);
        let args = args.clone();
        async move {
            tokio::time::timeout(per_attempt, confirmer.confirm(args))
                .await
                .ok()
        }
    };
    match poll_until(opts.clone(), f).await {
        Ok(decision) => decision,
        Err(_timed_out) => Decision::Deny,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PlaceholderConfirmer;

#[async_trait]
impl Confirmer for PlaceholderConfirmer {
    async fn confirm(&self, _args: ConfirmArgs) -> Decision {
        Decision::Accept
    }

    fn kind(&self) -> ConfirmerKind {
        ConfirmerKind::Placeholder
    }
}

#[cfg(test)]
mod tests;

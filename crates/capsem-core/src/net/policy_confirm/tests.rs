use super::*;
use crate::net::policy_v2::PolicyCallback;
use capsem_proto::poll::RetryOpts;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn sample_args() -> ConfirmArgs {
    ConfirmArgs {
        callback: PolicyCallback::HttpRequest,
        rule_id: "security.rules.http.example".to_string(),
        args_snapshot: json!({
            "request": {"host": "example.com", "path": "/"}
        }),
        trace_id: Some("trace-abc".to_string()),
        session_id: Some("vm-1".to_string()),
        reason: Some("matched rule asks for confirmation".to_string()),
    }
}

#[tokio::test]
async fn placeholder_confirmer_always_returns_accept() {
    let confirmer = PlaceholderConfirmer;
    let decision = confirmer.confirm(sample_args()).await;
    assert_eq!(decision, Decision::Accept);
}

#[test]
fn placeholder_confirmer_advertises_placeholder_kind() {
    let confirmer = PlaceholderConfirmer;
    assert_eq!(confirmer.kind(), ConfirmerKind::Placeholder);
}

#[test]
fn decision_serializes_to_lowercase_wire_form() {
    assert_eq!(
        serde_json::to_string(&Decision::Accept).unwrap(),
        "\"accept\""
    );
    assert_eq!(serde_json::to_string(&Decision::Deny).unwrap(), "\"deny\"");
}

#[test]
fn confirmer_kind_serializes_to_snake_case() {
    assert_eq!(
        serde_json::to_string(&ConfirmerKind::Placeholder).unwrap(),
        "\"placeholder\""
    );
    assert_eq!(
        serde_json::to_string(&ConfirmerKind::UserUi).unwrap(),
        "\"user_ui\""
    );
    assert_eq!(
        serde_json::to_string(&ConfirmerKind::RemotePlugin).unwrap(),
        "\"remote_plugin\""
    );
    assert_eq!(
        serde_json::to_string(&ConfirmerKind::Automated).unwrap(),
        "\"automated\""
    );
}

#[tokio::test]
async fn confirmer_trait_is_object_safe_for_dyn_dispatch() {
    let confirmer: Arc<dyn Confirmer> = Arc::new(PlaceholderConfirmer);
    let decision = confirmer.confirm(sample_args()).await;
    assert_eq!(decision, Decision::Accept);
}

/// Confirmer that returns a fixed decision after a configurable delay
/// and counts call attempts so backoff retry behavior can be observed.
struct FixedConfirmer {
    decision: Decision,
    delay: Duration,
    calls: Arc<AtomicU32>,
}

#[async_trait]
impl Confirmer for FixedConfirmer {
    async fn confirm(&self, _args: ConfirmArgs) -> Decision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.decision
    }
    fn kind(&self) -> ConfirmerKind {
        ConfirmerKind::Automated
    }
}

struct HangingConfirmer;

#[async_trait]
impl Confirmer for HangingConfirmer {
    async fn confirm(&self, _args: ConfirmArgs) -> Decision {
        // Sleep effectively forever; the wrapper must enforce the
        // per-attempt budget and the overall timeout.
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Decision::Accept
    }
    fn kind(&self) -> ConfirmerKind {
        ConfirmerKind::Automated
    }
}

struct PanickingConfirmer;

#[async_trait]
impl Confirmer for PanickingConfirmer {
    async fn confirm(&self, _args: ConfirmArgs) -> Decision {
        panic!("synthetic panic from confirmer for adversarial test");
    }
    fn kind(&self) -> ConfirmerKind {
        ConfirmerKind::Automated
    }
}

fn fast_opts() -> RetryOpts {
    // Total budget large enough that the confirmer's first attempt
    // returns well before timeout, but per-attempt max_delay still
    // bounds a single attempt to ~10ms.
    RetryOpts {
        label: "confirm-backoff-test",
        timeout: Duration::from_millis(500),
        initial_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(10),
    }
}

#[tokio::test]
async fn confirm_with_backoff_passes_accept_through() {
    let calls = Arc::new(AtomicU32::new(0));
    let confirmer: Arc<dyn Confirmer> = Arc::new(FixedConfirmer {
        decision: Decision::Accept,
        delay: Duration::ZERO,
        calls: Arc::clone(&calls),
    });
    let opts = fast_opts();
    let decision = confirm_with_backoff(&confirmer, sample_args(), &opts).await;
    assert_eq!(decision, Decision::Accept);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a non-hanging confirmer must be called exactly once"
    );
}

#[tokio::test]
async fn confirm_with_backoff_passes_deny_through() {
    let calls = Arc::new(AtomicU32::new(0));
    let confirmer: Arc<dyn Confirmer> = Arc::new(FixedConfirmer {
        decision: Decision::Deny,
        delay: Duration::ZERO,
        calls: Arc::clone(&calls),
    });
    let opts = fast_opts();
    let decision = confirm_with_backoff(&confirmer, sample_args(), &opts).await;
    assert_eq!(decision, Decision::Deny);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn confirm_with_backoff_returns_deny_on_overall_timeout() {
    // A hanging confirmer must fail closed at the overall RetryOpts
    // timeout, not wedge the caller. Use a tight 80 ms overall budget
    // and a 10 ms per-attempt cap, then wall-clock-guard the wrapper
    // against a 1 s upper bound -- if the wrapper ever stopped
    // enforcing the deadline this guard would fail the test instead
    // of timing out the suite.
    let confirmer: Arc<dyn Confirmer> = Arc::new(HangingConfirmer);
    let opts = RetryOpts {
        label: "confirm-backoff-hang-test",
        timeout: Duration::from_millis(80),
        initial_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(10),
    };
    let start = Instant::now();
    let decision = confirm_with_backoff(&confirmer, sample_args(), &opts).await;
    let elapsed = start.elapsed();
    assert_eq!(decision, Decision::Deny, "hang must fail closed");
    assert!(
        elapsed < Duration::from_secs(1),
        "wrapper must enforce timeout, took {elapsed:?}"
    );
}

#[tokio::test]
async fn confirm_with_backoff_propagates_confirmer_panic() {
    // A panicking confirmer must surface as a panic across the await
    // boundary (not be silently swallowed). Production callsites all
    // run inside a spawned task, so the runtime contains the panic at
    // the task boundary -- locked here by asserting the JoinError
    // reports `is_panic()`.
    let confirmer: Arc<dyn Confirmer> = Arc::new(PanickingConfirmer);
    let opts = fast_opts();
    let args = sample_args();
    let join = tokio::spawn(async move { confirm_with_backoff(&confirmer, args, &opts).await });
    let err = match join.await {
        Ok(d) => panic!("confirmer panic must not be swallowed; got {d:?}"),
        Err(e) => e,
    };
    assert!(
        err.is_panic(),
        "panic must surface as a panic JoinError, got {err:?}"
    );
}

#[test]
fn default_confirm_backoff_applies_documented_defaults() {
    // The doc-comment on `default_confirm_backoff` promises:
    // 30 s overall budget, 50 ms initial delay, 500 ms max delay,
    // and the caller-supplied label. Lock those values so a future
    // edit to the defaults can't silently shift production timing.
    let opts = default_confirm_backoff("dns.request");
    assert_eq!(opts.label, "dns.request");
    assert_eq!(opts.timeout, Duration::from_secs(30));
    assert_eq!(opts.initial_delay, Duration::from_millis(50));
    assert_eq!(opts.max_delay, Duration::from_millis(500));
}

//! An HTTP exchange's rule rows record the outcome the proxy enforced.
//!
//! The rows are written after the response, when nothing is enforced any
//! more, so they start from the request's own decision: a denied request is a
//! block, an allowed one stays allowed even if a rule matches it after the
//! fact (google/capsem#203, owned by #229).
use super::*;
use crate::security_engine::SecurityDecisionKind;

#[test]
fn a_denied_request_enters_the_rule_ledger_as_blocked() {
    for (decision, expected) in [
        (Decision::Denied, SecurityDecisionKind::Block),
        (Decision::Allowed, SecurityDecisionKind::Allow),
    ] {
        let mut req_ctx = anthropic_req_ctx();
        req_ctx.decision = decision;
        let event = security_event_from_net_event(&build_net_event(&req_ctx, &empty_resp_stats()));
        assert_eq!(event.decision.effective, expected, "{decision:?}");
    }
}

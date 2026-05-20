use super::*;

use crate::net::dns::server::DnsHandlerResult;
use crate::net::parsers::dns_parser::DnsQuery;
use capsem_logger::events::Decision;

fn allowed_result() -> DnsHandlerResult {
    DnsHandlerResult {
        answer_bytes: vec![1, 2, 3, 4],
        query: Some(DnsQuery {
            id: 0x1234,
            qname: "anthropic.com".into(),
            qtype: 1,
            qclass: 1,
            extra_questions: 0,
        }),
        decision: Decision::Allowed,
        matched_rule: None,
        upstream_resolver_ms: 42,
        rcode: 0,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
    }
}

fn denied_result() -> DnsHandlerResult {
    DnsHandlerResult {
        answer_bytes: vec![1, 2],
        query: Some(DnsQuery {
            id: 1,
            qname: "api.openai.com".into(),
            qtype: 1,
            qclass: 1,
            extra_questions: 0,
        }),
        decision: Decision::Denied,
        matched_rule: Some("api.openai.com".into()),
        upstream_resolver_ms: 0,
        rcode: 3,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
    }
}

#[test]
fn build_event_for_allowed_query() {
    let res = allowed_result();
    let evt = build_dns_event(&res, Some("udp"), None, Some("trace_abc".into()));
    assert_eq!(evt.qname, "anthropic.com");
    assert_eq!(evt.qtype, 1);
    assert_eq!(evt.qclass, 1);
    assert_eq!(evt.rcode, 0);
    assert_eq!(evt.decision, "allowed");
    assert!(evt.matched_rule.is_none());
    assert_eq!(evt.source_proto.as_deref(), Some("udp"));
    assert_eq!(evt.upstream_resolver_ms, 42);
    assert_eq!(evt.trace_id.as_deref(), Some("trace_abc"));
    assert!(evt.process_name.is_none());
    assert!(evt.policy_mode.is_none());
    assert!(evt.policy_action.is_none());
    assert!(evt.policy_rule.is_none());
    assert!(evt.policy_reason.is_none());
}

#[test]
fn build_event_for_denied_query_carries_matched_rule() {
    let res = denied_result();
    let evt = build_dns_event(&res, Some("tcp"), None, None);
    assert_eq!(evt.qname, "api.openai.com");
    assert_eq!(evt.decision, "denied");
    assert_eq!(evt.matched_rule.as_deref(), Some("api.openai.com"));
    assert_eq!(evt.rcode, 3);
    assert_eq!(evt.upstream_resolver_ms, 0); // policy short-circuit
    assert_eq!(evt.source_proto.as_deref(), Some("tcp"));
    assert!(evt.trace_id.is_none());
}

#[test]
fn build_event_for_undecodable_query_uses_sentinel_qname() {
    // When parse_query failed, the handler returns a result with
    // query=None. The telemetry row still gets emitted (so the
    // operator can see "the agent sent us garbage at this time").
    let res = DnsHandlerResult {
        answer_bytes: Vec::new(),
        query: None,
        decision: Decision::Error,
        matched_rule: None,
        upstream_resolver_ms: 0,
        rcode: 1,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
    };
    let evt = build_dns_event(&res, Some("udp"), None, None);
    assert_eq!(evt.qname, "INVALID_DNS_BYTES");
    assert_eq!(evt.qtype, 0);
    assert_eq!(evt.qclass, 0);
    assert_eq!(evt.decision, "error");
    assert_eq!(evt.rcode, 1);
}

#[test]
fn build_event_decision_strings_match_logger_convention() {
    // The decision string is what gets stored verbatim in
    // dns_events.decision; the inspect-session reader matches on
    // exactly these strings, so a typo would break joins. Assert
    // the round-trip with Decision::parse_str so any future variant
    // doesn't drift.
    for d in [Decision::Allowed, Decision::Denied, Decision::Error] {
        let mut res = allowed_result();
        res.decision = d;
        let evt = build_dns_event(&res, Some("udp"), None, None);
        assert_eq!(evt.decision, d.as_str());
        assert_eq!(Decision::parse_str(&evt.decision), d);
    }
}

#[test]
fn build_event_source_proto_optional() {
    let res = allowed_result();
    let evt = build_dns_event(&res, None, None, None);
    assert!(evt.source_proto.is_none());
}

#[test]
fn build_event_process_name_passthrough() {
    let res = allowed_result();
    let evt = build_dns_event(&res, Some("udp"), Some("curl".into()), None);
    assert_eq!(evt.process_name.as_deref(), Some("curl"));
}

#[test]
fn build_event_carries_policy_fields() {
    let mut res = denied_result();
    res.matched_rule = Some("policy.dns.block_openai".into());
    res.policy_mode = Some("enforce".into());
    res.policy_action = Some("block".into());
    res.policy_rule = Some("policy.dns.block_openai".into());
    res.policy_reason = Some("DNS to OpenAI API is blocked".into());

    let evt = build_dns_event(
        &res,
        Some("udp"),
        Some("claude".into()),
        Some("trace_dns".into()),
    );

    assert_eq!(evt.decision, "denied");
    assert_eq!(evt.matched_rule.as_deref(), Some("policy.dns.block_openai"));
    assert_eq!(evt.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(evt.policy_action.as_deref(), Some("block"));
    assert_eq!(evt.policy_rule.as_deref(), Some("policy.dns.block_openai"));
    assert_eq!(
        evt.policy_reason.as_deref(),
        Some("DNS to OpenAI API is blocked")
    );
    assert_eq!(evt.process_name.as_deref(), Some("claude"));
    assert_eq!(evt.trace_id.as_deref(), Some("trace_dns"));
}

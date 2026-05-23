use super::*;

use crate::net::dns::server::DnsHandlerResult;
use capsem_logger::events::Decision;
use capsem_network_engine::dns_parser::DnsQuery;
use capsem_security_engine::{
    CelEnforcementEvaluator, CelEnforcementRule, SecurityDecisionAction, SecurityEngine,
};
use std::time::{Duration, SystemTime};

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

fn dns_query() -> DnsQuery {
    DnsQuery {
        id: 0x1234,
        qname: "blocked.example.com".into(),
        qtype: 1,
        qclass: 1,
        extra_questions: 0,
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

#[test]
fn build_resolved_security_event_for_denied_query() {
    let mut res = denied_result();
    res.matched_rule = Some("policy.dns.block_openai".into());
    res.policy_mode = Some("enforce".into());
    res.policy_action = Some("block".into());
    res.policy_rule = Some("policy.dns.block_openai".into());
    res.policy_reason = Some("DNS to OpenAI API is blocked".into());
    let evt = build_dns_event(
        &res,
        Some("udp"),
        Some("agent".into()),
        Some("trace_dns".into()),
    );

    let resolved = build_dns_resolved_security_event(&evt);

    assert_eq!(resolved.event.common.event_type, "dns.request");
    assert!(matches!(
        resolved.final_action,
        capsem_security_engine::SecurityAction::Block(_)
    ));
    assert_eq!(
        resolved.event.decision.as_ref().unwrap().rule.as_deref(),
        Some("policy.dns.block_openai")
    );
    assert_eq!(
        resolved.steps[0].rule_id.as_deref(),
        Some("policy.dns.block_openai")
    );
    match resolved.event.subject {
        capsem_security_engine::SecurityEventSubject::Dns(subject) => {
            assert_eq!(subject.qname, "api.openai.com");
            assert_eq!(subject.domain_class, "external");
        }
        other => panic!("expected DNS subject, got {other:?}"),
    }
}

#[test]
fn build_dns_security_event_from_query_uses_canonical_dns_policy_root() {
    let event = build_dns_security_event_from_query(&dns_query(), Some("trace_dns".into()));

    assert_eq!(event.common.event_type, "dns.request");
    assert_eq!(event.common.trace_id.as_deref(), Some("trace_dns"));
    match event.subject {
        capsem_security_engine::SecurityEventSubject::Dns(subject) => {
            assert_eq!(subject.qname, "blocked.example.com");
            assert_eq!(subject.domain_class, "external");
        }
        other => panic!("expected DNS subject, got {other:?}"),
    }
}

#[test]
fn runtime_dns_block_projects_to_denied_dns_result_without_upstream() {
    let query = dns_query();
    let event = build_dns_security_event_from_query(&query, Some("trace_dns".into()));
    let evaluator = CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
        id: "runtime.block-dns".into(),
        pack_id: Some("runtime-benchmark".into()),
        condition: "dns.request.qname == 'blocked.example.com'".into(),
        decision: SecurityDecisionAction::Block,
        reason: Some("blocked DNS benchmark domain".into()),
    }])
    .unwrap();

    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(evaluator));

    let result = engine.evaluate(event).unwrap();
    assert!(!dns_security_result_allows_transport(&result));
    let dns_result = build_dns_runtime_denied_result(&[], query, &result);

    assert_eq!(dns_result.decision, Decision::Denied);
    assert_eq!(dns_result.upstream_resolver_ms, 0);
    assert_eq!(dns_result.rcode, 3);
    assert_eq!(dns_result.policy_mode.as_deref(), Some("runtime"));
    assert_eq!(dns_result.policy_action.as_deref(), Some("block"));
    assert_eq!(dns_result.policy_rule.as_deref(), Some("runtime.block-dns"));
    assert_eq!(
        dns_result.policy_reason.as_deref(),
        Some("blocked DNS benchmark domain")
    );
}

#[test]
fn same_millisecond_dns_events_keep_distinct_security_ids() {
    let evt = build_dns_event(
        &allowed_result(),
        Some("udp"),
        Some("agent".into()),
        Some("trace_dns".into()),
    );
    let mut first = evt.clone();
    first.timestamp = SystemTime::UNIX_EPOCH + Duration::from_millis(42);
    let mut second = evt;
    second.timestamp = SystemTime::UNIX_EPOCH + Duration::from_millis(42) + Duration::from_nanos(1);

    let first_resolved = build_dns_resolved_security_event(&first);
    let second_resolved = build_dns_resolved_security_event(&second);

    assert_ne!(
        first_resolved.event.common.event_id,
        second_resolved.event.common.event_id
    );
}

//! Microbenchmarks for Policy V2 matching and hook wire decoding.

use capsem_core::net::policy_config::{PolicyCallback, PolicyConfig};
use capsem_core::net::policy_hook_spec::HookDecisionResponse;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde_json::json;

fn config() -> PolicyConfig {
    serde_json::from_value(json!({
        "http": {
            "bench_http": {
                "on": "http.request",
                "if": "request.host == 'api.openai.com' && request.path == '/v1/chat/completions' && request.method == 'POST'",
                "decision": "block",
                "priority": 10,
                "reason": "bench http"
            }
        },
        "dns": {
            "bench_dns": {
                "on": "dns.query",
                "if": "qname == 'api.openai.com' && qtype == 'A'",
                "decision": "block",
                "priority": 10,
                "reason": "bench dns"
            }
        },
        "model": {
            "bench_model_response": {
                "on": "model.response",
                "if": "provider == 'openai' && model == 'gpt-4o-mini' && response.text.contains('secret')",
                "decision": "block",
                "priority": 10,
                "reason": "bench model"
            },
            "bench_model_tool_call": {
                "on": "model.tool_call",
                "if": "provider == 'openai' && model == 'gpt-4o-mini' && tool.name == 'search' && tool.arguments.query == 'secret'",
                "decision": "rewrite",
                "priority": 20,
                "reason": "bench tool call",
                "rewrite_target": "tool.arguments.query =~ 'secret'",
                "rewrite_value": "[redacted]"
            }
        },
        "hook": {
            "bench_hook": {
                "on": "hook.decision",
                "if": "callback == 'model.tool_call' && decision == 'block' && endpoint.id == 'corp'",
                "decision": "block",
                "priority": 10,
                "reason": "bench hook"
            }
        }
    }))
    .expect("bench policy config")
}

fn bench_policy_matching(c: &mut Criterion) {
    let policy = config();
    let http_subject = json!({
        "request": {
            "host": "api.openai.com",
            "path": "/v1/chat/completions",
            "method": "POST"
        }
    });
    let dns_subject = json!({
        "qname": "api.openai.com",
        "qtype": "A",
        "protocol": "udp"
    });
    let model_response_subject = json!({
        "provider": "openai",
        "model": "gpt-4o-mini",
        "response": {"text": "contains secret"},
        "response.text": "contains secret"
    });
    let tool_call_subject = json!({
        "provider": "openai",
        "model": "gpt-4o-mini",
        "tool": {
            "name": "search",
            "arguments": {"query": "secret"}
        }
    });
    let hook_subject = json!({
        "callback": "model.tool_call",
        "decision": "block",
        "endpoint": {"id": "corp"},
        "rule": {"id": "policy.model.block_secret"}
    });

    c.bench_function("policy_v2_http_request_match", |b| {
        b.iter(|| {
            black_box(
                policy
                    .find_matching_rule(PolicyCallback::HttpRequest, black_box(&http_subject))
                    .unwrap(),
            )
        })
    });
    c.bench_function("policy_v2_dns_query_match", |b| {
        b.iter(|| {
            black_box(
                policy
                    .find_matching_rule(PolicyCallback::DnsQuery, black_box(&dns_subject))
                    .unwrap(),
            )
        })
    });
    c.bench_function("policy_v2_model_response_match", |b| {
        b.iter(|| {
            black_box(
                policy
                    .find_matching_rule(
                        PolicyCallback::ModelResponse,
                        black_box(&model_response_subject),
                    )
                    .unwrap(),
            )
        })
    });
    c.bench_function("policy_v2_model_tool_call_match", |b| {
        b.iter(|| {
            black_box(
                policy
                    .find_matching_rule(
                        PolicyCallback::ModelToolCall,
                        black_box(&tool_call_subject),
                    )
                    .unwrap(),
            )
        })
    });
    c.bench_function("policy_v2_hook_decision_match", |b| {
        b.iter(|| {
            black_box(
                policy
                    .find_matching_rule(PolicyCallback::HookDecision, black_box(&hook_subject))
                    .unwrap(),
            )
        })
    });
}

fn bench_hook_wire(c: &mut Criterion) {
    let response = br#"{"decision":"rewrite","decision_id":"decision-1","rule_id":"hook.fixture","reason":"redact","rewrite_target":"tool.arguments.query","rewrite_value":"[redacted]","redactions":["rewrite_value"],"audit_tags":["bench"]}"#;
    c.bench_function("policy_hook_response_decode", |b| {
        b.iter(|| {
            black_box(serde_json::from_slice::<HookDecisionResponse>(black_box(response)).unwrap())
        })
    });
}

criterion_group!(benches, bench_policy_matching, bench_hook_wire);
criterion_main!(benches);

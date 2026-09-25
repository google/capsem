use serde_json::json;

use super::fixtures::*;
use super::*;

/// Record ops the way the writer does after they commit: every op stored, a
/// rule event's payload parsed as the archive parses it.
fn tally(ops: &[WriteOp]) -> LedgerCounters {
    let mut tally = LedgerTally::default();
    for op in ops {
        tally.record(op, &stored(op));
    }
    tally.counters().clone()
}

fn stored(op: &WriteOp) -> StoredEffect {
    StoredEffect {
        exec_completed: matches!(op, WriteOp::ExecEventComplete(_)),
        updated_disk_row: false,
        forensic: match op {
            WriteOp::SecurityRuleEvent(event) => {
                SecurityForensicEvent::from_json(&event.event_json, &event.event_type).ok()
            }
            _ => None,
        },
    }
}

#[test]
fn net_events_count_decisions_and_bytes() {
    let counters = tally(&[
        net("allowed", 10, 100),
        net("denied", 1, 0),
        net("error", 2, 3),
        net("redirected", 0, 0),
    ]);
    assert_eq!(counters.net.total, 4);
    assert_eq!(
        (counters.net.allowed, counters.net.denied, counters.net.error),
        (1, 1, 1)
    );
    assert_eq!((counters.net.bytes_sent, counters.net.bytes_received), (13, 103));
}

#[test]
fn model_calls_count_usage_cost_and_only_counted_tool_origins() {
    let counters = tally(&[
        model(
            "anthropic",
            Some("claude"),
            100,
            20,
            0.0012345,
            &[("bash", "native"), ("srv__tool", "mcp_proxy")],
        ),
        model("anthropic", None, 1, 2, 0.000_000_4, &[("fetch_http", "local")]),
    ]);
    let total = &counters.model.total;
    assert_eq!((total.calls, total.input_tokens, total.output_tokens), (2, 101, 22));
    // Rounded per call: 1234.5 -> 1235 and 0.4 -> 0.
    assert_eq!(total.cost_micro_usd, 1_235);
    assert_eq!(counters.model.usage_details["thinking"], 14);
    assert_eq!(counters.model.by_model["anthropic"]["claude"].calls, 1);
    assert_eq!(counters.model.by_model["anthropic"]["unknown"].input_tokens, 1);
    // mcp_proxy names a call the MCP gateway records again as `mcp`.
    assert_eq!(counters.tools.calls, 2);
    assert!(!counters.tools.by_tool.contains_key("srv__tool"));
    assert_eq!(counters.tools.by_tool["bash"].duration_ms, 20);
}

#[test]
fn only_mcp_tools_call_is_a_tool_call() {
    let counters = tally(&[
        mcp("tools/list", "github", "", 1),
        mcp("tools/call", "github", "search", 7),
        mcp("tools/call", "github", "search", 3),
    ]);
    assert_eq!(counters.tools.calls, 2);
    let search = &counters.tools.mcp["github"]["search"];
    assert_eq!(
        (
            search.calls,
            search.duration_ms,
            search.bytes_sent,
            search.bytes_received
        ),
        (2, 10, 10, 18)
    );
    assert_eq!(counters.tools.by_tool["search"], *search);
}

#[test]
fn an_exec_completion_counts_only_when_it_completed_a_start() {
    let mut tally = LedgerTally::default();
    tally.record(&exec(1), &StoredEffect::default());
    tally.record(
        &exec_done(1),
        &StoredEffect {
            exec_completed: true,
            ..StoredEffect::default()
        },
    );
    // A duplicate completion, or one for a start that never landed, updates no row.
    tally.record(&exec_done(1), &StoredEffect::default());
    tally.record(&exec_done(9), &StoredEffect::default());
    assert_eq!((tally.counters().exec.started, tally.counters().exec.completed), (1, 1));
}

#[test]
fn file_and_audit_events_count_by_action_and_executable() {
    let counters = tally(&[
        file("created", "/a"),
        file("modified", "/a"),
        file("modified", "/b"),
        audit("/bin/ls", 1_700_000_005.0),
        audit("/bin/ls", 1_700_000_001.0),
        audit("/bin/cat", 1_700_000_003.0),
    ]);
    assert_eq!(counters.files.events, 3);
    assert_eq!(counters.files.by_action["modified"], 2);
    let ls = &counters.audit.by_exe["/bin/ls"];
    assert_eq!(counters.audit.events, 3);
    assert_eq!(ls.count, 2);
    assert!(ls.first_seen < ls.last_seen, "{ls:?}");
    assert_eq!(
        ls.first_seen,
        format_timestamp(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_001))
    );
}

#[test]
fn rule_matches_keep_the_latest_event_per_rule_label() {
    let counters = tally(&[
        rule("ev-1", "r.block", "block", "high", 10, json!({})),
        rule("ev-2", "r.block", "block", "high", 30, json!({})),
        rule("ev-3", "r.block", "block", "high", 30, json!({})),
        rule("ev-4", "r.block", "block", "high", 20, json!({})),
        rule("ev-5", "r.allow", "allow", "none", 5, json!({})),
    ]);
    let security = &counters.security;
    assert_eq!(security.matches, 5);
    assert_eq!(security.by_action["block"], 4);
    assert_eq!(security.by_level["none"], 1);
    assert_eq!(security.by_event_type["http.request"], 5);
    let block = &security.by_rule["r.block"]["block"]["high"];
    assert_eq!(block.count, 4);
    // A tie on time goes to the later row.
    assert_eq!(
        (block.latest_event_id.as_str(), block.latest_timestamp_unix_ms),
        ("ev-3", 30)
    );
}

#[test]
fn a_payload_is_counted_once_per_event_however_many_rules_it_matched() {
    let payload = json!({
        "plugin_executions": [
            {"plugin_id": "dlp", "stage": "pre", "applied": true, "duration_us": 40},
            {"plugin_id": "dlp", "stage": "pre", "applied": true, "duration_us": 40},
            {"plugin_id": "dlp", "stage": "post", "applied": false, "duration_us": 60},
        ],
        "detections": [
            {"source": "plugin", "plugin_id": "dlp"},
            {"source": "plugin", "plugin_id": "dlp"},
            {"source": "rule", "plugin_id": "dlp"},
        ],
        "credential_observations": [{"credential_ref": "cred://a", "source": "header", "provider": "github"}],
        "credential_injections": [
            {"substitution_ref": "cred://a", "source": "header"},
            {"substitution_ref": "cred://a", "source": "header"},
        ],
    });
    let counters = tally(&[
        rule("ev-1", "r.one", "block", "high", 100, payload.clone()),
        rule("ev-1", "r.two", "allow", "low", 100, payload.clone()),
        rule("ev-2", "r.one", "block", "high", 200, payload),
    ]);
    let dlp = &counters.plugins["dlp"];
    assert_eq!((dlp.executions, dlp.applied, dlp.skipped), (4, 2, 2));
    assert_eq!(
        (dlp.total_duration_us, dlp.max_duration_us, dlp.detections),
        (200, 60, 2)
    );
    let credential = &counters.credentials["cred://a"];
    assert_eq!((credential.observations, credential.injections), (2, 2));
    assert_eq!(credential.provider.as_deref(), Some("github"));
    assert_eq!(credential.last_seen_unix_ms, 200);
    assert_eq!(counters.security.matches, 3);
}

#[test]
fn credential_substitutions_count_injections_and_the_greatest_provider() {
    let counters = tally(&[
        substitution("cred://b", "captured", None, 1_700_000_000.0),
        substitution("cred://b", "injected", Some("github"), 1_700_000_002.0),
        substitution("cred://b", "injected", Some("aws"), 1_700_000_001.0),
    ]);
    let credential = &counters.credentials["cred://b"];
    assert_eq!((credential.substitutions, credential.injected_substitutions), (3, 2));
    assert_eq!(credential.provider.as_deref(), Some("github"));
    assert_eq!(credential.last_seen_unix_ms, 1_700_000_002_000);
}

#[test]
fn asks_stay_open_until_resolved_and_overflow_is_counted() {
    let mut ops = vec![
        ask("a", "pending"),
        ask("a", "pending"),
        ask("b", "pending"),
        ask("a", "approved"),
    ];
    ops.push(ask("never-pending", "denied"));
    let counters = tally(&ops);
    assert_eq!(counters.security.open_asks, ["b".to_string()].into());

    let mut flood: Vec<WriteOp> = (0..MAX_OPEN_ASKS + 2)
        .map(|index| ask(&format!("f{index}"), "pending"))
        .collect();
    flood.push(ask("unnamed-resolution", "denied"));
    let counters = tally(&flood);
    assert_eq!(counters.security.open_asks.len(), MAX_OPEN_ASKS);
    assert_eq!(counters.security.open_asks_overflow, 1);
}

#[test]
fn guest_chosen_keys_fold_into_overflow_past_the_bound() {
    let ops: Vec<WriteOp> = (0..MAX_KEYS_PER_MAP + 10)
        .map(|index| audit(&format!("/tmp/bin-{index}"), 1_700_000_000.0))
        .collect();
    let counters = tally(&ops);
    assert_eq!(counters.audit.by_exe.len(), MAX_KEYS_PER_MAP + 1);
    assert_eq!(counters.audit.by_exe[OVERFLOW_KEY].count, 10);
    assert_eq!(counters.audit.events, (MAX_KEYS_PER_MAP + 10) as u64);
}

#[test]
fn cost_is_whole_micro_usd_and_never_negative() {
    assert_eq!(cost_micro_usd(0.0000005), 1);
    assert_eq!(cost_micro_usd(1.5), 1_500_000);
    assert_eq!(cost_micro_usd(-2.0), 0);
    assert_eq!(cost_micro_usd(f64::NAN), 0);
    assert_eq!(usd_from_micro(1_500_000), 1.5);
}

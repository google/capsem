use super::*;

fn stopped_with(net_total: u64, cost: u64, provider: &str) -> HostSessionDetail {
    let mut counters = LedgerCounters::default();
    counters.net.total = net_total;
    counters.model.total.cost_micro_usd = cost;
    capsem_proto::ledger_counters::bounded_entry(
        capsem_proto::ledger_counters::bounded_entry(&mut counters.model.by_model, provider),
        "m",
    )
    .calls = net_total;
    stopped_detail("stopped", Some(counters))
}

fn created(persistent: bool) -> HostSessionDetail {
    HostSessionDetail {
        persistent,
        ram_bytes: 1 << 30,
        ..HostSessionDetail::default()
    }
}

#[test]
fn the_fold_counts_every_session_and_sums_finished_counters() {
    let mut stats = HostStats::default();
    stats.apply(&host_session_event(
        HostEventKind::SessionCreated,
        "a",
        1_000,
        &created(false),
    ));
    stats.apply(&host_session_event(
        HostEventKind::SessionCreated,
        "b",
        2_000,
        &created(true),
    ));
    stats.apply(&host_session_event(
        HostEventKind::SessionStopped,
        "a",
        3_000,
        &stopped_with(4, 1_000_000, "anthropic"),
    ));

    assert_eq!(stats.total_sessions, 2);
    assert_eq!(stats.finished.net.total, 4);
    assert_eq!(stats.running_ids(), vec!["b".to_string()]);

    let body: serde_json::Value = serde_json::from_slice(&render_stats(
        stats.total_sessions,
        &stats.finished,
        &stats
            .running
            .values()
            .cloned()
            .chain(stats.recent_finished.iter().cloned())
            .collect::<Vec<_>>(),
    ))
    .unwrap();
    assert_eq!(body["global"]["total_sessions"], 2);
    assert_eq!(body["global"]["total_requests"], 4);
    assert_eq!(body["global"]["total_estimated_cost"], 1.0);
    assert_eq!(body["top_providers"][0]["provider"], "anthropic");
    assert_eq!(body["top_providers"][0]["call_count"], 4);
    let sessions = body["sessions"].as_array().unwrap();
    let a = sessions.iter().find(|session| session["id"] == "a").unwrap();
    assert_eq!(a["status"], "stopped");
    assert_eq!(a["total_requests"], 4);
    let b = sessions.iter().find(|session| session["id"] == "b").unwrap();
    assert_eq!(b["status"], "running");
    assert_eq!(b["mode"], "persistent");
    for key in ["global", "sessions", "top_providers", "top_tools", "top_mcp_tools"] {
        assert!(body.get(key).is_some(), "/stats keeps its {key} field");
    }
}

#[test]
fn the_listed_finished_sessions_are_bounded_but_totals_are_not() {
    let mut stats = HostStats::default();
    for index in 0..(LISTED_FINISHED_SESSIONS as i64 + 5) {
        let id = format!("s{index}");
        stats.apply(&host_session_event(
            HostEventKind::SessionCreated,
            &id,
            index,
            &created(false),
        ));
        stats.apply(&host_session_event(
            HostEventKind::SessionStopped,
            &id,
            index,
            &stopped_with(1, 0, "p"),
        ));
    }
    assert_eq!(stats.recent_finished.len(), LISTED_FINISHED_SESSIONS);
    assert_eq!(stats.finished.net.total, LISTED_FINISHED_SESSIONS as u64 + 5);
    assert_eq!(stats.total_sessions, LISTED_FINISHED_SESSIONS as u64 + 5);
}

#[test]
fn service_events_do_not_touch_session_totals() {
    let mut stats = HostStats::default();
    stats.apply(&HostEvent {
        timestamp_unix_ms: 1,
        kind: HostEventKind::ServiceStarted,
        session_id: None,
        actor: "service".into(),
        detail: Vec::new(),
        trace_id: None,
    });
    assert_eq!(stats.total_sessions, 0);
}

use super::*;

// =====================================================================
// (T3.f) -- metrics emission assertions
//
// Use a thread-local DebuggingRecorder so each test snapshots only
// its own emissions (parallel tests don't pollute each other).
// =====================================================================

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

pub(super) fn count_for(snapshotter: &Snapshotter, metric: &str, decision: Option<&str>) -> u64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(k, _, _, v)| {
            if k.key().name() != metric {
                return None;
            }
            if let Some(want) = decision {
                let has_label = k
                    .key()
                    .labels()
                    .any(|l| l.key() == "decision" && l.value() == want);
                if !has_label {
                    return None;
                }
            }
            match v {
                DebugValue::Counter(c) => Some(c),
                _ => None,
            }
        })
        .sum()
}

fn histogram_present(snapshotter: &Snapshotter, metric: &str) -> bool {
    snapshotter
        .snapshot()
        .into_vec()
        .iter()
        .any(|(k, _, _, v)| k.key().name() == metric && matches!(v, DebugValue::Histogram(_)))
}

#[tokio::test]
async fn metrics_increment_for_allowed_query() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_fake_upstream([1, 2, 3, 4], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let q = build_query_bytes("example.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;

    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("allowed")),
        1
    );
    assert!(histogram_present(&snap, "mitm.dns_handle_duration_ms"));
    assert!(histogram_present(&snap, "mitm.dns_upstream_duration_ms"));
}

#[tokio::test]
async fn metrics_increment_for_denied_query() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = block_specific_policy("api.openai.com");
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("api.openai.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;

    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("denied")),
        1
    );
    // Denied path short-circuits before upstream -- the upstream
    // duration histogram MUST be absent.
    assert!(!histogram_present(&snap, "mitm.dns_upstream_duration_ms"));
}

#[tokio::test]
async fn metrics_increment_for_redirected_query() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    let policy = policy_with_redirect(
        "anthropic.com",
        Some(1),
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
    );
    let handler = DnsHandler::new(shared(policy), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;

    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("redirected")),
        1
    );
}

#[tokio::test]
async fn metrics_increment_upstream_failures() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(150)),
    );
    let handler = DnsHandler::new(shared(allow_all_policy()), resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;

    assert_eq!(count_for(&snap, "mitm.dns_queries_total", Some("error")), 1);
    assert_eq!(
        count_for(&snap, "mitm.dns_upstream_failures_total", None),
        1
    );
}

#[tokio::test]
async fn metrics_decision_label_distinct_per_outcome() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_fake_upstream([1, 2, 3, 4], Duration::ZERO).await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(500)),
    );
    // Mix: one allowed, one redirected, one denied via Policy
    // rules on the same handler.
    let policy = PolicyConfig::from_policy_toml_str(
        r#"
        [policy.dns.blocked]
        on = "dns.query"
        if = 'qname == "blocked.example.com"'
        decision = "block"
        priority = 1

        [policy.dns.redirected]
        on = "dns.query"
        if = 'qname == "redirect.example.com" && qtype == "A"'
        decision = "rewrite"
        priority = 1
        rewrite_target = 'answer.ip =~ ".*"'
        rewrite_value = "127.0.0.1"
        "#,
    )
    .unwrap();
    let handler = DnsHandler::new(shared(policy), resolver);

    let _ = handler
        .handle(&build_query_bytes("ok.example.com.", RecordType::A, 1))
        .await;
    let _ = handler
        .handle(&build_query_bytes("blocked.example.com.", RecordType::A, 2))
        .await;
    let _ = handler
        .handle(&build_query_bytes(
            "redirect.example.com.",
            RecordType::A,
            3,
        ))
        .await;

    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("allowed")),
        1
    );
    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("denied")),
        1
    );
    assert_eq!(
        count_for(&snap, "mitm.dns_queries_total", Some("redirected")),
        1
    );
}

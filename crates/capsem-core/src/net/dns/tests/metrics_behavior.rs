use super::*;

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
    let handler = DnsHandler::new(resolver);

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
async fn metrics_increment_upstream_failures() {
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let upstream = spawn_blackhole_upstream().await;
    let resolver = Arc::new(
        DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(150)),
    );
    let handler = DnsHandler::new(resolver);

    let q = build_query_bytes("anthropic.com.", RecordType::A, 1);
    let _ = handler.handle(&q).await;

    assert_eq!(count_for(&snap, "mitm.dns_queries_total", Some("error")), 1);
    assert_eq!(
        count_for(&snap, "mitm.dns_upstream_failures_total", None),
        1
    );
}

use super::*;

#[test]
fn summary_separates_connection_setup_from_warm_requests() {
    let report = summarize(
        vec![
            Sample {
                latency_ms: 10.0,
                bytes: 24,
                error: None,
                cold: true,
            },
            Sample {
                latency_ms: 20.0,
                bytes: 24,
                error: None,
                cold: true,
            },
            Sample {
                latency_ms: 1.0,
                bytes: 24,
                error: None,
                cold: false,
            },
            Sample {
                latency_ms: 2.0,
                bytes: 24,
                error: None,
                cold: false,
            },
            Sample {
                latency_ms: 4.0,
                bytes: 0,
                error: Some("HTTP 502".to_string()),
                cold: false,
            },
        ],
        2,
        Duration::from_secs(1),
        Duration::from_secs(2),
    );

    assert_eq!(report.total_requests, 5);
    assert_eq!(report.errors, 1);
    assert_eq!(report.error_ratio, 0.2);
    assert_eq!(report.rps, 2.0);
    assert_eq!(report.cold_p50_ms, 15.0);
    assert_eq!(report.warm_p50_ms, 1.5);
    assert_eq!(report.response_body_bytes, 96);
    assert_eq!(report.error_distribution["HTTP 502"], 1);
}

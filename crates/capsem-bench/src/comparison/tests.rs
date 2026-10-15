use super::*;

fn record(at: &str, value: f64) -> schema::Record {
    schema::Record {
        schema: schema::SCHEMA.to_string(),
        dimension: schema::Dimension::Routes,
        recorded_at: at.to_string(),
        release: schema::Release {
            version: "0.7.0".to_string(),
            channel: "dev".to_string(),
            commit: "head".to_string(),
        },
        host: schema::Host {
            arch: "x86_64".to_string(),
            os: "linux".to_string(),
            cpu_count: 8,
            kvm: true,
            governor: None,
            load_before: 0.0,
        },
        quick: false,
        metrics: vec![schema::Metric {
            key: "routes.service./status.cpu_s".to_string(),
            unit: schema::Unit::Seconds,
            summary: stats::Summary::of(&[value]).unwrap(),
        }],
        sidecar: None,
    }
}

#[test]
fn repeated_runs_become_the_comparison_distribution() {
    let records = [record("first", 0.03), record("second", 0.05)];
    let combined = aggregate(&records).expect("aggregates");
    let summary = &combined.metrics[0].summary;

    assert_eq!(summary.n, 2);
    assert_eq!(summary.median, 0.04);
    assert_eq!(summary.cv, 0.25);
}

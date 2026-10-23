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
        measurement: None,
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

#[test]
fn historical_rows_without_fitness_cannot_be_comparison_authority() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("history.db");
    let mut db = store::open(&path).unwrap();
    for at in ["first", "second"] {
        store::insert(&mut db, &record(at, 0.03)).unwrap();
    }
    let thresholds = Thresholds {
        maximum_factor: 1.1,
        noise_factor: 1.0,
        minimum_time_resolution_ms: 1.0,
    };
    assert!(compare(&path, &path, schema::Dimension::Routes, "x86_64", 2, thresholds).is_err());
}

#[test]
fn comparison_requires_every_requested_repeat() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("single.db");
    let mut db = store::open(&path).unwrap();
    store::insert(&mut db, &record("one", 0.03)).unwrap();
    let thresholds = Thresholds {
        maximum_factor: 1.1,
        noise_factor: 1.0,
        minimum_time_resolution_ms: 1.0,
    };
    assert!(compare(&path, &path, schema::Dimension::Routes, "x86_64", 2, thresholds).is_err());
}

fn authoritative(at: &str) -> schema::Record {
    let mut row = record(at, 0.03);
    row.release.commit = "a".repeat(40);
    let fitness = crate::machine::Fitness {
        host: row.host.clone(),
        objections: vec![],
    };
    row.measurement = Some(schema::Measurement {
        before: fitness.clone(),
        after: fitness,
        profile: "same-config".into(),
    });
    row
}

#[test]
fn complete_fit_repeats_compare_across_intentionally_different_sources() {
    let root = tempfile::tempdir().unwrap();
    let before = root.path().join("before.db");
    let after = root.path().join("after.db");
    for (path, sha) in [(&before, "a"), (&after, "b")] {
        let mut db = store::open(path).unwrap();
        for at in ["first", "second"] {
            let mut row = authoritative(at);
            row.release.commit = sha.repeat(40);
            store::insert(&mut db, &row).unwrap();
        }
    }
    let thresholds = Thresholds {
        maximum_factor: 1.1,
        noise_factor: 1.0,
        minimum_time_resolution_ms: 1.0,
    };
    compare(&before, &after, schema::Dimension::Routes, "x86_64", 2, thresholds).unwrap();
    verify(&after, &before, 2, thresholds).unwrap();
}

#[test]
fn mismatched_configuration_metric_or_unit_never_silently_disappears() {
    let first = authoritative("first");
    let mut changed = first.clone();
    changed.measurement.as_mut().unwrap().profile = "different".into();
    assert!(evidence::compatible(&first, &changed).is_err());
    changed = first.clone();
    changed.metrics[0].unit = schema::Unit::Count;
    assert!(evidence::compatible(&first, &changed).is_err());
    changed = first.clone();
    changed.metrics.clear();
    assert!(evidence::compatible(&first, &changed).is_err());
    changed = first.clone();
    changed.quick = true;
    assert!(evidence::eligible(&changed).is_err());
    changed = first;
    changed
        .measurement
        .as_mut()
        .unwrap()
        .after
        .objections
        .push(crate::machine::Objection {
            what: "load".into(),
            detail: "busy".into(),
        });
    assert!(evidence::eligible(&changed).is_err());
}

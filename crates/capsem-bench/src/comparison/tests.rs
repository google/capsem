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
            samples: Some(vec![value]),
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

fn thresholds() -> Thresholds {
    Thresholds {
        maximum_factor: 1.1,
        noise_factor: 1.0,
        minimum_time_resolution_ms: 1.0,
    }
}

fn insert_repeats(path: &Path, dimension: schema::Dimension, change: impl Fn(&mut schema::Record)) {
    let mut db = store::open(path).unwrap();
    for at in ["first", "second"] {
        let mut row = authoritative(at);
        row.dimension = dimension;
        change(&mut row);
        store::insert(&mut db, &row).unwrap();
    }
}

#[test]
fn verification_never_seeds_a_missing_baseline_subject() {
    let root = tempfile::tempdir().unwrap();
    let before = root.path().join("before.db");
    let after = root.path().join("after.db");
    insert_repeats(&after, schema::Dimension::Routes, |_| {});
    assert!(verify(&after, &before, 2, thresholds()).is_err());
}

#[test]
fn verification_requires_every_subject_seen_on_either_side() {
    for extra_baseline in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let before = root.path().join("before.db");
        let after = root.path().join("after.db");
        for path in [&before, &after] {
            insert_repeats(path, schema::Dimension::Routes, |_| {});
        }
        let extra = if extra_baseline { &before } else { &after };
        insert_repeats(extra, schema::Dimension::Restart, |_| {});
        let error = verify(&after, &before, 2, thresholds()).unwrap_err();
        let detail = format!("{error:#}");
        assert!(detail.contains("restart") && detail.contains("x86_64"), "{detail}");
    }
}

#[test]
fn verification_does_not_pair_different_architectures() {
    let root = tempfile::tempdir().unwrap();
    let before = root.path().join("before.db");
    let after = root.path().join("after.db");
    insert_repeats(&before, schema::Dimension::Routes, |_| {});
    insert_repeats(&after, schema::Dimension::Routes, |row| {
        row.host.arch = "aarch64".into();
        let measurement = row.measurement.as_mut().unwrap();
        measurement.before.host.arch = "aarch64".into();
        measurement.after.host.arch = "aarch64".into();
    });
    assert!(verify(&after, &before, 2, thresholds()).is_err());
}

#[test]
fn verification_rejects_empty_stores_and_zero_repeats() {
    let root = tempfile::tempdir().unwrap();
    let before = root.path().join("before.db");
    let after = root.path().join("after.db");
    assert!(verify(&after, &before, 2, thresholds()).is_err());
    for path in [&before, &after] {
        insert_repeats(path, schema::Dimension::Routes, |_| {});
    }
    assert!(verify(&after, &before, 0, thresholds()).is_err());
}

#[test]
fn verification_rejects_incomplete_or_ineligible_evidence_on_either_side() {
    for bad_baseline in [true, false] {
        for invalid in [
            "single",
            "rawless",
            "historical",
            "quick",
            "unfit",
            "profile",
            "unit",
            "key",
            "source",
        ] {
            let root = tempfile::tempdir().unwrap();
            let before = root.path().join("before.db");
            let after = root.path().join("after.db");
            let good = if bad_baseline { &after } else { &before };
            let bad = if bad_baseline { &before } else { &after };
            insert_repeats(good, schema::Dimension::Routes, |_| {});
            let mut db = store::open(bad).unwrap();
            for at in ["first", "second"] {
                let mut row = authoritative(at);
                match invalid {
                    "single" if at == "second" => continue,
                    "rawless" => row.metrics[0].samples = None,
                    "historical" => row.measurement = None,
                    "quick" => row.quick = true,
                    "unfit" => row
                        .measurement
                        .as_mut()
                        .unwrap()
                        .after
                        .objections
                        .push(crate::machine::Objection {
                            what: "load".into(),
                            detail: "busy".into(),
                        }),
                    "profile" => row.measurement.as_mut().unwrap().profile = "different".into(),
                    "unit" => row.metrics[0].unit = schema::Unit::Count,
                    "key" => row.metrics[0].key = "different".into(),
                    "source" if at == "second" => row.release.commit = "b".repeat(40),
                    _ => {}
                }
                store::insert(&mut db, &row).unwrap();
            }
            assert!(
                verify(&after, &before, 2, thresholds()).is_err(),
                "{invalid}, bad baseline={bad_baseline}"
            );
        }
    }
}

#[test]
fn complete_paired_subject_union_passes_without_changing_regression_verdicts() {
    let root = tempfile::tempdir().unwrap();
    let before = root.path().join("before.db");
    let after = root.path().join("after.db");
    for dimension in [schema::Dimension::Routes, schema::Dimension::Restart] {
        insert_repeats(&before, dimension, |_| {});
        insert_repeats(&after, dimension, |row| row.release.commit = "b".repeat(40));
    }
    verify(&after, &before, 2, thresholds()).unwrap();
    insert_repeats(&after, schema::Dimension::Routes, |row| {
        row.recorded_at = format!("third-{}", row.recorded_at);
        row.release.commit = "b".repeat(40);
        row.metrics[0].samples = Some(vec![0.06]);
        row.metrics[0].summary = stats::Summary::of(&[0.06]).unwrap();
    });
    let error = verify(&after, &before, 2, thresholds()).unwrap_err();
    assert!(error.to_string().contains("1 metric(s) regressed"), "{error}");
}

use std::collections::{BTreeMap, BTreeSet};

use metrics_util::debugging::DebuggingRecorder;

use super::*;

/// Module sources, so a name declared as a `pub const` but left out of its
/// module's `SPECS` table fails here instead of recording without metadata.
const SOURCES: &[(&str, &str)] = &[
    ("db", include_str!("db.rs")),
    ("dns", include_str!("dns.rs")),
    ("mitm", include_str!("mitm.rs")),
    ("security", include_str!("security.rs")),
    ("virtio_blk", include_str!("virtio_blk.rs")),
];

fn declared_names(source: &str) -> BTreeSet<&str> {
    source
        .lines()
        .filter_map(|line| line.strip_prefix("pub const "))
        .filter(|rest| rest.contains(": &str = \""))
        .filter_map(|rest| rest.split('"').nth(1))
        .collect()
}

/// `domain.snake_case`, where a domain may nest (`virtio.blk.requests_total`).
fn well_formed(name: &str) -> bool {
    let segments: Vec<&str> = name.split('.').collect();
    segments.len() >= 2
        && segments.iter().all(|segment| {
            segment.starts_with(|c: char| c.is_ascii_lowercase())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && !segment.ends_with('_')
                && !segment.contains("__")
        })
}

#[test]
fn every_name_is_unique() {
    let mut seen = BTreeSet::new();
    let duplicates: Vec<&str> = all().map(|spec| spec.name).filter(|name| !seen.insert(*name)).collect();
    assert!(duplicates.is_empty(), "metric names declared twice: {duplicates:?}");
    assert!(seen.len() > 1, "the catalog is empty; this guard is vacuous");
}

#[test]
fn constructors_set_kind_and_unit() {
    let name = "domain.example_total";
    assert_eq!(
        MetricSpec::counter(name, Unit::Count, "d"),
        MetricSpec::new(name, MetricKind::Counter, Some(Unit::Count), "d")
    );
    assert_eq!(
        MetricSpec::gauge(name, Unit::Bytes, "d"),
        MetricSpec::new(name, MetricKind::Gauge, Some(Unit::Bytes), "d")
    );
    assert_eq!(
        MetricSpec::histogram(name, Unit::Milliseconds, "d"),
        MetricSpec::new(name, MetricKind::Histogram, Some(Unit::Milliseconds), "d")
    );
}

#[test]
fn every_spec_has_a_description() {
    for spec in all() {
        assert!(!spec.description.trim().is_empty(), "{} has no description", spec.name);
    }
}

#[test]
fn every_name_is_domain_dot_snake_case() {
    for spec in all() {
        assert!(well_formed(spec.name), "{} is not domain.snake_case", spec.name);
    }
    for bad in [
        "db",
        "db.",
        ".total",
        "db.Query",
        "db.query-total",
        "db.query_",
        "db..query",
        "1db.query",
    ] {
        assert!(!well_formed(bad), "{bad} should be rejected");
    }
}

#[test]
fn counters_end_in_total_and_totals_are_counters() {
    for spec in all() {
        assert_eq!(
            spec.kind == MetricKind::Counter,
            spec.name.ends_with("_total"),
            "{} is a {:?}: counters, and only counters, end in _total",
            spec.name,
            spec.kind,
        );
    }
}

#[test]
fn every_declared_name_has_a_spec_in_its_own_domain() {
    assert_eq!(
        SOURCES.iter().map(|(domain, _)| *domain).collect::<Vec<_>>(),
        DOMAINS.iter().map(|(domain, _)| *domain).collect::<Vec<_>>(),
        "every domain module must be both scanned here and listed in DOMAINS",
    );
    for ((domain, source), (_, specs)) in SOURCES.iter().zip(DOMAINS) {
        let declared = declared_names(source);
        let specified: BTreeSet<&str> = specs.iter().map(|spec| spec.name).collect();
        assert!(
            !declared.is_empty(),
            "{domain} declares no names; this guard is vacuous"
        );
        assert_eq!(declared, specified, "{domain}: declared names and SPECS disagree");
    }
}

#[test]
fn describe_all_attaches_each_specs_unit_and_description() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    metrics::with_local_recorder(&recorder, || {
        describe_all();
        // The debugging recorder only reports metrics that were registered.
        for spec in all() {
            match spec.kind {
                MetricKind::Counter => metrics::counter!(spec.name).increment(0),
                MetricKind::Gauge => metrics::gauge!(spec.name).set(0.0),
                MetricKind::Histogram => metrics::histogram!(spec.name).record(0.0),
            }
        }
    });

    let recorded: BTreeMap<String, _> = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .map(|(key, unit, description, _)| {
            let kind = match key.kind() {
                metrics_util::MetricKind::Counter => MetricKind::Counter,
                metrics_util::MetricKind::Gauge => MetricKind::Gauge,
                metrics_util::MetricKind::Histogram => MetricKind::Histogram,
            };
            (
                key.key().name().to_string(),
                (kind, unit, description.map(|d| d.to_string())),
            )
        })
        .collect();
    assert_eq!(recorded.len(), all().count());
    for spec in all() {
        assert_eq!(
            recorded.get(spec.name),
            Some(&(spec.kind, spec.unit, Some(spec.description.to_string()))),
            "{} was not described as specified",
            spec.name,
        );
    }
}

#[test]
fn describe_without_a_recorder_is_a_no_op() {
    describe_all();
    describe_all();
}

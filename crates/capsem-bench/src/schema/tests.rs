use super::{Dimension, Host, Metric, Record, Release, Unit, SCHEMA};
use crate::stats::Summary;

fn record() -> Record {
    Record {
        schema: SCHEMA.to_string(),
        dimension: Dimension::Routes,
        recorded_at: "2026-08-21T12:00:00Z".to_string(),
        release: Release {
            version: "0.6.0".to_string(),
            channel: "stable".to_string(),
            commit: "60d214c8".to_string(),
        },
        host: Host {
            arch: "x86_64".to_string(),
            os: "linux".to_string(),
            cpu_count: 16,
            kvm: true,
            governor: Some("performance".to_string()),
            load_before: 0.1,
        },
        profile: "code".to_string(),
        quick: false,
        metrics: vec![Metric {
            key: "gateway./vms/list.cpu_s".to_string(),
            unit: Unit::Seconds,
            summary: Summary::of(&[0.14, 0.15, 0.16]).expect("summarizes"),
        }],
        sidecar: None,
    }
}

#[test]
fn a_stored_record_is_stable_across_a_round_trip() {
    // Not float equality: serde_json writes f64 exactly but can parse a value
    // back one ULP off, for nested and flattened fields alike. That is
    // irrelevant to a ratio-based ratchet and must never be relied on, so the
    // property asserted here is the one that matters for storage -- reading a
    // record and writing it again produces the same bytes.
    let first = serde_json::to_string(&record()).expect("serializes");
    let parsed: Record = serde_json::from_str(&first).expect("parses");
    let second = serde_json::to_string(&parsed).expect("re-serializes");
    assert_eq!(first, second);
}

#[test]
fn a_round_trip_preserves_every_field_that_is_not_a_float() {
    let json = serde_json::to_string(&record()).expect("serializes");
    let parsed: Record = serde_json::from_str(&json).expect("parses");
    assert_eq!(parsed.schema, SCHEMA);
    assert_eq!(parsed.dimension, Dimension::Routes);
    assert_eq!(parsed.release, record().release);
    assert_eq!(parsed.profile, "code");
    assert!(!parsed.quick);
    assert_eq!(parsed.metrics.len(), 1);
    assert_eq!(parsed.metrics[0].key, "gateway./vms/list.cpu_s");
    assert_eq!(parsed.metrics[0].unit, Unit::Seconds);
    assert_eq!(parsed.metrics[0].summary.n, 3);
    // Values survive far tighter than any threshold this system applies.
    let drift = (parsed.metrics[0].summary.mad - record().metrics[0].summary.mad).abs();
    assert!(drift < 1e-15, "drift {drift} exceeds one ULP");
}

#[test]
fn an_unknown_field_is_refused_rather_than_ignored() {
    // `deny_unknown_fields`, so a producer writing a field no reader knows
    // fails loudly instead of having it silently dropped -- which is how the
    // old shapes drifted apart.
    let mut value = serde_json::to_value(record()).expect("serializes");
    value["surprise"] = serde_json::json!("unexpected");
    let error = serde_json::from_value::<Record>(value).expect_err("must refuse");
    assert!(error.to_string().contains("surprise"), "{error}");
}

#[test]
fn a_missing_field_is_refused() {
    let mut value = serde_json::to_value(record()).expect("serializes");
    value.as_object_mut().expect("object").remove("release");
    assert!(serde_json::from_value::<Record>(value).is_err());
}

#[test]
fn statistics_are_flattened_beside_the_key() {
    // The metric reads `{key, unit, n, min, ...}` rather than nesting a
    // summary object, so a plotter can address `p99` without knowing the
    // envelope.
    let json = serde_json::to_value(&record().metrics[0]).expect("serializes");
    assert!(json.get("p99").is_some(), "{json}");
    assert!(json.get("summary").is_none(), "summary should be flattened");
}

#[test]
fn the_filename_carries_every_identity_axis() {
    // route-latency wrote `data_<version>.json` with neither arch nor profile,
    // so one gate's `code` and `co-work` runs overwrote each other.
    assert_eq!(record().filename(), "routes_0.6.0_x86_64_code.json");
}

#[test]
fn two_profiles_of_one_release_do_not_collide() {
    let mut other = record();
    other.profile = "co-work".to_string();
    assert_ne!(record().filename(), other.filename());
}

#[test]
fn two_architectures_of_one_release_do_not_collide() {
    let mut other = record();
    other.host.arch = "arm64".to_string();
    assert_ne!(record().filename(), other.filename());
}

#[test]
fn every_dimension_is_listed_once() {
    let mut names: Vec<&str> = Dimension::ALL.iter().map(|d| d.as_str()).collect();
    let listed = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), listed, "a dimension is listed twice");
    assert_eq!(listed, 18, "ALL must carry every dimension");
}

#[test]
fn dimension_names_round_trip_through_their_wire_form() {
    for dimension in Dimension::ALL {
        let json = serde_json::to_string(dimension).expect("serializes");
        assert_eq!(json, format!("\"{}\"", dimension.as_str()));
        let parsed: Dimension = serde_json::from_str(&json).expect("parses");
        assert_eq!(parsed, *dimension);
    }
}

#[test]
fn the_quick_lane_holds_only_what_finishes_in_seconds() {
    let quick: Vec<&str> = Dimension::ALL
        .iter()
        .filter(|d| d.in_quick_lane())
        .map(|d| d.as_str())
        .collect();
    assert!(quick.contains(&"routes"));
    assert!(quick.contains(&"websocket"));
    assert!(!quick.contains(&"disk"), "disk needs a booted guest");
    // Needs no guest and is still far too slow: it compiles benchmarks and
    // runs each to a confidence interval.
    assert!(!Dimension::Criterion.needs_vm());
    assert!(!quick.contains(&"criterion"), "criterion takes minutes");
    assert!(!quick.contains(&"protocol"), "protocol sends 50k requests");
}

#[test]
fn a_metric_is_found_by_its_key() {
    assert!(record().metric("gateway./vms/list.cpu_s").is_some());
    assert!(record().metric("gateway./vms/list.p99").is_none());
}

#[test]
fn an_absent_sidecar_is_omitted_from_the_wire() {
    let json = serde_json::to_value(record()).expect("serializes");
    assert!(json.get("sidecar").is_none());
}

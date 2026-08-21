use std::fs;

use super::{latest_for, read, read_all, write};
use crate::schema::{Dimension, Host, Metric, Record, Release, Unit, SCHEMA};
use crate::stats::Summary;

fn record(dimension: Dimension, arch: &str, profile: &str, at: &str) -> Record {
    Record {
        schema: SCHEMA.to_string(),
        dimension,
        recorded_at: at.to_string(),
        release: Release {
            version: "0.6.0".to_string(),
            channel: "stable".to_string(),
            commit: "60d214c8".to_string(),
        },
        host: Host {
            arch: arch.to_string(),
            os: "linux".to_string(),
            cpu_count: 16,
            kvm: true,
            governor: Some("performance".to_string()),
            load_before: 0.1,
        },
        profile: profile.to_string(),
        quick: false,
        metrics: vec![Metric {
            key: "gateway./vms/list.cpu_s".to_string(),
            unit: Unit::Seconds,
            summary: Summary::of(&[0.14, 0.15, 0.16]).expect("summarizes"),
        }],
        sidecar: None,
    }
}

fn base() -> Record {
    record(Dimension::Routes, "x86_64", "code", "2026-08-21T12:00:00Z")
}

#[test]
fn a_written_record_reads_back() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = write(root.path(), &base()).expect("writes");
    assert_eq!(read(&path).expect("reads").filename(), base().filename());
}

#[test]
fn writing_creates_the_directory() {
    let root = tempfile::tempdir().expect("tempdir");
    let nested = root.path().join("deep").join("deeper");
    assert!(write(&nested, &base()).is_ok());
}

#[test]
fn no_partial_file_survives_a_successful_write() {
    // The record is staged and renamed, so an interrupted run cannot leave a
    // truncated file that later reads as evidence.
    let root = tempfile::tempdir().expect("tempdir");
    write(root.path(), &base()).expect("writes");
    let leftovers: Vec<_> = fs::read_dir(root.path())
        .expect("lists")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().to_string_lossy().contains("partial"))
        .collect();
    assert!(leftovers.is_empty(), "a staging file was left behind");
}

#[test]
fn a_second_run_of_the_same_identity_replaces_the_first() {
    let root = tempfile::tempdir().expect("tempdir");
    write(root.path(), &base()).expect("writes");
    write(root.path(), &base()).expect("rewrites");
    assert_eq!(read_all(root.path()).expect("reads").len(), 1);
}

#[test]
fn two_profiles_of_one_release_both_survive() {
    // The route-latency bug: `code` and `co-work` in one gate wrote the same
    // path and the second silently replaced the first.
    let root = tempfile::tempdir().expect("tempdir");
    write(root.path(), &base()).expect("writes");
    write(
        root.path(),
        &record(Dimension::Routes, "x86_64", "co-work", "2026-08-21T12:00:00Z"),
    )
    .expect("writes");
    assert_eq!(read_all(root.path()).expect("reads").len(), 2);
}

#[test]
fn an_absent_directory_reads_as_empty_rather_than_failing() {
    // The first run on a machine has nothing to compare against and must be
    // able to say so.
    let root = tempfile::tempdir().expect("tempdir");
    let missing = root.path().join("never-created");
    assert!(read_all(&missing).expect("reads").is_empty());
}

#[test]
fn a_file_that_is_not_a_record_fails_loudly() {
    let root = tempfile::tempdir().expect("tempdir");
    fs::write(root.path().join("junk.json"), "{\"nope\": 1}").expect("writes");
    let error = read_all(root.path()).expect_err("must refuse");
    assert!(error.to_string().contains("not a benchmark record"), "{error}");
}

#[test]
fn non_json_files_are_ignored() {
    let root = tempfile::tempdir().expect("tempdir");
    write(root.path(), &base()).expect("writes");
    fs::write(root.path().join("notes.txt"), "ignore me").expect("writes");
    assert_eq!(read_all(root.path()).expect("reads").len(), 1);
}

#[test]
fn the_latest_record_wins() {
    let records = vec![
        record(Dimension::Routes, "x86_64", "code", "2026-08-20T12:00:00Z"),
        record(Dimension::Routes, "x86_64", "code", "2026-08-21T12:00:00Z"),
    ];
    let found = latest_for(&records, Dimension::Routes, "x86_64", "code").expect("found");
    assert_eq!(found.recorded_at, "2026-08-21T12:00:00Z");
}

#[test]
fn another_architecture_is_not_a_baseline() {
    // Comparing arm64 against x86_64 measures the difference between two
    // machines, not a change in Capsem.
    let records = vec![record(
        Dimension::Routes,
        "arm64",
        "code",
        "2026-08-21T12:00:00Z",
    )];
    assert!(latest_for(&records, Dimension::Routes, "x86_64", "code").is_none());
}

#[test]
fn another_profile_is_not_a_baseline() {
    let records = vec![record(
        Dimension::Routes,
        "x86_64",
        "co-work",
        "2026-08-21T12:00:00Z",
    )];
    assert!(latest_for(&records, Dimension::Routes, "x86_64", "code").is_none());
}

#[test]
fn another_dimension_is_not_a_baseline() {
    let records = vec![record(
        Dimension::Lifecycle,
        "x86_64",
        "code",
        "2026-08-21T12:00:00Z",
    )];
    assert!(latest_for(&records, Dimension::Routes, "x86_64", "code").is_none());
}

#[test]
fn a_quick_run_is_never_evidence() {
    // Reduced samples answer "how bad is it" while developing; they must not
    // become the baseline a release is judged against.
    let mut quick = base();
    quick.quick = true;
    assert!(latest_for(&[quick], Dimension::Routes, "x86_64", "code").is_none());
}

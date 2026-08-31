use super::{history, insert, latest, open, prune, SCHEMA_VERSION};
use crate::schema::{Dimension, Host, Metric, Record, Release, Unit, SCHEMA};
use crate::stats::Summary;

fn record(dimension: Dimension, arch: &str, profile: &str, at: &str, value: f64) -> Record {
    Record {
        schema: SCHEMA.to_string(),
        dimension,
        recorded_at: at.to_string(),
        release: Release {
            version: "0.6.0".to_string(),
            channel: "stable".to_string(),
            commit: "71e9526f".to_string(),
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
            summary: Summary::of(&[value; 8]).expect("summarizes"),
        }],
        sidecar: None,
    }
}

fn base() -> Record {
    record(Dimension::Routes, "x86_64", "code", "2026-08-21T12:00:00Z", 0.14)
}

fn store() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let connection = open(&dir.path().join("bench.db")).expect("opens");
    (dir, connection)
}

#[test]
fn a_record_round_trips_through_the_store() {
    let (_dir, mut connection) = store();
    insert(&mut connection, &base()).expect("inserts");
    let found = latest(&connection, Dimension::Routes, "x86_64", "code")
        .expect("queries")
        .expect("present");
    assert_eq!(found.release.version, "0.6.0");
    assert_eq!(found.metrics.len(), 1);
    assert_eq!(found.metrics[0].key, "gateway./vms/list.cpu_s");
    assert_eq!(found.metrics[0].unit, Unit::Seconds);
    assert_eq!(found.metrics[0].summary.median, 0.14);
    assert_eq!(found.metrics[0].summary.n, 8);
}

#[test]
fn opening_twice_is_safe() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bench.db");
    open(&path).expect("creates");
    open(&path).expect("reopens");
}

#[test]
fn the_directory_is_created() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(open(&dir.path().join("deep").join("bench.db")).is_ok());
}

#[test]
fn a_store_from_another_schema_version_is_refused() {
    // Rather than reading columns that may have moved.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bench.db");
    {
        let connection = open(&path).expect("creates");
        connection
            .execute("UPDATE meta SET value = '99' WHERE key = 'schema_version'", [])
            .expect("updates");
    }
    let error = open(&path).expect_err("must refuse");
    assert!(error.to_string().contains("schema v99"), "{error}");
}

#[test]
fn the_latest_run_wins() {
    let (_dir, mut connection) = store();
    for (at, value) in [("2026-08-20T12:00:00Z", 0.10), ("2026-08-21T12:00:00Z", 0.20)] {
        let row = record(Dimension::Routes, "x86_64", "code", at, value);
        insert(&mut connection, &row).expect("inserts");
    }
    let found = latest(&connection, Dimension::Routes, "x86_64", "code")
        .expect("queries")
        .expect("present");
    assert_eq!(found.recorded_at, "2026-08-21T12:00:00Z");
    assert_eq!(found.metrics[0].summary.median, 0.20);
}

#[test]
fn another_architecture_is_not_evidence() {
    let (_dir, mut connection) = store();
    let row = record(Dimension::Routes, "arm64", "code", "2026-08-21T12:00:00Z", 0.1);
    insert(&mut connection, &row).expect("inserts");
    assert!(latest(&connection, Dimension::Routes, "x86_64", "code")
        .expect("queries")
        .is_none());
}

#[test]
fn another_profile_is_not_evidence() {
    let (_dir, mut connection) = store();
    let row = record(Dimension::Routes, "x86_64", "co-work", "2026-08-21T12:00:00Z", 0.1);
    insert(&mut connection, &row).expect("inserts");
    assert!(latest(&connection, Dimension::Routes, "x86_64", "code")
        .expect("queries")
        .is_none());
}

#[test]
fn two_profiles_of_one_release_both_survive() {
    // The old scheme keyed on a filename that carried no profile, so the
    // second lane of a gate silently overwrote the first.
    let (_dir, mut connection) = store();
    for profile in ["code", "co-work"] {
        let row = record(Dimension::Routes, "x86_64", profile, "2026-08-21T12:00:00Z", 0.1);
        insert(&mut connection, &row).expect("inserts");
    }
    for profile in ["code", "co-work"] {
        assert!(latest(&connection, Dimension::Routes, "x86_64", profile)
            .expect("queries")
            .is_some());
    }
}

#[test]
fn a_quick_run_is_recorded_but_never_evidence() {
    // Visible in the history, never a baseline.
    let (_dir, mut connection) = store();
    let mut quick = base();
    quick.quick = true;
    insert(&mut connection, &quick).expect("inserts");
    assert!(latest(&connection, Dimension::Routes, "x86_64", "code")
        .expect("queries")
        .is_none());
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM runs", [], |row| row.get(0))
        .expect("counts");
    assert_eq!(rows, 1, "the quick run should still be stored");
}

#[test]
fn history_is_a_query_rather_than_a_glob() {
    let (_dir, mut connection) = store();
    for (at, value) in [
        ("2026-08-19T12:00:00Z", 0.10),
        ("2026-08-20T12:00:00Z", 0.12),
        ("2026-08-21T12:00:00Z", 0.11),
    ] {
        let row = record(Dimension::Routes, "x86_64", "code", at, value);
        insert(&mut connection, &row).expect("inserts");
    }
    let trend = history(&connection, "gateway./vms/list.cpu_s", "x86_64", "code").expect("queries");
    assert_eq!(trend.len(), 3, "oldest first, one point per run");
    assert_eq!(trend[0].2, 0.10);
    assert_eq!(trend[2].2, 0.11);
}

#[test]
fn deleting_a_run_takes_its_metrics_with_it() {
    let (_dir, mut connection) = store();
    let run_id = insert(&mut connection, &base()).expect("inserts");
    connection.execute("PRAGMA foreign_keys = ON", []).expect("enables");
    connection
        .execute("DELETE FROM runs WHERE id = ?1", rusqlite::params![run_id])
        .expect("deletes");
    let orphans: i64 = connection
        .query_row("SELECT count(*) FROM metrics", [], |row| row.get(0))
        .expect("counts");
    assert_eq!(orphans, 0, "retention must not leave metrics behind");
}

#[test]
fn the_schema_version_is_stamped() {
    let (_dir, connection) = store();
    let stamped: String = connection
        .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |row| {
            row.get(0)
        })
        .expect("reads");
    assert_eq!(stamped, SCHEMA_VERSION.to_string());
}

// ---------------------------------------------------------------------------
// Retention. The JSON tree this store replaces grew to 82 files and 1.4 MB
// with a pruner that could not read its own filenames: `RECORDING` requires a
// six-digit timestamp, which the retired `1.5.1783712334` scheme had and
// semver does not, so under `0.6.0` every recording matched nothing and was
// treated as a curated baseline. A store with no retention repeats that in a
// shape nobody can see, because a growing file has no filenames to notice.
// ---------------------------------------------------------------------------

fn versioned(at: &str, version: &str, value: f64) -> Record {
    let mut row = record(Dimension::Routes, "x86_64", "code", at, value);
    row.release.version = version.to_string();
    row
}

#[test]
fn every_run_of_the_current_version_is_kept() {
    // Several samples of the release being worked on is what a threshold that
    // does not flap is set from. One is a guess.
    let (_dir, mut connection) = store();
    for (index, at) in ["2026-08-21T10:00:00Z", "2026-08-21T11:00:00Z", "2026-08-21T12:00:00Z"]
        .iter()
        .enumerate()
    {
        insert(&mut connection, &versioned(at, "0.6.0", 0.1 + index as f64)).expect("inserts");
    }
    assert_eq!(prune(&mut connection, "0.6.0").expect("prunes"), 0);
    assert_eq!(runs(&connection), 3);
}

#[test]
fn only_the_newest_run_of_an_older_version_survives() {
    let (_dir, mut connection) = store();
    for at in ["2026-06-01T10:00:00Z", "2026-06-02T10:00:00Z", "2026-06-03T10:00:00Z"] {
        insert(&mut connection, &versioned(at, "0.5.0", 0.1)).expect("inserts");
    }
    assert_eq!(prune(&mut connection, "0.6.0").expect("prunes"), 2);
    let kept = latest(&connection, Dimension::Routes, "x86_64", "code")
        .expect("queries")
        .expect("present");
    assert_eq!(kept.recorded_at, "2026-06-03T10:00:00Z");
}

#[test]
fn each_subject_keeps_its_own_survivor() {
    // An arm64 number and an x86_64 number are not two samples of one thing,
    // so retention groups by what makes runs comparable.
    let (_dir, mut connection) = store();
    for arch in ["x86_64", "arm64"] {
        for at in ["2026-06-01T10:00:00Z", "2026-06-02T10:00:00Z"] {
            let mut row = versioned(at, "0.5.0", 0.1);
            row.host.arch = arch.to_string();
            insert(&mut connection, &row).expect("inserts");
        }
    }
    assert_eq!(prune(&mut connection, "0.6.0").expect("prunes"), 2);
    for arch in ["x86_64", "arm64"] {
        assert!(latest(&connection, Dimension::Routes, arch, "code")
            .expect("queries")
            .is_some());
    }
}

#[test]
fn a_pruned_run_takes_its_metrics_with_it() {
    // Rows with no run are invisible growth: nothing queries them and nothing
    // counts them.
    let (_dir, mut connection) = store();
    for at in ["2026-06-01T10:00:00Z", "2026-06-02T10:00:00Z"] {
        insert(&mut connection, &versioned(at, "0.5.0", 0.1)).expect("inserts");
    }
    prune(&mut connection, "0.6.0").expect("prunes");
    let orphans: i64 = connection
        .query_row(
            "SELECT count(*) FROM metrics WHERE run_id NOT IN (SELECT id FROM runs)",
            [],
            |row| row.get(0),
        )
        .expect("counts");
    assert_eq!(orphans, 0);
}

#[test]
fn a_quick_run_of_an_older_version_is_not_the_survivor() {
    // It was never evidence, so keeping it as the one surviving sample would
    // leave the older version represented by a number that may not be used.
    let (_dir, mut connection) = store();
    insert(&mut connection, &versioned("2026-06-01T10:00:00Z", "0.5.0", 0.1)).expect("inserts");
    let mut quick = versioned("2026-06-02T10:00:00Z", "0.5.0", 0.9);
    quick.quick = true;
    insert(&mut connection, &quick).expect("inserts");

    prune(&mut connection, "0.6.0").expect("prunes");
    let kept = latest(&connection, Dimension::Routes, "x86_64", "code")
        .expect("queries")
        .expect("present");
    assert_eq!(kept.recorded_at, "2026-06-01T10:00:00Z");
}

#[test]
fn pruning_an_empty_store_is_not_an_error() {
    let (_dir, mut connection) = store();
    assert_eq!(prune(&mut connection, "0.6.0").expect("prunes"), 0);
}

fn runs(connection: &rusqlite::Connection) -> i64 {
    connection
        .query_row("SELECT count(*) FROM runs", [], |row| row.get(0))
        .expect("counts")
}

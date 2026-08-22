//! Where measurements live: one SQLite database, not a pile of JSON.
//!
//! The scheme this replaces was a file per dimension per release per
//! architecture per profile, each holding a nested document with its own
//! shape. Eleven such directories accumulated ~80 files in ten mutually
//! incompatible formats, one of them 80 KB of captured stdout. Asking "is
//! `/vms/list` slower than it was three releases ago" meant globbing
//! filenames, parsing every match, and knowing which of ten shapes each used.
//!
//! A benchmark history is a time series with a fixed set of columns, which is
//! what a table is. `capsem-logger` already owns this pattern for the session
//! ledger; this is the same answer to the same question.
//!
//! JSON stays where it belongs: the collector wire format, which is a pipe
//! rather than storage.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::schema::{Dimension, Metric, Record, Unit};
use crate::stats::Summary;

/// Bumped when a reader must change; checked on open.
pub const SCHEMA_VERSION: i64 = 1;

const CREATE: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- One row per dimension per run. `quick` runs are recorded like any other and
-- excluded when evidence is selected, so a dev-loop measurement is visible
-- without ever becoming a baseline.
CREATE TABLE IF NOT EXISTS runs (
    id          INTEGER PRIMARY KEY,
    dimension   TEXT    NOT NULL,
    recorded_at TEXT    NOT NULL,
    version     TEXT    NOT NULL,
    channel     TEXT    NOT NULL,
    commit_sha  TEXT    NOT NULL,
    arch        TEXT    NOT NULL,
    os          TEXT    NOT NULL,
    cpu_count   INTEGER NOT NULL,
    kvm         INTEGER NOT NULL,
    governor    TEXT,
    load_before REAL    NOT NULL,
    profile     TEXT    NOT NULL,
    quick       INTEGER NOT NULL,
    sidecar     TEXT
);

-- One row per metric per run. Columns rather than a document, so a trend is a
-- query instead of a glob and a parser.
CREATE TABLE IF NOT EXISTS metrics (
    run_id  INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    key     TEXT    NOT NULL,
    unit    TEXT    NOT NULL,
    n       INTEGER NOT NULL,
    min     REAL    NOT NULL,
    max     REAL    NOT NULL,
    mean    REAL    NOT NULL,
    median  REAL    NOT NULL,
    p90     REAL    NOT NULL,
    p95     REAL    NOT NULL,
    p99     REAL    NOT NULL,
    p999    REAL    NOT NULL,
    stddev  REAL    NOT NULL,
    cv      REAL    NOT NULL,
    mad     REAL    NOT NULL,
    PRIMARY KEY (run_id, key)
);

-- The two questions asked of this data: one metric over time, and one run's
-- metrics. Both are covered rather than scanned.
CREATE INDEX IF NOT EXISTS metrics_by_key ON metrics(key);
CREATE INDEX IF NOT EXISTS runs_by_subject
    ON runs(dimension, arch, profile, quick, recorded_at);
";

/// Open or create the store at `path`.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let connection = Connection::open(path)
        .with_context(|| format!("cannot open benchmark store {}", path.display()))?;
    connection.execute_batch(CREATE)?;

    let found: Option<i64> = connection
        .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |row| {
            row.get::<_, String>(0)
        })
        .ok()
        .and_then(|value| value.parse().ok());
    match found {
        None => {
            connection.execute(
                "INSERT INTO meta(key, value) VALUES('schema_version', ?1)",
                params![SCHEMA_VERSION.to_string()],
            )?;
        }
        Some(version) if version != SCHEMA_VERSION => {
            anyhow::bail!(
                "benchmark store {} is schema v{version}, this build writes v{SCHEMA_VERSION}",
                path.display()
            );
        }
        Some(_) => {}
    }
    Ok(connection)
}

/// Insert one dimension's measurements.
///
/// A rerun of the same subject at the same instant replaces its metrics rather
/// than accumulating a second copy, which is what the old per-file scheme did
/// by overwriting a filename -- except there the identity was accidental and
/// two profiles silently shared one.
pub fn insert(connection: &mut Connection, record: &Record) -> Result<i64> {
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO runs(
            dimension, recorded_at, version, channel, commit_sha, arch, os,
            cpu_count, kvm, governor, load_before, profile, quick, sidecar
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            record.dimension.as_str(),
            record.recorded_at,
            record.release.version,
            record.release.channel,
            record.release.commit,
            record.host.arch,
            record.host.os,
            record.host.cpu_count as i64,
            i64::from(record.host.kvm),
            record.host.governor,
            record.host.load_before,
            record.profile,
            i64::from(record.quick),
            record.sidecar,
        ],
    )?;
    let run_id = transaction.last_insert_rowid();

    {
        let mut insert = transaction.prepare(
            "INSERT INTO metrics(
                run_id, key, unit, n, min, max, mean, median,
                p90, p95, p99, p999, stddev, cv, mad
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        )?;
        for metric in &record.metrics {
            let summary = &metric.summary;
            insert.execute(params![
                run_id,
                metric.key,
                serde_json::to_string(&metric.unit)?.trim_matches('"'),
                summary.n as i64,
                summary.min,
                summary.max,
                summary.mean,
                summary.median,
                summary.p90,
                summary.p95,
                summary.p99,
                summary.p999,
                summary.stddev,
                summary.cv,
                summary.mad,
            ])?;
        }
    }
    transaction.commit()?;
    Ok(run_id)
}

/// The most recent non-quick run of one subject, as a `Record`.
///
/// Architecture and profile must match: comparing arm64 against x86_64, or
/// `code` against `co-work`, measures the difference between them rather than
/// a change in Capsem. A quick run is never evidence.
pub fn latest(
    connection: &Connection,
    dimension: Dimension,
    arch: &str,
    profile: &str,
) -> Result<Option<Record>> {
    let run = connection
        .query_row(
            "SELECT id, recorded_at, version, channel, commit_sha, os, cpu_count,
                    kvm, governor, load_before, sidecar
               FROM runs
              WHERE dimension = ?1 AND arch = ?2 AND profile = ?3 AND quick = 0
              ORDER BY recorded_at DESC, id DESC
              LIMIT 1",
            params![dimension.as_str(), arch, profile],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    crate::schema::Record {
                        schema: crate::schema::SCHEMA.to_string(),
                        dimension,
                        recorded_at: row.get(1)?,
                        release: crate::schema::Release {
                            version: row.get(2)?,
                            channel: row.get(3)?,
                            commit: row.get(4)?,
                        },
                        host: crate::schema::Host {
                            arch: arch.to_string(),
                            os: row.get(5)?,
                            cpu_count: row.get::<_, i64>(6)? as usize,
                            kvm: row.get::<_, i64>(7)? != 0,
                            governor: row.get(8)?,
                            load_before: row.get(9)?,
                        },
                        profile: profile.to_string(),
                        quick: false,
                        metrics: Vec::new(),
                        sidecar: row.get(10)?,
                    },
                ))
            },
        )
        .ok();

    let Some((run_id, mut record)) = run else {
        return Ok(None);
    };
    record.metrics = metrics_of(connection, run_id)?;
    Ok(Some(record))
}

fn metrics_of(connection: &Connection, run_id: i64) -> Result<Vec<Metric>> {
    let mut query = connection.prepare(
        "SELECT key, unit, n, min, max, mean, median, p90, p95, p99, p999,
                stddev, cv, mad
           FROM metrics WHERE run_id = ?1 ORDER BY key",
    )?;
    let rows = query.query_map(params![run_id], |row| {
        let unit: String = row.get(1)?;
        Ok(Metric {
            key: row.get(0)?,
            unit: serde_json::from_str::<Unit>(&format!("\"{unit}\"")).unwrap_or(Unit::Count),
            summary: Summary {
                n: row.get::<_, i64>(2)? as usize,
                min: row.get(3)?,
                max: row.get(4)?,
                mean: row.get(5)?,
                median: row.get(6)?,
                p90: row.get(7)?,
                p95: row.get(8)?,
                p99: row.get(9)?,
                p999: row.get(10)?,
                stddev: row.get(11)?,
                cv: row.get(12)?,
                mad: row.get(13)?,
            },
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Every (dimension, arch, profile) this store holds a non-quick run for.
///
/// What `verify` iterates: the subjects measured, each judged against the
/// evidence for the same subject.
pub fn subjects(connection: &Connection) -> Result<Vec<(Dimension, String, String)>> {
    let mut query = connection.prepare(
        "SELECT DISTINCT dimension, arch, profile FROM runs WHERE quick = 0
          ORDER BY dimension, arch, profile",
    )?;
    let rows = query.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut found = Vec::new();
    for row in rows {
        let (name, arch, profile) = row?;
        if let Some(dimension) = Dimension::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == name)
        {
            found.push((dimension, arch, profile));
        }
    }
    Ok(found)
}

/// Bound the history without losing either of its two uses.
///
/// The history answers two questions that need different amounts of data:
/// "did this release regress against the last one", which one recording per
/// release answers, and "what threshold would not flap", which needs several
/// recordings of the release being worked on. So: keep every run of
/// `current`, and only the newest non-quick run of each older version, per
/// subject -- an arm64 number and an x86_64 number are not two samples of one
/// thing.
///
/// This is the rule `scripts/prune-benchmark-history.py` states and could not
/// apply. Its filename pattern requires a six-digit timestamp, which the
/// retired `1.5.1783712334` scheme had and semver does not, so under `0.6.0`
/// every recording fell through to "curated baseline, never pruned". The tree
/// reached 82 files. A table would have reached the same place more quietly,
/// having no filenames for anyone to notice.
///
/// Returns how many runs were removed. Their metrics go with them: rows with
/// no run are growth nothing queries and nothing counts.
pub fn prune(connection: &mut Connection, current: &str) -> Result<usize> {
    let transaction = connection.transaction()?;
    transaction.execute("PRAGMA foreign_keys = ON", [])?;
    let removed = transaction.execute(
        "DELETE FROM runs
          WHERE version != ?1
            AND id NOT IN (
                SELECT id FROM (
                    SELECT id,
                           row_number() OVER (
                               PARTITION BY version, dimension, arch, profile
                               ORDER BY quick, recorded_at DESC, id DESC
                           ) AS rank
                      FROM runs
                     WHERE version != ?1
                ) WHERE rank = 1
            )",
        params![current],
    )?;
    // `ON DELETE CASCADE` is only honoured with foreign keys enabled, and the
    // pragma is per-connection rather than stored in the schema, so the sweep
    // is stated here too rather than trusted to whoever opened this handle.
    transaction.execute(
        "DELETE FROM metrics WHERE run_id NOT IN (SELECT id FROM runs)",
        [],
    )?;
    transaction.commit()?;
    Ok(removed)
}

/// One metric's history, oldest first: what a trend line is drawn from.
pub fn history(
    connection: &Connection,
    key: &str,
    arch: &str,
    profile: &str,
) -> Result<Vec<(String, String, f64)>> {
    let mut query = connection.prepare(
        "SELECT runs.version, runs.recorded_at, metrics.median
           FROM metrics JOIN runs ON runs.id = metrics.run_id
          WHERE metrics.key = ?1 AND runs.arch = ?2 AND runs.profile = ?3
                AND runs.quick = 0
          ORDER BY runs.recorded_at",
    )?;
    let rows = query.query_map(params![key, arch, profile], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests;

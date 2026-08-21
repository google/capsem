//! Where a record goes, and how it is found again.
//!
//! Six producers each spelled their own filename, and the names disagreed on
//! identity: `lifecycle` carried a profile, `capsem-bench` carried an
//! architecture, `route-latency` carried neither -- so one gate's `code` and
//! `co-work` runs wrote the same path and the second overwrote the first --
//! and `parallel` hardcoded `data_1.0.json` and overwrote itself every run.
//! One writer, one name, every identity axis in it.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::schema::{Dimension, Record};

/// Write a record under `root`, returning the path written.
///
/// Written whole and renamed into place: a run interrupted mid-write would
/// otherwise leave a truncated record that reads as evidence.
pub fn write(root: &Path, record: &Record) -> Result<PathBuf> {
    fs::create_dir_all(root)
        .with_context(|| format!("cannot create benchmark output directory {}", root.display()))?;

    let destination = root.join(record.filename());
    let staging = destination.with_extension("json.partial");
    let mut text = serde_json::to_string_pretty(record).context("cannot serialize record")?;
    text.push('\n');

    fs::write(&staging, &text)
        .with_context(|| format!("cannot write {}", staging.display()))?;
    fs::rename(&staging, &destination)
        .with_context(|| format!("cannot place {}", destination.display()))?;
    Ok(destination)
}

/// Read one record.
pub fn read(path: &Path) -> Result<Record> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} is not a benchmark record", path.display()))
}

/// Every record under `root`, oldest name first.
///
/// A directory that does not exist yet is empty, not an error: the first run
/// on a machine has nothing to compare against and must still be able to say
/// so rather than fail.
pub fn read_all(root: &Path) -> Result<Vec<Record>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(root)
        .with_context(|| format!("cannot list {}", root.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect();
    paths.sort();
    paths.iter().map(|path| read(path)).collect()
}

/// The most recent record for a dimension, matched on the identity axes that
/// make two runs comparable.
///
/// Architecture and profile must match: comparing an arm64 run against an
/// x86_64 one, or `code` against `co-work`, measures the difference between
/// them rather than a change in Capsem.
pub fn latest_for(
    records: &[Record],
    dimension: Dimension,
    arch: &str,
    profile: &str,
) -> Option<Record> {
    records
        .iter()
        .filter(|record| {
            record.dimension == dimension
                && record.host.arch == arch
                && record.profile == profile
                // A reduced-sample dev-loop run is not evidence.
                && !record.quick
        })
        .max_by(|left, right| left.recorded_at.cmp(&right.recorded_at))
        .cloned()
}

#[cfg(test)]
mod tests;

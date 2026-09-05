//! The commands that read and judge records.
//!
//! Split from `main.rs`, which parses and dispatches. The binary's single
//! file was already the largest thing in the crate before this work added a
//! record pipeline to it.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};

use crate::collector;
use crate::machine;
use crate::schema;
use crate::stats;
use crate::store;

/// What can be measured, and what a quick run leaves out.
pub(crate) fn list_dimensions() {
    println!("{:<12}  QUICK", "DIMENSION");
    for dimension in schema::Dimension::ALL {
        let quick = if dimension.in_quick_lane() {
            "yes"
        } else if dimension.needs_vm() {
            "no (boots a guest)"
        } else {
            "no (slow)"
        };
        println!("{:<12}  {}", dimension.as_str(), quick);
    }
}

/// Refuse to measure on a machine whose numbers would not mean anything.
///
/// Exits non-zero when unfit, so a caller can gate on it rather than read
/// prose. Every measurement in this repository predating this command was
/// taken without knowing any of these facts.
pub(crate) fn doctor(json: bool, strays: Vec<String>) -> Result<()> {
    let fitness = machine::examine(std::env::consts::ARCH, std::env::consts::OS, &strays);

    if json {
        println!("{}", serde_json::to_string_pretty(&fitness)?);
    } else {
        let host = &fitness.host;
        println!(
            "{} {}, {} cores, kvm={}, governor={}, load={:.2}",
            host.os,
            host.arch,
            host.cpu_count,
            host.kvm,
            host.governor.as_deref().unwrap_or("n/a"),
            host.load_before
        );
        for objection in &fitness.objections {
            println!("  unfit ({}): {}", objection.what, objection.detail);
        }
        if fitness.fit() {
            println!("  fit to measure");
        }
    }

    if fitness.fit() {
        Ok(())
    } else {
        bail!("this machine is not fit to measure on")
    }
}

/// Run the collectors for the requested dimensions and record what they measured.
///
/// A collector reports raw samples; every statistic is computed here, so `p99`
/// means the same thing across dimensions and adding one costs a program that
/// prints numbers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_dimensions(
    wanted: &[schema::Dimension],
    collectors: &Path,
    out: &Path,
    timeout: Duration,
    interpreter: Option<&str>,
    quick: bool,
    channel: &str,
    commit: &str,
    profile: &str,
    strays: Vec<String>,
) -> Result<()> {
    let fitness = machine::examine(std::env::consts::ARCH, std::env::consts::OS, &strays);
    let mut connection = store::open(out)?;
    let mut ran = 0usize;

    for dimension in wanted {
        // A quick run answers "how bad is it" while developing, so it takes
        // only the dimensions that finish in seconds.
        if quick && !dimension.in_quick_lane() {
            continue;
        }
        let program = collectors.join(dimension.as_str());
        if !program.exists() {
            println!("{}: no collector yet", dimension.as_str());
            continue;
        }

        // An interpreter prefix runs the collector as an argument to it, so a
        // collector needing the project's Python gets that environment rather
        // than whatever `#!/usr/bin/env python3` happens to resolve to.
        let (program, mut args) = match interpreter {
            None => (program.clone(), Vec::new()),
            Some(prefix) => {
                let mut words = prefix.split_whitespace().map(str::to_string);
                let head = words.next().unwrap_or_default();
                let mut rest: Vec<String> = words.collect();
                rest.push(program.to_string_lossy().into_owned());
                (std::path::PathBuf::from(head), rest)
            }
        };
        if quick {
            args.push("--quick".to_string());
        }
        let collected = collector::run(&program, &args, timeout)?;

        let metrics = collected
            .metrics
            .iter()
            .filter_map(|(key, raw)| {
                stats::Summary::of(&raw.samples).map(|summary| schema::Metric {
                    key: format!("{}.{key}", dimension.as_str()),
                    unit: raw.unit,
                    summary,
                })
            })
            .collect::<Vec<_>>();

        let record = schema::Record {
            schema: schema::SCHEMA.to_string(),
            dimension: *dimension,
            recorded_at: rfc3339_now(),
            release: schema::Release {
                version: env!("CARGO_PKG_VERSION").to_string(),
                channel: channel.to_string(),
                commit: commit.to_string(),
            },
            host: fitness.host.clone(),
            profile: profile.to_string(),
            quick,
            metrics,
            sidecar: collected.sidecar,
        };
        let run_id = store::insert(&mut connection, &record)?;
        println!(
            "{}: {} metrics recorded as run {run_id}",
            dimension.as_str(),
            record.metrics.len()
        );
        ran += 1;
    }

    if ran == 0 {
        bail!("no dimension produced a record");
    }

    // Recording bounds itself. A retention rule that has to be remembered as
    // a separate command is one that gets run until the day it stops being
    // run, which is what happened to the JSON tree this replaced.
    let removed = store::prune(&mut connection, env!("CARGO_PKG_VERSION"))?;
    if removed > 0 {
        println!("pruned {removed} superseded run(s) of older releases");
    }
    Ok(())
}

/// UTC, to the second. One clock for every record, host and guest alike: the
/// old artifacts carried a guest `timestamp`, a host `host_recorded_at`, or
/// neither, and could not be ordered against each other.
pub(crate) fn rfc3339_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let days = seconds / 86_400;
    let time = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

/// Howard Hinnant's days-from-civil, inverted. Avoids a date dependency for
/// the one timestamp this binary writes.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Resolve dimension names from the command line.
///
/// An unknown name is refused with the full list rather than silently
/// measuring nothing, which is how a typo becomes a green run that proved
/// nothing.
pub(crate) fn select_dimensions(names: &[String]) -> Result<Vec<schema::Dimension>> {
    if names.is_empty() {
        return Ok(schema::Dimension::ALL.to_vec());
    }
    names
        .iter()
        .map(|name| {
            schema::Dimension::ALL
                .iter()
                .copied()
                .find(|dimension| dimension.as_str() == name)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown dimension {name}; known: {}",
                        schema::Dimension::ALL
                            .iter()
                            .map(|d| d.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
        })
        .collect()
}

/// What every measured subject currently reads, and how it has moved.
///
/// The question the old layout could not answer without globbing filenames
/// and parsing ten shapes: is this slower than it was?
pub(crate) fn report(store_db: &Path, arch: &str, profile: &str) -> Result<()> {
    let connection = store::open(store_db)?;
    let subjects = store::subjects(&connection)?;
    if subjects.is_empty() {
        bail!("no runs recorded in {}", store_db.display());
    }

    for (dimension, subject_arch, subject_profile) in subjects {
        if subject_arch != arch || subject_profile != profile {
            continue;
        }
        let Some(record) = store::latest(&connection, dimension, arch, profile)? else {
            continue;
        };
        println!(
            "\n## {} -- {} {} {}, recorded {}",
            dimension.as_str(),
            record.release.version,
            arch,
            profile,
            record.recorded_at
        );
        println!("| metric | median | p99 | cv | n | trend |");
        println!("|---|---:|---:|---:|---:|---|");
        for metric in &record.metrics {
            let points = store::history(&connection, &metric.key, arch, profile)?;
            // Oldest to newest, so the direction is visible without a chart.
            let trend = points
                .iter()
                .rev()
                .take(5)
                .rev()
                .map(|(_, _, value)| format!("{value:.2}"))
                .collect::<Vec<_>>()
                .join(" -> ");
            println!(
                "| {} | {:.2} | {:.2} | {:.2} | {} | {trend} |",
                metric.key, metric.summary.median, metric.summary.p99, metric.summary.cv, metric.summary.n
            );
        }
    }
    Ok(())
}

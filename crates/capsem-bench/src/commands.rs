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
use crate::record;
use crate::schema;
use crate::stats;
use crate::{Artifact, Thresholds};

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
    quick: bool,
    channel: &str,
    commit: &str,
    profile: &str,
    strays: Vec<String>,
) -> Result<()> {
    let fitness = machine::examine(std::env::consts::ARCH, std::env::consts::OS, &strays);
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

        let mut args = Vec::new();
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
        let path = record::write(out, &record)?;
        println!(
            "{}: {} metrics -> {}",
            dimension.as_str(),
            record.metrics.len(),
            path.display()
        );
        ran += 1;
    }

    if ran == 0 {
        bail!("no dimension produced a record");
    }
    Ok(())
}

/// Compare every metric the two records share.
///
/// A metric present in only one of them is reported rather than skipped: a
/// disappearing metric is how coverage quietly shrinks.
pub(crate) fn compare(baseline_path: &Path, current_path: &Path, thresholds: Thresholds) -> Result<()> {
    let baseline = record::read(baseline_path)?;
    let current = record::read(current_path)?;
    let verdicts = judge(&baseline, &current, thresholds);
    report_comparisons(&baseline, &verdicts);
    Ok(())
}

/// Ratchet this run's records against the evidence.
///
/// Exits non-zero only on a `significant` move. A ratio breach inside the
/// baseline's own noise is printed and forgiven -- the 0.6.0 release was held
/// for two hours by exactly such a reading, which then did not reproduce.
pub(crate) fn verify(records: &Path, evidence_dir: &Path, thresholds: Thresholds) -> Result<()> {
    let evidence = record::read_all(evidence_dir)?;
    let current = record::read_all(records)?;
    if current.is_empty() {
        bail!("no records to verify in {}", records.display());
    }

    let mut significant = 0usize;
    for record in &current {
        let Some(baseline) = record::latest_for(
            &evidence,
            record.dimension,
            &record.host.arch,
            &record.profile,
        ) else {
            // Seeding, not failing: a dimension measured for the first time
            // has nothing to regress against.
            println!(
                "{}: no evidence yet for {} {} -- seeding",
                record.dimension.as_str(),
                record.host.arch,
                record.profile
            );
            continue;
        };
        let verdicts = judge(&baseline, record, thresholds);
        significant += verdicts.iter().filter(|v| v.significant).count();
        report_comparisons(&baseline, &verdicts);
    }

    if significant == 0 {
        Ok(())
    } else {
        bail!("{significant} metric(s) regressed beyond their own noise")
    }
}

/// Compare two records on the statistic that describes each metric.
fn judge(
    baseline: &schema::Record,
    current: &schema::Record,
    thresholds: Thresholds,
) -> Vec<stats::Comparison> {
    let mut verdicts = Vec::new();
    for metric in &current.metrics {
        let Some(before) = baseline.metric(&metric.key) else {
            continue;
        };
        // The median describes a distribution's centre without the tail that
        // one scheduler stall writes into a mean.
        verdicts.push(stats::compare(
            &metric.key,
            stats::Statistic::Median,
            &before.summary,
            &metric.summary,
            thresholds.maximum_factor,
            thresholds.noise_factor,
        ));
    }
    verdicts
}

fn report_comparisons(baseline: &schema::Record, verdicts: &[stats::Comparison]) {
    for verdict in verdicts {
        let verdict_label = if verdict.significant {
            "REGRESSED"
        } else if verdict.regressed {
            "noisy"
        } else {
            "ok"
        };
        println!(
            "{verdict_label:>9}  {:<48} {:>10.4} -> {:>10.4}  {:+.1}%  (baseline cv {:.3})",
            verdict.key,
            verdict.baseline,
            verdict.current,
            verdict.delta_pct,
            baseline
                .metric(&verdict.key)
                .map(|metric| metric.summary.cv)
                .unwrap_or_default()
        );
    }
}

/// Turn a protocol run into a `capsem.bench.v1` record.
///
/// Every statistic is recomputed here from the raw latencies rather than
/// copied from the artifact's own `latency_ms`, so `p99` means what it means
/// everywhere else. Throughput and error counts are single observations of a
/// whole run, so they summarize as one sample and say so through `n`.
pub(crate) fn protocol_record(
    artifact: &Artifact,
    channel: &str,
    commit: &str,
    profile: &str,
    strays: Vec<String>,
) -> schema::Record {
    let fitness = machine::examine(std::env::consts::ARCH, std::env::consts::OS, &strays);
    let report = &artifact.mock_server_protocol;

    let mut metrics = Vec::new();
    for scenario in &report.scenarios {
        let mut push = |suffix: &str, unit: schema::Unit, samples: &[f64]| {
            if let Some(summary) = stats::Summary::of(samples) {
                metrics.push(schema::Metric {
                    key: format!("protocol.{}.{}.{suffix}", report.lane, scenario.name),
                    unit,
                    summary,
                });
            }
        };
        push(
            "latency_ms",
            schema::Unit::Milliseconds,
            &scenario.latency_samples,
        );
        push(
            "requests_per_sec",
            schema::Unit::RequestsPerSecond,
            &[scenario.requests_per_sec],
        );
        push(
            "bytes_per_sec",
            schema::Unit::Bytes,
            &[scenario.bytes_per_sec],
        );
        push("failed", schema::Unit::Count, &[scenario.failed as f64]);
    }

    schema::Record {
        schema: schema::SCHEMA.to_string(),
        dimension: schema::Dimension::Protocol,
        recorded_at: rfc3339_now(),
        release: schema::Release {
            version: env!("CARGO_PKG_VERSION").to_string(),
            channel: channel.to_string(),
            commit: commit.to_string(),
        },
        host: fitness.host,
        profile: profile.to_string(),
        quick: false,
        metrics,
        sidecar: None,
    }
}

/// UTC, to the second. One clock for every record, host and guest alike: the
/// old artifacts carried a guest `timestamp`, a host `host_recorded_at`, or
/// neither, and could not be ordered against each other.
fn rfc3339_now() -> String {
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

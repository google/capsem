//! Evidence comparison and release verdicts.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{evidence, schema, stats, store, Thresholds};

/// Compare one subject in two stores, metric by metric.
pub(crate) fn compare(
    baseline_db: &Path,
    current_db: &Path,
    dimension: schema::Dimension,
    arch: &str,
    runs: usize,
    thresholds: Thresholds,
) -> Result<()> {
    if runs == 0 {
        bail!("--runs must be at least 1");
    }
    let baseline_store = store::open(baseline_db)?;
    let current_store = store::open(current_db)?;
    let baseline_runs = store::latest_many(&baseline_store, dimension, arch, runs)?;
    let current_runs = store::latest_many(&current_store, dimension, arch, runs)?;
    evidence::repeats(&baseline_runs, runs)?;
    evidence::repeats(&current_runs, runs)?;
    evidence::compatible(&baseline_runs[0], &current_runs[0])?;
    let baseline =
        aggregate(&baseline_runs).with_context(|| format!("no {} evidence for {arch}", dimension.as_str()))?;
    let current = aggregate(&current_runs).with_context(|| format!("no {} run for {arch}", dimension.as_str()))?;
    println!(
        "comparing {} baseline run(s) with {} current run(s)",
        baseline_runs.len(),
        current_runs.len()
    );
    report(&baseline, &judge(&baseline, &current, thresholds));
    Ok(())
}

fn aggregate(records: &[schema::Record]) -> Option<schema::Record> {
    let mut aggregate = records.first()?.clone();
    aggregate.metrics = aggregate
        .metrics
        .iter()
        .filter_map(|metric| {
            let samples: Option<Vec<_>> = records
                .iter()
                .map(|record| record.metric(&metric.key).map(|found| found.summary.median))
                .collect();
            let samples = samples?;
            let summary = stats::Summary::of(&samples)?;
            Some(schema::Metric {
                samples: Some(samples),
                key: metric.key.clone(),
                unit: metric.unit,
                summary,
            })
        })
        .collect();
    Some(aggregate)
}

/// Ratchet this run against evidence without granting noise release authority.
pub(crate) fn verify(records: &Path, evidence_dir: &Path, runs: usize, thresholds: Thresholds) -> Result<()> {
    let evidence = store::open(evidence_dir)?;
    let measured = store::open(records)?;
    let subjects = store::subjects(&measured)?;
    if subjects.is_empty() {
        bail!("no records to verify in {}", records.display());
    }

    let mut significant = 0usize;
    for (dimension, arch) in subjects {
        let current_runs = store::latest_many(&measured, dimension, &arch, runs)?;
        evidence::repeats(&current_runs, runs)?;
        let baseline_runs = store::latest_many(&evidence, dimension, &arch, runs)?;
        if baseline_runs.is_empty() {
            println!("{}: no evidence yet for {arch} -- seeding", dimension.as_str());
            continue;
        }
        evidence::repeats(&baseline_runs, runs)?;
        evidence::compatible(&baseline_runs[0], &current_runs[0])?;
        let record = aggregate(&current_runs).context("current repeats")?;
        let baseline = aggregate(&baseline_runs).context("baseline repeats")?;
        let verdicts = judge(&baseline, &record, thresholds);
        significant += verdicts.iter().filter(|verdict| verdict.significant).count();
        report(&baseline, &verdicts);
    }
    if significant == 0 {
        Ok(())
    } else {
        bail!("{significant} metric(s) regressed beyond their operating envelope")
    }
}

fn judge(baseline: &schema::Record, current: &schema::Record, thresholds: Thresholds) -> Vec<stats::Comparison> {
    current
        .metrics
        .iter()
        .filter_map(|metric| {
            let before = baseline.metric(&metric.key)?;
            Some(stats::compare(
                &metric.key,
                stats::Statistic::Median,
                &before.summary,
                &metric.summary,
                metric.unit,
                thresholds,
            ))
        })
        .collect()
}

fn report(baseline: &schema::Record, verdicts: &[stats::Comparison]) {
    for verdict in verdicts {
        let label = if verdict.significant {
            "REGRESSED"
        } else if verdict.regressed && !verdict.material {
            "tiny"
        } else if verdict.regressed {
            "noisy"
        } else {
            "ok"
        };
        let cv = baseline
            .metric(&verdict.key)
            .map(|metric| metric.summary.cv)
            .unwrap_or_default();
        println!(
            "{label:>9}  {:<48} {:>10.4} -> {:>10.4}  {:+.1}%  (baseline cv {cv:.3})",
            verdict.key, verdict.baseline, verdict.current, verdict.delta_pct
        );
    }
}

#[cfg(test)]
mod tests;

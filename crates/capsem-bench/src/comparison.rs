//! Evidence comparison and release verdicts.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{schema, stats, store, Thresholds};

/// Compare one subject in two stores, metric by metric.
pub(crate) fn compare(
    baseline_db: &Path,
    current_db: &Path,
    dimension: schema::Dimension,
    arch: &str,
    profile: &str,
    thresholds: Thresholds,
) -> Result<()> {
    let baseline_store = store::open(baseline_db)?;
    let current_store = store::open(current_db)?;
    let baseline = store::latest(&baseline_store, dimension, arch, profile)?
        .with_context(|| format!("no {} evidence for {arch} {profile}", dimension.as_str()))?;
    let current = store::latest(&current_store, dimension, arch, profile)?
        .with_context(|| format!("no {} run for {arch} {profile}", dimension.as_str()))?;
    report(&baseline, &judge(&baseline, &current, thresholds));
    Ok(())
}

/// Ratchet this run against evidence without granting noise release authority.
pub(crate) fn verify(records: &Path, evidence_dir: &Path, thresholds: Thresholds) -> Result<()> {
    let evidence = store::open(evidence_dir)?;
    let measured = store::open(records)?;
    let subjects = store::subjects(&measured)?;
    if subjects.is_empty() {
        bail!("no records to verify in {}", records.display());
    }

    let mut significant = 0usize;
    for (dimension, arch, profile) in subjects {
        let Some(record) = store::latest(&measured, dimension, &arch, &profile)? else {
            continue;
        };
        let Some(baseline) = store::latest(&evidence, dimension, &arch, &profile)? else {
            println!(
                "{}: no evidence yet for {arch} {profile} -- seeding",
                dimension.as_str()
            );
            continue;
        };
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
                thresholds.maximum_factor,
                thresholds.noise_factor,
                thresholds.minimum_time_resolution_ms,
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

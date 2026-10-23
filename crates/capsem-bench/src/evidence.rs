//! One admission rule for stored benchmark authority, never inferred from a summary.

use crate::schema::{Host, Record};
use anyhow::{bail, Context, Result};

fn same_machine(a: &Host, b: &Host) -> bool {
    a.arch == b.arch && a.os == b.os && a.cpu_count == b.cpu_count && a.kvm == b.kvm && a.governor == b.governor
}

pub(crate) fn eligible(record: &Record) -> Result<()> {
    let measurement = record
        .measurement
        .as_ref()
        .context("historical observation has no machine-fitness receipts")?;
    if record.quick || !measurement.before.fit() || !measurement.after.fit() {
        bail!("diagnostic or unfit benchmark observation cannot be evidence");
    }
    if !same_machine(&record.host, &measurement.before.host)
        || !same_machine(&measurement.before.host, &measurement.after.host)
    {
        bail!("machine identity changed across the measurement");
    }
    if measurement.profile.is_empty()
        || record.release.commit.len() != 40
        || !record.release.commit.bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("benchmark lacks exact source/configuration identity");
    }
    if record.metrics.is_empty() {
        bail!("benchmark has no metrics");
    }
    if record.metrics.iter().any(|metric| {
        metric
            .samples
            .as_ref()
            .is_none_or(|samples| samples.is_empty() || samples.len() != metric.summary.n)
    }) {
        bail!("benchmark has no complete original sample arrays");
    }
    Ok(())
}

pub(crate) fn compatible(a: &Record, b: &Record) -> Result<()> {
    eligible(a)?;
    eligible(b)?;
    if a.dimension != b.dimension
        || a.schema != b.schema
        || !same_machine(&a.host, &b.host)
        || a.measurement.as_ref().unwrap().profile != b.measurement.as_ref().unwrap().profile
    {
        bail!("benchmark machine, subject or configuration cohorts differ");
    }
    if a.metrics.len() != b.metrics.len()
        || a.metrics
            .iter()
            .any(|metric| b.metric(&metric.key).is_none_or(|found| found.unit != metric.unit))
    {
        bail!("benchmark metric coverage or units differ");
    }
    Ok(())
}

pub(crate) fn repeats(records: &[Record], required: usize) -> Result<()> {
    if required == 0 || records.len() != required {
        bail!("need {required} complete benchmark repeats, found {}", records.len());
    }
    for record in records {
        compatible(&records[0], record)?;
        if record.release.commit != records[0].release.commit {
            bail!("repeated observations mix source commits");
        }
    }
    Ok(())
}

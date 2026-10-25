//! Bounded original f64 observations; summaries remain owned by stats.rs.

use crate::{sample_limits::Limits, schema::Record};
use anyhow::{bail, Result};

pub(super) fn validate(record: &Record, limits: Limits) -> Result<()> {
    let mut total = 0usize;
    for metric in &record.metrics {
        if let Some(samples) = &metric.samples {
            if samples.is_empty() || samples.len() != metric.summary.n || samples.iter().any(|value| !value.is_finite())
            {
                bail!("incomplete or non-finite original samples for {}", metric.key);
            }
            if samples.len() > limits.max_samples_per_metric
                || samples.len() > limits.max_samples_per_run.saturating_sub(total)
            {
                bail!("benchmark original sample limit exceeded");
            }
            total += samples.len();
        }
    }
    Ok(())
}

pub(super) fn encode(samples: &[f64]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 8);
    for value in samples {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub(super) fn decode(bytes: &[u8], limits: Limits, total: &mut usize) -> Result<Vec<f64>> {
    let count = bytes.len() / 8;
    if !bytes.len().is_multiple_of(8)
        || count == 0
        || count > limits.max_samples_per_metric
        || count > limits.max_samples_per_run.saturating_sub(*total)
    {
        bail!("invalid or oversized original sample blob");
    }
    let values = bytes
        .chunks_exact(8)
        .map(|chunk| f64::from_le_bytes(chunk.try_into().expect("exact f64 width")))
        .collect::<Vec<_>>();
    if values.iter().any(|value| !value.is_finite()) {
        bail!("non-finite original sample blob");
    }
    *total += count;
    Ok(values)
}

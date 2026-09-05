//! Every derived number in a benchmark record, computed in one place.
//!
//! Six producers each computed their own percentiles, so `p99` did not mean
//! the same thing twice and only eight metrics across two of eleven categories
//! were machine-addressable. Collectors now emit raw samples and nothing else;
//! this module turns them into a `Summary`. A dimension is added by printing
//! numbers, not by reimplementing statistics.
//!
//! `cv` and `mad` are here for one reason: a ratio alone cannot tell a
//! regression from a noisy machine. `gateway /vms/list CPU=0.160s > 0.140s`
//! held the 0.6.0 release for two hours and then did not reproduce. A
//! comparison that knows the spread of its own evidence can say so.

use serde::{Deserialize, Serialize};

use crate::{schema::Unit, Thresholds};

/// The distribution of one metric, as recorded.
///
/// Serialized into `capsem.bench.v1`, so field names are the wire format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub n: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub p999: f64,
    pub stddev: f64,
    /// Relative spread: `stddev / mean`. The noise band a comparison judges
    /// against, and the reason a 14% move on a quiet metric is a regression
    /// while the same move on a jittery one is not.
    pub cv: f64,
    /// Median absolute deviation. Survives the outlier that drags `stddev`.
    pub mad: f64,
}

/// Recorded statistics are rounded to hundredths.
///
/// This is CPU time and milliseconds, not astronomy. Full f64 precision is
/// noise, and it is unstable noise: `serde_json` writes a full-precision value
/// exactly but parses it back up to one ULP off, so reading a record and
/// writing it again changed the bytes -- and records are digested and attached
/// to releases as evidence.
const DECIMALS: i32 = 2;

fn round_recorded(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let factor = 10f64.powi(DECIMALS);
    (value * factor).round() / factor
}

/// Linear-interpolated percentile over an already-sorted slice.
///
/// Nearest-rank jumps between samples, which makes p999 of 384 samples equal
/// to the maximum and hides the tail it exists to describe.
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    debug_assert!(!sorted.is_empty());
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = fraction * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return sorted[lower];
    }
    let weight = rank - lower as f64;
    sorted[lower] + (sorted[upper] - sorted[lower]) * weight
}

impl Summary {
    /// Summarize raw samples.
    ///
    /// Returns `None` for an empty slice: a metric with no samples is a broken
    /// collector, and inventing a zero would let it pass a ratchet forever.
    pub fn of(samples: &[f64]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<f64> = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("benchmark samples are never NaN"));

        let n = sorted.len();
        let mean = sorted.iter().sum::<f64>() / n as f64;
        let median = percentile(&sorted, 0.5);

        // Population, not sample: these are every observation taken, not a
        // draw from a larger set, so there is no Bessel correction to make.
        let variance = sorted.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / n as f64;
        let stddev = variance.sqrt();

        let mut deviations: Vec<f64> = sorted.iter().map(|value| (value - median).abs()).collect();
        deviations.sort_by(|a, b| a.partial_cmp(b).expect("deviations are never NaN"));

        Some(Self {
            n,
            min: round_recorded(sorted[0]),
            max: round_recorded(sorted[n - 1]),
            mean: round_recorded(mean),
            median: round_recorded(median),
            p90: round_recorded(percentile(&sorted, 0.90)),
            p95: round_recorded(percentile(&sorted, 0.95)),
            p99: round_recorded(percentile(&sorted, 0.99)),
            p999: round_recorded(percentile(&sorted, 0.999)),
            stddev: round_recorded(stddev),
            // A zero mean makes the ratio meaningless rather than infinite.
            cv: round_recorded(if mean == 0.0 { 0.0 } else { stddev / mean }),
            mad: round_recorded(percentile(&deviations, 0.5)),
        })
    }

    /// The value a budget is judged against.
    pub fn at(&self, statistic: Statistic) -> f64 {
        match statistic {
            Statistic::Min => self.min,
            Statistic::Max => self.max,
            Statistic::Mean => self.mean,
            Statistic::Median => self.median,
            Statistic::P90 => self.p90,
            Statistic::P95 => self.p95,
            Statistic::P99 => self.p99,
            Statistic::P999 => self.p999,
        }
    }
}

/// Which number of a distribution a budget names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Statistic {
    Min,
    Max,
    Mean,
    Median,
    P90,
    P95,
    P99,
    P999,
}

/// One metric, this run against the evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub key: String,
    pub statistic: Statistic,
    pub baseline: f64,
    pub current: f64,
    pub delta_abs: f64,
    pub delta_pct: f64,
    pub ratio: f64,
    /// Grew past the allowed ratio.
    pub regressed: bool,
    /// Moved by at least the smallest meaningful resolution in this unit.
    pub material: bool,
    /// Grew past the allowed ratio *and* past the noise the evidence itself
    /// shows, by a materially large amount. Only this should fail a release.
    pub significant: bool,
}

/// Compare one metric against its evidence.
///
/// `thresholds.maximum_factor` is the config-owned relative ceiling.
/// `thresholds.noise_factor` scales the evidence's own coefficient of
/// variation into the band a move must clear before it counts: a metric whose
/// baseline wobbles by 8% cannot report an 8% move as a discovery.
pub fn compare(
    key: &str,
    statistic: Statistic,
    baseline: &Summary,
    current: &Summary,
    unit: Unit,
    thresholds: Thresholds,
) -> Comparison {
    let before = baseline.at(statistic);
    let after = current.at(statistic);
    let delta_abs = after - before;

    // A zero baseline has no ratio. Any growth away from it is a change, but
    // not one this scale can quantify, so it is reported and never silently
    // treated as 1.0.
    let ratio = if before == 0.0 {
        if after == 0.0 {
            1.0
        } else {
            f64::INFINITY
        }
    } else {
        after / before
    };
    let delta_pct = if before == 0.0 { 0.0 } else { delta_abs / before * 100.0 };

    let regressed = ratio > thresholds.maximum_factor;
    let noise_band = baseline.cv * thresholds.noise_factor;
    let material = delta_abs >= minimum_delta(unit, thresholds.minimum_time_resolution_ms);
    let significant = regressed && material && (ratio - 1.0) > noise_band;

    Comparison {
        key: key.to_string(),
        statistic,
        baseline: before,
        current: after,
        delta_abs,
        delta_pct,
        ratio,
        regressed,
        material,
        significant,
    }
}

/// Express one time floor in the native unit of each metric.
///
/// A 300 ns operation becoming 600 ns is a dramatic percentage attached to
/// no product-visible duration. Non-time metrics retain ratio-only judgment.
fn minimum_delta(unit: Unit, minimum_time_resolution_ms: f64) -> f64 {
    match unit {
        Unit::Seconds => minimum_time_resolution_ms / 1_000.0,
        Unit::Milliseconds => minimum_time_resolution_ms,
        Unit::Nanoseconds => minimum_time_resolution_ms * 1_000_000.0,
        Unit::Bytes
        | Unit::Megabytes
        | Unit::RequestsPerSecond
        | Unit::MegabitsPerSecond
        | Unit::Operations
        | Unit::Ratio
        | Unit::Count => 0.0,
    }
}

#[cfg(test)]
mod tests;

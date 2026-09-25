//! The one owner of every `metrics`-facade metric Capsem records.
//!
//! Each domain module declares its metric names as `pub const` strings and a
//! `SPECS` table giving each one its kind, unit and description. Emitting
//! crates import the name from here and never spell it themselves, so a name
//! exists in exactly one place and every recorded metric has metadata an
//! exporter can publish.
//!
//! Names are `domain.snake_case` (a domain may nest, as in `virtio.blk.*`).
//! They are a recorded contract: dashboards and benchmark baselines key on
//! them, so renaming one is a breaking change, not a refactor.
//!
//! No recorder is installed by default, so every `counter!` / `gauge!` /
//! `histogram!` resolves to the facade's no-op recorder. The `export` feature
//! adds OTLP/HTTP export (`export.rs`) for the processes that own it.

use metrics::{KeyName, SharedString, Unit};

pub mod db;
pub mod dns;
#[cfg(feature = "export")]
pub mod export;
pub mod mitm;
pub mod security;
pub mod session;
pub mod virtio_blk;

/// Which facade macro records a metric.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

/// Name, shape and human-readable metadata of one metric.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricSpec {
    pub name: &'static str,
    pub kind: MetricKind,
    /// `None` for a dimensionless value such as a 0..1 ratio.
    pub unit: Option<Unit>,
    pub description: &'static str,
}

impl MetricSpec {
    pub const fn counter(name: &'static str, unit: Unit, description: &'static str) -> Self {
        Self::new(name, MetricKind::Counter, Some(unit), description)
    }

    pub const fn gauge(name: &'static str, unit: Unit, description: &'static str) -> Self {
        Self::new(name, MetricKind::Gauge, Some(unit), description)
    }

    pub const fn histogram(name: &'static str, unit: Unit, description: &'static str) -> Self {
        Self::new(name, MetricKind::Histogram, Some(unit), description)
    }

    pub const fn new(name: &'static str, kind: MetricKind, unit: Option<Unit>, description: &'static str) -> Self {
        Self {
            name,
            kind,
            unit,
            description,
        }
    }
}

/// Every domain's table, keyed by the module that owns it.
pub const DOMAINS: &[(&str, &[MetricSpec])] = &[
    ("db", db::SPECS),
    ("dns", dns::SPECS),
    ("mitm", mitm::SPECS),
    ("security", security::SPECS),
    ("session", session::SPECS),
    ("virtio_blk", virtio_blk::SPECS),
];

/// Every metric Capsem records, across all domains.
pub fn all() -> impl Iterator<Item = &'static MetricSpec> {
    DOMAINS.iter().flat_map(|(_, specs)| specs.iter())
}

/// Attach unit and description metadata for `specs` to the installed
/// recorder. Idempotent, and a no-op without a recorder.
pub fn describe(specs: &[MetricSpec]) {
    metrics::with_recorder(|recorder| {
        for spec in specs {
            let key = KeyName::from_const_str(spec.name);
            let description = SharedString::const_str(spec.description);
            match spec.kind {
                MetricKind::Counter => recorder.describe_counter(key, spec.unit, description),
                MetricKind::Gauge => recorder.describe_gauge(key, spec.unit, description),
                MetricKind::Histogram => recorder.describe_histogram(key, spec.unit, description),
            }
        }
    });
}

/// [`describe`] every domain.
pub fn describe_all() {
    for (_, specs) in DOMAINS {
        describe(specs);
    }
}

#[cfg(test)]
mod tests;

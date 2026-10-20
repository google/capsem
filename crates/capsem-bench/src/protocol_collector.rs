//! Raw protocol samples for the shared benchmark collector boundary.

use crate::scenarios::Artifact;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Serialize)]
pub(crate) struct Document {
    metrics: BTreeMap<String, Metric>,
    sidecar: String,
}

#[derive(Debug, Serialize)]
struct Metric {
    unit: &'static str,
    samples: Vec<f64>,
}

pub(crate) fn build(artifact: &Artifact) -> Result<Document> {
    let report = &artifact.mock_server_protocol;
    let mut metrics = BTreeMap::new();
    for scenario in &report.scenarios {
        let prefix = format!("{}.{}", report.lane, scenario.name);
        metrics.insert(
            format!("{prefix}.latency_ms"),
            Metric {
                unit: "milliseconds",
                samples: scenario.latency_samples.clone(),
            },
        );
        metrics.insert(
            format!("{prefix}.requests_per_sec"),
            Metric {
                unit: "requests_per_second",
                samples: vec![scenario.requests_per_sec],
            },
        );
        metrics.insert(
            format!("{prefix}.bytes_per_sec"),
            Metric {
                unit: "bytes",
                samples: vec![scenario.bytes_per_sec],
            },
        );
        metrics.insert(
            format!("{prefix}.failed"),
            Metric {
                unit: "count",
                samples: vec![scenario.failed as f64],
            },
        );
    }
    Ok(Document {
        metrics,
        sidecar: serde_json::to_string(artifact)?,
    })
}

pub(crate) fn write(path: &Path, artifact: &Artifact) -> Result<()> {
    std::fs::write(path, serde_json::to_vec(&build(artifact)?)?)?;
    Ok(())
}

#[cfg(test)]
mod tests;

//! Conversion from one protocol artifact into the shared benchmark schema.

use crate::scenarios::Artifact;
use crate::{commands, machine, schema, stats};

pub(crate) fn build(
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
        push("latency_ms", schema::Unit::Milliseconds, &scenario.latency_samples);
        push(
            "requests_per_sec",
            schema::Unit::RequestsPerSecond,
            &[scenario.requests_per_sec],
        );
        push("bytes_per_sec", schema::Unit::Bytes, &[scenario.bytes_per_sec]);
        push("failed", schema::Unit::Count, &[scenario.failed as f64]);
    }

    schema::Record {
        schema: schema::SCHEMA.to_string(),
        dimension: schema::Dimension::Protocol,
        recorded_at: commands::rfc3339_now(),
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

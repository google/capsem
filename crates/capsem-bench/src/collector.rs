//! How a dimension reports what it measured.
//!
//! A collector is a subprocess that prints one JSON document of raw samples to
//! stdout and nothing else. It computes no statistics: `stats` does that, once,
//! so `p99` means the same thing in every dimension. Adding a dimension is
//! then a matter of printing numbers.
//!
//! Diagnostics go to stderr, which is passed through to the terminal. The
//! guest package already splits this way -- Rich on stderr, JSON on stdout --
//! and that is the convention every collector follows.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::schema::Unit;

/// One metric's raw observations, as a collector reports them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawMetric {
    pub unit: Unit,
    /// Every observation. Not a mean, not a percentile -- the samples.
    pub samples: Vec<f64>,
}

/// What a collector prints on stdout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collected {
    /// Metric key to its samples. Keys are dotted and stable; the dimension
    /// name is prepended by the runner, so a collector names only its own part.
    pub metrics: BTreeMap<String, RawMetric>,
    /// Bulk output that belongs beside a record rather than inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<String>,
}

/// Parse a collector's stdout.
///
/// Tolerates leading noise, because a subprocess that prints a warning before
/// its JSON is common and refusing it would push collectors toward swallowing
/// their own diagnostics. Anything after the document is refused: two
/// documents mean the collector ran twice and only one would be read.
pub fn parse(stdout: &str) -> Result<Collected> {
    let start = stdout
        .find('{')
        .context("collector printed no JSON document on stdout")?;
    let mut stream =
        serde_json::Deserializer::from_str(&stdout[start..]).into_iter::<Collected>();
    let collected = stream
        .next()
        .context("collector printed no JSON document on stdout")?
        .context("collector output is not a valid sample document")?;

    let rest = &stdout[start + stream.byte_offset()..];
    if !rest.trim().is_empty() {
        bail!("collector printed more than one document; only the first would be read");
    }
    if collected.metrics.is_empty() {
        bail!("collector reported no metrics");
    }
    for (key, metric) in &collected.metrics {
        if metric.samples.is_empty() {
            bail!("collector reported metric {key} with no samples");
        }
        if let Some(bad) = metric.samples.iter().find(|value| !value.is_finite()) {
            bail!("collector reported a non-finite sample for {key}: {bad}");
        }
    }
    Ok(collected)
}

/// Run a collector and read what it measured.
///
/// Bounded: a collector that hangs would otherwise hold the machine lock the
/// whole gate runs under.
pub fn run(program: &Path, args: &[String], timeout: Duration) -> Result<Collected> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("cannot start collector {}", program.display()))?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(status) => {
                let output = child.wait_with_output()?;
                if !status.success() {
                    bail!(
                        "collector {} exited with {}",
                        program.display(),
                        status.code().unwrap_or(-1)
                    );
                }
                return parse(&String::from_utf8_lossy(&output.stdout))
                    .with_context(|| format!("collector {}", program.display()));
            }
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "collector {} did not finish within {}s",
                    program.display(),
                    timeout.as_secs()
                );
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

#[cfg(test)]
mod tests;

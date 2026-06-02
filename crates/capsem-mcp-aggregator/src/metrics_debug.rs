use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use tracing::{info, warn};

const ENV_INTERVAL_SECS: &str = "CAPSEM_METRICS_DEBUG_INTERVAL_SECS";
const LOG_TARGET: &str = "capsem_mcp_aggregator::metrics_debug";
const MCP_METRIC_NAMES: &[&str] = &[capsem_core::mcp::aggregator::MCP_AGGREGATOR_STAGE_MS];

pub(crate) struct MetricsDebugGuard {
    stop_tx: Option<Sender<()>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MetricsDebugGuard {
    pub(crate) fn maybe_start() -> Option<Self> {
        let interval = debug_interval()?;
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        if recorder.install().is_err() {
            warn!(
                target: LOG_TARGET,
                "metrics debug recorder requested but a global metrics recorder is already installed"
            );
            return None;
        }

        let (stop_tx, stop_rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("capsem-aggregator-metrics-debug".into())
            .spawn(move || metrics_debug_loop(snapshotter, interval, stop_rx))
            .ok()?;
        info!(
            target: LOG_TARGET,
            interval_secs = interval.as_secs_f64(),
            "metrics_debug_recorder_started"
        );
        Some(Self {
            stop_tx: Some(stop_tx),
            handle: Some(handle),
        })
    }
}

impl Drop for MetricsDebugGuard {
    fn drop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn debug_interval() -> Option<Duration> {
    std::env::var(ENV_INTERVAL_SECS)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|secs| *secs > 0.0)
        .map(Duration::from_secs_f64)
}

fn metrics_debug_loop(snapshotter: Snapshotter, interval: Duration, stop_rx: mpsc::Receiver<()>) {
    loop {
        match stop_rx.recv_timeout(interval) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                emit_mcp_metric_snapshot(&snapshotter);
                break;
            }
            Err(RecvTimeoutError::Timeout) => emit_mcp_metric_snapshot(&snapshotter),
        }
    }
}

fn emit_mcp_metric_snapshot(snapshotter: &Snapshotter) {
    for (key, _, _, value) in snapshotter.snapshot().into_vec() {
        let name = key.key().name();
        if !MCP_METRIC_NAMES.contains(&name) {
            continue;
        }
        let DebugValue::Histogram(values) = value else {
            continue;
        };
        let Some(summary) = summarize_histogram(values.into_iter().map(|value| value.into_inner()))
        else {
            continue;
        };
        info!(
            target: LOG_TARGET,
            metric = name,
            stage = label_value(&key, "stage").unwrap_or("none"),
            method_kind = label_value(&key, "method_kind").unwrap_or("unknown"),
            tool_kind = label_value(&key, "tool_kind").unwrap_or("unknown"),
            result = label_value(&key, "result").unwrap_or("unknown"),
            count = summary.count,
            avg_ms = summary.avg_ms,
            p50_ms = summary.p50_ms,
            p95_ms = summary.p95_ms,
            p99_ms = summary.p99_ms,
            max_ms = summary.max_ms,
            "mcp_metric_snapshot"
        );
    }
}

fn label_value<'a>(key: &'a metrics_util::CompositeKey, name: &str) -> Option<&'a str> {
    key.key()
        .labels()
        .find(|label| label.key() == name)
        .map(|label| label.value())
}

#[derive(Debug, PartialEq)]
struct HistogramSummary {
    count: usize,
    avg_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
}

fn summarize_histogram<I>(values: I) -> Option<HistogramSummary>
where
    I: IntoIterator<Item = f64>,
{
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let count = values.len();
    let sum: f64 = values.iter().sum();
    Some(HistogramSummary {
        count,
        avg_ms: sum / count as f64,
        p50_ms: percentile(&values, 50.0),
        p95_ms: percentile(&values, 95.0),
        p99_ms: percentile(&values, 99.0),
        max_ms: *values.last().expect("non-empty histogram values"),
    })
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (pct / 100.0) * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let weight = rank - lo as f64;
        sorted[lo] * (1.0 - weight) + sorted[hi] * weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_histogram_reports_percentiles() {
        let summary = summarize_histogram([1.0, 10.0, 2.0, 3.0]).unwrap();

        assert_eq!(summary.count, 4);
        assert_close(summary.avg_ms, 4.0);
        assert_close(summary.p50_ms, 2.5);
        assert_close(summary.p95_ms, 8.95);
        assert_close(summary.p99_ms, 9.79);
        assert_close(summary.max_ms, 10.0);
    }

    #[test]
    fn summarize_empty_histogram_returns_none() {
        assert!(summarize_histogram([]).is_none());
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.000_001,
            "expected {actual} to be close to {expected}"
        );
    }
}

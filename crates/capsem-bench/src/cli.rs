//! Command-line contract for the host harness and its smaller guest build.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version = env!("CARGO_PKG_VERSION"), about = "Capsem benchmark harness")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Run deterministic protocol scenarios against capsem-mock-server.
    Protocol(ProtocolArgs),
    /// Run host-direct and guest-through-Capsem protocol lanes, then report delta.
    ProtocolDelta(ProtocolDeltaArgs),
    /// Compare host-direct and guest-through-Capsem artifacts.
    Delta(DeltaArgs),
    /// Every dimension this binary can measure, and whether a quick run covers it.
    #[cfg(feature = "host")]
    List,
    /// Report whether this machine is fit to measure on.
    #[cfg(feature = "host")]
    Doctor(DoctorArgs),
    /// Compare two records metric by metric.
    #[cfg(feature = "host")]
    Compare(CompareArgs),
    /// Ratchet a directory of records against checked-in evidence.
    #[cfg(feature = "host")]
    Verify(VerifyArgs),
    /// Measure dimensions and record what they measured.
    #[cfg(feature = "host")]
    Run(RunArgs),
    /// What every measured subject reads, and how it has moved.
    #[cfg(feature = "host")]
    Report(ReportArgs),
}

#[cfg(feature = "host")]
#[derive(Parser, Debug)]
pub(crate) struct ReportArgs {
    /// The benchmark store to read.
    #[arg(long, default_value = "cache/target/test-benchmarks/benchmarks.db")]
    pub(crate) store: PathBuf,
    #[arg(long, default_value = "code")]
    pub(crate) profile: String,
}

#[cfg(feature = "host")]
#[derive(Parser, Debug)]
pub(crate) struct RunArgs {
    /// Dimensions to measure. Every one when omitted.
    pub(crate) dimensions: Vec<String>,
    /// Directory holding one executable per dimension.
    #[arg(long, default_value = "benchmarks/collectors")]
    pub(crate) collectors: PathBuf,
    /// The benchmark store to record into.
    #[arg(long, default_value = "cache/target/test-benchmarks/benchmarks.db")]
    pub(crate) out: PathBuf,
    /// Reduced samples, skipping everything that boots a guest.
    #[arg(long)]
    pub(crate) quick: bool,
    /// Seconds a single collector may take.
    #[arg(long, default_value_t = 900)]
    pub(crate) timeout_secs: u64,
    /// Run each collector through this interpreter rather than executing it directly.
    #[arg(long)]
    pub(crate) interpreter: Option<String>,
    #[arg(long, default_value = "unknown")]
    pub(crate) channel: String,
    #[arg(long, default_value = "unknown")]
    pub(crate) commit: String,
    #[arg(long, default_value = "code")]
    pub(crate) profile: String,
}

/// How much growth is allowed, and how much of a move is just the machine.
#[cfg(feature = "host")]
#[derive(Parser, Debug, Clone, Copy)]
pub(crate) struct Thresholds {
    #[arg(long, default_value_t = 1.1)]
    pub(crate) maximum_factor: f64,
    #[arg(long, default_value_t = 1.0)]
    pub(crate) noise_factor: f64,
    #[arg(long, default_value_t = 1.0)]
    pub(crate) minimum_time_resolution_ms: f64,
}

#[cfg(feature = "host")]
#[derive(Parser, Debug)]
pub(crate) struct CompareArgs {
    pub(crate) baseline: PathBuf,
    pub(crate) current: PathBuf,
    pub(crate) dimension: String,
    #[arg(long, default_value = "code")]
    pub(crate) profile: String,
    #[command(flatten)]
    pub(crate) thresholds: Thresholds,
}

#[cfg(feature = "host")]
#[derive(Parser, Debug)]
pub(crate) struct VerifyArgs {
    #[arg(long, default_value = "cache/target/test-benchmarks/benchmarks.db")]
    pub(crate) records: PathBuf,
    #[arg(long)]
    pub(crate) evidence: PathBuf,
    #[command(flatten)]
    pub(crate) thresholds: Thresholds,
}

#[cfg(feature = "host")]
#[derive(Parser, Debug)]
pub(crate) struct DoctorArgs {
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Parser, Debug)]
pub(crate) struct ProtocolArgs {
    #[arg(long)]
    pub(crate) base_url: Option<String>,
    #[arg(long)]
    pub(crate) dns_udp_addr: Option<String>,
    #[arg(long, default_value_t = 50_000)]
    pub(crate) requests: usize,
    #[arg(long, default_value_t = 64)]
    pub(crate) concurrency: usize,
    #[arg(long, default_value_t = 30_000)]
    pub(crate) timeout_ms: u64,
    #[arg(long)]
    pub(crate) scenarios: Option<String>,
    #[arg(long, default_value = "host_direct")]
    pub(crate) lane: String,
    #[arg(long, default_value = "/tmp/capsem-benchmark.json")]
    pub(crate) json_out: PathBuf,
    /// Also record into the host benchmark store.
    #[cfg(feature = "host")]
    #[arg(long)]
    pub(crate) record: Option<PathBuf>,
    #[cfg(feature = "host")]
    #[arg(long, default_value = "unknown")]
    pub(crate) channel: String,
    #[cfg(feature = "host")]
    #[arg(long, default_value = "unknown")]
    pub(crate) commit: String,
    #[cfg(feature = "host")]
    #[arg(long, default_value = "code")]
    pub(crate) profile: String,
}

#[derive(Parser, Debug)]
pub(crate) struct DeltaArgs {
    #[arg(long)]
    pub(crate) host: PathBuf,
    #[arg(long)]
    pub(crate) guest: PathBuf,
    #[arg(long, default_value = "/tmp/capsem-benchmark-delta.json")]
    pub(crate) json_out: PathBuf,
}

#[derive(Parser, Debug)]
pub(crate) struct ProtocolDeltaArgs {
    #[arg(long)]
    pub(crate) base_url: String,
    #[arg(long)]
    pub(crate) dns_udp_addr: Option<String>,
    #[arg(long)]
    pub(crate) guest_base_url: Option<String>,
    #[arg(long)]
    pub(crate) guest_dns_udp_addr: Option<String>,
    #[arg(long, default_value_t = 50_000)]
    pub(crate) requests: usize,
    #[arg(long, default_value_t = 64)]
    pub(crate) concurrency: usize,
    #[arg(long, default_value_t = 300)]
    pub(crate) guest_timeout_secs: u64,
    #[arg(long, default_value_t = 30_000)]
    pub(crate) timeout_ms: u64,
    #[arg(long)]
    pub(crate) scenarios: Option<String>,
    #[arg(long)]
    pub(crate) session: Option<String>,
    #[arg(long, default_value = "capsem")]
    pub(crate) capsem_bin: PathBuf,
    #[arg(long, default_value = "/tmp/capsem-benchmark-protocol-delta.json")]
    pub(crate) json_out: PathBuf,
}

mod collector;
mod commands;
mod machine;
mod schema;
mod stats;
mod store;
mod protocol;
mod scenarios;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

const VERSION: &str = "0.4.0-rust";
const SECRET_SHAPED_MARKER: &str = "capsem_test_";
const HTTP_REQUEST_ATTEMPTS: usize = 5;
const HTTP_RETRY_BACKOFF_BASE_MS: u64 = 2;

#[derive(Parser, Debug)]
#[command(version = env!("CARGO_PKG_VERSION"), about = "Capsem benchmark harness")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run deterministic protocol scenarios against capsem-mock-server.
    Protocol(ProtocolArgs),
    /// Run host-direct and guest-through-Capsem protocol lanes, then report delta.
    ProtocolDelta(ProtocolDeltaArgs),
    /// Compare host-direct and guest-through-Capsem artifacts.
    Delta(DeltaArgs),
    /// Every dimension this binary can measure, and whether a quick run covers it.
    List,
    /// Report whether this machine is fit to measure on.
    Doctor(DoctorArgs),
    /// Compare two records metric by metric.
    Compare(CompareArgs),
    /// Ratchet a directory of records against checked-in evidence.
    Verify(VerifyArgs),
    /// Measure dimensions and record what they measured.
    Run(RunArgs),
    /// What every measured subject reads, and how it has moved.
    Report(ReportArgs),
}

#[derive(Parser, Debug)]
struct ReportArgs {
    /// The benchmark store to read.
    #[arg(long, default_value = "cache/target/tests/benchmarks/benchmarks.db")]
    store: PathBuf,
    #[arg(long, default_value = "code")]
    profile: String,
}

#[derive(Parser, Debug)]
struct RunArgs {
    /// Dimensions to measure. Every one when omitted.
    dimensions: Vec<String>,
    /// Directory holding one executable per dimension.
    #[arg(long, default_value = "benchmarks/collectors")]
    collectors: PathBuf,
    /// The benchmark store to record into.
    #[arg(long, default_value = "cache/target/tests/benchmarks/benchmarks.db")]
    out: PathBuf,
    /// Reduced samples, skipping everything that boots a guest.
    #[arg(long)]
    quick: bool,
    /// Seconds a single collector may take.
    #[arg(long, default_value_t = 900)]
    timeout_secs: u64,
    /// Run each collector through this interpreter rather than executing it
    /// directly. Collectors that import the project's Python dependencies
    /// need its environment, not whatever `#!/usr/bin/env python3` resolves to.
    #[arg(long)]
    interpreter: Option<String>,
    #[arg(long, default_value = "unknown")]
    channel: String,
    #[arg(long, default_value = "unknown")]
    commit: String,
    #[arg(long, default_value = "code")]
    profile: String,
}

/// How much growth is allowed, and how much of a move is just the machine.
///
/// Defaults mirror `[benchmark_regression] maximum_factor` in
/// `config/gate.toml`; they become flags rather than constants so the gate can
/// pass the config-owned value in until this binary reads that file directly.
#[derive(Parser, Debug, Clone, Copy)]
pub(crate) struct Thresholds {
    /// A metric may grow by this ratio before it counts as a regression.
    #[arg(long, default_value_t = 1.2)]
    pub(crate) maximum_factor: f64,
    /// Multiplier on the evidence's own spread. A move inside its baseline's
    /// noise is reported but never called significant.
    #[arg(long, default_value_t = 1.0)]
    pub(crate) noise_factor: f64,
}

#[derive(Parser, Debug)]
struct CompareArgs {
    /// The store holding the evidence.
    baseline: PathBuf,
    /// The store holding this run.
    current: PathBuf,
    /// Which dimension to compare.
    dimension: String,
    #[arg(long, default_value = "code")]
    profile: String,
    #[command(flatten)]
    thresholds: Thresholds,
}

#[derive(Parser, Debug)]
struct VerifyArgs {
    /// The store holding this run.
    #[arg(long, default_value = "cache/target/tests/benchmarks/benchmarks.db")]
    records: PathBuf,
    /// The store holding checked-in evidence.
    #[arg(long)]
    evidence: PathBuf,
    #[command(flatten)]
    thresholds: Thresholds,
}

#[derive(Parser, Debug)]
struct DoctorArgs {
    /// Emit the verdict as JSON rather than prose.
    #[arg(long)]
    json: bool,
}

#[derive(Parser, Debug)]
struct ProtocolArgs {
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    dns_udp_addr: Option<String>,
    #[arg(long, default_value_t = 50_000)]
    requests: usize,
    #[arg(long, default_value_t = 64)]
    concurrency: usize,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    #[arg(long)]
    scenarios: Option<String>,
    #[arg(long, default_value = "host_direct")]
    lane: String,
    #[arg(long, default_value = "/tmp/capsem-benchmark.json")]
    json_out: PathBuf,
    /// Also record into this benchmark store.
    #[arg(long)]
    record: Option<PathBuf>,
    #[arg(long, default_value = "unknown")]
    channel: String,
    #[arg(long, default_value = "unknown")]
    commit: String,
    #[arg(long, default_value = "code")]
    profile: String,
}

#[derive(Parser, Debug)]
struct DeltaArgs {
    #[arg(long)]
    host: PathBuf,
    #[arg(long)]
    guest: PathBuf,
    #[arg(long, default_value = "/tmp/capsem-benchmark-delta.json")]
    json_out: PathBuf,
}

#[derive(Parser, Debug)]
struct ProtocolDeltaArgs {
    #[arg(long)]
    base_url: String,
    #[arg(long)]
    dns_udp_addr: Option<String>,
    #[arg(long)]
    guest_base_url: Option<String>,
    #[arg(long)]
    guest_dns_udp_addr: Option<String>,
    #[arg(long, default_value_t = 50_000)]
    requests: usize,
    #[arg(long, default_value_t = 64)]
    concurrency: usize,
    #[arg(long, default_value_t = 300)]
    guest_timeout_secs: u64,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    #[arg(long)]
    scenarios: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long, default_value = "capsem")]
    capsem_bin: PathBuf,
    #[arg(long, default_value = "/tmp/capsem-benchmark-protocol-delta.json")]
    json_out: PathBuf,
}


use protocol::*;
use scenarios::*;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Protocol(ProtocolArgs {
        base_url: None,
        dns_udp_addr: None,
        requests: 50_000,
        concurrency: 64,
        timeout_ms: 30_000,
        scenarios: None,
        lane: "host_direct".to_string(),
        json_out: PathBuf::from("/tmp/capsem-benchmark.json"),
        record: None,
        channel: "unknown".to_string(),
        commit: "unknown".to_string(),
        profile: "code".to_string(),
    })) {
        Command::Protocol(args) => {
            let destination = args.record.clone();
            let (channel, commit, profile) =
                (args.channel.clone(), args.commit.clone(), args.profile.clone());
            let artifact = run_protocol(args).await?;
            if let Some(root) = destination {
                let record = commands::protocol_record(
                    &artifact,
                    &channel,
                    &commit,
                    &profile,
                    machine::running_capsem_processes(),
                );
                let mut connection = store::open(&root)?;
                let run_id = store::insert(&mut connection, &record)?;
                eprintln!(
                    "recorded {} metrics as run {run_id} in {}",
                    record.metrics.len(),
                    root.display()
                );
            }
            println!("{}", serde_json::to_string_pretty(&artifact)?);
        }
        Command::ProtocolDelta(args) => {
            let artifact = run_protocol_delta(args).await?;
            println!("{}", serde_json::to_string_pretty(&artifact)?);
        }
        Command::Delta(args) => {
            let artifact = run_delta(args)?;
            println!("{}", serde_json::to_string_pretty(&artifact)?);
        }
        Command::Report(args) => {
            return commands::report(&args.store, std::env::consts::ARCH, &args.profile)
        }
        Command::List => commands::list_dimensions(),
        Command::Doctor(args) => {
            return commands::doctor(args.json, machine::running_capsem_processes())
        }
        Command::Compare(args) => {
            let dimension = commands::select_dimensions(std::slice::from_ref(&args.dimension))?[0];
            return commands::compare(
                &args.baseline,
                &args.current,
                dimension,
                std::env::consts::ARCH,
                &args.profile,
                args.thresholds,
            );
        }
        Command::Verify(args) => return commands::verify(&args.records, &args.evidence, args.thresholds),
        Command::Run(args) => {
            let wanted = commands::select_dimensions(&args.dimensions)?;
            return commands::run_dimensions(
                &wanted,
                &args.collectors,
                &args.out,
                std::time::Duration::from_secs(args.timeout_secs),
                args.interpreter.as_deref(),
                args.quick,
                &args.channel,
                &args.commit,
                &args.profile,
                machine::running_capsem_processes(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

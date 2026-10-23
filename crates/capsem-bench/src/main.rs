mod cli;
#[cfg(feature = "host")]
mod collector;
#[cfg(feature = "host")]
mod commands;
#[cfg(feature = "host")]
mod comparison;
mod dns_load;
#[cfg(feature = "host")]
mod evidence;
mod http_load;
#[cfg(feature = "host")]
mod machine;
mod ping;
mod protocol;
mod protocol_collector;
#[cfg(feature = "host")]
mod protocol_record;
#[path = "stats/rates.rs"]
mod rates;
mod redis;
mod runtime_limits;
#[cfg(feature = "host")]
mod sample_limits;
mod scenarios;
#[cfg(feature = "host")]
mod schema;
#[cfg(feature = "host")]
mod stats;
#[cfg(feature = "host")]
mod store;
mod throughput;
mod udp;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

pub(crate) use cli::*;

const VERSION: &str = "0.4.0-rust";
const SECRET_SHAPED_MARKER: &str = "capsem_test_";
const HTTP_REQUEST_ATTEMPTS: usize = 5;
const HTTP_RETRY_BACKOFF_BASE_MS: u64 = 2;

use protocol::*;
#[cfg(test)]
use scenarios::*;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Protocol(ProtocolArgs {
        base_url: None,
        query: None,
        dns_udp_addr: None,
        requests: 50_000,
        concurrency: 64,
        timeout_ms: 30_000,
        scenarios: None,
        lane: "host_direct".to_string(),
        json_out: PathBuf::from("/tmp/capsem-benchmark.json"),
        collector_out: None,
        #[cfg(feature = "host")]
        record: None,
        #[cfg(feature = "host")]
        channel: "unknown".to_string(),
        #[cfg(feature = "host")]
        commit: "unknown".to_string(),
    })) {
        Command::Redis(args) => println!("{}", serde_json::to_string(&redis::run(args).await?)?),
        Command::Throughput(args) => println!("{}", serde_json::to_string(&throughput::run(args).await?)?),
        Command::Udp(args) => println!("{}", serde_json::to_string(&udp::run(args).await?)?),
        Command::Ping(args) => println!("{}", serde_json::to_string(&ping::run(args).await?)?),
        Command::HttpLoad(args) => println!("{}", serde_json::to_string(&http_load::run(args).await?)?),
        Command::DnsLoad(args) => println!("{}", serde_json::to_string(&dns_load::run(args).await?)?),
        Command::RuntimeLimits => println!("{}", serde_json::to_string(&runtime_limits::report())?),
        Command::Protocol(args) => {
            let collector_out = args.collector_out.clone();
            #[cfg(feature = "host")]
            let destination = args.record.clone();
            #[cfg(feature = "host")]
            let (channel, commit) = (args.channel.clone(), args.commit.clone());
            let artifact = run_protocol(args).await?;
            if let Some(path) = collector_out {
                protocol_collector::write(&path, &artifact)?;
            }
            #[cfg(feature = "host")]
            if let Some(root) = destination {
                let record = protocol_record::build(&artifact, &channel, &commit, machine::running_capsem_processes()?);
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
        #[cfg(feature = "host")]
        Command::Report(args) => return commands::report(&args.store, std::env::consts::ARCH),
        #[cfg(feature = "host")]
        Command::List => commands::list_dimensions(),
        #[cfg(feature = "host")]
        Command::Doctor(args) => {
            return commands::doctor(args.json, args.standing, machine::running_capsem_processes()?)
        }
        #[cfg(feature = "host")]
        Command::Compare(args) => {
            let dimension = commands::select_dimensions(std::slice::from_ref(&args.dimension))?[0];
            return comparison::compare(
                &args.baseline,
                &args.current,
                dimension,
                std::env::consts::ARCH,
                args.runs,
                args.thresholds,
            );
        }
        #[cfg(feature = "host")]
        Command::Verify(args) => return comparison::verify(&args.records, &args.evidence, args.runs, args.thresholds),
        #[cfg(feature = "host")]
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
                &std::fs::read_to_string(&args.cohort_config)?,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

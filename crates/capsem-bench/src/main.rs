mod cli;
#[cfg(feature = "host")]
mod collector;
#[cfg(feature = "host")]
mod commands;
#[cfg(feature = "host")]
mod comparison;
#[cfg(feature = "host")]
mod machine;
mod protocol;
#[cfg(feature = "host")]
mod protocol_record;
mod scenarios;
#[cfg(feature = "host")]
mod schema;
#[cfg(feature = "host")]
mod stats;
#[cfg(feature = "host")]
mod store;

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
        dns_udp_addr: None,
        requests: 50_000,
        concurrency: 64,
        timeout_ms: 30_000,
        scenarios: None,
        lane: "host_direct".to_string(),
        json_out: PathBuf::from("/tmp/capsem-benchmark.json"),
        #[cfg(feature = "host")]
        record: None,
        #[cfg(feature = "host")]
        channel: "unknown".to_string(),
        #[cfg(feature = "host")]
        commit: "unknown".to_string(),
        #[cfg(feature = "host")]
        profile: "code".to_string(),
    })) {
        Command::Protocol(args) => {
            #[cfg(feature = "host")]
            let destination = args.record.clone();
            #[cfg(feature = "host")]
            let (channel, commit, profile) = (args.channel.clone(), args.commit.clone(), args.profile.clone());
            let artifact = run_protocol(args).await?;
            #[cfg(feature = "host")]
            if let Some(root) = destination {
                let record = protocol_record::build(
                    &artifact,
                    &channel,
                    &commit,
                    &profile,
                    machine::running_capsem_processes()?,
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
        #[cfg(feature = "host")]
        Command::Report(args) => return commands::report(&args.store, std::env::consts::ARCH, &args.profile),
        #[cfg(feature = "host")]
        Command::List => commands::list_dimensions(),
        #[cfg(feature = "host")]
        Command::Doctor(args) => return commands::doctor(args.json, machine::running_capsem_processes()?),
        #[cfg(feature = "host")]
        Command::Compare(args) => {
            let dimension = commands::select_dimensions(std::slice::from_ref(&args.dimension))?[0];
            return comparison::compare(
                &args.baseline,
                &args.current,
                dimension,
                std::env::consts::ARCH,
                &args.profile,
                args.thresholds,
            );
        }
        #[cfg(feature = "host")]
        Command::Verify(args) => return comparison::verify(&args.records, &args.evidence, args.thresholds),
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
                &args.profile,
                machine::running_capsem_processes()?,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

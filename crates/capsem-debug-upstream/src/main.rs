use std::net::SocketAddr;

use anyhow::Context;
use capsem_debug_upstream::{ready_payload, serve_debug_upstream};
use clap::Parser;
use tokio::net::TcpListener;

#[derive(Debug, Parser)]
#[command(about = "Run Capsem's deterministic local debug upstream")]
struct Args {
    /// Address to bind. Use port 0 for an ephemeral local port.
    #[arg(long, default_value = "127.0.0.1:0")]
    addr: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("CAPSEM_DEBUG_UPSTREAM_LOG")
                .unwrap_or_else(|_| "capsem_debug_upstream=info,warn".to_string()),
        )
        .with_writer(std::io::stderr)
        .init();

    let listener = TcpListener::bind(args.addr)
        .await
        .with_context(|| format!("bind debug upstream at {}", args.addr))?;
    let addr = listener.local_addr().context("read bound address")?;
    println!("{}", serde_json::to_string(&ready_payload(addr))?);

    serve_debug_upstream(listener, async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %err, "failed to wait for ctrl-c");
        }
    })
    .await
}

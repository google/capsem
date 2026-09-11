use capsem_foundation::unix::{fd, router_channel::Receiver, router_sandbox};
use capsem_router::{Event, Grant};
use clap::Parser;
use std::io;
use std::os::fd::AsFd;
use std::time::Duration;

#[derive(Parser)]
#[command(version, about = "Confined descriptor-pair network companion")]
struct Args {
    #[arg(long)]
    parent_pid: u32,
    #[arg(long, default_value_t = capsem_router::CONNECTIONS_PER_CLASS as u16)]
    expose_limit: u16,
    #[arg(long, default_value_t = capsem_router::CONNECTIONS_PER_CLASS as u16)]
    private_limit: u16,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: process entry before descriptor owners or threads exist.
    unsafe { router_sandbox::close_inherited_descriptors()? };
    let _telemetry = capsem_foundation::telemetry::init(capsem_foundation::telemetry::TelemetryConfig {
        service: "capsem-router",
        sink: capsem_foundation::telemetry::LogSink::Stderr,
        default_filter: "capsem_router=info",
    })?;
    let args = Args::parse();
    let limits = capsem_router::ConnectionLimits::new(args.expose_limit, args.private_limit)?;
    capsem_guard::watch_parent_or_exit(Some(args.parent_pid))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let stdin = std::io::stdin();
        let socket = std::os::unix::net::UnixStream::from(fd::duplicate(stdin.as_fd())?);
        socket.set_nonblocking(true)?;
        let grants = Receiver::new(socket.try_clone()?)?;
        let mut events = tokio::net::UnixStream::from_std(socket)?;
        if let Err(error) = router_sandbox::confine() {
            tracing::error!(%error, "router confinement failed");
            Event::ConfinementFailed.write(&mut events).await?;
            return Err(error);
        }
        tracing::info!("router confinement installed");
        let hello = tokio::time::timeout(Duration::from_secs(5), grants.recv()).await??;
        if !matches!(Grant::decode(hello)?, Grant::Hello) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "router requires versioned hello",
            ));
        }
        capsem_router::relay(grants, events, limits).await
    })?;
    Ok(())
}

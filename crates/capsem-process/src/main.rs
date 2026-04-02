use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result};
use clap::Parser;
use capsem_core::{
    boot_vm, create_net_state, SandboxInstance, VirtioFsShare,
};
use capsem_logger::DbWriter;
use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
use tokio::net::UnixListener;
use tokio_unix_ipc::{channel_from_std, Sender, Receiver};
use tracing::{info, error, debug};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Unique ID for this sandbox session
    #[arg(long)]
    id: String,

    /// Path to assets directory (vmlinuz, initrd.img)
    #[arg(long)]
    assets_dir: PathBuf,

    /// Path to rootfs image/squashfs
    #[arg(long)]
    rootfs: PathBuf,

    /// Path to session database directory
    #[arg(long)]
    session_dir: PathBuf,

    /// CPU count
    #[arg(long, default_value_t = 2)]
    cpus: u32,

    /// RAM in MB
    #[arg(long, default_value_t = 2048)]
    ram_mb: u64,

    /// Path to Unix Domain Socket for control/terminal from capsem-service
    #[arg(long)]
    uds_path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    info!(id = %args.id, "capsem-sandbox-process starting");

    let ram_bytes = args.ram_mb * 1024 * 1024;
    let cmdline = "console=hvc0 ro loglevel=1 init_on_alloc=1 slab_nomerge page_alloc.shuffle=1";

    // 1. Create session directory structure.
    std::fs::create_dir_all(&args.session_dir).context("failed to create session dir")?;
    capsem_core::create_virtiofs_session(&args.session_dir, 2).context("failed to create VirtioFS session")?;

    let virtiofs_shares = vec![VirtioFsShare {
        tag: "capsem".to_string(),
        host_path: args.session_dir.clone(),
        read_only: false,
    }];

    // 2. Open session DB.
    let db_path = args.session_dir.join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 1000).context("failed to open session db")?);

    // 3. Boot VM.
    let (vm, _vsock_rx, sm) = boot_vm(
        &args.assets_dir,
        Some(&args.rootfs),
        cmdline,
        None,
        &virtiofs_shares,
        args.cpus,
        ram_bytes,
    ).context("failed to boot VM")?;

    info!(id = %args.id, "VM booted successfully");

    let _rx = vm.serial().subscribe();
    let input_fd = vm.serial().input_fd();

    // 4. Create network state.
    let net_state = create_net_state(&args.id, Arc::clone(&db)).ok();

    // 5. Store instance state.
    let _instance = SandboxInstance {
        vm,
        serial_input_fd: input_fd,
        vsock_terminal_fd: None,
        vsock_control_fd: None,
        net_state,
        mcp_state: None,
        state_machine: sm,
        scratch_disk_path: None,
        fs_monitor: None,
    };

    // 6. Start UDS listener for control messages.
    if args.uds_path.exists() {
        std::fs::remove_file(&args.uds_path).ok();
    }
    let listener = UnixListener::bind(&args.uds_path).context("failed to bind UDS")?;
    
    info!(socket = %args.uds_path.display(), "listening for IPC");

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let id = args.id.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_ipc_connection(stream, id).await {
                        error!("IPC connection error: {e:#}");
                    }
                });
            }
            Err(e) => {
                error!("failed to accept IPC connection: {e}");
            }
        }
    }
}

async fn handle_ipc_connection(stream: tokio::net::UnixStream, id: String) -> Result<()> {
    debug!(id = %id, "new IPC connection");

    let std_stream = stream.into_std()?;
    std_stream.set_nonblocking(false)?;
    let (tx, rx): (Sender<ProcessToService>, Receiver<ServiceToProcess>) = channel_from_std(std_stream)?;

    loop {
        match rx.recv().await {
            Ok(msg) => {
                match msg {
                    ServiceToProcess::Ping => {
                        tx.send(ProcessToService::Pong).await?;
                    }
                    ServiceToProcess::TerminalInput { data } => {
                        debug!(bytes = data.len(), "received terminal input");
                    }
                    ServiceToProcess::TerminalResize { cols, rows } => {
                        debug!(cols, rows, "received terminal resize");
                    }
                    ServiceToProcess::Shutdown => {
                        info!("shutdown requested via IPC");
                        std::process::exit(0);
                    }
                }
            }
            Err(e) => {
                debug!(id = %id, error = %e, "IPC connection closed or errored");
                break;
            }
        }
    }

    Ok(())
}

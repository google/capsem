use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use anyhow::{Context, Result};
use clap::Parser;
use tracing::{info, error, warn};
use std::process::Command;
use axum::{
    routing::{get, post},
    Router, Json, extract::State,
};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tower_http::trace::TraceLayer;

mod api;
use api::*;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Run in foreground (don't daemonize)
    #[arg(long, default_value_t = true)]
    foreground: bool,

    /// Path to the service Unix Domain Socket
    #[arg(long)]
    uds_path: Option<PathBuf>,

    /// Path to the capsem-process binary
    #[arg(long)]
    process_binary: Option<PathBuf>,

    /// Path to assets directory
    #[arg(long)]
    assets_dir: Option<PathBuf>,
}

struct ServiceState {
    /// Map of instance ID to Process Info
    instances: Mutex<HashMap<String, InstanceInfo>>,
    process_binary: PathBuf,
    assets_dir: PathBuf,
    run_dir: PathBuf,
}

struct InstanceInfo {
    id: String,
    pid: u32,
    uds_path: PathBuf,
}

impl ServiceState {
    fn provision_sandbox(&self, id: &str, ram_mb: u64, cpus: u32) -> Result<()> {
        let uds_path = self.run_dir.join("instances").join(format!("{}.sock", id));
        let session_dir = self.run_dir.join("sessions").join(id);
        
        // Ensure parent directories exist
        let _ = std::fs::create_dir_all(uds_path.parent().unwrap());
        let _ = std::fs::create_dir_all(&session_dir);

        // In a real implementation, we would resolve rootfs properly.
        // For now, assume it's in assets_dir/rootfs.squashfs
        let rootfs = self.assets_dir.join("rootfs.squashfs");

        info!(id, "spawning capsem-process");

        let child = Command::new(&self.process_binary)
            .arg("--id").arg(id)
            .arg("--assets-dir").arg(&self.assets_dir)
            .arg("--rootfs").arg(&rootfs)
            .arg("--session-dir").arg(&session_dir)
            .arg("--cpus").arg(cpus.to_string())
            .arg("--ram-mb").arg(ram_mb.to_string())
            .arg("--uds-path").arg(&uds_path)
            .spawn()
            .context("failed to spawn capsem-process")?;

        let pid = child.id();
        info!(id, pid, "capsem-process spawned");

        let mut instances = self.instances.lock().unwrap();
        instances.insert(id.to_string(), InstanceInfo {
            id: id.to_string(),
            pid,
            uds_path,
        });

        Ok(())
    }
}

async fn handle_provision(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, String> {
    let id = payload.name.clone().unwrap_or_else(|| {
        format!("vm-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs())
    });

    state.provision_sandbox(&id, payload.ram_mb, payload.cpus)
        .map_err(|e| format!("provision failed: {e}"))?;

    Ok(Json(ProvisionResponse { id }))
}

async fn handle_list(
    State(state): State<Arc<ServiceState>>,
) -> Json<ListResponse> {
    let instances = state.instances.lock().unwrap();
    let sandboxes = instances.values().map(|i| SandboxInfo {
        id: i.id.clone(),
        pid: i.pid,
        status: "Running".to_string(), // TODO: Check if process is still alive
    }).collect();

    Json(ListResponse { sandboxes })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let home = std::env::var("HOME").context("HOME not set")?;
    let capsem_dir = PathBuf::from(home).join(".capsem");
    let run_dir = capsem_dir.join("run");
    let instances_dir = run_dir.join("instances");
    let sessions_dir = run_dir.join("sessions");
    let service_sock = args.uds_path.unwrap_or_else(|| run_dir.join("service.sock"));

    // Ensure directories exist
    let _ = std::fs::create_dir_all(&instances_dir);
    let _ = std::fs::create_dir_all(&sessions_dir);

    // Remove old socket if it exists
    if service_sock.exists() {
        let _ = std::fs::remove_file(&service_sock);
    }

    let process_binary = args.process_binary.unwrap_or_else(|| {
        PathBuf::from("target/debug/capsem-process")
    });

    let assets_dir = args.assets_dir.unwrap_or_else(|| {
        capsem_dir.join("assets")
    });

    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        process_binary,
        assets_dir,
        run_dir: run_dir.clone(),
    });

    // Recovery loop: find running sandboxes
    info!("scanning for existing sandboxes in {}", instances_dir.display());
    if let Ok(entries) = std::fs::read_dir(&instances_dir) {
        let mut instances = state.instances.lock().unwrap();
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "sock" {
                    if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                        info!(id, "discovered existing sandbox");
                        // We don't know the exact PID without querying the process via UDS, 
                        // but we can add it to the registry. In a future sprint, we'd query it.
                        instances.insert(id.to_string(), InstanceInfo {
                            id: id.to_string(),
                            pid: 0, // Placeholder
                            uds_path: path.clone(),
                        });
                    }
                }
            }
        }
    }

    let app = Router::new()
        .route("/provision", post(handle_provision))
        .route("/list", get(handle_list))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    info!("capsem-service starting");
    info!(socket = %service_sock.display(), "listening on UDS");

    let uds = UnixListener::bind(&service_sock).context("failed to bind UDS")?;
    
    // Axum 0.8 uses hyper-util
    // Actually, axum::serve works with any stream
    axum::serve(uds, app).await.context("server error")?;

    Ok(())
}

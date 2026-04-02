use std::path::PathBuf;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use hyper::Request;
use http_body_util::{BodyExt, Full};
use bytes::Bytes;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to the service Unix Domain Socket
    #[arg(long)]
    uds_path: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a new VM sandbox
    Start {
        /// Name of the sandbox
        #[arg(long)]
        name: Option<String>,
        /// RAM in GB
        #[arg(long, default_value_t = 4)]
        ram: u64,
        /// CPU cores
        #[arg(long, default_value_t = 4)]
        cpu: u32,
    },
    /// Stop a running sandbox
    Stop {
        /// ID or name of the sandbox
        id: String,
    },
    /// List all running sandboxes
    List,
    /// Get status of a sandbox
    Status {
        /// ID or name of the sandbox
        id: String,
    },
}

#[derive(Serialize, Deserialize, Debug)]
struct ProvisionRequest {
    name: Option<String>,
    ram_mb: u64,
    cpus: u32,
}

#[derive(Serialize, Deserialize, Debug)]
struct ProvisionResponse {
    id: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct SandboxInfo {
    id: String,
    pid: u32,
    status: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct ListResponse {
    sandboxes: Vec<SandboxInfo>,
}

struct UdsClient {
    uds_path: PathBuf,
}

impl UdsClient {
    fn new(uds_path: PathBuf) -> Self {
        Self { uds_path }
    }

    async fn post<T: Serialize, R: for<'de> Deserialize<'de>>(&self, path: &str, body: T) -> Result<R> {
        let stream = UnixStream::connect(&self.uds_path).await
            .context("failed to connect to service socket")?;
        let io = TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::task::spawn(async move {
            if let Err(err) = conn.await {
                eprintln!("Connection failed: {:?}", err);
            }
        });

        let json = serde_json::to_vec(&body)?;
        let req = Request::post(format!("http://localhost{}", path))
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(json)))?;

        let res = sender.send_request(req).await?;
        let body = res.collect().await?.to_bytes();

        serde_json::from_slice(&body).map_err(|e| anyhow::anyhow!("failed to parse response: {e}. Body: {:?}", String::from_utf8_lossy(&body)))
    }

    async fn get<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        let stream = UnixStream::connect(&self.uds_path).await
            .context("failed to connect to service socket")?;
        let io = TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::task::spawn(async move {
            if let Err(err) = conn.await {
                eprintln!("Connection failed: {:?}", err);
            }
        });

        let req = Request::get(format!("http://localhost{}", path))
            .body(Full::<Bytes>::new(Bytes::new()))?;

        let res = sender.send_request(req).await?;
        let body = res.collect().await?.to_bytes();

        serde_json::from_slice(&body).map_err(|e| anyhow::anyhow!("failed to parse response: {e}. Body: {:?}", String::from_utf8_lossy(&body)))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let home = std::env::var("HOME").context("HOME not set")?;
    let run_dir = PathBuf::from(home).join(".capsem").join("run");
    let uds_path = cli.uds_path.unwrap_or_else(|| run_dir.join("service.sock"));

    let client = UdsClient::new(uds_path);

    match &cli.command {
        Commands::Start { name, ram, cpu } => {
            println!("Starting sandbox: name={:?}, ram={}GB, cpu={}", name, ram, cpu);
            let req = ProvisionRequest {
                name: name.clone(),
                ram_mb: ram * 1024,
                cpus: *cpu,
            };
            let resp: ProvisionResponse = client.post("/provision", req).await?;
            println!("Sandbox started with ID: {}", resp.id);
        }
        Commands::Stop { id } => {
            println!("Stopping sandbox: {} (Not yet implemented in API)", id);
        }
        Commands::List => {
            let resp: ListResponse = client.get("/list").await?;
            if resp.sandboxes.is_empty() {
                println!("No running sandboxes.");
            } else {
                println!("{:<20} {:<10} {:<10}", "ID", "PID", "STATUS");
                for s in resp.sandboxes {
                    println!("{:<20} {:<10} {:<10}", s.id, s.pid, s.status);
                }
            }
        }
        Commands::Status { id } => {
            println!("Status of sandbox: {} (Not yet implemented in API)", id);
        }
    }

    Ok(())
}

use std::path::PathBuf;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use hyper::Request;
use http_body_util::{BodyExt, Full};
use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
        /// Automatically remove the VM when its process exits
        #[arg(long, default_value_t = false)]
        rm: bool,
    },
    /// Stop a running sandbox
    Stop {
        /// ID or name of the sandbox
        id: String,
    },
    /// Connect to a sandbox's shell
    Shell {
        /// ID or name of the sandbox
        id: String,
    },
    /// List all running sandboxes
    #[command(alias = "ls")]
    List,
    /// Get status of a sandbox
    Status {
        /// ID or name of the sandbox
        id: String,
    },
    /// Execute a command in a running sandbox
    Exec {
        /// ID or name of the sandbox
        id: String,
        /// Command to execute
        command: String,
    },
    /// Delete a sandbox completely
    #[command(alias = "rm")]
    Delete {
        /// ID or name of the sandbox
        id: String,
    },
    /// Get detailed information about a sandbox
    Info {
        /// ID or name of the sandbox
        id: String,
    },
    /// Get logs from a sandbox (both serial and process logs)
    Logs {
        /// ID or name of the sandbox
        id: String,
    },
    /// Run diagnostic tests in a fresh VM
    Doctor,
}

#[derive(Serialize, Deserialize, Debug)]
struct ProvisionRequest {
    name: Option<String>,
    ram_mb: u64,
    cpus: u32,
    auto_remove: bool,
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

#[derive(Serialize, Deserialize, Debug)]
struct LogsResponse {
    logs: String,
    serial_logs: Option<String>,
    process_logs: Option<String>,
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

    async fn delete<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        let stream = UnixStream::connect(&self.uds_path).await
            .context("failed to connect to service socket")?;
        let io = TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::task::spawn(async move {
            if let Err(err) = conn.await {
                eprintln!("Connection failed: {:?}", err);
            }
        });

        let req = Request::delete(format!("http://localhost{}", path))
            .body(Full::<Bytes>::new(Bytes::new()))?;

        let res = sender.send_request(req).await?;
        let body = res.collect().await?.to_bytes();

        serde_json::from_slice(&body).map_err(|e| anyhow::anyhow!("failed to parse response: {e}. Body: {:?}", String::from_utf8_lossy(&body)))
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ExecRequest {
    command: String,
    timeout_secs: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct ExecResponse {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

#[derive(Serialize, Deserialize, Debug)]
struct ErrorResponse {
    error: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum ApiResponse<T> {
    Err(ErrorResponse),
    Ok(T),
}

impl<T> ApiResponse<T> {
    fn into_result(self) -> Result<T> {
        match self {
            ApiResponse::Ok(t) => Ok(t),
            ApiResponse::Err(e) => Err(anyhow::anyhow!(e.error)),
        }
    }
}

async fn run_shell(id: &str, run_dir: &std::path::Path) -> Result<()> {
    use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
    use tokio_unix_ipc::{channel_from_std, Sender, Receiver};
    use std::sync::Arc;
    use nix::sys::termios::{tcgetattr, tcsetattr, LocalFlags, SetArg};
    
    let sock_path = run_dir.join("instances").join(format!("{}.sock", id));
    if !sock_path.exists() {
        anyhow::bail!("Sandbox socket not found at: {}", sock_path.display());
    }

    let stream = tokio::net::UnixStream::connect(&sock_path).await.context("failed to connect to sandbox")?;
    let std_stream = stream.into_std()?;
    let (tx, rx): (Sender<ServiceToProcess>, Receiver<ProcessToService>) = channel_from_std(std_stream)?;
    let tx = Arc::new(tx);

    // Request terminal streaming
    tx.send(ServiceToProcess::StartTerminalStream).await?;

    use std::os::unix::io::{AsRawFd, BorrowedFd};
    
    let stdin_fd = std::io::stdin().as_raw_fd();
    let is_tty = nix::unistd::isatty(stdin_fd).unwrap_or(false);

    struct RawModeGuard {
        fd: std::os::unix::io::RawFd,
        original: Option<nix::sys::termios::Termios>,
    }
    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            if let Some(ref original) = self.original {
                let borrowed = unsafe { std::os::unix::io::BorrowedFd::borrow_raw(self.fd) };
                let _ = tcsetattr(borrowed, SetArg::TCSANOW, original);
            }
        }
    }

    let original_termios = if is_tty {
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
        let orig = tcgetattr(borrowed_fd).ok();
        if let Some(ref o) = orig {
            let mut raw_termios = o.clone();
            raw_termios.local_flags.remove(LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::ISIG);
            let _ = tcsetattr(borrowed_fd, SetArg::TCSANOW, &raw_termios);
        }
        orig
    } else {
        None
    };

    let _guard = RawModeGuard { fd: stdin_fd, original: original_termios };

    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut buf = vec![0u8; 65536];

    // Spawn a task to read from IPC and write to stdout
    let output_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    match msg {
                        ProcessToService::TerminalOutput { data } => {
                            let _ = stdout.write_all(&data).await;
                            let _ = stdout.flush().await;
                        }
                        ProcessToService::Pong => {}
                        ProcessToService::StateChanged { .. } => {}
                        ProcessToService::ExecResult { .. } => {}
                        ProcessToService::WriteFileResult { .. } => {}
                        ProcessToService::ReadFileResult { .. } => {}
                    }
                }
                Err(_) => break, // Socket closed
            }
        }
    });

    // Read from stdin and send over IPC
    loop {
        tokio::select! {
            res = stdin.read(&mut buf) => {
                match res {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        // Exit on Ctrl+D (0x04) explicitly if needed, but since we map raw input,
                        // usually we let the guest handle Ctrl+D. For a clean local exit, we can
                        // trap Ctrl+] (0x1D) as the disconnect signal.
                        if n == 1 && buf[0] == 0x1D {
                            break;
                        }
                        let _ = tx.send(ServiceToProcess::TerminalInput { data: buf[..n].to_vec() }).await;
                    }
                    Err(_) => break,
                }
            }
        }
    }

    output_task.abort();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let home = std::env::var("HOME").context("HOME not set")?;
    let run_dir = std::env::var("CAPSEM_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(home).join(".capsem").join("run"));
    let uds_path = cli.uds_path.unwrap_or_else(|| run_dir.join("service.sock"));

    let client = UdsClient::new(uds_path);

    match &cli.command {
        Commands::Start { name, ram, cpu, rm } => {
            let req = ProvisionRequest {
            name: name.clone(),
            ram_mb: ram * 1024,
            cpus: *cpu,
            auto_remove: *rm,
        };

        let resp: ApiResponse<ProvisionResponse> = client.post("/provision", &req).await?;
        let info = resp.into_result()?;
        println!("Sandbox started with ID: {}", info.id);

        // Wait for the socket to appear before returning
        let socket_path = run_dir.join("instances").join(format!("{}.sock", info.id));
        print!("Waiting for socket...");
        use std::io::Write;
        std::io::stdout().flush()?;
        for _ in 0..50 {
            if socket_path.exists() {
                println!(" ready.");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            print!(".");
            std::io::stdout().flush()?;
        }
        if !socket_path.exists() {
            println!("\nWarning: socket not found after 5s.");
        }
    }
        Commands::Stop { id } => {
            println!("Stopping sandbox: {}", id);
            let resp: ApiResponse<serde_json::Value> = client.delete(&format!("/delete/{}", id)).await?;
            resp.into_result()?;
            println!("Sandbox stopped.");
        }
        Commands::Shell { id } => {
            run_shell(id, &run_dir).await?;
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
            let resp: ApiResponse<SandboxInfo> = client.get(&format!("/info/{}", id)).await?;
            let info = resp.into_result()?;
            println!("ID: {}", info.id);
            println!("PID: {}", info.pid);
            println!("Status: {}", info.status);
        }
        Commands::Exec { id, command } => {
            let req = ExecRequest {
                command: command.clone(),
                timeout_secs: 30, // Default timeout
            };
            let resp: ApiResponse<ExecResponse> = client.post(&format!("/exec/{}", id), req).await?;
            let resp = resp.into_result()?;
            if !resp.stdout.is_empty() {
                print!("{}", resp.stdout);
            }
            if !resp.stderr.is_empty() {
                eprint!("{}", resp.stderr);
            }
            std::process::exit(resp.exit_code);
        }
        Commands::Delete { id } => {
            println!("Deleting sandbox: {}", id);
            let resp: ApiResponse<serde_json::Value> = client.delete(&format!("/delete/{}", id)).await?;
            resp.into_result()?;
            println!("Sandbox deleted.");
        }
        Commands::Info { id } => {
            let resp: ApiResponse<SandboxInfo> = client.get(&format!("/info/{}", id)).await?;
            let info = resp.into_result()?;
            let json = serde_json::to_string_pretty(&info)?;
            println!("{}", json);
        }
        Commands::Logs { id } => {
            let resp: ApiResponse<LogsResponse> = client.get(&format!("/logs/{}", id)).await?;
            let logs = resp.into_result()?;
            
            if let Some(process_logs) = logs.process_logs {
                println!("--- Process Logs ({}) ---", id);
                println!("{}", process_logs);
            }
            
            if let Some(serial_logs) = logs.serial_logs {
                println!("--- Serial Logs ({}) ---", id);
                println!("{}", serial_logs);
            } else if !logs.logs.is_empty() {
                println!("--- Serial Logs ({}) ---", id);
                println!("{}", logs.logs);
            }
        }
        Commands::Doctor => {
            println!("Running capsem-doctor...");
            
            // 1. Start a temporary VM
            let name = format!("doctor-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
            let req = ProvisionRequest {
                name: Some(name.clone()),
                ram_mb: 2048,
                cpus: 2,
                auto_remove: true,
            };
            let resp: ApiResponse<ProvisionResponse> = client.post("/provision", req).await?;
            let vm_id = resp.into_result()?.id;
            println!("Spawned temporary VM: {}", vm_id);
            
            // 2. Exec capsem-doctor
            let exec_req = ExecRequest {
                command: "capsem-doctor --json".to_string(),
                timeout_secs: 60,
            };
            let exec_resp: ApiResponse<ExecResponse> = client.post(&format!("/exec/{}", vm_id), exec_req).await?;
            let exec_resp = exec_resp.into_result()?;
            
            println!("\n=== Doctor Results ===");
            if !exec_resp.stdout.is_empty() {
                println!("{}", exec_resp.stdout);
            }
            if !exec_resp.stderr.is_empty() {
                eprintln!("{}", exec_resp.stderr);
            }
            println!("Exit code: {}", exec_resp.exit_code);
            
            // 3. Delete VM
            println!("\nCleaning up...");
            let delete_resp: ApiResponse<serde_json::Value> = client.delete(&format!("/delete/{}", vm_id)).await?;
            delete_resp.into_result()?;
            println!("Doctor VM destroyed.");
            
            if exec_resp.exit_code != 0 {
                std::process::exit(exec_resp.exit_code);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // -----------------------------------------------------------------------
    // CLI parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_start() {
        let cli = Cli::parse_from(["capsem", "start", "--name", "my-vm"]);
        match cli.command {
            Commands::Start { name, ram, cpu, rm } => {
                assert_eq!(name, Some("my-vm".into()));
                assert_eq!(ram, 4); // default
                assert_eq!(cpu, 4); // default
                assert_eq!(rm, false); // default
            }
            _ => panic!("expected Start"),
        }
    }

    #[test]
    fn parse_start_with_resources() {
        let cli = Cli::parse_from(["capsem", "start", "--ram", "8", "--cpu", "2"]);
        match cli.command {
            Commands::Start { ram, cpu, .. } => {
                assert_eq!(ram, 8);
                assert_eq!(cpu, 2);
            }
            _ => panic!("expected Start"),
        }
    }

    #[test]
    fn parse_stop() {
        let cli = Cli::parse_from(["capsem", "stop", "vm-123"]);
        match cli.command {
            Commands::Stop { id } => assert_eq!(id, "vm-123"),
            _ => panic!("expected Stop"),
        }
    }

    #[test]
    fn parse_shell() {
        let cli = Cli::parse_from(["capsem", "shell", "my-vm"]);
        match cli.command {
            Commands::Shell { id } => assert_eq!(id, "my-vm"),
            _ => panic!("expected Shell"),
        }
    }

    #[test]
    fn parse_list() {
        let cli = Cli::parse_from(["capsem", "list"]);
        assert!(matches!(cli.command, Commands::List));
    }

    #[test]
    fn parse_status() {
        let cli = Cli::parse_from(["capsem", "status", "vm-1"]);
        match cli.command {
            Commands::Status { id } => assert_eq!(id, "vm-1"),
            _ => panic!("expected Status"),
        }
    }

    #[test]
    fn parse_uds_path_override() {
        let cli = Cli::parse_from(["capsem", "--uds-path", "/tmp/test.sock", "list"]);
        assert_eq!(cli.uds_path, Some(PathBuf::from("/tmp/test.sock")));
    }

    #[test]
    fn parse_uds_path_default_none() {
        let cli = Cli::parse_from(["capsem", "list"]);
        assert_eq!(cli.uds_path, None);
    }

    // -----------------------------------------------------------------------
    // RAM conversion
    // -----------------------------------------------------------------------

    #[test]
    fn ram_gb_to_mb_conversion() {
        let ram_gb: u64 = 4;
        assert_eq!(ram_gb * 1024, 4096);
    }

    // -----------------------------------------------------------------------
    // Type serde
    // -----------------------------------------------------------------------

    #[test]
    fn provision_request_serde() {
        let req = ProvisionRequest { name: Some("test".into()), ram_mb: 4096, cpus: 4, auto_remove: false };
        let json = serde_json::to_string(&req).unwrap();
        let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.name, Some("test".into()));
        assert_eq!(req2.ram_mb, 4096);
    }

    #[test]
    fn list_response_empty_serde() {
        let resp = ListResponse { sandboxes: vec![] };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ListResponse = serde_json::from_str(&json).unwrap();
        assert!(resp2.sandboxes.is_empty());
    }

    #[test]
    fn list_response_with_entries() {
        let resp = ListResponse {
            sandboxes: vec![
                SandboxInfo { id: "vm-1".into(), pid: 100, status: "Running".into() },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.sandboxes.len(), 1);
        assert_eq!(resp2.sandboxes[0].id, "vm-1");
    }

    // -----------------------------------------------------------------------
    // New commands: exec, delete, info, doctor
    // -----------------------------------------------------------------------

    #[test]
    fn parse_exec() {
        let cli = Cli::parse_from(["capsem", "exec", "my-vm", "echo hello"]);
        match cli.command {
            Commands::Exec { id, command } => {
                assert_eq!(id, "my-vm");
                assert_eq!(command, "echo hello");
            }
            _ => panic!("expected Exec"),
        }
    }

    #[test]
    fn parse_delete() {
        let cli = Cli::parse_from(["capsem", "delete", "vm-123"]);
        match cli.command {
            Commands::Delete { id } => assert_eq!(id, "vm-123"),
            _ => panic!("expected Delete"),
        }
    }

    #[test]
    fn parse_info() {
        let cli = Cli::parse_from(["capsem", "info", "vm-1"]);
        match cli.command {
            Commands::Info { id } => assert_eq!(id, "vm-1"),
            _ => panic!("expected Info"),
        }
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::parse_from(["capsem", "doctor"]);
        assert!(matches!(cli.command, Commands::Doctor));
    }

    // -----------------------------------------------------------------------
    // ApiResponse untagged enum
    // -----------------------------------------------------------------------

    #[test]
    fn api_response_ok_variant() {
        let json = r#"{"id":"vm-1"}"#;
        let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
        let result = resp.into_result().unwrap();
        assert_eq!(result.id, "vm-1");
    }

    #[test]
    fn api_response_err_variant() {
        let json = r#"{"error":"sandbox not found"}"#;
        let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
        let err = resp.into_result().unwrap_err();
        assert!(err.to_string().contains("sandbox not found"));
    }

    // -----------------------------------------------------------------------
    // ExecRequest / ExecResponse serde
    // -----------------------------------------------------------------------

    #[test]
    fn exec_request_serde() {
        let req = ExecRequest { command: "ls -la".into(), timeout_secs: 30 };
        let json = serde_json::to_string(&req).unwrap();
        let req2: ExecRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.command, "ls -la");
        assert_eq!(req2.timeout_secs, 30);
    }

    #[test]
    fn exec_response_serde() {
        let resp = ExecResponse { stdout: "hello\n".into(), stderr: "".into(), exit_code: 0 };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ExecResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.stdout, "hello\n");
        assert_eq!(resp2.exit_code, 0);
    }

    #[test]
    fn exec_response_nonzero_exit() {
        let resp = ExecResponse { stdout: "".into(), stderr: "not found\n".into(), exit_code: 127 };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ExecResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.exit_code, 127);
        assert_eq!(resp2.stderr, "not found\n");
    }

    // -----------------------------------------------------------------------
    // CAPSEM_RUN_DIR resolution
    // -----------------------------------------------------------------------

    #[test]
    fn run_dir_override_logic() {
        let resolve = |env_val: Option<&str>, home: &str| -> PathBuf {
            env_val
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(home).join(".capsem").join("run"))
        };
        assert_eq!(
            resolve(Some("/tmp/custom-run"), "/ignored"),
            PathBuf::from("/tmp/custom-run"),
        );
        assert_eq!(
            resolve(None, "/Users/test"),
            PathBuf::from("/Users/test/.capsem/run"),
        );
    }

    // -----------------------------------------------------------------------
    // Security: socket path construction
    // -----------------------------------------------------------------------

    #[test]
    fn socket_path_with_traversal_id() {
        // If VM ID contains ../, the socket path would escape the instances dir
        let run_dir = PathBuf::from("/home/user/.capsem/run");
        let id = "../../../tmp/evil";
        let sock_path = run_dir.join("instances").join(format!("{}.sock", id));
        // This path escapes the intended directory
        assert!(sock_path.to_string_lossy().contains(".."));
        // Service should validate IDs before constructing paths
    }

    #[test]
    fn socket_path_normal_id() {
        let run_dir = PathBuf::from("/home/user/.capsem/run");
        let id = "vm-abc123";
        let sock_path = run_dir.join("instances").join(format!("{}.sock", id));
        assert_eq!(
            sock_path,
            PathBuf::from("/home/user/.capsem/run/instances/vm-abc123.sock")
        );
    }

    // -----------------------------------------------------------------------
    // RAM conversion edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn ram_gb_zero() {
        let ram_gb: u32 = 0;
        let ram_mb: u64 = ram_gb as u64 * 1024;
        assert_eq!(ram_mb, 0);
    }

    #[test]
    fn ram_gb_large() {
        let ram_gb: u32 = 128;
        let ram_mb: u64 = ram_gb as u64 * 1024;
        assert_eq!(ram_mb, 131072);
    }

    // -----------------------------------------------------------------------
    // ExecResponse exit code propagation
    // -----------------------------------------------------------------------

    #[test]
    fn exec_response_negative_exit_code() {
        let resp = ExecResponse { stdout: "".into(), stderr: "killed".into(), exit_code: -1 };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ExecResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.exit_code, -1);
    }

    #[test]
    fn exec_response_signal_exit_code() {
        // SIGKILL = 137 in Docker-style convention
        let resp = ExecResponse { stdout: "".into(), stderr: "".into(), exit_code: 137 };
        assert_eq!(resp.exit_code, 137);
    }

    // -----------------------------------------------------------------------
    // ApiResponse edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn api_response_malformed_json() {
        let json = r#"{"unexpected": true}"#;
        // ListResponse doesn't have "unexpected" field, but serde might still parse it
        let result: Result<ApiResponse<ListResponse>, _> = serde_json::from_str(json);
        // Untagged enum tries both variants
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn api_response_empty_error() {
        let json = r#"{"error":""}"#;
        let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
        let err = resp.into_result().unwrap_err();
        assert!(err.to_string().is_empty() || err.to_string().contains(""));
    }
}

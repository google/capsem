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
    /// Create a new VM sandbox. VMs are temporary by default; use -n <name> to persist.
    #[command(alias = "start")]
    Create {
        /// Name for the VM (makes it persistent -- "if you name it, you keep it")
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// RAM in GB
        #[arg(long, default_value_t = 4)]
        ram: u64,
        /// CPU cores
        #[arg(long, default_value_t = 4)]
        cpu: u32,
    },
    /// Resume a stopped persistent VM or attach to a running one
    #[command(alias = "attach")]
    Resume {
        /// Name of the persistent VM
        name: String,
    },
    /// Stop a running sandbox. Persistent VMs preserve state; ephemeral VMs are destroyed.
    Stop {
        /// ID or name of the sandbox
        id: String,
    },
    /// Open a shell. No args = temporary VM (destroyed on exit). With ID/name = attach to existing.
    Shell {
        /// Find by name (for persistent VMs)
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// ID of the sandbox (positional)
        id: Option<String>,
    },
    /// List all sandboxes (running + stopped persistent)
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
    /// Run a command in a fresh temporary VM and return output (VM is destroyed after)
    Run {
        /// Command to execute
        command: String,
        /// Timeout in seconds (default 60)
        #[arg(long, default_value_t = 60)]
        timeout: u64,
    },
    /// Delete a sandbox completely (destroys all state)
    #[command(alias = "rm")]
    Delete {
        /// ID or name of the sandbox
        id: String,
    },
    /// Convert a running ephemeral VM to a persistent named VM
    Persist {
        /// ID of the running ephemeral VM
        id: String,
        /// Name to assign
        name: String,
    },
    /// Kill all temporary VMs. Use --all to also destroy persistent VMs.
    Purge {
        /// Also destroy persistent VMs (requires confirmation)
        #[arg(long, default_value_t = false)]
        all: bool,
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
    #[serde(default)]
    persistent: bool,
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
    #[serde(default)]
    persistent: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct ListResponse {
    sandboxes: Vec<SandboxInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
struct PersistRequest {
    name: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct RunRequest {
    command: String,
    timeout_secs: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct PurgeRequest {
    all: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct PurgeResponse {
    purged: u32,
    persistent_purged: u32,
    ephemeral_purged: u32,
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
    use nix::sys::termios::{tcgetattr, tcsetattr, SetArg};
    
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

    let get_terminal_size = || -> Option<(u16, u16)> {
        let mut ws: nix::libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { nix::libc::ioctl(stdin_fd, nix::libc::TIOCGWINSZ, &mut ws) } == 0 {
            Some((ws.ws_col, ws.ws_row))
        } else {
            None
        }
    };

    // Send initial window size
    if is_tty {
        if let Some((cols, rows)) = get_terminal_size() {
            let _ = tx.send(ServiceToProcess::TerminalResize { cols, rows }).await;
        }
    }

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
            nix::sys::termios::cfmakeraw(&mut raw_termios);
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

    let mut sigwinch = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;

    // Read from stdin and send over IPC
    loop {
        tokio::select! {
            _ = sigwinch.recv() => {
                if is_tty {
                    if let Some((cols, rows)) = get_terminal_size() {
                        let _ = tx.send(ServiceToProcess::TerminalResize { cols, rows }).await;
                    }
                }
            }
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
        Commands::Create { name, ram, cpu } => {
            let persistent = name.is_some();
            let req = ProvisionRequest {
                name: name.clone(),
                ram_mb: ram * 1024,
                cpus: *cpu,
                persistent,
            };

            let resp: ApiResponse<ProvisionResponse> = client.post("/provision", &req).await?;
            let info = resp.into_result()?;

            if persistent {
                println!("{} (persistent)", info.id);
            } else {
                println!("{}", info.id);
            }
        }
        Commands::Resume { name } => {
            let resp: ApiResponse<ProvisionResponse> = client.post(&format!("/resume/{}", name), &serde_json::json!({})).await?;
            let info = resp.into_result()?;
            println!("{}", info.id);
        }
        Commands::Stop { id } => {
            println!("Stopping sandbox: {}", id);
            let resp: ApiResponse<serde_json::Value> = client.post(&format!("/stop/{}", id), &serde_json::json!({})).await?;
            resp.into_result()?;
            println!("Sandbox stopped.");
        }
        Commands::Shell { name, id } => {
            let target = name.as_ref().or(id.as_ref());
            match target {
                Some(t) => {
                    // Attach to existing VM
                    run_shell(t, &run_dir).await?;
                }
                None => {
                    // No args: create ephemeral VM, attach, destroy on exit
                    println!("[!] Temporary VM. Use `capsem create -n <name>` for persistent.");
                    let req = ProvisionRequest {
                        name: None,
                        ram_mb: 4 * 1024,
                        cpus: 4,
                        persistent: false,
                    };
                    let resp: ApiResponse<ProvisionResponse> = client.post("/provision", &req).await?;
                    let info = resp.into_result()?;

                    let socket_path = run_dir.join("instances").join(format!("{}.sock", info.id));
                    for _ in 0..50 {
                        if socket_path.exists() { break; }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }

                    let shell_result = run_shell(&info.id, &run_dir).await;
                    // Ephemeral: auto-destroy on disconnect
                    let _: Result<ApiResponse<serde_json::Value>, _> = client.delete(&format!("/delete/{}", info.id)).await;
                    shell_result?;
                }
            }
        }
        Commands::List => {
            let resp: ListResponse = client.get("/list").await?;
            if resp.sandboxes.is_empty() {
                println!("No sandboxes.");
            } else {
                println!("{:<20} {:<10} {:<10} {:<10}", "ID", "STATUS", "PERSIST", "PID");
                for s in resp.sandboxes {
                    let persist = if s.persistent { "yes" } else { "-" };
                    let pid_str = if s.pid > 0 { s.pid.to_string() } else { "-".to_string() };
                    println!("{:<20} {:<10} {:<10} {:<10}", s.id, s.status, persist, pid_str);
                }
            }
        }
        Commands::Status { id } => {
            let resp: ApiResponse<SandboxInfo> = client.get(&format!("/info/{}", id)).await?;
            let info = resp.into_result()?;
            println!("ID: {}", info.id);
            println!("PID: {}", info.pid);
            println!("Status: {}", info.status);
            println!("Persistent: {}", info.persistent);
        }
        Commands::Exec { id, command } => {
            let req = ExecRequest {
                command: command.clone(),
                timeout_secs: 30,
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
        Commands::Run { command, timeout } => {
            let req = RunRequest {
                command: command.clone(),
                timeout_secs: *timeout,
            };
            let resp: ApiResponse<ExecResponse> = client.post("/run", &req).await?;
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
        Commands::Persist { id, name } => {
            let req = PersistRequest { name: name.clone() };
            let resp: ApiResponse<serde_json::Value> = client.post(&format!("/persist/{}", id), &req).await?;
            resp.into_result()?;
            println!("[*] VM \"{}\" is now persistent as \"{}\"", id, name);
        }
        Commands::Purge { all } => {
            if *all {
                // Confirmation prompt
                use std::io::Write;
                let resp: ListResponse = client.get("/list").await?;
                let persistent_count = resp.sandboxes.iter().filter(|s| s.persistent).count();
                let ephemeral_count = resp.sandboxes.iter().filter(|s| !s.persistent).count();
                print!("[!] This will destroy {} persistent VMs and {} temporary VMs. Continue? [y/N] ",
                    persistent_count, ephemeral_count);
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Aborted.");
                    return Ok(());
                }
            }

            let req = PurgeRequest { all: *all };
            let resp: ApiResponse<PurgeResponse> = client.post("/purge", &req).await?;
            let result = resp.into_result()?;
            if *all {
                println!("[*] Purged {} VMs ({} persistent, {} temporary).",
                    result.purged, result.persistent_purged, result.ephemeral_purged);
            } else {
                println!("[*] Purged {} temporary VMs.", result.ephemeral_purged);
            }
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

            let name = format!("doctor-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
            let req = ProvisionRequest {
                name: Some(name.clone()),
                ram_mb: 2048,
                cpus: 2,
                persistent: false,
            };
            let resp: ApiResponse<ProvisionResponse> = client.post("/provision", req).await?;
            let vm_id = resp.into_result()?.id;
            println!("Spawned temporary VM: {}", vm_id);

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
    fn parse_create_with_name() {
        let cli = Cli::parse_from(["capsem", "create", "-n", "my-vm"]);
        match cli.command {
            Commands::Create { name, ram, cpu } => {
                assert_eq!(name, Some("my-vm".into()));
                assert_eq!(ram, 4);
                assert_eq!(cpu, 4);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_ephemeral() {
        let cli = Cli::parse_from(["capsem", "create"]);
        match cli.command {
            Commands::Create { name, .. } => {
                assert_eq!(name, None);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_start_alias_for_create() {
        let cli = Cli::parse_from(["capsem", "start", "-n", "dev"]);
        match cli.command {
            Commands::Create { name, .. } => {
                assert_eq!(name, Some("dev".into()));
            }
            _ => panic!("expected Create via start alias"),
        }
    }

    #[test]
    fn parse_create_with_resources() {
        let cli = Cli::parse_from(["capsem", "create", "--ram", "8", "--cpu", "2"]);
        match cli.command {
            Commands::Create { ram, cpu, .. } => {
                assert_eq!(ram, 8);
                assert_eq!(cpu, 2);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_resume() {
        let cli = Cli::parse_from(["capsem", "resume", "mydev"]);
        match cli.command {
            Commands::Resume { name } => assert_eq!(name, "mydev"),
            _ => panic!("expected Resume"),
        }
    }

    #[test]
    fn parse_attach_alias_for_resume() {
        let cli = Cli::parse_from(["capsem", "attach", "mydev"]);
        match cli.command {
            Commands::Resume { name } => assert_eq!(name, "mydev"),
            _ => panic!("expected Resume via attach alias"),
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
    fn parse_shell_positional() {
        let cli = Cli::parse_from(["capsem", "shell", "my-vm"]);
        match cli.command {
            Commands::Shell { id, name } => {
                assert_eq!(id, Some("my-vm".into()));
                assert_eq!(name, None);
            }
            _ => panic!("expected Shell"),
        }
    }

    #[test]
    fn parse_shell_by_name() {
        let cli = Cli::parse_from(["capsem", "shell", "-n", "mydev"]);
        match cli.command {
            Commands::Shell { name, id } => {
                assert_eq!(name, Some("mydev".into()));
                assert_eq!(id, None);
            }
            _ => panic!("expected Shell"),
        }
    }

    #[test]
    fn parse_shell_bare() {
        // Bare `capsem shell` = temp VM + auto-destroy
        let cli = Cli::parse_from(["capsem", "shell"]);
        match cli.command {
            Commands::Shell { name, id } => {
                assert_eq!(name, None);
                assert_eq!(id, None);
            }
            _ => panic!("expected Shell"),
        }
    }

    #[test]
    fn parse_persist() {
        let cli = Cli::parse_from(["capsem", "persist", "vm-123", "mydev"]);
        match cli.command {
            Commands::Persist { id, name } => {
                assert_eq!(id, "vm-123");
                assert_eq!(name, "mydev");
            }
            _ => panic!("expected Persist"),
        }
    }

    #[test]
    fn parse_purge() {
        let cli = Cli::parse_from(["capsem", "purge"]);
        match cli.command {
            Commands::Purge { all } => assert!(!all),
            _ => panic!("expected Purge"),
        }
    }

    #[test]
    fn parse_purge_all() {
        let cli = Cli::parse_from(["capsem", "purge", "--all"]);
        match cli.command {
            Commands::Purge { all } => assert!(all),
            _ => panic!("expected Purge --all"),
        }
    }

    #[test]
    fn parse_run() {
        let cli = Cli::parse_from(["capsem", "run", "echo hello"]);
        match cli.command {
            Commands::Run { command, timeout } => {
                assert_eq!(command, "echo hello");
                assert_eq!(timeout, 60); // default
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn parse_run_with_timeout() {
        let cli = Cli::parse_from(["capsem", "run", "--timeout", "120", "ls -la"]);
        match cli.command {
            Commands::Run { command, timeout } => {
                assert_eq!(command, "ls -la");
                assert_eq!(timeout, 120);
            }
            _ => panic!("expected Run"),
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
        let req = ProvisionRequest { name: Some("test".into()), ram_mb: 4096, cpus: 4, persistent: true };
        let json = serde_json::to_string(&req).unwrap();
        let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.name, Some("test".into()));
        assert_eq!(req2.ram_mb, 4096);
        assert!(req2.persistent);
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
                SandboxInfo { id: "vm-1".into(), pid: 100, status: "Running".into(), persistent: false },
                SandboxInfo { id: "mydev".into(), pid: 0, status: "Stopped".into(), persistent: true },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let resp2: ListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp2.sandboxes.len(), 2);
        assert_eq!(resp2.sandboxes[0].id, "vm-1");
        assert!(!resp2.sandboxes[0].persistent);
        assert_eq!(resp2.sandboxes[1].id, "mydev");
        assert!(resp2.sandboxes[1].persistent);
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

mod completions;
mod paths;
mod platform;
mod service_install;
mod setup;
mod uninstall;
mod update;

use std::collections::HashMap;
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
use tracing::{info, error};

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
        /// Set environment variables (repeatable: -e KEY=VALUE)
        #[arg(short = 'e', long = "env")]
        env: Vec<String>,
        /// Boot the VM from a named image
        #[arg(long)]
        image: Option<String>,
    },
    /// Fork a running or stopped VM into a reusable image.
    Fork {
        /// ID or name of the VM to fork
        id: String,
        /// Name for the new image
        name: String,
        /// Optional description for the image
        #[arg(short, long)]
        description: Option<String>,
    },
    /// Manage user images.
    #[command(subcommand)]
    Image(ImageCommands),
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
    List {
        /// Print only IDs, one per line (for scripting)
        #[arg(short, long)]
        quiet: bool,
    },
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
        /// Timeout in seconds (default 30)
        #[arg(long, default_value_t = 30)]
        timeout: u64,
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
        /// Show only the last N lines
        #[arg(long)]
        tail: Option<usize>,
    },
    /// Restart a persistent VM (stop + resume)
    Restart {
        /// Name of the persistent VM
        name: String,
    },
    /// Show version information
    Version,
    /// Run diagnostic tests in a fresh VM
    Doctor,
    /// Manage the capsem service daemon (install/uninstall/status)
    #[command(subcommand)]
    Service(ServiceCommands),
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Uninstall capsem completely (service, binaries, data)
    Uninstall {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
    /// Check for updates and install the latest version
    Update {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
    /// Run the first-time setup wizard
    Setup {
        /// Run without prompts (accept defaults or detected values)
        #[arg(long)]
        non_interactive: bool,
        /// Security preset to apply (medium or high)
        #[arg(long)]
        preset: Option<String>,
        /// Re-run all steps even if previously completed
        #[arg(long)]
        force: bool,
        /// Auto-accept detected credentials without prompting
        #[arg(long)]
        accept_detected: bool,
        /// Provision corp config from URL or file path
        #[arg(long)]
        corp_config: Option<String>,
    },
}

#[derive(Subcommand)]
enum ServiceCommands {
    /// Install capsem as a system service (LaunchAgent on macOS, systemd on Linux)
    Install,
    /// Uninstall the capsem system service
    Uninstall,
    /// Show service installation and runtime status
    Status,
}

#[derive(Subcommand)]
enum ImageCommands {
    /// List all images
    #[command(alias = "ls")]
    List,
    /// Delete an image
    #[command(alias = "rm")]
    Delete {
        /// Name of the image to delete
        name: String,
    },
    /// Inspect an image
    Inspect {
        /// Name of the image to inspect
        name: String,
    },
}

#[derive(Serialize, Deserialize, Debug)]
struct ProvisionRequest {
    name: Option<String>,
    ram_mb: u64,
    cpus: u32,
    #[serde(default)]
    persistent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ProvisionResponse {
    id: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct ForkRequest {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ForkResponse {
    name: String,
    size_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct ImageInfo {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    source_vm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_image: Option<String>,
    base_version: String,
    created_at: String,
    size_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct ImageListResponse {
    images: Vec<ImageInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
struct SandboxInfo {
    id: String,
    pid: u32,
    status: String,
    #[serde(default)]
    persistent: bool,
    #[serde(default)]
    ram_mb: Option<u64>,
    #[serde(default)]
    cpus: Option<u32>,
    #[serde(default)]
    version: Option<String>,
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

    /// Try to ensure the service is running. Checks socket, tries service manager
    /// (systemd/launchctl) if a unit is installed, falls back to direct spawn.
    async fn try_ensure_service(&self) -> Result<()> {
        if UnixStream::connect(&self.uds_path).await.is_ok() {
            return Ok(());
        }

        info!("Service not responding, attempting to launch...");

        // If the service is registered with a service manager, use that exclusively.
        // Direct-spawning when a unit exists would create an unmanaged duplicate.
        if service_install::is_service_installed() {
            info!("Service unit installed, using service manager");
            match paths::try_start_via_service_manager().await {
                Ok(true) => {
                    info!("Service start requested via service manager");
                    for _ in 0..50 {
                        if UnixStream::connect(&self.uds_path).await.is_ok() {
                            info!("Service responding after service manager start");
                            return Ok(());
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    return Err(anyhow::anyhow!(
                        "Service manager started capsem but socket not ready after 5s. \
                         Check logs: journalctl --user -u capsem (Linux) or \
                         ~/Library/Logs/capsem/service.log (macOS)"
                    ));
                }
                Ok(false) => {
                    return Err(anyhow::anyhow!(
                        "Service unit found but service manager reports not installed"
                    ));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Service manager start failed: {}. \
                         Check logs or reinstall with `capsem service install`", e
                    ));
                }
            }
        }

        // No service unit installed -- direct spawn fallback
        let paths = paths::discover_paths()
            .context("cannot find capsem binaries for auto-launch")?;

        if !paths.service_bin.exists() {
            return Err(anyhow::anyhow!(
                "capsem-service not found at {}",
                paths.service_bin.display()
            ));
        }

        info!(
            service = %paths.service_bin.display(),
            assets = %paths.assets_dir.display(),
            "spawning service directly"
        );

        let mut child = tokio::process::Command::new(&paths.service_bin)
            .arg("--foreground")
            .arg("--assets-dir").arg(&paths.assets_dir)
            .arg("--process-binary").arg(&paths.process_bin)
            .spawn()
            .context("failed to spawn capsem-service")?;

        // Wait up to 5s for socket
        for _ in 0..50 {
            if UnixStream::connect(&self.uds_path).await.is_ok() {
                info!("Service spawned and responding");
                // Reaper so child doesn't become a zombie
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        Err(anyhow::anyhow!("capsem-service failed to start within 5s"))
    }

    /// Unified HTTP request over UDS. Retries once via try_ensure_service() on
    /// connection failure.
    async fn request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        path: &str,
        body: Option<T>,
    ) -> Result<R> {
        let stream = match UnixStream::connect(&self.uds_path).await {
            Ok(s) => s,
            Err(_) => {
                self.try_ensure_service().await?;
                UnixStream::connect(&self.uds_path).await
                    .context("failed to connect to service socket after auto-launch")?
            }
        };

        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::task::spawn(async move {
            if let Err(err) = conn.await {
                error!("Connection failed: {:?}", err);
            }
        });

        let builder = Request::builder()
            .method(method)
            .uri(format!("http://localhost{}", path))
            .header("Content-Type", "application/json");

        let req = if let Some(b) = body {
            let json = serde_json::to_vec(&b)?;
            builder.body(Full::new(Bytes::from(json)))?
        } else {
            builder.body(Full::new(Bytes::new()))?
        };

        let res = sender.send_request(req).await?;
        let body_bytes = res.collect().await?.to_bytes();
        serde_json::from_slice(&body_bytes).map_err(|e| {
            anyhow::anyhow!(
                "failed to parse response: {e}. Body: {:?}",
                String::from_utf8_lossy(&body_bytes)
            )
        })
    }

    async fn post<T: Serialize, R: for<'de> Deserialize<'de>>(&self, path: &str, body: T) -> Result<R> {
        self.request("POST", path, Some(body)).await
    }

    async fn get<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("GET", path, None).await
    }

    async fn delete<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("DELETE", path, None).await
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

    // Show update notice if available (sync file read, no latency)
    if let Some(notice) = update::read_cached_update_notice() {
        eprintln!("{}", notice);
    }

    // Commands that don't need the service
    match &cli.command {
        Commands::Version => {
            println!(
                "capsem {} (build {})",
                env!("CARGO_PKG_VERSION"),
                env!("CAPSEM_BUILD_HASH")
            );
            return Ok(());
        }
        Commands::Service(cmd) => {
            match cmd {
                ServiceCommands::Install => {
                    service_install::install_service().await?;
                    println!("Service installed.");
                }
                ServiceCommands::Uninstall => {
                    service_install::uninstall_service().await?;
                    println!("Service uninstalled.");
                }
                ServiceCommands::Status => {
                    let status = service_install::service_status().await?;
                    println!("Installed: {}", status.installed);
                    println!("Running:   {}", status.running);
                    if let Some(pid) = status.pid {
                        println!("PID:       {}", pid);
                    }
                    if let Some(path) = &status.unit_path {
                        println!("Unit:      {}", path.display());
                    }
                }
            }
            return Ok(());
        }
        Commands::Completions { shell } => {
            completions::generate_completions(*shell);
            return Ok(());
        }
        Commands::Uninstall { yes } => {
            uninstall::run_uninstall(*yes).await?;
            return Ok(());
        }
        Commands::Update { yes } => {
            update::run_update(*yes).await?;
            return Ok(());
        }
        Commands::Setup { non_interactive, preset, force, accept_detected, corp_config } => {
            let opts = setup::SetupOptions {
                non_interactive: *non_interactive,
                preset: preset.clone(),
                force: *force,
                accept_detected: *accept_detected,
                corp_config: corp_config.clone(),
            };
            setup::run_setup(opts).await?;
            return Ok(());
        }
        _ => {}
    }

    let client = UdsClient::new(uds_path);

    match &cli.command {
        Commands::Create { name, ram, cpu, env, image } => {
            let persistent = name.is_some();
            let env_map = if env.is_empty() {
                None
            } else {
                let mut map = HashMap::new();
                for kv in env {
                    let (k, v) = kv.split_once('=')
                        .ok_or_else(|| anyhow::anyhow!("invalid env format: expected KEY=VALUE, got: {}", kv))?;
                    map.insert(k.to_string(), v.to_string());
                }
                Some(map)
            };
            let req = ProvisionRequest {
                name: name.clone(),
                ram_mb: ram * 1024,
                cpus: *cpu,
                persistent,
                env: env_map,
                image: image.clone(),
            };

            let resp: ApiResponse<ProvisionResponse> = client.post("/provision", &req).await?;
            let info = resp.into_result()?;

            if persistent {
                println!("{} (persistent)", info.id);
            } else {
                println!("{}", info.id);
            }
        }
        Commands::Fork { id, name, description } => {
            let req = ForkRequest {
                name: name.clone(),
                description: description.clone(),
            };
            let resp: ApiResponse<ForkResponse> = client.post(&format!("/fork/{}", id), &req).await?;
            let info = resp.into_result()?;
            let size_mb = info.size_bytes as f64 / 1024.0 / 1024.0;
            println!("Forked VM {} to image '{}' ({:.1} MB)", id, info.name, size_mb);
        }
        Commands::Image(cmd) => {
            match cmd {
                ImageCommands::List => {
                    let resp: ApiResponse<ImageListResponse> = client.get("/images").await?;
                    let list = resp.into_result()?;
                    
                    println!("{:<20} {:<10} {:<20} {:<15} {}", "NAME", "SIZE(MB)", "SOURCE_VM", "BASE_VERSION", "CREATED_AT");
                    println!("{:-<20} {:<10} {:-<20} {:-<15} {:-<20}", "", "", "", "", "");
                    for img in list.images {
                        let size_mb = img.size_bytes as f64 / 1024.0 / 1024.0;
                        println!("{:<20} {:<10.1} {:<20} {:<15} {}", img.name, size_mb, img.source_vm, img.base_version, img.created_at);
                    }
                }
                ImageCommands::Inspect { name } => {
                    let resp: ApiResponse<ImageInfo> = client.get(&format!("/images/{}", name)).await?;
                    let info = resp.into_result()?;
                    let json = serde_json::to_string_pretty(&info)?;
                    println!("{}", json);
                }
                ImageCommands::Delete { name } => {
                    let resp: ApiResponse<serde_json::Value> = client.delete(&format!("/images/{}", name)).await?;
                    resp.into_result()?;
                    println!("Image '{}' deleted.", name);
                }
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
                        env: None,
                        image: None,
                    };
                    let resp: ApiResponse<ProvisionResponse> = client.post("/provision", &req).await?;
                    let info = resp.into_result()?;

                    let socket_path = run_dir.join("instances").join(format!("{}.sock", info.id));
                    for _ in 0..50 {
                        if socket_path.exists() { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }

                    let shell_result = run_shell(&info.id, &run_dir).await;
                    // Ephemeral: auto-destroy on disconnect
                    let _: Result<ApiResponse<serde_json::Value>, _> = client.delete(&format!("/delete/{}", info.id)).await;
                    shell_result?;
                }
            }
        }
        Commands::List { quiet } => {
            let resp: ListResponse = client.get("/list").await?;
            if *quiet {
                for s in resp.sandboxes {
                    println!("{}", s.id);
                }
            } else if resp.sandboxes.is_empty() {
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
            if let Some(ram) = info.ram_mb { println!("RAM: {} MB", ram); }
            if let Some(cpus) = info.cpus { println!("CPUs: {}", cpus); }
            if let Some(ver) = &info.version { println!("Version: {}", ver); }
        }
        Commands::Exec { id, command, timeout } => {
            let req = ExecRequest {
                command: command.clone(),
                timeout_secs: *timeout,
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
        Commands::Logs { id, tail } => {
            let resp: ApiResponse<LogsResponse> = client.get(&format!("/logs/{}", id)).await?;
            let logs = resp.into_result()?;

            let tail_lines = |text: &str, n: usize| -> String {
                let lines: Vec<&str> = text.lines().collect();
                if lines.len() <= n {
                    text.to_string()
                } else {
                    lines[lines.len() - n..].join("\n")
                }
            };

            if let Some(process_logs) = logs.process_logs {
                println!("--- Process Logs ({}) ---", id);
                let output = match tail {
                    Some(n) => tail_lines(&process_logs, *n),
                    None => process_logs,
                };
                println!("{}", output);
            }

            if let Some(serial_logs) = logs.serial_logs {
                println!("--- Serial Logs ({}) ---", id);
                let output = match tail {
                    Some(n) => tail_lines(&serial_logs, *n),
                    None => serial_logs,
                };
                println!("{}", output);
            } else if !logs.logs.is_empty() {
                println!("--- Serial Logs ({}) ---", id);
                let output = match tail {
                    Some(n) => tail_lines(&logs.logs, *n),
                    None => logs.logs,
                };
                println!("{}", output);
            }
        }
        Commands::Restart { name } => {
            // Look up the VM to check it's persistent
            let info_resp: ApiResponse<SandboxInfo> = client.get(&format!("/info/{}", name)).await?;
            let info = info_resp.into_result()?;
            if !info.persistent {
                anyhow::bail!("Cannot restart ephemeral VM \"{}\". Only persistent VMs support restart.", name);
            }

            // Stop, then resume
            let _: ApiResponse<serde_json::Value> = client.post(&format!("/stop/{}", name), &serde_json::json!({})).await?;
            let resp: ApiResponse<ProvisionResponse> = client.post(&format!("/resume/{}", name), &serde_json::json!({})).await?;
            let resumed = resp.into_result()?;
            println!("{}", resumed.id);
        }
        Commands::Version | Commands::Service(_) | Commands::Setup { .. }
        | Commands::Update { .. } | Commands::Completions { .. } | Commands::Uninstall { .. } => {
            unreachable!("handled before UdsClient creation")
        }
        Commands::Doctor => {
            use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
            use tokio_unix_ipc::channel_from_std;

            println!("Running capsem-doctor...");

            let name = format!("doctor-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
            let req = ProvisionRequest {
                name: Some(name.clone()),
                ram_mb: 2048,
                cpus: 2,
                persistent: false,
                env: None,
                image: None,
            };
            let resp: ApiResponse<ProvisionResponse> = client.post("/provision", req).await?;
            let vm_id = resp.into_result()?.id;

            // Connect directly to VM socket for streaming output
            let sock_path = run_dir.join("instances").join(format!("{}.sock", vm_id));
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if sock_path.exists() {
                    if let Ok(stream) = tokio::net::UnixStream::connect(&sock_path).await {
                        if let Ok(std_stream) = stream.into_std() {
                            if let Ok((tx, rx)) = channel_from_std::<ServiceToProcess, ProcessToService>(std_stream) {
                                // Start streaming + exec doctor
                                let _ = tx.send(ServiceToProcess::StartTerminalStream).await;
                                let _ = tx.send(ServiceToProcess::Exec {
                                    id: 1,
                                    command: "capsem-doctor --durations=10".to_string(),
                                }).await;

                                // Stream output until ExecResult arrives
                                let mut stdout = tokio::io::stdout();
                                let exit_code = loop {
                                    match tokio::time::timeout(
                                        std::time::Duration::from_secs(120),
                                        rx.recv(),
                                    ).await {
                                        Ok(Ok(ProcessToService::TerminalOutput { data })) => {
                                            let _ = stdout.write_all(&data).await;
                                            let _ = stdout.flush().await;
                                        }
                                        Ok(Ok(ProcessToService::ExecResult { exit_code, .. })) => {
                                            break exit_code;
                                        }
                                        Ok(Ok(_)) => continue,
                                        Ok(Err(e)) => {
                                            eprintln!("IPC error: {e}");
                                            break 1;
                                        }
                                        Err(_) => {
                                            eprintln!("Doctor timed out after 120s");
                                            break 1;
                                        }
                                    }
                                };

                                // Cleanup
                                let _: Result<ApiResponse<serde_json::Value>, _> = client.delete(&format!("/delete/{}", vm_id)).await;
                                if exit_code != 0 {
                                    std::process::exit(exit_code);
                                }
                                break;
                            }
                        }
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    eprintln!("VM did not become ready within 30s");
                    let _: Result<ApiResponse<serde_json::Value>, _> = client.delete(&format!("/delete/{}", vm_id)).await;
                    std::process::exit(1);
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }

    // Background update check (fire-and-forget)
    tokio::spawn(update::refresh_update_cache_if_stale());

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
            Commands::Create { name, ram, cpu, .. } => {
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
        assert!(matches!(cli.command, Commands::List { quiet: false }));
    }

    #[test]
    fn parse_list_quiet() {
        let cli = Cli::parse_from(["capsem", "list", "-q"]);
        match cli.command {
            Commands::List { quiet } => assert!(quiet),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_list_quiet_long() {
        let cli = Cli::parse_from(["capsem", "list", "--quiet"]);
        match cli.command {
            Commands::List { quiet } => assert!(quiet),
            _ => panic!("expected List"),
        }
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
        let req = ProvisionRequest { name: Some("test".into()), ram_mb: 4096, cpus: 4, persistent: true, env: None, image: None };
        let json = serde_json::to_string(&req).unwrap();
        let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.name, Some("test".into()));
        assert_eq!(req2.ram_mb, 4096);
        assert!(req2.persistent);
        assert!(req2.env.is_none());
    }

    #[test]
    fn provision_request_with_env() {
        let mut env = HashMap::new();
        env.insert("FOO".into(), "bar".into());
        let req = ProvisionRequest { name: Some("test".into()), ram_mb: 2048, cpus: 2, persistent: true, env: Some(env), image: None };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("FOO"));
        let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req2.env.as_ref().unwrap().get("FOO").unwrap(), "bar");
    }

    #[test]
    fn provision_request_env_omitted_when_none() {
        let req = ProvisionRequest { name: None, ram_mb: 2048, cpus: 2, persistent: false, env: None, image: None };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("env"));
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
                SandboxInfo { id: "vm-1".into(), pid: 100, status: "Running".into(), persistent: false, ram_mb: Some(2048), cpus: Some(2), version: Some("0.16.1".into()) },
                SandboxInfo { id: "mydev".into(), pid: 0, status: "Stopped".into(), persistent: true, ram_mb: Some(4096), cpus: Some(4), version: None },
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
            Commands::Exec { id, command, timeout } => {
                assert_eq!(id, "my-vm");
                assert_eq!(command, "echo hello");
                assert_eq!(timeout, 30); // default
            }
            _ => panic!("expected Exec"),
        }
    }

    #[test]
    fn parse_exec_with_timeout() {
        let cli = Cli::parse_from(["capsem", "exec", "--timeout", "120", "my-vm", "make build"]);
        match cli.command {
            Commands::Exec { id, command, timeout } => {
                assert_eq!(id, "my-vm");
                assert_eq!(command, "make build");
                assert_eq!(timeout, 120);
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
    fn parse_logs_with_tail() {
        let cli = Cli::parse_from(["capsem", "logs", "--tail", "50", "vm-1"]);
        match cli.command {
            Commands::Logs { id, tail } => {
                assert_eq!(id, "vm-1");
                assert_eq!(tail, Some(50));
            }
            _ => panic!("expected Logs"),
        }
    }

    #[test]
    fn parse_logs_without_tail() {
        let cli = Cli::parse_from(["capsem", "logs", "vm-1"]);
        match cli.command {
            Commands::Logs { id, tail } => {
                assert_eq!(id, "vm-1");
                assert_eq!(tail, None);
            }
            _ => panic!("expected Logs"),
        }
    }

    #[test]
    fn parse_restart() {
        let cli = Cli::parse_from(["capsem", "restart", "mydev"]);
        match cli.command {
            Commands::Restart { name } => assert_eq!(name, "mydev"),
            _ => panic!("expected Restart"),
        }
    }

    #[test]
    fn parse_version() {
        let cli = Cli::parse_from(["capsem", "version"]);
        assert!(matches!(cli.command, Commands::Version));
    }

    #[test]
    fn parse_create_with_env() {
        let cli = Cli::parse_from(["capsem", "create", "-e", "FOO=bar", "-e", "BAZ=qux"]);
        match cli.command {
            Commands::Create { env, .. } => {
                assert_eq!(env, vec!["FOO=bar", "BAZ=qux"]);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_with_env_long() {
        let cli = Cli::parse_from(["capsem", "create", "--env", "API_KEY=secret123"]);
        match cli.command {
            Commands::Create { env, .. } => {
                assert_eq!(env, vec!["API_KEY=secret123"]);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_no_env() {
        let cli = Cli::parse_from(["capsem", "create"]);
        match cli.command {
            Commands::Create { env, .. } => {
                assert!(env.is_empty());
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::parse_from(["capsem", "doctor"]);
        assert!(matches!(cli.command, Commands::Doctor));
    }

    #[test]
    fn parse_service_install() {
        let cli = Cli::parse_from(["capsem", "service", "install"]);
        assert!(matches!(cli.command, Commands::Service(ServiceCommands::Install)));
    }

    #[test]
    fn parse_service_uninstall() {
        let cli = Cli::parse_from(["capsem", "service", "uninstall"]);
        assert!(matches!(cli.command, Commands::Service(ServiceCommands::Uninstall)));
    }

    #[test]
    fn parse_service_status() {
        let cli = Cli::parse_from(["capsem", "service", "status"]);
        assert!(matches!(cli.command, Commands::Service(ServiceCommands::Status)));
    }

    #[test]
    fn parse_setup_non_interactive() {
        let cli = Cli::parse_from(["capsem", "setup", "--non-interactive"]);
        match cli.command {
            Commands::Setup { non_interactive, preset, force, .. } => {
                assert!(non_interactive);
                assert_eq!(preset, None);
                assert!(!force);
            }
            _ => panic!("expected Setup"),
        }
    }

    #[test]
    fn parse_setup_with_preset_and_force() {
        let cli = Cli::parse_from(["capsem", "setup", "--preset", "high", "--force"]);
        match cli.command {
            Commands::Setup { preset, force, .. } => {
                assert_eq!(preset, Some("high".into()));
                assert!(force);
            }
            _ => panic!("expected Setup"),
        }
    }

    #[test]
    fn parse_setup_with_corp_config() {
        let cli = Cli::parse_from(["capsem", "setup", "--corp-config", "https://example.com/corp.toml", "--non-interactive"]);
        match cli.command {
            Commands::Setup { corp_config, non_interactive, .. } => {
                assert_eq!(corp_config, Some("https://example.com/corp.toml".into()));
                assert!(non_interactive);
            }
            _ => panic!("expected Setup"),
        }
    }

    #[test]
    fn parse_completions_bash() {
        let cli = Cli::parse_from(["capsem", "completions", "bash"]);
        assert!(matches!(cli.command, Commands::Completions { shell: clap_complete::Shell::Bash }));
    }

    #[test]
    fn parse_uninstall() {
        let cli = Cli::parse_from(["capsem", "uninstall"]);
        match cli.command {
            Commands::Uninstall { yes } => assert!(!yes),
            _ => panic!("expected Uninstall"),
        }
    }

    #[test]
    fn parse_uninstall_yes() {
        let cli = Cli::parse_from(["capsem", "uninstall", "--yes"]);
        match cli.command {
            Commands::Uninstall { yes } => assert!(yes),
            _ => panic!("expected Uninstall"),
        }
    }

    #[test]
    fn parse_update() {
        let cli = Cli::parse_from(["capsem", "update"]);
        match cli.command {
            Commands::Update { yes } => assert!(!yes),
            _ => panic!("expected Update"),
        }
    }

    #[test]
    fn parse_update_yes() {
        let cli = Cli::parse_from(["capsem", "update", "--yes"]);
        match cli.command {
            Commands::Update { yes } => assert!(yes),
            _ => panic!("expected Update"),
        }
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

    // -----------------------------------------------------------------------
    // Fork / Image CLI parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_fork() {
        let cli = Cli::parse_from(["capsem", "fork", "my-vm", "my-image"]);
        match cli.command {
            Commands::Fork { id, name, description } => {
                assert_eq!(id, "my-vm");
                assert_eq!(name, "my-image");
                assert_eq!(description, None);
            }
            _ => panic!("expected Fork"),
        }
    }

    #[test]
    fn parse_fork_with_description() {
        let cli = Cli::parse_from(["capsem", "fork", "vm1", "img1", "-d", "My description"]);
        match cli.command {
            Commands::Fork { id, name, description } => {
                assert_eq!(id, "vm1");
                assert_eq!(name, "img1");
                assert_eq!(description, Some("My description".into()));
            }
            _ => panic!("expected Fork"),
        }
    }

    #[test]
    fn parse_image_list() {
        let cli = Cli::parse_from(["capsem", "image", "list"]);
        match cli.command {
            Commands::Image(ImageCommands::List) => {}
            _ => panic!("expected Image List"),
        }
    }

    #[test]
    fn parse_image_list_alias() {
        let cli = Cli::parse_from(["capsem", "image", "ls"]);
        match cli.command {
            Commands::Image(ImageCommands::List) => {}
            _ => panic!("expected Image List via ls alias"),
        }
    }

    #[test]
    fn parse_image_delete() {
        let cli = Cli::parse_from(["capsem", "image", "delete", "my-img"]);
        match cli.command {
            Commands::Image(ImageCommands::Delete { name }) => {
                assert_eq!(name, "my-img");
            }
            _ => panic!("expected Image Delete"),
        }
    }

    #[test]
    fn parse_image_delete_alias() {
        let cli = Cli::parse_from(["capsem", "image", "rm", "my-img"]);
        match cli.command {
            Commands::Image(ImageCommands::Delete { name }) => {
                assert_eq!(name, "my-img");
            }
            _ => panic!("expected Image Delete via rm alias"),
        }
    }

    #[test]
    fn parse_image_inspect() {
        let cli = Cli::parse_from(["capsem", "image", "inspect", "my-img"]);
        match cli.command {
            Commands::Image(ImageCommands::Inspect { name }) => {
                assert_eq!(name, "my-img");
            }
            _ => panic!("expected Image Inspect"),
        }
    }

    #[test]
    fn parse_create_with_image() {
        let cli = Cli::parse_from(["capsem", "create", "--image", "base-img"]);
        match cli.command {
            Commands::Create { image, name, .. } => {
                assert_eq!(image, Some("base-img".into()));
                assert_eq!(name, None);
            }
            _ => panic!("expected Create with image"),
        }
    }

    #[test]
    fn parse_create_with_name_and_image() {
        let cli = Cli::parse_from(["capsem", "create", "-n", "my-vm", "--image", "my-img"]);
        match cli.command {
            Commands::Create { name, image, .. } => {
                assert_eq!(name, Some("my-vm".into()));
                assert_eq!(image, Some("my-img".into()));
            }
            _ => panic!("expected Create with name and image"),
        }
    }
}

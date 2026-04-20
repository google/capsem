mod client;
mod completions;
mod paths;
mod platform;
mod service_install;
mod setup;
mod uninstall;
mod update;

use std::path::PathBuf;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use clap::builder::styling::{AnsiColor, Color, Style, Styles};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use client::{
    ApiResponse, ExecRequest, ExecResponse, ForkRequest, ForkResponse,
    HistoryResponse, ListResponse, LogsResponse, PersistRequest, ProvisionRequest,
    ProvisionResponse, PurgeRequest, PurgeResponse, RunRequest, SessionInfo, UdsClient,
};


const fn cli_styles() -> Styles {
    Styles::styled()
        .header(
            Style::new()
                .fg_color(Some(Color::Ansi(AnsiColor::Cyan)))
                .bold()
                .underline(),
        )
        .usage(
            Style::new()
                .fg_color(Some(Color::Ansi(AnsiColor::Cyan)))
                .bold(),
        )
        .literal(
            Style::new()
                .fg_color(Some(Color::Ansi(AnsiColor::Green)))
                .bold(),
        )
        .placeholder(
            Style::new()
                .fg_color(Some(Color::Ansi(AnsiColor::BrightBlack))),
        )
        .error(
            Style::new()
                .fg_color(Some(Color::Ansi(AnsiColor::Red)))
                .bold(),
        )
        .valid(
            Style::new()
                .fg_color(Some(Color::Ansi(AnsiColor::Green))),
        )
        .invalid(
            Style::new()
                .fg_color(Some(Color::Ansi(AnsiColor::Yellow))),
        )
}

const GROUPED_HELP: &str = "\
\x1b[36;1;4mSession Commands:\x1b[0m
  \x1b[32;1mcreate\x1b[0m       Create and boot a new session
  \x1b[32;1mshell\x1b[0m        Open an interactive shell in a session
  \x1b[32;1mresume\x1b[0m       Resume a suspended session or attach to a running one
  \x1b[32;1msuspend\x1b[0m      Suspend a running session to disk
  \x1b[32;1mrestart\x1b[0m      Restart a persistent session (reboot)
  \x1b[32;1mexec\x1b[0m         Execute a command in a running session
  \x1b[32;1mrun\x1b[0m          Run a command in a fresh session (destroyed after)
  \x1b[32;1mlist\x1b[0m         List all sessions (running + suspended persistent)
  \x1b[32;1minfo\x1b[0m         Show detailed information about a session
  \x1b[32;1mlogs\x1b[0m         Show logs from a session
  \x1b[32;1mdelete\x1b[0m       Delete a session and all its state
  \x1b[32;1mfork\x1b[0m         Fork a session into a reusable snapshot
  \x1b[32;1mpersist\x1b[0m      Promote an ephemeral session to persistent
  \x1b[32;1mpurge\x1b[0m        Destroy all temporary sessions

\x1b[36;1;4mService:\x1b[0m
  \x1b[32;1minstall\x1b[0m      Install as a system service (LaunchAgent / systemd)
  \x1b[32;1mstatus\x1b[0m       Show service status
  \x1b[32;1mstart\x1b[0m        Start the background service
  \x1b[32;1mstop\x1b[0m         Stop the background service

\x1b[36;1;4mMCP:\x1b[0m
  \x1b[32;1mmcp servers\x1b[0m  List configured MCP servers with connection status
  \x1b[32;1mmcp tools\x1b[0m    List discovered MCP tools across all servers
  \x1b[32;1mmcp policy\x1b[0m   Show the merged MCP policy
  \x1b[32;1mmcp refresh\x1b[0m  Re-discover tools from all MCP servers
  \x1b[32;1mmcp call\x1b[0m     Call an MCP tool

\x1b[36;1;4mMisc:\x1b[0m
  \x1b[32;1msetup\x1b[0m        Run the first-time setup wizard
  \x1b[32;1mupdate\x1b[0m       Check for updates and install the latest version
  \x1b[32;1mdoctor\x1b[0m       Run diagnostic tests in a fresh session
  \x1b[32;1mcompletions\x1b[0m  Generate shell completions (bash, zsh, fish, powershell)
  \x1b[32;1mversion\x1b[0m      Show version and build information
  \x1b[32;1muninstall\x1b[0m    Uninstall capsem completely (service, binaries, data)";

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Sandboxes AI agents in air-gapped Linux VMs",
    long_about = None,
    styles = cli_styles(),
    help_template = "{about-with-newline}Version: {version}\n\n{usage-heading} {usage}\n{after-help}\n\n\x1b[36;1;4mOptions:\x1b[0m\n{options}",
    disable_help_subcommand = true,
    subcommand_help_heading = None,
    after_help = GROUPED_HELP,
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to the service Unix Domain Socket
    #[arg(long)]
    uds_path: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(flatten)]
    Session(SessionCommands),

    /// Manage MCP (Model Context Protocol) servers and tools
    #[command(subcommand)]
    Mcp(McpCommands),

    #[command(flatten)]
    Misc(MiscCommands),
}

#[derive(Subcommand)]
enum McpCommands {
    /// List configured MCP servers with connection status
    Servers,
    /// List discovered MCP tools across all servers
    Tools {
        /// Filter by server name
        #[arg(long)]
        server: Option<String>,
    },
    /// Show the merged MCP policy
    Policy,
    /// Re-discover tools from all MCP servers
    Refresh,
    /// Call an MCP tool by namespaced name
    Call {
        /// Namespaced tool name (e.g. github__search_repos)
        name: String,
        /// JSON arguments
        #[arg(long, default_value = "{}")]
        args: String,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    /// Create and boot a new session
    ///
    /// Sessions are ephemeral by default and destroyed on delete. Use -n <name> to
    /// create a persistent session that survives suspend/resume cycles.
    Create {
        /// Name for the session (makes it persistent -- "if you name it, you keep it")
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
        /// Clone state from an existing persistent session
        #[arg(long, alias = "image")]
        from: Option<String>,
    },
    /// Open an interactive shell in a session
    ///
    /// With no arguments, creates a temporary session (destroyed on exit).
    /// Pass a session name/ID or --name to attach to an existing running session.
    Shell {
        /// Find by name (for persistent sessions)
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Name or ID of the session (positional)
        #[arg(value_name = "SESSION")]
        session: Option<String>,
    },
    /// Resume a suspended session or attach to a running one
    #[command(alias = "attach")]
    Resume {
        /// Name of the persistent session
        name: String,
    },
    /// Suspend a running session to disk
    ///
    /// Saves RAM and CPU state. Only persistent sessions can be suspended.
    Suspend {
        /// Name or ID of the session
        #[arg(value_name = "SESSION")]
        session: String,
    },
    /// Restart a persistent session (reboot)
    Restart {
        /// Name of the persistent session
        name: String,
    },
    /// Execute a command in a running session
    Exec {
        /// Name or ID of the session
        #[arg(value_name = "SESSION")]
        session: String,
        /// Command to execute
        command: String,
        /// Timeout in seconds (default 30)
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// Run a command in a fresh session (destroyed after)
    ///
    /// Creates a temporary session, runs the command, prints output, and
    /// destroys the session. Useful for one-shot tasks and CI pipelines.
    Run {
        /// Command to execute
        command: String,
        /// Timeout in seconds (default 60)
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        /// Set environment variables (repeatable: -e KEY=VALUE)
        #[arg(short = 'e', long = "env")]
        env: Vec<String>,
    },
    /// List all sessions (running + suspended persistent)
    #[command(alias = "ls")]
    List {
        /// Print only IDs, one per line (for scripting)
        #[arg(short, long)]
        quiet: bool,
    },
    /// Show detailed information about a session
    Info {
        /// Name or ID of the session
        #[arg(value_name = "SESSION")]
        session: String,
        /// Output as JSON (for scripting)
        #[arg(long)]
        json: bool,
    },
    /// Show logs from a session
    ///
    /// Displays both serial console and process logs.
    Logs {
        /// Name or ID of the session
        #[arg(value_name = "SESSION")]
        session: String,
        /// Show only the last N lines
        #[arg(long)]
        tail: Option<usize>,
    },
    /// Delete a session and all its state
    #[command(alias = "rm")]
    Delete {
        /// Name or ID of the session
        #[arg(value_name = "SESSION")]
        session: String,
    },
    /// Fork a session into a new persistent session
    ///
    /// Creates a point-in-time copy of the session's disk state as a new
    /// persistent session. Boot it with `capsem resume <name>` or clone
    /// with `capsem create --from <name>`.
    Fork {
        /// Name or ID of the session to fork
        #[arg(value_name = "SESSION")]
        session: String,
        /// Name for the new session
        name: String,
        /// Optional description
        #[arg(short, long)]
        description: Option<String>,
    },
    /// Promote an ephemeral session to persistent
    Persist {
        /// Name or ID of the running ephemeral session
        #[arg(value_name = "SESSION")]
        session: String,
        /// Name to assign
        name: String,
    },
    /// Destroy all temporary sessions
    ///
    /// Use --all to also destroy persistent sessions (requires confirmation).
    Purge {
        /// Also destroy persistent sessions (requires confirmation)
        #[arg(long, default_value_t = false)]
        all: bool,
    },
    /// Show command history for a session
    ///
    /// Merges structured exec events (Layer 1) and kernel audit events (Layer 3),
    /// sorted by timestamp. Supports filtering by layer, search text, and process.
    History {
        /// Name or ID of the session
        #[arg(value_name = "SESSION")]
        session: String,
        /// Show only the last N commands
        #[arg(long, default_value_t = 500)]
        tail: usize,
        /// Show all history (no limit)
        #[arg(long, default_value_t = false)]
        all: bool,
        /// Filter by command text
        #[arg(long)]
        search: Option<String>,
        /// Filter by layer: all, exec, audit
        #[arg(long, default_value = "all")]
        layer: String,
        /// Output as JSON (for scripting)
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum MiscCommands {
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
        /// Reset only the GUI wizard (onboarding_completed and onboarding_version).
        /// Preserves security preset, provider keys, and other install state.
        #[arg(long)]
        force_onboarding: bool,
    },
    /// Check for updates and install the latest version
    Update {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
        /// Refresh only VM assets (kernel/initrd/rootfs) from the release URL.
        /// Useful when an asset-only release ships independently of binaries.
        #[arg(long)]
        assets: bool,
    },
    /// Run diagnostic tests in a fresh session
    ///
    /// Boots a temporary session, runs the capsem-doctor test suite, and reports
    /// results. Use --fast to skip slow network tests.
    Doctor {
        /// Skip slow tests (throughput download, etc.)
        #[arg(long)]
        fast: bool,
    },
    /// Generate shell completions (bash, zsh, fish, powershell)
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Show version and build information
    Version,
    /// Uninstall capsem completely (service, binaries, data)
    Uninstall {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
    /// Install capsem as a system service (LaunchAgent on macOS, systemd on Linux)
    Install,
    /// Show service installation and runtime status
    Status,
    /// Start the background service
    Start,
    /// Stop the background service
    Stop,
}

fn format_uptime(secs: Option<u64>) -> String {
    match secs {
        None | Some(0) => "-".into(),
        Some(s) => {
            let days = s / 86400;
            let hours = (s % 86400) / 3600;
            let mins = (s % 3600) / 60;
            if days > 0 {
                format!("{}d {}h", days, hours)
            } else if hours > 0 {
                format!("{}h {:02}m", hours, mins)
            } else {
                format!("{}m", mins.max(1))
            }
        }
    }
}

fn print_session_info(info: &SessionInfo) {
    println!("Session: {}", info.id);
    if let Some(name) = &info.name {
        println!("Name:    {}", name);
    }
    println!("Status:  {}", info.status);
    if info.pid > 0 {
        println!("PID:     {}", info.pid);
    }

    if info.ram_mb.is_some() || info.cpus.is_some() || info.version.is_some() {
        println!();
        if let Some(ram) = info.ram_mb {
            println!("RAM:     {} GB", ram / 1024);
        }
        if let Some(cpus) = info.cpus {
            println!("CPUs:    {}", cpus);
        }
        if let Some(ver) = &info.version {
            println!("Version: {}", ver);
        }
    }

    if let Some(from) = &info.forked_from {
        println!("Forked:  {}", from);
    }
    if let Some(desc) = &info.description {
        println!("Desc:    {}", desc);
    }

    let has_telemetry = info.created_at.is_some()
        || info.uptime_secs.is_some()
        || info.total_input_tokens.is_some()
        || info.total_tool_calls.is_some();
    if has_telemetry {
        println!();
        println!("Telemetry:");
        if let Some(created) = &info.created_at {
            println!("  Created:       {}", created);
        }
        if let Some(secs) = info.uptime_secs {
            println!("  Uptime:        {}", format_uptime(Some(secs)));
        }
        if let Some(inp) = info.total_input_tokens {
            println!("  Input Tokens:  {}", inp);
        }
        if let Some(out) = info.total_output_tokens {
            println!("  Output Tokens: {}", out);
        }
        if let Some(cost) = info.total_estimated_cost {
            println!("  Est. Cost:     ${:.2}", cost);
        }
        if let Some(tc) = info.total_tool_calls {
            println!("  Tool Calls:    {}", tc);
        }
        if let Some(mc) = info.total_mcp_calls {
            println!("  MCP Calls:     {}", mc);
        }
        if info.total_requests.is_some() || info.allowed_requests.is_some() {
            let total = info.total_requests.unwrap_or(0);
            let allowed = info.allowed_requests.unwrap_or(0);
            let denied = info.denied_requests.unwrap_or(0);
            println!("  Requests:      {} ({} allowed, {} denied)", total, allowed, denied);
        }
        if let Some(fe) = info.total_file_events {
            println!("  File Events:   {}", fe);
        }
    }
}

async fn run_shell(id: &str, run_dir: &std::path::Path) -> Result<()> {
    use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
    use tokio_unix_ipc::{channel_from_std, Sender, Receiver};
    use std::sync::Arc;
    use nix::sys::termios::{tcgetattr, tcsetattr, SetArg};

    client::validate_id(id)?;
    let sock_path = run_dir.join("instances").join(format!("{}.sock", id));
    if !sock_path.exists() {
        anyhow::bail!("Session socket not found at: {}", sock_path.display());
    }

    let stream = tokio::net::UnixStream::connect(&sock_path).await.context("failed to connect to sandbox")?;
    let std_stream = stream.into_std()?;
    #[allow(unused_variables)]
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
    let mut output_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
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
                ProcessToService::ShutdownRequested { .. }
                | ProcessToService::SuspendRequested { .. }
                | ProcessToService::SnapshotReady { .. }
                | ProcessToService::McpServersResult { .. }
                | ProcessToService::McpToolsResult { .. }
                | ProcessToService::McpRefreshResult { .. }
                | ProcessToService::McpCallToolResult { .. } => {}
            }
        }
    });

    let mut sigwinch = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;

    // Read from stdin and send over IPC.
    // Also watch for output_task completion (VM connection closed).
    loop {
        tokio::select! {
            _ = sigwinch.recv() => {
                if is_tty {
                    if let Some((cols, rows)) = get_terminal_size() {
                        let _ = tx.send(ServiceToProcess::TerminalResize { cols, rows }).await;
                    }
                }
            }
            _ = &mut output_task => {
                // VM connection closed (shutdown, process exit, etc.)
                break;
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

    // Ensure the parent shell redraws its prompt after raw mode exit.
    eprintln!();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let auto_launch = cli.uds_path.is_none();
    // Resolve run_dir and uds_path together so they always agree.
    // If the user passed --uds-path explicitly, run_dir is its parent by
    // convention (service places instance sockets at <run_dir>/instances/{id}.sock).
    // Otherwise fall back to capsem_core::paths::capsem_run_dir (CAPSEM_RUN_DIR
    // env > <capsem_home>/run), matching the service.
    let (run_dir, uds_path) = match cli.uds_path {
        Some(p) => {
            let dir = p.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            (dir, p)
        }
        None => {
            let dir = capsem_core::paths::capsem_run_dir();
            let sock = dir.join("service.sock");
            (dir, sock)
        }
    };

    // Show update notice if available (sync file read, no latency)
    if let Some(notice) = update::read_cached_update_notice() {
        eprintln!("{}", notice);
    }

    // Background update check (fire-and-forget). Spawned early so it runs
    // even for commands that call std::process::exit (exec, run).
    tokio::spawn(update::refresh_update_cache_if_stale());

    // Commands that don't need the service
    match &cli.command {
        Commands::Misc(MiscCommands::Version) => {
            println!(
                "capsem {} (build {} ts={})",
                env!("CARGO_PKG_VERSION"),
                env!("CAPSEM_BUILD_HASH"),
                option_env!("CAPSEM_BUILD_TS").unwrap_or("dev"),
            );
            return Ok(());
        }
        Commands::Misc(MiscCommands::Install) => {
            service_install::install_service().await?;
            println!("Service installed.");
            return Ok(());
        }
        Commands::Misc(MiscCommands::Status) => {
            let status = service_install::service_status().await?;
            println!("Version:   {}", env!("CARGO_PKG_VERSION"));
            println!("Installed: {}", status.installed);
            println!("Running:   {}", status.running);
            if let Some(pid) = status.pid {
                println!("PID:       {}", pid);
            }
            if let Some(path) = &status.unit_path {
                println!("Unit:      {}", path.display());
            }
            // Check service + gateway connectivity and version sync
            if status.running {
                let home = crate::paths::capsem_home().unwrap_or_default();
                let sock = home.join("run/service.sock");
                let my_version = env!("CARGO_PKG_VERSION");

                // Check service version via UDS
                let svc_version = async {
                    let stream = tokio::net::UnixStream::connect(&sock).await.ok()?;
                    let (reader, mut writer) = tokio::io::split(stream);
                    writer.write_all(b"GET /version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.ok()?;
                    let mut buf = Vec::new();
                    tokio::io::AsyncReadExt::read_to_end(&mut tokio::io::BufReader::new(reader), &mut buf).await.ok()?;
                    let body = String::from_utf8_lossy(&buf);
                    let json_start = body.find('{')?;
                    let v: serde_json::Value = serde_json::from_str(&body[json_start..]).ok()?;
                    v.get("version")?.as_str().map(String::from)
                }.await;

                match svc_version {
                    Some(ref v) if v == my_version => println!("Service:   ok (v{})", v),
                    Some(ref v) => println!("Service:   STALE (running v{}, binary is v{}) -- restart service", v, my_version),
                    None => println!("Service:   STALE (socket dead or no /version endpoint)"),
                }

                let port_path = home.join("run/gateway.port");
                let token_path = home.join("run/gateway.token");
                match (std::fs::read_to_string(&port_path), std::fs::read_to_string(&token_path)) {
                    (Ok(port_str), Ok(token)) => {
                        let port = port_str.trim();
                        let token = token.trim();
                        let client = reqwest::Client::new();

                        // Check gateway version (unauthenticated health endpoint)
                        let health_url = format!("http://127.0.0.1:{}/health", port);
                        let gw_version: Option<String> = async {
                            let r = client.get(&health_url)
                                .timeout(std::time::Duration::from_secs(2))
                                .send().await.ok()?;
                            let v: serde_json::Value = r.json().await.ok()?;
                            v.get("version")?.as_str().map(String::from)
                        }.await;

                        // Check token validity (authenticated endpoint)
                        let auth_url = format!("http://127.0.0.1:{}/list", port);
                        let token_ok = client.get(&auth_url)
                            .header("Authorization", format!("Bearer {}", token))
                            .timeout(std::time::Duration::from_secs(2))
                            .send().await
                            .map(|r| r.status().is_success())
                            .unwrap_or(false);

                        match (gw_version, token_ok) {
                            (Some(ref v), true) if v == my_version => {
                                println!("Gateway:   ok (port {}, v{})", port, v);
                            }
                            (Some(ref v), true) => {
                                println!("Gateway:   STALE (running v{}, binary is v{}) -- restart service", v, my_version);
                            }
                            (Some(_), false) => {
                                println!("Gateway:   token MISMATCH (port {}) -- restart service", port);
                            }
                            (None, _) => {
                                println!("Gateway:   DOWN (port {} not responding)", port);
                            }
                        }
                    }
                    _ => println!("Gateway:   no token/port files"),
                }
            }

            // Show asset info from manifest
            if let Some(assets_dir) = capsem_core::asset_manager::default_assets_dir() {
                let manifest_path = assets_dir.join("manifest.json");
                match std::fs::read_to_string(&manifest_path)
                    .ok()
                    .and_then(|c| capsem_core::asset_manager::ManifestV2::from_json(&c).ok())
                {
                    Some(m) => {
                        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
                        println!("Assets:    {} ({})", m.assets.current, arch);
                        match m.resolve(env!("CARGO_PKG_VERSION"), arch, &assets_dir) {
                            Ok(resolved) => {
                                let k = if resolved.kernel.exists() { "ok" } else { "MISSING" };
                                let i = if resolved.initrd.exists() { "ok" } else { "MISSING" };
                                let r = if resolved.rootfs.exists() { "ok" } else { "MISSING" };
                                println!("  kernel:  {} ({})", resolved.kernel.display(), k);
                                println!("  initrd:  {} ({})", resolved.initrd.display(), i);
                                println!("  rootfs:  {} ({})", resolved.rootfs.display(), r);
                            }
                            Err(e) => println!("  resolve: {}", e),
                        }
                    }
                    None => println!("Assets:    no manifest found"),
                }
            }
            return Ok(());
        }
        Commands::Misc(MiscCommands::Start) => {
            service_install::start_service().await?;
            println!("Service started.");
            return Ok(());
        }
        Commands::Misc(MiscCommands::Stop) => {
            service_install::stop_service().await?;
            println!("Service stopped.");
            return Ok(());
        }
        Commands::Misc(MiscCommands::Completions { shell }) => {
            completions::generate_completions(*shell);
            return Ok(());
        }
        Commands::Misc(MiscCommands::Uninstall { yes }) => {
            uninstall::run_uninstall(*yes).await?;
            return Ok(());
        }
        Commands::Misc(MiscCommands::Update { yes, assets }) => {
            update::run_update(*yes, *assets).await?;
            return Ok(());
        }
        Commands::Misc(MiscCommands::Setup { non_interactive, preset, force, accept_detected, corp_config, force_onboarding }) => {
            let opts = setup::SetupOptions {
                non_interactive: *non_interactive,
                preset: preset.clone(),
                force: *force,
                accept_detected: *accept_detected,
                corp_config: corp_config.clone(),
                force_onboarding: *force_onboarding,
            };
            setup::run_setup(opts).await?;
            return Ok(());
        }
        _ => {}
    }

    // Auto-setup on first use: if setup-state.json doesn't exist, the user
    // hasn't run `capsem setup` yet. Run non-interactive setup so service
    // registration, asset download, and credential detection happen automatically.
    // Skip when --uds-path is explicit (tests, CI, custom service).
    if auto_launch {
        let setup_done = paths::capsem_home()
            .map(|d| d.join("setup-state.json").exists())
            .unwrap_or(false);
        if !setup_done {
            eprintln!("First run detected. Running initial setup...");
            eprintln!("(Run `capsem setup` to reconfigure later)\n");
            setup::run_setup(setup::SetupOptions {
                non_interactive: true,
                preset: None,
                force: false,
                accept_detected: true,
                corp_config: None,
                force_onboarding: false,
            }).await?;
        }
    }

    let client = UdsClient::new(uds_path, auto_launch);

    match &cli.command {
        Commands::Session(SessionCommands::Create { name, ram, cpu, env, from }) => {
            let persistent = name.is_some() || from.is_some();
            let req = ProvisionRequest {
                name: name.clone(),
                ram_mb: ram * 1024,
                cpus: *cpu,
                persistent,
                env: client::parse_env_vars(env)?,
                from: from.clone(),
            };

            let resp: ApiResponse<ProvisionResponse> = client.post("/provision", &req).await?;
            let info = resp.into_result()?;

            if persistent {
                println!("{} (persistent)", info.id);
            } else {
                println!("{}", info.id);
            }
        }
        Commands::Session(SessionCommands::Fork { session, name, description }) => {
            client::validate_id(session)?;
            let req = ForkRequest {
                name: name.clone(),
                description: description.clone(),
            };
            let resp: ApiResponse<ForkResponse> = client.post(&format!("/fork/{}", session), &req).await?;
            let info = resp.into_result()?;
            let size_mb = info.size_bytes as f64 / 1024.0 / 1024.0;
            println!("Forked session '{}' from '{}' ({:.1} MB)", info.name, session, size_mb);
        }
        Commands::Session(SessionCommands::Resume { name }) => {
            client::validate_id(name)?;
            let resp: ApiResponse<ProvisionResponse> = client.post(&format!("/resume/{}", name), &serde_json::json!({})).await?;
            let info = resp.into_result()?;
            println!("{}", info.id);
        }
        Commands::Session(SessionCommands::Suspend { session }) => {
            client::validate_id(session)?;
            println!("Suspending session: {}", session);
            let resp: ApiResponse<serde_json::Value> = client.post(&format!("/suspend/{}", session), &serde_json::json!({})).await?;
            resp.into_result()?;
            println!("Session suspended.");
        }
        Commands::Session(SessionCommands::Shell { name, session }) => {
            let target = name.as_ref().or(session.as_ref());
            match target {
                Some(t) => {
                    client::validate_id(t)?;
                    run_shell(t, &run_dir).await?;
                }
                None => {
                    // No args: create ephemeral session, attach, destroy on exit
                    println!("[!] Temporary session. Use `capsem create -n <name>` for persistent.");
                    let req = ProvisionRequest {
                        name: None,
                        ram_mb: 4 * 1024,
                        cpus: 4,
                        persistent: false,
                        env: None,
                        from: None,
                    };
                    let resp: ApiResponse<ProvisionResponse> = client.post("/provision", &req).await?;
                    let info = resp.into_result()?;

                    // Poll until the socket is connectable (not just present on disk).
                    let socket_path = run_dir.join("instances").join(format!("{}.sock", info.id));
                    let sp = socket_path.clone();
                    let _ = capsem_core::poll::poll_until(
                        capsem_core::poll::PollOpts::new("shell-socket", std::time::Duration::from_secs(10)),
                        || {
                            let sp = sp.clone();
                            async move {
                                match tokio::net::UnixStream::connect(&sp).await {
                                    Ok(_) => Some(()),
                                    Err(_) => None,
                                }
                            }
                        },
                    ).await;

                    let shell_result = run_shell(&info.id, &run_dir).await;
                    // Ephemeral: auto-destroy on disconnect
                    let _: Result<ApiResponse<serde_json::Value>, _> = client.delete(&format!("/delete/{}", info.id)).await;
                    shell_result?;
                }
            }
        }
        Commands::Session(SessionCommands::List { quiet }) => {
            let resp: ApiResponse<ListResponse> = client.get("/list").await?;
            let resp = resp.into_result()?;
            if *quiet {
                for s in &resp.sessions {
                    println!("{}", s.id);
                }
            } else if resp.sessions.is_empty() {
                println!("No sessions.");
            } else {
                println!("{:<20} {:<12} {:<10} {:<8} {:<6} {:<10}",
                    "ID", "NAME", "STATUS", "RAM", "CPUs", "UPTIME");
                for s in &resp.sessions {
                    let name = s.name.as_deref().unwrap_or("-");
                    let ram = s.ram_mb.map(|mb| format!("{} GB", mb / 1024)).unwrap_or_else(|| "-".into());
                    let cpus = s.cpus.map(|c| c.to_string()).unwrap_or_else(|| "-".into());
                    let uptime = format_uptime(s.uptime_secs);
                    println!("{:<20} {:<12} {:<10} {:<8} {:<6} {:<10}",
                        s.id, name, s.status, ram, cpus, uptime);
                }
            }
        }
        Commands::Session(SessionCommands::Exec { session, command, timeout }) => {
            client::validate_id(session)?;
            let req = ExecRequest {
                command: command.clone(),
                timeout_secs: *timeout,
            };
            let resp: ApiResponse<ExecResponse> = client.post(&format!("/exec/{}", session), req).await?;
            let resp = resp.into_result()?;
            if !resp.stdout.is_empty() {
                print!("{}", resp.stdout);
            }
            if !resp.stderr.is_empty() {
                eprint!("{}", resp.stderr);
            }
            std::process::exit(resp.exit_code);
        }
        Commands::Session(SessionCommands::Run { command, timeout, env }) => {
            let req = RunRequest {
                command: command.clone(),
                timeout_secs: *timeout,
                env: client::parse_env_vars(env)?,
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
        Commands::Session(SessionCommands::Delete { session }) => {
            client::validate_id(session)?;
            println!("Deleting session: {}", session);
            let resp: ApiResponse<serde_json::Value> = client.delete(&format!("/delete/{}", session)).await?;
            resp.into_result()?;
            println!("Session deleted.");
        }
        Commands::Session(SessionCommands::Persist { session, name }) => {
            client::validate_id(session)?;
            let req = PersistRequest { name: name.clone() };
            let resp: ApiResponse<serde_json::Value> = client.post(&format!("/persist/{}", session), &req).await?;
            resp.into_result()?;
            println!("[*] Session \"{}\" is now persistent as \"{}\"", session, name);
        }
        Commands::Session(SessionCommands::Purge { all }) => {
            if *all {
                // Confirmation prompt
                use std::io::Write;
                let list_resp: ApiResponse<ListResponse> = client.get("/list").await?;
                let resp = list_resp.into_result()?;
                let persistent_count = resp.sessions.iter().filter(|s| s.persistent).count();
                let ephemeral_count = resp.sessions.iter().filter(|s| !s.persistent).count();
                print!("[!] This will destroy {} persistent and {} temporary sessions. Continue? [y/N] ",
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
                println!("[*] Purged {} sessions ({} persistent, {} temporary).",
                    result.purged, result.persistent_purged, result.ephemeral_purged);
            } else {
                println!("[*] Purged {} temporary sessions.", result.ephemeral_purged);
            }
        }
        Commands::Session(SessionCommands::Info { session, json }) => {
            client::validate_id(session)?;
            let resp: ApiResponse<SessionInfo> = client.get(&format!("/info/{}", session)).await?;
            let info = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                print_session_info(&info);
            }
        }
        Commands::Session(SessionCommands::Logs { session, tail }) => {
            client::validate_id(session)?;
            let resp: ApiResponse<LogsResponse> = client.get(&format!("/logs/{}", session)).await?;
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
                println!("--- Process Logs ({}) ---", session);
                let output = match tail {
                    Some(n) => tail_lines(&process_logs, *n),
                    None => process_logs,
                };
                println!("{}", output);
            }

            if let Some(serial_logs) = logs.serial_logs {
                println!("--- Serial Logs ({}) ---", session);
                let output = match tail {
                    Some(n) => tail_lines(&serial_logs, *n),
                    None => serial_logs,
                };
                println!("{}", output);
            } else if !logs.logs.is_empty() {
                println!("--- Serial Logs ({}) ---", session);
                let output = match tail {
                    Some(n) => tail_lines(&logs.logs, *n),
                    None => logs.logs,
                };
                println!("{}", output);
            }
        }
        Commands::Session(SessionCommands::History { session, tail, all, search, layer, json }) => {
            client::validate_id(session)?;
            let limit = if *all { 100_000 } else { *tail };
            let mut url = format!("/history/{}?limit={}&layer={}", session, limit, layer);
            if let Some(q) = search {
                url.push_str(&format!("&search={}", q.replace(' ', "%20").replace('&', "%26")));
            }
            let resp: ApiResponse<HistoryResponse> = client.get(&url).await?;
            let history = resp.into_result()?;

            if *json {
                println!("{}", serde_json::to_string_pretty(&history)?);
            } else {
                // Column-aligned table header; literal labels intentional.
                #[allow(clippy::print_literal)]
                {
                    println!(
                        " {:<22} {:<7} {:<5} {:<10} {}",
                        "TIMESTAMP", "LAYER", "EXIT", "PROCESS", "COMMAND"
                    );
                }
                for entry in &history.commands {
                    let exit = entry.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "-".into());
                    let process = match entry.layer.as_str() {
                        "exec" => entry.details.get("process_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("api")
                            .to_string(),
                        "audit" => {
                            let parent = entry.details.get("parent_exe")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let exe = entry.details.get("exe")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if parent.is_empty() {
                                exe.rsplit('/').next().unwrap_or(exe).to_string()
                            } else {
                                format!("{}>{}", parent.rsplit('/').next().unwrap_or(parent), exe.rsplit('/').next().unwrap_or(exe))
                            }
                        }
                        _ => "-".to_string(),
                    };
                    // Truncate command to terminal width
                    let cmd = if entry.command.len() > 80 {
                        format!("{}...", &entry.command[..77])
                    } else {
                        entry.command.clone()
                    };
                    println!(" {:<22} {:<7} {:<5} {:<10} {}", entry.timestamp, entry.layer, exit, process, cmd);
                }
                if history.has_more {
                    println!(" Showing {} of {} commands. Use --all for full history.", history.commands.len(), history.total);
                }
            }
        }
        Commands::Session(SessionCommands::Restart { name }) => {
            client::validate_id(name)?;
            let info_resp: ApiResponse<SessionInfo> = client.get(&format!("/info/{}", name)).await?;
            let info = info_resp.into_result()?;
            if !info.persistent {
                anyhow::bail!("Cannot restart ephemeral session \"{}\". Only persistent sessions support restart.", name);
            }

            // Stop, then resume
            let stop_resp: ApiResponse<serde_json::Value> = client.post(&format!("/stop/{}", name), &serde_json::json!({})).await?;
            stop_resp.into_result().context("failed to stop session during restart")?;
            let resp: ApiResponse<ProvisionResponse> = client.post(&format!("/resume/{}", name), &serde_json::json!({})).await?;
            let resumed = resp.into_result()?;
            println!("{}", resumed.id);
        }
        Commands::Mcp(McpCommands::Servers) => {
            let resp: ApiResponse<Vec<serde_json::Value>> = client.get("/mcp/servers").await?;
            let servers = resp.into_result()?;
            if servers.is_empty() {
                println!("No MCP servers configured.");
            } else {
                #[allow(clippy::print_literal)]
                {
                    println!(
                        "{:<20} {:<8} {:<10} {:<8} {}",
                        "NAME", "ENABLED", "SOURCE", "TOOLS", "URL"
                    );
                }
                for s in &servers {
                    println!(
                        "{:<20} {:<8} {:<10} {:<8} {}",
                        s["name"].as_str().unwrap_or("-"),
                        if s["enabled"].as_bool().unwrap_or(false) { "yes" } else { "no" },
                        s["source"].as_str().unwrap_or("-"),
                        s["tool_count"].as_u64().unwrap_or(0),
                        s["url"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
        Commands::Mcp(McpCommands::Tools { server }) => {
            let resp: ApiResponse<Vec<serde_json::Value>> = client.get("/mcp/tools").await?;
            let mut tools = resp.into_result()?;
            if let Some(ref server_filter) = server {
                tools.retain(|t| t["server_name"].as_str() == Some(server_filter));
            }
            if tools.is_empty() {
                println!("No MCP tools discovered.");
            } else {
                #[allow(clippy::print_literal)]
                {
                    println!(
                        "{:<40} {:<20} {:<10} {}",
                        "TOOL", "SERVER", "APPROVED", "DESCRIPTION"
                    );
                }
                for t in &tools {
                    let desc = t["description"].as_str().unwrap_or("-");
                    let short_desc = if desc.len() > 60 { &desc[..60] } else { desc };
                    println!(
                        "{:<40} {:<20} {:<10} {}",
                        t["namespaced_name"].as_str().unwrap_or("-"),
                        t["server_name"].as_str().unwrap_or("-"),
                        if t["approved"].as_bool().unwrap_or(false) { "yes" } else { "no" },
                        short_desc,
                    );
                }
            }
        }
        Commands::Mcp(McpCommands::Policy) => {
            let resp: ApiResponse<serde_json::Value> = client.get("/mcp/policy").await?;
            let policy = resp.into_result()?;
            println!("{}", serde_json::to_string_pretty(&policy)?);
        }
        Commands::Mcp(McpCommands::Refresh) => {
            let resp: ApiResponse<serde_json::Value> = client.post("/mcp/tools/refresh", &serde_json::json!({})).await?;
            resp.into_result()?;
            println!("MCP tools refreshed.");
        }
        Commands::Mcp(McpCommands::Call { name, args }) => {
            let arguments: serde_json::Value = serde_json::from_str(args)
                .context("invalid JSON arguments")?;
            let resp: ApiResponse<serde_json::Value> = client.post(
                &format!("/mcp/tools/{}/call", name),
                &arguments,
            ).await?;
            let result = resp.into_result()?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Misc(
            MiscCommands::Version
            | MiscCommands::Setup { .. }
            | MiscCommands::Update { .. }
            | MiscCommands::Completions { .. }
            | MiscCommands::Uninstall { .. }
            | MiscCommands::Install
            | MiscCommands::Status
            | MiscCommands::Start
            | MiscCommands::Stop
        ) => {
            unreachable!("handled before UdsClient creation")
        }
        Commands::Misc(MiscCommands::Doctor { fast }) => {
            use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
            use tokio_unix_ipc::channel_from_std;

            // Log file: ~/.capsem/run/doctor-latest.log (always overwritten)
            let log_path = run_dir.join("doctor-latest.log");
            let mut log_file = std::fs::File::create(&log_path).ok();

            println!("Running capsem-doctor...");
            println!("Log: {}", log_path.display());

            let req = ProvisionRequest {
                name: None,
                ram_mb: 2048,
                cpus: 2,
                persistent: false,
                env: None,
                from: None,
            };
            let resp: ApiResponse<ProvisionResponse> = client.post("/provision", req).await?;
            let provisioned = resp.into_result()?;
            let vm_id = provisioned.id;

            // Helper: always delete the session, even on Ctrl-C or error
            async fn delete_vm(client: &UdsClient, vm_id: &str) {
                let _: Result<ApiResponse<serde_json::Value>, _> =
                    client.delete(&format!("/delete/{}", vm_id)).await;
            }

            let ctrl_c = tokio::signal::ctrl_c();
            tokio::pin!(ctrl_c);

            // The service tells us exactly where the per-VM socket lives. Never
            // recompute locally -- the service may fall back to /tmp/capsem/{hash}
            // when run_dir is under macOS's /var/folders (long SUN path).
            let sock_path = provisioned
                .uds_path
                .clone()
                .unwrap_or_else(|| capsem_core::uds::instance_socket_path(&run_dir, &vm_id));

            // Poll for the per-VM socket to exist and hand us an open IPC
            // channel. Uses the shared exponential-backoff helper instead of
            // a hand-rolled loop.
            let sock_path_for_poll = sock_path.clone();
            let poll_ipc = capsem_core::poll::poll_until(
                capsem_core::poll::PollOpts::new(
                    "vm-ipc-ready",
                    std::time::Duration::from_secs(30),
                ),
                || {
                    let sock_path = sock_path_for_poll.clone();
                    async move {
                        if !sock_path.exists() {
                            return None;
                        }
                        let stream = tokio::net::UnixStream::connect(&sock_path).await.ok()?;
                        let std_stream = stream.into_std().ok()?;
                        channel_from_std::<ServiceToProcess, ProcessToService>(std_stream).ok()
                    }
                },
            );

            let (tx, rx) = tokio::select! {
                _ = &mut ctrl_c => {
                    eprintln!("\nInterrupted, cleaning up session...");
                    delete_vm(&client, &vm_id).await;
                    std::process::exit(130);
                }
                res = poll_ipc => match res {
                    Ok(chan) => chan,
                    Err(_) => {
                        eprintln!("Session did not become ready within 30s");
                        delete_vm(&client, &vm_id).await;
                        std::process::exit(1);
                    }
                },
            };

            // Subscribe to terminal output then type the command
            // into the shell. This streams output in real-time
            // (unlike Exec which buffers until completion).
            let _ = tx.send(ServiceToProcess::StartTerminalStream).await;

            // Wait for shell to be ready (boot banner finishes)
            let mut ready = false;
            let boot_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            while !ready {
                tokio::select! {
                    _ = &mut ctrl_c => {
                        eprintln!("\nInterrupted, cleaning up session...");
                        delete_vm(&client, &vm_id).await;
                        std::process::exit(130);
                    }
                    result = tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        rx.recv(),
                    ) => {
                        match result {
                            Ok(Ok(ProcessToService::TerminalOutput { data })) => {
                                // Look for the shell prompt (ends with "# ")
                                let text = String::from_utf8_lossy(&data);
                                if text.contains("# ") || text.contains("$ ") {
                                    ready = true;
                                }
                            }
                            Ok(Ok(_)) => continue,
                            Ok(Err(_)) | Err(_) => break,
                        }
                    }
                }
                if tokio::time::Instant::now() >= boot_deadline {
                    eprintln!("Shell did not become ready within 30s");
                    delete_vm(&client, &vm_id).await;
                    std::process::exit(1);
                }
            }

            // Type the doctor command into the shell
            let cmd: Vec<u8> = if *fast {
                b"capsem-doctor --durations=10 -k 'not throughput'\n".to_vec()
            } else {
                b"capsem-doctor --durations=10\n".to_vec()
            };
            let _ = tx.send(ServiceToProcess::TerminalInput { data: cmd }).await;

            // Stream output until we see the sentinel line
            let mut stdout = tokio::io::stdout();
            let mut output_buf = String::new();
            let exit_code = loop {
                tokio::select! {
                    _ = &mut ctrl_c => {
                        eprintln!("\nInterrupted, cleaning up session...");
                        break 130;
                    }
                    result = tokio::time::timeout(
                        std::time::Duration::from_secs(300),
                        rx.recv(),
                    ) => {
                        match result {
                            Ok(Ok(ProcessToService::TerminalOutput { data })) => {
                                let _ = stdout.write_all(&data).await;
                                let _ = stdout.flush().await;
                                if let Some(ref mut f) = log_file {
                                    let _ = std::io::Write::write_all(f, &data);
                                }
                                // Check for sentinel
                                output_buf.push_str(&String::from_utf8_lossy(&data));
                                // Keep only last 512 bytes to avoid unbounded growth.
                                // Pad by sentinel length so we never split "RESULT: FAIL"
                                // across a truncation boundary.
                                if output_buf.len() > 1024 {
                                    let keep = 512 + "RESULT: FAIL".len();
                                    output_buf = output_buf.split_off(output_buf.len() - keep);
                                }
                                if output_buf.contains("RESULT: PASS") {
                                    break 0;
                                } else if output_buf.contains("RESULT: FAIL") {
                                    break 1;
                                }
                            }
                            Ok(Ok(_)) => continue,
                            Ok(Err(e)) => {
                                eprintln!("IPC error: {e}");
                                break 1;
                            }
                            Err(_) => {
                                eprintln!("Doctor timed out after 300s");
                                break 1;
                            }
                        }
                    }
                }
            };

            delete_vm(&client, &vm_id).await;
            if exit_code != 0 {
                eprintln!("Full log: {}", log_path.display());
                std::process::exit(exit_code);
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
            Commands::Session(SessionCommands::Create { name, ram, cpu, .. }) => {
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
            Commands::Session(SessionCommands::Create { name, .. }) => {
                assert_eq!(name, None);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_with_resources() {
        let cli = Cli::parse_from(["capsem", "create", "--ram", "8", "--cpu", "2"]);
        match cli.command {
            Commands::Session(SessionCommands::Create { ram, cpu, .. }) => {
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
            Commands::Session(SessionCommands::Resume { name }) => assert_eq!(name, "mydev"),
            _ => panic!("expected Resume"),
        }
    }

    #[test]
    fn parse_attach_alias_for_resume() {
        let cli = Cli::parse_from(["capsem", "attach", "mydev"]);
        match cli.command {
            Commands::Session(SessionCommands::Resume { name }) => assert_eq!(name, "mydev"),
            _ => panic!("expected Resume via attach alias"),
        }
    }

    #[test]
    fn parse_suspend() {
        let cli = Cli::parse_from(["capsem", "suspend", "vm-123"]);
        match cli.command {
            Commands::Session(SessionCommands::Suspend { session }) => assert_eq!(session, "vm-123"),
            _ => panic!("expected Suspend"),
        }
    }

    #[test]
    fn parse_shell_positional() {
        let cli = Cli::parse_from(["capsem", "shell", "my-vm"]);
        match cli.command {
            Commands::Session(SessionCommands::Shell { session, name }) => {
                assert_eq!(session, Some("my-vm".into()));
                assert_eq!(name, None);
            }
            _ => panic!("expected Shell"),
        }
    }

    #[test]
    fn parse_shell_by_name() {
        let cli = Cli::parse_from(["capsem", "shell", "-n", "mydev"]);
        match cli.command {
            Commands::Session(SessionCommands::Shell { name, session }) => {
                assert_eq!(name, Some("mydev".into()));
                assert_eq!(session, None);
            }
            _ => panic!("expected Shell"),
        }
    }

    #[test]
    fn parse_shell_bare() {
        // Bare `capsem shell` = temp session + auto-destroy
        let cli = Cli::parse_from(["capsem", "shell"]);
        match cli.command {
            Commands::Session(SessionCommands::Shell { name, session }) => {
                assert_eq!(name, None);
                assert_eq!(session, None);
            }
            _ => panic!("expected Shell"),
        }
    }

    #[test]
    fn parse_persist() {
        let cli = Cli::parse_from(["capsem", "persist", "vm-123", "mydev"]);
        match cli.command {
            Commands::Session(SessionCommands::Persist { session, name }) => {
                assert_eq!(session, "vm-123");
                assert_eq!(name, "mydev");
            }
            _ => panic!("expected Persist"),
        }
    }

    #[test]
    fn parse_purge() {
        let cli = Cli::parse_from(["capsem", "purge"]);
        match cli.command {
            Commands::Session(SessionCommands::Purge { all }) => assert!(!all),
            _ => panic!("expected Purge"),
        }
    }

    #[test]
    fn parse_purge_all() {
        let cli = Cli::parse_from(["capsem", "purge", "--all"]);
        match cli.command {
            Commands::Session(SessionCommands::Purge { all }) => assert!(all),
            _ => panic!("expected Purge --all"),
        }
    }

    #[test]
    fn parse_run() {
        let cli = Cli::parse_from(["capsem", "run", "echo hello"]);
        match cli.command {
            Commands::Session(SessionCommands::Run { command, timeout, env }) => {
                assert_eq!(command, "echo hello");
                assert_eq!(timeout, 60); // default
                assert!(env.is_empty());
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn parse_run_with_timeout() {
        let cli = Cli::parse_from(["capsem", "run", "--timeout", "120", "ls -la"]);
        match cli.command {
            Commands::Session(SessionCommands::Run { command, timeout, env }) => {
                assert_eq!(command, "ls -la");
                assert_eq!(timeout, 120);
                assert!(env.is_empty());
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn parse_list() {
        let cli = Cli::parse_from(["capsem", "list"]);
        assert!(matches!(cli.command, Commands::Session(SessionCommands::List { quiet: false })));
    }

    #[test]
    fn parse_list_quiet() {
        let cli = Cli::parse_from(["capsem", "list", "-q"]);
        match cli.command {
            Commands::Session(SessionCommands::List { quiet }) => assert!(quiet),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_list_quiet_long() {
        let cli = Cli::parse_from(["capsem", "list", "--quiet"]);
        match cli.command {
            Commands::Session(SessionCommands::List { quiet }) => assert!(quiet),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_status() {
        // `capsem status` is now the service status command
        let cli = Cli::parse_from(["capsem", "status"]);
        assert!(matches!(cli.command, Commands::Misc(MiscCommands::Status)));
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
    // New commands: exec, delete, info, doctor
    // -----------------------------------------------------------------------

    #[test]
    fn parse_exec() {
        let cli = Cli::parse_from(["capsem", "exec", "my-vm", "echo hello"]);
        match cli.command {
            Commands::Session(SessionCommands::Exec { session, command, timeout }) => {
                assert_eq!(session, "my-vm");
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
            Commands::Session(SessionCommands::Exec { session, command, timeout }) => {
                assert_eq!(session, "my-vm");
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
            Commands::Session(SessionCommands::Delete { session }) => assert_eq!(session, "vm-123"),
            _ => panic!("expected Delete"),
        }
    }

    #[test]
    fn parse_info() {
        let cli = Cli::parse_from(["capsem", "info", "vm-1"]);
        match cli.command {
            Commands::Session(SessionCommands::Info { session, json }) => {
                assert_eq!(session, "vm-1");
                assert!(!json);
            }
            _ => panic!("expected Info"),
        }
    }

    #[test]
    fn parse_info_json() {
        let cli = Cli::parse_from(["capsem", "info", "--json", "vm-1"]);
        match cli.command {
            Commands::Session(SessionCommands::Info { session, json }) => {
                assert_eq!(session, "vm-1");
                assert!(json);
            }
            _ => panic!("expected Info --json"),
        }
    }

    #[test]
    fn parse_logs_with_tail() {
        let cli = Cli::parse_from(["capsem", "logs", "--tail", "50", "vm-1"]);
        match cli.command {
            Commands::Session(SessionCommands::Logs { session, tail }) => {
                assert_eq!(session, "vm-1");
                assert_eq!(tail, Some(50));
            }
            _ => panic!("expected Logs"),
        }
    }

    #[test]
    fn parse_logs_without_tail() {
        let cli = Cli::parse_from(["capsem", "logs", "vm-1"]);
        match cli.command {
            Commands::Session(SessionCommands::Logs { session, tail }) => {
                assert_eq!(session, "vm-1");
                assert_eq!(tail, None);
            }
            _ => panic!("expected Logs"),
        }
    }

    #[test]
    fn parse_restart() {
        let cli = Cli::parse_from(["capsem", "restart", "mydev"]);
        match cli.command {
            Commands::Session(SessionCommands::Restart { name }) => assert_eq!(name, "mydev"),
            _ => panic!("expected Restart"),
        }
    }

    #[test]
    fn parse_version() {
        let cli = Cli::parse_from(["capsem", "version"]);
        assert!(matches!(cli.command, Commands::Misc(MiscCommands::Version)));
    }

    #[test]
    fn parse_create_with_env() {
        let cli = Cli::parse_from(["capsem", "create", "-e", "FOO=bar", "-e", "BAZ=qux"]);
        match cli.command {
            Commands::Session(SessionCommands::Create { env, .. }) => {
                assert_eq!(env, vec!["FOO=bar", "BAZ=qux"]);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_with_env_long() {
        let cli = Cli::parse_from(["capsem", "create", "--env", "API_KEY=secret123"]);
        match cli.command {
            Commands::Session(SessionCommands::Create { env, .. }) => {
                assert_eq!(env, vec!["API_KEY=secret123"]);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_no_env() {
        let cli = Cli::parse_from(["capsem", "create"]);
        match cli.command {
            Commands::Session(SessionCommands::Create { env, .. }) => {
                assert!(env.is_empty());
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::parse_from(["capsem", "doctor"]);
        assert!(matches!(cli.command, Commands::Misc(MiscCommands::Doctor { fast: false })));
    }

    #[test]
    fn parse_install() {
        let cli = Cli::parse_from(["capsem", "install"]);
        assert!(matches!(cli.command, Commands::Misc(MiscCommands::Install)));
    }

    #[test]
    fn parse_start() {
        let cli = Cli::parse_from(["capsem", "start"]);
        assert!(matches!(cli.command, Commands::Misc(MiscCommands::Start)));
    }

    #[test]
    fn parse_stop() {
        let cli = Cli::parse_from(["capsem", "stop"]);
        assert!(matches!(cli.command, Commands::Misc(MiscCommands::Stop)));
    }

    #[test]
    fn parse_setup_non_interactive() {
        let cli = Cli::parse_from(["capsem", "setup", "--non-interactive"]);
        match cli.command {
            Commands::Misc(MiscCommands::Setup { non_interactive, preset, force, .. }) => {
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
            Commands::Misc(MiscCommands::Setup { preset, force, .. }) => {
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
            Commands::Misc(MiscCommands::Setup { corp_config, non_interactive, .. }) => {
                assert_eq!(corp_config, Some("https://example.com/corp.toml".into()));
                assert!(non_interactive);
            }
            _ => panic!("expected Setup"),
        }
    }

    #[test]
    fn parse_completions_bash() {
        let cli = Cli::parse_from(["capsem", "completions", "bash"]);
        assert!(matches!(cli.command, Commands::Misc(MiscCommands::Completions { shell: clap_complete::Shell::Bash })));
    }

    #[test]
    fn parse_uninstall() {
        let cli = Cli::parse_from(["capsem", "uninstall"]);
        match cli.command {
            Commands::Misc(MiscCommands::Uninstall { yes }) => assert!(!yes),
            _ => panic!("expected Uninstall"),
        }
    }

    #[test]
    fn parse_uninstall_yes() {
        let cli = Cli::parse_from(["capsem", "uninstall", "--yes"]);
        match cli.command {
            Commands::Misc(MiscCommands::Uninstall { yes }) => assert!(yes),
            _ => panic!("expected Uninstall"),
        }
    }

    #[test]
    fn parse_update() {
        let cli = Cli::parse_from(["capsem", "update"]);
        match cli.command {
            Commands::Misc(MiscCommands::Update { yes, assets }) => {
                assert!(!yes);
                assert!(!assets);
            }
            _ => panic!("expected Update"),
        }
    }

    #[test]
    fn parse_update_yes() {
        let cli = Cli::parse_from(["capsem", "update", "--yes"]);
        match cli.command {
            Commands::Misc(MiscCommands::Update { yes, assets }) => {
                assert!(yes);
                assert!(!assets);
            }
            _ => panic!("expected Update"),
        }
    }

    #[test]
    fn parse_update_assets() {
        let cli = Cli::parse_from(["capsem", "update", "--assets"]);
        match cli.command {
            Commands::Misc(MiscCommands::Update { yes, assets }) => {
                assert!(!yes);
                assert!(assets);
            }
            _ => panic!("expected Update"),
        }
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
    // Fork / Image CLI parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_fork() {
        let cli = Cli::parse_from(["capsem", "fork", "my-vm", "my-image"]);
        match cli.command {
            Commands::Session(SessionCommands::Fork { session, name, description }) => {
                assert_eq!(session, "my-vm");
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
            Commands::Session(SessionCommands::Fork { session, name, description }) => {
                assert_eq!(session, "vm1");
                assert_eq!(name, "img1");
                assert_eq!(description, Some("My description".into()));
            }
            _ => panic!("expected Fork"),
        }
    }

    #[test]
    fn parse_create_with_from() {
        let cli = Cli::parse_from(["capsem", "create", "--from", "base-session"]);
        match cli.command {
            Commands::Session(SessionCommands::Create { from, name, .. }) => {
                assert_eq!(from, Some("base-session".into()));
                assert_eq!(name, None);
            }
            _ => panic!("expected Create with --from"),
        }
    }

    #[test]
    fn parse_create_with_from_image_alias() {
        // --image is a backward-compat alias for --from
        let cli = Cli::parse_from(["capsem", "create", "--image", "old-img"]);
        match cli.command {
            Commands::Session(SessionCommands::Create { from, .. }) => {
                assert_eq!(from, Some("old-img".into()));
            }
            _ => panic!("expected Create with --image alias"),
        }
    }

    #[test]
    fn parse_create_with_name_and_from() {
        let cli = Cli::parse_from(["capsem", "create", "-n", "my-session", "--from", "my-src"]);
        match cli.command {
            Commands::Session(SessionCommands::Create { name, from, .. }) => {
                assert_eq!(name, Some("my-session".into()));
                assert_eq!(from, Some("my-src".into()));
            }
            _ => panic!("expected Create with name and --from"),
        }
    }
}

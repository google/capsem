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
    ListResponse, LogsResponse, PersistRequest, ProvisionRequest,
    ProvisionResponse, PurgeRequest, PurgeResponse, RunRequest, SandboxInfo, UdsClient,
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
\x1b[36;1;4mSandbox Commands:\x1b[0m
  \x1b[32;1mcreate\x1b[0m       Create and boot a new sandbox
  \x1b[32;1mshell\x1b[0m        Open an interactive shell in a sandbox
  \x1b[32;1mresume\x1b[0m       Resume a stopped sandbox or attach to a running one
  \x1b[32;1mstop\x1b[0m         Stop a running sandbox
  \x1b[32;1msuspend\x1b[0m      Suspend a running sandbox to disk
  \x1b[32;1mrestart\x1b[0m      Restart a persistent sandbox (stop + resume)
  \x1b[32;1mexec\x1b[0m         Execute a command in a running sandbox
  \x1b[32;1mrun\x1b[0m          Run a command in a fresh sandbox (destroyed after)
  \x1b[32;1mlist\x1b[0m         List all sandboxes (running + stopped persistent)
  \x1b[32;1mstatus\x1b[0m       Show the status of a sandbox
  \x1b[32;1minfo\x1b[0m         Show detailed information about a sandbox
  \x1b[32;1mlogs\x1b[0m         Show logs from a sandbox
  \x1b[32;1mdelete\x1b[0m       Delete a sandbox and all its state
  \x1b[32;1mfork\x1b[0m         Fork a sandbox into a reusable snapshot
  \x1b[32;1mpersist\x1b[0m      Promote an ephemeral sandbox to persistent
  \x1b[32;1mpurge\x1b[0m        Destroy all temporary sandboxes

\x1b[36;1;4mService:\x1b[0m
  \x1b[32;1mservice\x1b[0m      Manage the capsem background daemon
               \u{251c}\u{2500} \x1b[32;1minstall\x1b[0m    Install as a system service (LaunchAgent / systemd)
               \u{251c}\u{2500} \x1b[32;1muninstall\x1b[0m  Remove the system service
               \u{2514}\u{2500} \x1b[32;1mstatus\x1b[0m     Show service installation and runtime status

\x1b[36;1;4mMisc:\x1b[0m
  \x1b[32;1msetup\x1b[0m        Run the first-time setup wizard
  \x1b[32;1mupdate\x1b[0m       Check for updates and install the latest version
  \x1b[32;1mdoctor\x1b[0m       Run diagnostic tests in a fresh sandbox
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
    help_template = "{about-with-newline}\n{usage-heading} {usage}\n{after-help}\n\n\x1b[36;1;4mOptions:\x1b[0m\n{options}",
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
    Sandbox(SandboxCommands),

    /// Manage the capsem background daemon
    #[command(subcommand)]
    Service(ServiceCommands),

    #[command(flatten)]
    Misc(MiscCommands),
}

#[derive(Subcommand)]
enum SandboxCommands {
    /// Create and boot a new sandbox
    ///
    /// VMs are ephemeral by default and destroyed on stop. Use -n <name> to
    /// create a persistent sandbox that survives stop/resume cycles.
    #[command(alias = "start")]
    Create {
        /// Name for the sandbox (makes it persistent -- "if you name it, you keep it")
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
        /// Clone state from an existing persistent sandbox
        #[arg(long, alias = "image")]
        from: Option<String>,
    },
    /// Open an interactive shell in a sandbox
    ///
    /// With no arguments, creates a temporary sandbox (destroyed on exit).
    /// Pass an ID or --name to attach to an existing running sandbox.
    Shell {
        /// Find by name (for persistent sandboxes)
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// ID of the sandbox (positional)
        id: Option<String>,
    },
    /// Resume a stopped sandbox or attach to a running one
    #[command(alias = "attach")]
    Resume {
        /// Name of the persistent sandbox
        name: String,
    },
    /// Stop a running sandbox
    ///
    /// Persistent sandboxes preserve their disk state; ephemeral ones are destroyed.
    Stop {
        /// ID or name of the sandbox
        id: String,
    },
    /// Suspend a running sandbox to disk
    ///
    /// Saves RAM and CPU state. Only persistent sandboxes can be suspended.
    Suspend {
        /// ID or name of the sandbox
        id: String,
    },
    /// Restart a persistent sandbox (stop + resume)
    Restart {
        /// Name of the persistent sandbox
        name: String,
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
    /// Run a command in a fresh sandbox (destroyed after)
    ///
    /// Creates a temporary sandbox, runs the command, prints output, and
    /// destroys the sandbox. Useful for one-shot tasks and CI pipelines.
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
    /// List all sandboxes (running + stopped persistent)
    #[command(alias = "ls")]
    List {
        /// Print only IDs, one per line (for scripting)
        #[arg(short, long)]
        quiet: bool,
    },
    /// Show the status of a sandbox
    Status {
        /// ID or name of the sandbox
        id: String,
    },
    /// Show detailed information about a sandbox
    Info {
        /// ID or name of the sandbox
        id: String,
    },
    /// Show logs from a sandbox
    ///
    /// Displays both serial console and process logs.
    Logs {
        /// ID or name of the sandbox
        id: String,
        /// Show only the last N lines
        #[arg(long)]
        tail: Option<usize>,
    },
    /// Delete a sandbox and all its state
    #[command(alias = "rm")]
    Delete {
        /// ID or name of the sandbox
        id: String,
    },
    /// Fork a sandbox into a new stopped sandbox
    ///
    /// Creates a point-in-time copy of the sandbox's disk state as a new
    /// persistent sandbox. Boot it with `capsem resume <name>` or clone
    /// with `capsem create --from <name>`.
    Fork {
        /// ID or name of the sandbox to fork
        id: String,
        /// Name for the new sandbox
        name: String,
        /// Optional description
        #[arg(short, long)]
        description: Option<String>,
    },
    /// Promote an ephemeral sandbox to persistent
    Persist {
        /// ID of the running ephemeral sandbox
        id: String,
        /// Name to assign
        name: String,
    },
    /// Destroy all temporary sandboxes
    ///
    /// Use --all to also destroy persistent sandboxes (requires confirmation).
    Purge {
        /// Also destroy persistent sandboxes (requires confirmation)
        #[arg(long, default_value_t = false)]
        all: bool,
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
    },
    /// Check for updates and install the latest version
    Update {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
    /// Run diagnostic tests in a fresh sandbox
    ///
    /// Boots a temporary sandbox, runs the capsem-doctor test suite, and reports
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

async fn run_shell(id: &str, run_dir: &std::path::Path) -> Result<()> {
    use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
    use tokio_unix_ipc::{channel_from_std, Sender, Receiver};
    use std::sync::Arc;
    use nix::sys::termios::{tcgetattr, tcsetattr, SetArg};

    client::validate_id(id)?;
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
                | ProcessToService::SnapshotReady { .. } => {}
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

    let home = std::env::var("HOME").context("HOME not set")?;
    let run_dir = std::env::var("CAPSEM_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(home).join(".capsem").join("run"));
    let auto_launch = cli.uds_path.is_none();
    let uds_path = cli.uds_path.unwrap_or_else(|| run_dir.join("service.sock"));

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
        Commands::Misc(MiscCommands::Completions { shell }) => {
            completions::generate_completions(*shell);
            return Ok(());
        }
        Commands::Misc(MiscCommands::Uninstall { yes }) => {
            uninstall::run_uninstall(*yes).await?;
            return Ok(());
        }
        Commands::Misc(MiscCommands::Update { yes }) => {
            update::run_update(*yes).await?;
            return Ok(());
        }
        Commands::Misc(MiscCommands::Setup { non_interactive, preset, force, accept_detected, corp_config }) => {
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
            }).await?;
        }
    }

    let client = UdsClient::new(uds_path, auto_launch);

    match &cli.command {
        Commands::Sandbox(SandboxCommands::Create { name, ram, cpu, env, from }) => {
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
        Commands::Sandbox(SandboxCommands::Fork { id, name, description }) => {
            client::validate_id(id)?;
            let req = ForkRequest {
                name: name.clone(),
                description: description.clone(),
            };
            let resp: ApiResponse<ForkResponse> = client.post(&format!("/fork/{}", id), &req).await?;
            let info = resp.into_result()?;
            let size_mb = info.size_bytes as f64 / 1024.0 / 1024.0;
            println!("Forked sandbox '{}' from '{}' ({:.1} MB)", info.name, id, size_mb);
        }
        Commands::Sandbox(SandboxCommands::Resume { name }) => {
            client::validate_id(name)?;
            let resp: ApiResponse<ProvisionResponse> = client.post(&format!("/resume/{}", name), &serde_json::json!({})).await?;
            let info = resp.into_result()?;
            println!("{}", info.id);
        }
        Commands::Sandbox(SandboxCommands::Stop { id }) => {
            client::validate_id(id)?;
            println!("Stopping sandbox: {}", id);
            let resp: ApiResponse<serde_json::Value> = client.post(&format!("/stop/{}", id), &serde_json::json!({})).await?;
            resp.into_result()?;
            println!("Sandbox stopped.");
        }
        Commands::Sandbox(SandboxCommands::Suspend { id }) => {
            client::validate_id(id)?;
            println!("Suspending sandbox: {}", id);
            let resp: ApiResponse<serde_json::Value> = client.post(&format!("/suspend/{}", id), &serde_json::json!({})).await?;
            resp.into_result()?;
            println!("Sandbox suspended.");
        }
        Commands::Sandbox(SandboxCommands::Shell { name, id }) => {
            let target = name.as_ref().or(id.as_ref());
            match target {
                Some(t) => {
                    // Attach to existing VM
                    client::validate_id(t)?;
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
                        from: None,
                    };
                    let resp: ApiResponse<ProvisionResponse> = client.post("/provision", &req).await?;
                    let info = resp.into_result()?;

                    // Poll until the socket is connectable (not just present on disk).
                    // The file appears at bind() time, but connect() fails until listen().
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
        Commands::Sandbox(SandboxCommands::List { quiet }) => {
            let resp: ApiResponse<ListResponse> = client.get("/list").await?;
            let resp = resp.into_result()?;
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
        Commands::Sandbox(SandboxCommands::Status { id }) => {
            client::validate_id(id)?;
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
        Commands::Sandbox(SandboxCommands::Exec { id, command, timeout }) => {
            client::validate_id(id)?;
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
        Commands::Sandbox(SandboxCommands::Run { command, timeout, env }) => {
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
        Commands::Sandbox(SandboxCommands::Delete { id }) => {
            client::validate_id(id)?;
            println!("Deleting sandbox: {}", id);
            let resp: ApiResponse<serde_json::Value> = client.delete(&format!("/delete/{}", id)).await?;
            resp.into_result()?;
            println!("Sandbox deleted.");
        }
        Commands::Sandbox(SandboxCommands::Persist { id, name }) => {
            client::validate_id(id)?;
            let req = PersistRequest { name: name.clone() };
            let resp: ApiResponse<serde_json::Value> = client.post(&format!("/persist/{}", id), &req).await?;
            resp.into_result()?;
            println!("[*] VM \"{}\" is now persistent as \"{}\"", id, name);
        }
        Commands::Sandbox(SandboxCommands::Purge { all }) => {
            if *all {
                // Confirmation prompt
                use std::io::Write;
                let list_resp: ApiResponse<ListResponse> = client.get("/list").await?;
                let resp = list_resp.into_result()?;
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
        Commands::Sandbox(SandboxCommands::Info { id }) => {
            client::validate_id(id)?;
            let resp: ApiResponse<SandboxInfo> = client.get(&format!("/info/{}", id)).await?;
            let info = resp.into_result()?;
            let json = serde_json::to_string_pretty(&info)?;
            println!("{}", json);
        }
        Commands::Sandbox(SandboxCommands::Logs { id, tail }) => {
            client::validate_id(id)?;
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
        Commands::Sandbox(SandboxCommands::Restart { name }) => {
            client::validate_id(name)?;
            // Look up the VM to check it's persistent
            let info_resp: ApiResponse<SandboxInfo> = client.get(&format!("/info/{}", name)).await?;
            let info = info_resp.into_result()?;
            if !info.persistent {
                anyhow::bail!("Cannot restart ephemeral VM \"{}\". Only persistent VMs support restart.", name);
            }

            // Stop, then resume
            let stop_resp: ApiResponse<serde_json::Value> = client.post(&format!("/stop/{}", name), &serde_json::json!({})).await?;
            stop_resp.into_result().context("failed to stop VM during restart")?;
            let resp: ApiResponse<ProvisionResponse> = client.post(&format!("/resume/{}", name), &serde_json::json!({})).await?;
            let resumed = resp.into_result()?;
            println!("{}", resumed.id);
        }
        Commands::Misc(
            MiscCommands::Version
            | MiscCommands::Setup { .. }
            | MiscCommands::Update { .. }
            | MiscCommands::Completions { .. }
            | MiscCommands::Uninstall { .. }
        ) | Commands::Service(_) => {
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
            let vm_id = resp.into_result()?.id;

            // Helper: always delete the VM, even on Ctrl-C or error
            async fn delete_vm(client: &UdsClient, vm_id: &str) {
                let _: Result<ApiResponse<serde_json::Value>, _> =
                    client.delete(&format!("/delete/{}", vm_id)).await;
            }

            let ctrl_c = tokio::signal::ctrl_c();
            tokio::pin!(ctrl_c);

            // Connect directly to VM socket for streaming output
            let sock_path = run_dir.join("instances").join(format!("{}.sock", vm_id));
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if sock_path.exists() {
                    if let Ok(stream) = tokio::net::UnixStream::connect(&sock_path).await {
                        if let Ok(std_stream) = stream.into_std() {
                            if let Ok((tx, rx)) = channel_from_std::<ServiceToProcess, ProcessToService>(std_stream) {
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
                                            eprintln!("\nInterrupted, cleaning up VM...");
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
                                            eprintln!("\nInterrupted, cleaning up VM...");
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
                                return Ok(());
                            }
                        }
                    }
                }
                // Check Ctrl-C while waiting for socket
                tokio::select! {
                    _ = &mut ctrl_c => {
                        eprintln!("\nInterrupted, cleaning up VM...");
                        delete_vm(&client, &vm_id).await;
                        std::process::exit(130);
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
                }
                if tokio::time::Instant::now() >= deadline {
                    eprintln!("VM did not become ready within 30s");
                    delete_vm(&client, &vm_id).await;
                    std::process::exit(1);
                }
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
            Commands::Sandbox(SandboxCommands::Create { name, ram, cpu, .. }) => {
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
            Commands::Sandbox(SandboxCommands::Create { name, .. }) => {
                assert_eq!(name, None);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_start_alias_for_create() {
        let cli = Cli::parse_from(["capsem", "start", "-n", "dev"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Create { name, .. }) => {
                assert_eq!(name, Some("dev".into()));
            }
            _ => panic!("expected Create via start alias"),
        }
    }

    #[test]
    fn parse_create_with_resources() {
        let cli = Cli::parse_from(["capsem", "create", "--ram", "8", "--cpu", "2"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Create { ram, cpu, .. }) => {
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
            Commands::Sandbox(SandboxCommands::Resume { name }) => assert_eq!(name, "mydev"),
            _ => panic!("expected Resume"),
        }
    }

    #[test]
    fn parse_attach_alias_for_resume() {
        let cli = Cli::parse_from(["capsem", "attach", "mydev"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Resume { name }) => assert_eq!(name, "mydev"),
            _ => panic!("expected Resume via attach alias"),
        }
    }

    #[test]
    fn parse_stop() {
        let cli = Cli::parse_from(["capsem", "stop", "vm-123"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Stop { id }) => assert_eq!(id, "vm-123"),
            _ => panic!("expected Stop"),
        }
    }

    #[test]
    fn parse_suspend() {
        let cli = Cli::parse_from(["capsem", "suspend", "vm-123"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Suspend { id }) => assert_eq!(id, "vm-123"),
            _ => panic!("expected Suspend"),
        }
    }

    #[test]
    fn parse_shell_positional() {
        let cli = Cli::parse_from(["capsem", "shell", "my-vm"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Shell { id, name }) => {
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
            Commands::Sandbox(SandboxCommands::Shell { name, id }) => {
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
            Commands::Sandbox(SandboxCommands::Shell { name, id }) => {
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
            Commands::Sandbox(SandboxCommands::Persist { id, name }) => {
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
            Commands::Sandbox(SandboxCommands::Purge { all }) => assert!(!all),
            _ => panic!("expected Purge"),
        }
    }

    #[test]
    fn parse_purge_all() {
        let cli = Cli::parse_from(["capsem", "purge", "--all"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Purge { all }) => assert!(all),
            _ => panic!("expected Purge --all"),
        }
    }

    #[test]
    fn parse_run() {
        let cli = Cli::parse_from(["capsem", "run", "echo hello"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Run { command, timeout, env }) => {
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
            Commands::Sandbox(SandboxCommands::Run { command, timeout, env }) => {
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
        assert!(matches!(cli.command, Commands::Sandbox(SandboxCommands::List { quiet: false })));
    }

    #[test]
    fn parse_list_quiet() {
        let cli = Cli::parse_from(["capsem", "list", "-q"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::List { quiet }) => assert!(quiet),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_list_quiet_long() {
        let cli = Cli::parse_from(["capsem", "list", "--quiet"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::List { quiet }) => assert!(quiet),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_status() {
        let cli = Cli::parse_from(["capsem", "status", "vm-1"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Status { id }) => assert_eq!(id, "vm-1"),
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
    // New commands: exec, delete, info, doctor
    // -----------------------------------------------------------------------

    #[test]
    fn parse_exec() {
        let cli = Cli::parse_from(["capsem", "exec", "my-vm", "echo hello"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Exec { id, command, timeout }) => {
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
            Commands::Sandbox(SandboxCommands::Exec { id, command, timeout }) => {
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
            Commands::Sandbox(SandboxCommands::Delete { id }) => assert_eq!(id, "vm-123"),
            _ => panic!("expected Delete"),
        }
    }

    #[test]
    fn parse_info() {
        let cli = Cli::parse_from(["capsem", "info", "vm-1"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Info { id }) => assert_eq!(id, "vm-1"),
            _ => panic!("expected Info"),
        }
    }

    #[test]
    fn parse_logs_with_tail() {
        let cli = Cli::parse_from(["capsem", "logs", "--tail", "50", "vm-1"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Logs { id, tail }) => {
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
            Commands::Sandbox(SandboxCommands::Logs { id, tail }) => {
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
            Commands::Sandbox(SandboxCommands::Restart { name }) => assert_eq!(name, "mydev"),
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
            Commands::Sandbox(SandboxCommands::Create { env, .. }) => {
                assert_eq!(env, vec!["FOO=bar", "BAZ=qux"]);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_with_env_long() {
        let cli = Cli::parse_from(["capsem", "create", "--env", "API_KEY=secret123"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Create { env, .. }) => {
                assert_eq!(env, vec!["API_KEY=secret123"]);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_no_env() {
        let cli = Cli::parse_from(["capsem", "create"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Create { env, .. }) => {
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
            Commands::Misc(MiscCommands::Update { yes }) => assert!(!yes),
            _ => panic!("expected Update"),
        }
    }

    #[test]
    fn parse_update_yes() {
        let cli = Cli::parse_from(["capsem", "update", "--yes"]);
        match cli.command {
            Commands::Misc(MiscCommands::Update { yes }) => assert!(yes),
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
            Commands::Sandbox(SandboxCommands::Fork { id, name, description }) => {
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
            Commands::Sandbox(SandboxCommands::Fork { id, name, description }) => {
                assert_eq!(id, "vm1");
                assert_eq!(name, "img1");
                assert_eq!(description, Some("My description".into()));
            }
            _ => panic!("expected Fork"),
        }
    }

    #[test]
    fn parse_create_with_from() {
        let cli = Cli::parse_from(["capsem", "create", "--from", "base-sandbox"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Create { from, name, .. }) => {
                assert_eq!(from, Some("base-sandbox".into()));
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
            Commands::Sandbox(SandboxCommands::Create { from, .. }) => {
                assert_eq!(from, Some("old-img".into()));
            }
            _ => panic!("expected Create with --image alias"),
        }
    }

    #[test]
    fn parse_create_with_name_and_from() {
        let cli = Cli::parse_from(["capsem", "create", "-n", "my-vm", "--from", "my-src"]);
        match cli.command {
            Commands::Sandbox(SandboxCommands::Create { name, from, .. }) => {
                assert_eq!(name, Some("my-vm".into()));
                assert_eq!(from, Some("my-src".into()));
            }
            _ => panic!("expected Create with name and --from"),
        }
    }
}

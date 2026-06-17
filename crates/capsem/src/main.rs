mod client;
mod completions;
mod paths;
mod platform;
mod service_install;
mod support;
mod support_bundle;
mod uninstall;
mod update;

#[cfg(test)]
static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_test_env() -> std::sync::MutexGuard<'static, ()> {
    TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

use anyhow::{anyhow, Context, Result};
use clap::builder::styling::{AnsiColor, Color, Style, Styles};
use clap::{Parser, Subcommand};
use std::{
    io::BufRead,
    path::PathBuf,
    process::{Child, Command as StdCommand, Stdio},
};
use tokio::io::AsyncWriteExt;

use client::{
    ApiResponse, AssetStatusResponse, ExecRequest, ExecResponse, ForkRequest, ForkResponse,
    HistoryResponse, ListResponse, LogsResponse, PersistRequest, ProvisionRequest,
    ProvisionResponse, PurgeRequest, PurgeResponse, RunRequest, SessionInfo, UdsClient,
    VmLifecycleState,
};

const DEFAULT_PROFILE_ID: &str = "code";
const DOCTOR_MOCK_SERVER_ADDR: &str = "127.0.0.1:3713";

struct DoctorMockServer {
    child: Child,
    base_url: String,
}

impl DoctorMockServer {
    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for DoctorMockServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn mock_server_impl_path() -> Result<PathBuf> {
    let cwd_candidate = std::env::current_dir()
        .context("read current directory")?
        .join("scripts/mock_server_impl.py");
    if cwd_candidate.exists() {
        return Ok(cwd_candidate);
    }

    let manifest_candidate =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/mock_server_impl.py");
    if manifest_candidate.exists() {
        return manifest_candidate
            .canonicalize()
            .context("resolve source-tree scripts/mock_server_impl.py");
    }

    Err(anyhow!(
        "scripts/mock_server_impl.py not found; restore the shared Python mock server implementation"
    ))
}

fn spawn_doctor_mock_server() -> Result<DoctorMockServer> {
    let script = mock_server_impl_path()?;
    let mut child = StdCommand::new("python3")
        .arg(&script)
        .arg("--addr")
        .arg(DOCTOR_MOCK_SERVER_ADDR)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("start {}", script.display()))?;

    let stdout = child
        .stdout
        .take()
        .context("mock server stdout must be piped")?;
    let mut reader = std::io::BufReader::new(stdout);
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .context("read mock server ready JSON")?;
    if bytes == 0 {
        let status = child.try_wait().context("read mock server status")?;
        return Err(anyhow!(
            "mock server exited before ready JSON; status={status:?}"
        ));
    }

    let ready: serde_json::Value =
        serde_json::from_str(&line).context("parse mock server ready JSON")?;
    if ready.get("service").and_then(serde_json::Value::as_str) != Some("capsem-mock-server") {
        child.kill().ok();
        return Err(anyhow!("unexpected mock server ready payload: {line}"));
    }
    let base_url = ready
        .get("base_url")
        .and_then(serde_json::Value::as_str)
        .context("mock server ready JSON missing base_url")?
        .to_string();

    Ok(DoctorMockServer { child, base_url })
}

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
        .placeholder(Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack))))
        .error(
            Style::new()
                .fg_color(Some(Color::Ansi(AnsiColor::Red)))
                .bold(),
        )
        .valid(Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green))))
        .invalid(Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow))))
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
  \x1b[32;1massets\x1b[0m       Inspect or repair VM assets

\x1b[36;1;4mMCP:\x1b[0m
  \x1b[32;1mmcp servers\x1b[0m  List configured MCP servers with connection status
  \x1b[32;1mmcp tools\x1b[0m    List discovered MCP tools across all servers
  \x1b[32;1mmcp refresh\x1b[0m  Re-discover tools from all MCP servers
  \x1b[32;1mmcp call\x1b[0m     Call an MCP tool

\x1b[36;1;4mMisc:\x1b[0m
  \x1b[32;1mupdate\x1b[0m       Check for updates and install the latest version
  \x1b[32;1mdoctor\x1b[0m       Run diagnostic tests in a fresh session
  \x1b[32;1mdebug\x1b[0m        Write a redacted support bundle for bug reports
  \x1b[32;1mcompletions\x1b[0m  Generate shell completions (bash, zsh, fish, powershell)
  \x1b[32;1mversion\x1b[0m      Show version and build information
  \x1b[32;1muninstall\x1b[0m    Uninstall capsem completely (service, binaries, data)";

#[derive(Parser)]
#[command(
    author,
    version,
    about = "The fastest way to ship with AI securely.",
    long_about = None,
    styles = cli_styles(),
    help_template = "{about-with-newline}Version: {version}\n\n{usage-heading} {usage}\n{after-help}\n\n\x1b[36;1;4mOptions:\x1b[0m\n{options}",
    disable_help_subcommand = true,
    subcommand_help_heading = None,
    after_help = GROUPED_HELP,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

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

    /// Inspect or repair VM assets
    #[command(subcommand)]
    Assets(AssetsCommands),

    #[command(flatten)]
    Misc(MiscCommands),
}

#[derive(Subcommand)]
enum AssetsCommands {
    /// Show VM asset readiness
    Status {
        /// Profile whose VM assets should be inspected
        #[arg(long, default_value = DEFAULT_PROFILE_ID)]
        profile: String,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
    /// Download missing or corrupt VM assets, then show readiness
    Ensure {
        /// Profile whose VM assets should be repaired
        #[arg(long, default_value = DEFAULT_PROFILE_ID)]
        profile: String,
        /// Output JSON
        #[arg(long)]
        json: bool,
    },
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
        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Run a command in a fresh session (destroyed after)
    ///
    /// Creates a temporary session, runs the command, prints output, and
    /// destroys the session. Useful for one-shot tasks and CI pipelines.
    Run {
        /// Command to execute
        command: String,
        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
        /// Set environment variables (repeatable: -e KEY=VALUE)
        #[arg(short = 'e', long = "env")]
        env: Vec<String>,
    },
    /// Copy a file in or out of a session's workspace.
    ///
    /// Either `src` or `dst` (but not both) must use the form
    /// `SESSION:PATH` -- where SESSION is the session name or id and
    /// PATH is relative to the workspace root (`/root` in the guest).
    /// The other side is a local host path.
    ///
    /// Examples:
    ///   capsem cp foo.txt my-vm:foo.txt           # upload
    ///   capsem cp my-vm:bench.json ./bench.json   # download
    ///   capsem cp my-vm:/root/log.txt -           # download to stdout
    Cp {
        /// Source path (`SESSION:PATH` for guest, plain path for host).
        src: String,
        /// Destination path (`SESSION:PATH` for guest, plain path for host;
        /// `-` for stdout on download).
        dst: String,
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
    /// results.
    Doctor {
        /// Tell the in-VM doctor to package its diagnostic surface
        /// (pytest output + junit, /var/log, dmesg, /proc/{mounts,cmdline},
        /// session.db) into a tar that capsem support-bundle picks up
        /// at `~/.capsem/run/doctor-latest.tar`.
        #[arg(long)]
        bundle: bool,
    },
    /// Generate shell completions (bash, zsh, fish, powershell)
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Show version and build information
    Version,
    /// Bundle host logs, recent session telemetry, configs, and version
    /// info into a single redacted tar.gz for bug reports.
    ///
    /// Default output: `~/.capsem/support/capsem-support-<ts>-<host>.tar.gz`.
    /// Secrets in settings.toml/corp.toml and bearer tokens in log lines are
    /// stripped by default. The bundle excludes rootfs.img unless
    /// `--include-rootfs` is passed.
    #[command(alias = "debug")]
    SupportBundle {
        /// Output tar.gz path. Default: ~/.capsem/support/capsem-support-<ts>-<host>.tar.gz
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,
        /// Number of recent session directories to include. Max 10.
        #[arg(long, default_value_t = 3)]
        sessions: usize,
        /// Include the (potentially huge) rootfs.img in each session.
        /// Off by default: a 2GB image per session is rarely useful in
        /// a bug report.
        #[arg(long)]
        include_rootfs: bool,
        /// Skip the secret-redaction pass. Off by default: keep this off
        /// when sharing the bundle with anyone outside your team.
        #[arg(long)]
        no_redact: bool,
        /// Cap the total uncompressed size of session-DB content. When
        /// exceeded, sessions are dropped from oldest first. 0 = no cap.
        /// Default 50MB so the bundle stays attachable to bug reports.
        #[arg(long, default_value_t = 50 * 1024 * 1024)]
        max_session_bytes: u64,
    },
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

fn print_asset_status(status: &AssetStatusResponse) {
    println!(
        "Assets: {}{}",
        if status.ready { "ready" } else { "not ready" },
        status
            .asset_version
            .as_ref()
            .map(|v| format!(" ({v})"))
            .unwrap_or_default()
    );
    if status.downloading {
        println!("Downloading: true");
        if let Some(asset) = &status.current_asset {
            match (status.bytes_done, status.bytes_total) {
                (Some(done), Some(total)) => {
                    println!("Current:   {} ({}/{})", asset, done, total);
                }
                (Some(done), None) => {
                    println!("Current:   {} ({} bytes)", asset, done);
                }
                _ => println!("Current:   {}", asset),
            }
        }
    }
    if let Some(downloaded) = status.downloaded {
        println!("Downloaded: {downloaded}");
    }
    if let Some(error) = &status.error {
        println!("Error: {error}");
    }
    if let Some(error) = &status.reconcile_error {
        println!("Last error: {error}");
    }
    if let Some(manifest) = &status.manifest {
        println!("Manifest: {} ({})", manifest.origin, manifest.path);
        if let Some(source) = &manifest.origin_source {
            println!("Manifest source: {source}");
        }
        if let Some(packaged_at) = &manifest.packaged_at {
            println!("Packaged at: {packaged_at}");
        }
        if let Some(refreshed_at) = &manifest.refreshed_at {
            println!("Manifest refreshed: {refreshed_at}");
        }
        if let Some(status) = &manifest.validation_status {
            println!("Manifest status: {status}");
        }
        if let Some(error) = &manifest.validation_error {
            println!("Manifest error: {error}");
        }
        if let Some(hash) = &manifest.blake3 {
            println!("Manifest hash: blake3:{hash}");
        }
        if let Some(current) = &manifest.assets_current {
            println!("Asset set: {current}");
        }
        if let Some(current) = &manifest.binaries_current {
            println!("Binary set: {current}");
        }
    }
    for asset in &status.assets {
        match &asset.path {
            Some(path) => println!("  {:<14} {:<8} {}", asset.name, asset.status, path),
            None => println!("  {:<14} {}", asset.name, asset.status),
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
            println!(
                "  Requests:      {} ({} allowed, {} denied)",
                total, allowed, denied
            );
        }
        if let Some(fe) = info.total_file_events {
            println!("  File Events:   {}", fe);
        }
    }
}

fn purge_summary_message(result: &PurgeResponse, all: bool) -> String {
    if all {
        return format!(
            "[*] Purged {} sessions ({} persistent, {} temporary).",
            result.purged, result.persistent_purged, result.ephemeral_purged
        );
    }
    if result.persistent_purged > 0 {
        format!(
            "[*] Purged {} sessions ({} broken persistent, {} temporary).",
            result.purged, result.persistent_purged, result.ephemeral_purged
        )
    } else {
        format!("[*] Purged {} temporary sessions.", result.ephemeral_purged)
    }
}

fn capsem_shell_tui_args(session: Option<&str>) -> Vec<String> {
    session
        .map(|session| vec!["--session".to_string(), session.to_string()])
        .unwrap_or_default()
}

fn resolve_capsem_tui_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CAPSEM_SHELL_TUI_BINARY") {
        return PathBuf::from(path);
    }
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let sibling = parent.join("capsem-tui");
            if sibling.exists() {
                return sibling;
            }
        }
    }
    PathBuf::from("capsem-tui")
}

async fn run_tui_shell(session: Option<&str>) -> Result<()> {
    if let Some(session) = session {
        client::validate_id(session)?;
    }
    let binary = resolve_capsem_tui_binary();
    let status = tokio::process::Command::new(&binary)
        .args(capsem_shell_tui_args(session))
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .await
        .with_context(|| format!("launch {}", binary.display()))?;
    if !status.success() {
        anyhow::bail!("{} exited with {}", binary.display(), status);
    }
    Ok(())
}

async fn check_service_health() -> Result<Vec<String>> {
    let mut issues = Vec::new();
    let status = service_install::service_status().await?;

    if !status.running {
        issues.push("Service is not running. Run `capsem start` to start the service.".into());
        return Ok(issues);
    }

    let sock = cli_service_socket_path();
    let my_version = env!("CARGO_PKG_VERSION");

    // Check service version via UDS
    let svc_version = async {
        let stream = tokio::net::UnixStream::connect(&sock).await.ok()?;
        let (reader, mut writer) = tokio::io::split(stream);
        writer
            .write_all(b"GET /version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .ok()?;
        let mut buf = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut tokio::io::BufReader::new(reader), &mut buf)
            .await
            .ok()?;
        let body = String::from_utf8_lossy(&buf);
        let json_start = body.find('{')?;
        let v: serde_json::Value = serde_json::from_str(&body[json_start..]).ok()?;
        v.get("version")?.as_str().map(String::from)
    }
    .await;

    match svc_version {
        Some(ref v) if v == my_version => {}
        Some(ref v) => issues.push(format!(
            "Service is STALE (running v{}, binary is v{}) -- restart service",
            v, my_version
        )),
        None => issues.push("Service is STALE (socket dead or no /version endpoint)".into()),
    }

    let port_path = cli_gateway_port_path();
    let token_path = cli_gateway_token_path();
    match (
        std::fs::read_to_string(&port_path),
        std::fs::read_to_string(&token_path),
    ) {
        (Ok(port_str), Ok(token)) => {
            let port = port_str.trim();
            let token = token.trim();
            let client = reqwest::Client::new();

            // Check gateway version (unauthenticated health endpoint)
            let health_url = format!("http://127.0.0.1:{}/health", port);
            let gw_version: Option<String> = async {
                let r = client
                    .get(&health_url)
                    .timeout(std::time::Duration::from_secs(2))
                    .send()
                    .await
                    .ok()?;
                let v: serde_json::Value = r.json().await.ok()?;
                v.get("version")?.as_str().map(String::from)
            }
            .await;

            // Check token validity (authenticated endpoint)
            let auth_url = format!("http://127.0.0.1:{}/vms/list", port);
            let token_ok = client
                .get(&auth_url)
                .header("Authorization", format!("Bearer {}", token))
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);

            match (gw_version, token_ok) {
                (Some(ref v), true) if v == my_version => {}
                (Some(ref v), true) => {
                    issues.push(format!(
                        "Gateway is STALE (running v{}, binary is v{}) -- restart service",
                        v, my_version
                    ));
                }
                (Some(_), false) => {
                    issues.push(format!(
                        "Gateway token MISMATCH (port {}) -- restart service",
                        port
                    ));
                }
                (None, _) => {
                    issues.push(format!("Gateway is DOWN (port {} not responding)", port));
                }
            }
        }
        _ => issues.push("Gateway files not found (no token/port files)".into()),
    }

    let status_client = client::UdsClient::new(sock, false);
    match service_json(&status_client, "/profiles/status").await {
        Some(profile_status) => issues.extend(profile_status_issues(&profile_status)),
        None => issues.push("Profile status unavailable from service".into()),
    }

    Ok(issues)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliRuntimePaths {
    service_socket: PathBuf,
    gateway_port: PathBuf,
    gateway_token: PathBuf,
}

fn cli_runtime_paths_from_run_dir(run_dir: &std::path::Path) -> CliRuntimePaths {
    CliRuntimePaths {
        service_socket: run_dir.join("service.sock"),
        gateway_port: run_dir.join("gateway.port"),
        gateway_token: run_dir.join("gateway.token"),
    }
}

fn cli_runtime_paths() -> CliRuntimePaths {
    cli_runtime_paths_from_run_dir(&capsem_core::paths::capsem_run_dir())
}

fn cli_service_socket_path() -> PathBuf {
    cli_runtime_paths().service_socket
}

fn cli_gateway_port_path() -> PathBuf {
    cli_runtime_paths().gateway_port
}

fn cli_gateway_token_path() -> PathBuf {
    cli_runtime_paths().gateway_token
}

async fn service_json(client: &UdsClient, path: &str) -> Option<serde_json::Value> {
    client
        .get::<ApiResponse<serde_json::Value>>(path)
        .await
        .ok()?
        .into_result()
        .ok()
}

fn profile_status_summary_lines(status: &serde_json::Value) -> Vec<String> {
    let mut lines = Vec::new();
    let source = status["source"].as_str().unwrap_or("unknown");
    let profile_count = status["profile_count"].as_u64().unwrap_or(0);
    let ready_count = status["ready_count"].as_u64().unwrap_or(0);
    lines.push(format!(
        "Profiles:  {ready_count}/{profile_count} ready ({source})"
    ));
    if let Some(manifest) = status["asset_manifest"].as_object() {
        let origin = manifest
            .get("origin")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let path = manifest
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        lines.push(format!("Manifest:  {origin} ({path})"));
        if let Some(source) = manifest
            .get("origin_source")
            .and_then(|value| value.as_str())
        {
            lines.push(format!("  source:  {source}"));
        }
        if let Some(packaged_at) = manifest.get("packaged_at").and_then(|value| value.as_str()) {
            lines.push(format!("  built:   {packaged_at}"));
        }
        if let Some(refreshed_at) = manifest
            .get("refreshed_at")
            .and_then(|value| value.as_str())
        {
            lines.push(format!("  refresh: {refreshed_at}"));
        }
        if let Some(validation_status) = manifest
            .get("validation_status")
            .and_then(|value| value.as_str())
        {
            lines.push(format!("  status:  {validation_status}"));
        }
        if let Some(error) = manifest
            .get("validation_error")
            .and_then(|value| value.as_str())
        {
            lines.push(format!("  error:   {error}"));
        }
        if let Some(hash) = manifest.get("blake3").and_then(|value| value.as_str()) {
            lines.push(format!("  hash:    blake3:{hash}"));
        }
        if let Some(current) = manifest
            .get("assets_current")
            .and_then(|value| value.as_str())
        {
            lines.push(format!("  assets:  {current}"));
        }
        if let Some(current) = manifest
            .get("binaries_current")
            .and_then(|value| value.as_str())
        {
            lines.push(format!("  binary:  {current}"));
        }
    }
    if let Some(profiles) = status["profiles"].as_array() {
        for profile in profiles {
            let id = profile["id"].as_str().unwrap_or("-");
            let name = profile["name"].as_str().unwrap_or(id);
            let ready = profile["ready"].as_bool().unwrap_or(false);
            let arch = profile["current_arch"].as_str().unwrap_or("-");
            let hash = profile["profile_payload_hash"].as_str().unwrap_or("-");
            let missing = profile["missing_assets"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let readiness = if ready { "ready" } else { "not-ready" };
            lines.push(format!(
                "  - {id}: {name} ({readiness}, arch {arch}, hash {hash})"
            ));
            if !missing.is_empty() {
                lines.push(format!("    missing: {}", missing.join(", ")));
            }
        }
    }
    lines
}

fn print_profiles_status(status: &serde_json::Value) {
    for line in profile_status_summary_lines(status) {
        println!("{line}");
    }
}

fn profile_status_issues(status: &serde_json::Value) -> Vec<String> {
    let mut issues = Vec::new();
    if status["profile_count"].as_u64().unwrap_or(0) == 0 {
        issues.push("No profiles are installed".to_string());
        return issues;
    }
    if let Some(profiles) = status["profiles"].as_array() {
        for profile in profiles {
            if profile["ready"].as_bool().unwrap_or(false) {
                continue;
            }
            let id = profile["id"].as_str().unwrap_or("unknown");
            let missing_assets = profile["missing_assets"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let invalid_assets = profile["invalid_assets"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let invalid_files = profile["invalid_files"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut detail = Vec::new();
            if !missing_assets.is_empty() {
                detail.push(format!("missing assets: {}", missing_assets.join(", ")));
            }
            if !invalid_assets.is_empty() {
                detail.push(format!("invalid assets: {}", invalid_assets.join(", ")));
            }
            if !invalid_files.is_empty() {
                detail.push(format!(
                    "invalid profile files: {}",
                    invalid_files.join(", ")
                ));
            }
            if detail.is_empty() {
                issues.push(format!("Profile {id} is not ready"));
            } else {
                issues.push(format!("Profile {id} is not ready ({})", detail.join("; ")));
            }
        }
    }
    issues
}

fn print_corp_status(info: &serde_json::Value) {
    let installed = info["installed"].as_bool().unwrap_or(false);
    println!(
        "Corp:      {}",
        if installed {
            "installed"
        } else {
            "not installed"
        }
    );
    if let Some(source) = info["source"].as_object() {
        let url = source.get("url").and_then(|value| value.as_str());
        let file_path = source.get("file_path").and_then(|value| value.as_str());
        let hash = source
            .get("content_hash")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        let refresh = source
            .get("refresh_interval_hours")
            .and_then(|value| value.as_u64())
            .map(|hours| format!("{hours}h"))
            .unwrap_or_else(|| "-".to_string());
        if let Some(url) = url {
            println!("  source:  {url}");
        } else if let Some(path) = file_path {
            println!("  source:  {path}");
        }
        println!("  hash:    {hash}");
        println!("  refresh: {refresh}");
    }
}

fn should_refresh_update_cache_for_command(command: &Commands) -> bool {
    !matches!(
        command,
        Commands::Misc(
            MiscCommands::Install
                | MiscCommands::Status
                | MiscCommands::Start
                | MiscCommands::Stop
                | MiscCommands::Completions { .. }
                | MiscCommands::Uninstall { .. }
                | MiscCommands::SupportBundle { .. }
                | MiscCommands::Version
        )
    )
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
            let dir = p
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
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

    if cli.command.is_none() {
        tokio::spawn(update::refresh_update_cache_if_stale());
        let issues = check_service_health().await?;
        if !issues.is_empty() {
            eprintln!("\x1b[31;1m[!] Background service has issues:\x1b[0m");
            for issue in issues {
                eprintln!("  - {}", issue);
            }
            eprintln!();
        }
        // Print default grouped help
        println!("{}", GROUPED_HELP);
        return Ok(());
    }

    let command = cli.command.as_ref().unwrap();
    if should_refresh_update_cache_for_command(command) {
        tokio::spawn(update::refresh_update_cache_if_stale());
    }

    // Commands that don't need the service
    match command {
        Commands::Misc(MiscCommands::Version) => {
            println!(
                "capsem {} (build {} ts={})",
                env!("CARGO_PKG_VERSION"),
                env!("CAPSEM_BUILD_HASH"),
                option_env!("CAPSEM_BUILD_TS").unwrap_or("dev"),
            );
            return Ok(());
        }
        Commands::Misc(MiscCommands::SupportBundle {
            output,
            sessions,
            include_rootfs,
            no_redact,
            max_session_bytes,
        }) => {
            let path = support_bundle::run_with_opts(support_bundle::Opts {
                output: output.clone(),
                sessions: *sessions,
                include_rootfs: *include_rootfs,
                no_redact: *no_redact,
                max_session_bytes: *max_session_bytes,
            })?;
            println!("{}", path.display());
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
                let sock = cli_service_socket_path();
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
                    Some(ref v) => println!(
                        "Service:   STALE (running v{}, binary is v{}) -- restart service",
                        v, my_version
                    ),
                    None => println!("Service:   STALE (socket dead or no /version endpoint)"),
                }

                let port_path = cli_gateway_port_path();
                let token_path = cli_gateway_token_path();
                match (
                    std::fs::read_to_string(&port_path),
                    std::fs::read_to_string(&token_path),
                ) {
                    (Ok(port_str), Ok(token)) => {
                        let port = port_str.trim();
                        let token = token.trim();
                        let client = reqwest::Client::new();

                        // Check gateway version (unauthenticated health endpoint)
                        let health_url = format!("http://127.0.0.1:{}/health", port);
                        let gw_version: Option<String> = async {
                            let r = client
                                .get(&health_url)
                                .timeout(std::time::Duration::from_secs(2))
                                .send()
                                .await
                                .ok()?;
                            let v: serde_json::Value = r.json().await.ok()?;
                            v.get("version")?.as_str().map(String::from)
                        }
                        .await;

                        // Check token validity (authenticated endpoint)
                        let auth_url = format!("http://127.0.0.1:{}/vms/list", port);
                        let token_ok = client
                            .get(&auth_url)
                            .header("Authorization", format!("Bearer {}", token))
                            .timeout(std::time::Duration::from_secs(2))
                            .send()
                            .await
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
                                println!(
                                    "Gateway:   token MISMATCH (port {}) -- restart service",
                                    port
                                );
                            }
                            (None, _) => {
                                println!("Gateway:   DOWN (port {} not responding)", port);
                            }
                        }
                    }
                    _ => println!("Gateway:   no token/port files"),
                }
            }

            if status.running {
                let sock = cli_service_socket_path();
                let status_client = client::UdsClient::new(sock, false);
                println!();
                match service_json(&status_client, "/profiles/status").await {
                    Some(profile_status) => print_profiles_status(&profile_status),
                    None => println!("Profiles:  unavailable"),
                }
                match service_json(&status_client, "/corp/info").await {
                    Some(corp_info) => print_corp_status(&corp_info),
                    None => println!("Corp:      unavailable"),
                }
            }

            // Surface defunct sandboxes prominently -- a boot failure
            // otherwise only appears as a line in `capsem list`, and the
            // first command users reach for after "it doesn't work" is
            // `capsem status`. One-line banner + hint at `capsem logs`.
            if status.running {
                let sock = cli_service_socket_path();
                let list_client = client::UdsClient::new(sock, false);
                if let Ok(resp) = list_client
                    .get::<client::ApiResponse<client::ListResponse>>("/vms/list")
                    .await
                {
                    if let Ok(list) = resp.into_result() {
                        let defunct: Vec<&client::SessionInfo> = list
                            .sessions
                            .iter()
                            .filter(|s| s.status == VmLifecycleState::Defunct)
                            .collect();
                        if !defunct.is_empty() {
                            println!();
                            println!(
                                "Defunct:   {} sandbox(es) failed to boot -- run `capsem logs <name>`",
                                defunct.len()
                            );
                            for s in &defunct {
                                let name = s.name.as_deref().unwrap_or(&s.id);
                                if let Some(err) = &s.last_error {
                                    let last = err
                                        .lines()
                                        .rev()
                                        .find(|line| !line.trim().is_empty())
                                        .unwrap_or("(log empty)");
                                    println!("  - {}: {}", name, last);
                                } else {
                                    println!("  - {}", name);
                                }
                            }
                        }
                    }
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
        _ => {}
    }

    let client = UdsClient::new(uds_path, auto_launch);

    match cli.command.as_ref().unwrap() {
        Commands::Assets(AssetsCommands::Status { profile, json }) => {
            client::validate_id(profile)?;
            let encoded_profile = urlencoding::encode(profile);
            let resp: ApiResponse<AssetStatusResponse> = client
                .get(&format!("/profiles/{encoded_profile}/assets/status"))
                .await?;
            let status = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print_asset_status(&status);
            }
        }
        Commands::Assets(AssetsCommands::Ensure { profile, json }) => {
            client::validate_id(profile)?;
            let encoded_profile = urlencoding::encode(profile);
            let resp: ApiResponse<AssetStatusResponse> = client
                .post(
                    &format!("/profiles/{encoded_profile}/assets/ensure"),
                    serde_json::json!({}),
                )
                .await?;
            let status = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print_asset_status(&status);
            }
        }
        Commands::Session(SessionCommands::Create {
            name,
            ram,
            cpu,
            env,
            from,
        }) => {
            let persistent = name.is_some() || from.is_some();
            let req = ProvisionRequest {
                name: name.clone(),
                profile_id: DEFAULT_PROFILE_ID.to_string(),
                ram_mb: ram * 1024,
                cpus: *cpu,
                persistent,
                env: client::parse_env_vars(env)?,
                from: from.clone(),
            };

            let resp: ApiResponse<ProvisionResponse> = client.post("/vms/create", &req).await?;
            let info = resp.into_result()?;

            if persistent {
                println!("{} (persistent)", info.id);
            } else {
                println!("{}", info.id);
            }
        }
        Commands::Session(SessionCommands::Fork {
            session,
            name,
            description,
        }) => {
            client::validate_id(session)?;
            let req = ForkRequest {
                name: name.clone(),
                description: description.clone(),
            };
            let resp: ApiResponse<ForkResponse> =
                client.post(&format!("/vms/{}/fork", session), &req).await?;
            let info = resp.into_result()?;
            let size_mb = info.size_bytes as f64 / 1024.0 / 1024.0;
            println!(
                "Forked session '{}' from '{}' ({:.1} MB)",
                info.name, session, size_mb
            );
        }
        Commands::Session(SessionCommands::Resume { name }) => {
            client::validate_id(name)?;
            let resp: ApiResponse<ProvisionResponse> = client
                .post(&format!("/vms/{}/resume", name), &serde_json::json!({}))
                .await?;
            let info = resp.into_result()?;
            println!("{}", info.id);
        }
        Commands::Session(SessionCommands::Suspend { session }) => {
            client::validate_id(session)?;
            println!("Suspending session: {}", session);
            let resp: ApiResponse<serde_json::Value> = client
                .post(&format!("/vms/{}/pause", session), &serde_json::json!({}))
                .await?;
            resp.into_result()?;
            println!("Session suspended.");
        }
        Commands::Session(SessionCommands::Shell { name, session }) => {
            let target = name.as_ref().or(session.as_ref());
            run_tui_shell(target.map(String::as_str)).await?;
        }
        Commands::Session(SessionCommands::List { quiet }) => {
            let resp: ApiResponse<ListResponse> = client.get("/vms/list").await?;
            let resp = resp.into_result()?;
            if *quiet {
                for s in &resp.sessions {
                    println!("{}", s.id);
                }
            } else if resp.sessions.is_empty() {
                println!("No sessions.");
            } else {
                println!(
                    "{:<20} {:<12} {:<10} {:<8} {:<6} {:<10}",
                    "ID", "NAME", "STATUS", "RAM", "CPUs", "UPTIME"
                );
                for s in &resp.sessions {
                    let name = s.name.as_deref().unwrap_or("-");
                    let ram = s
                        .ram_mb
                        .map(|mb| format!("{} GB", mb / 1024))
                        .unwrap_or_else(|| "-".into());
                    let cpus = s.cpus.map(|c| c.to_string()).unwrap_or_else(|| "-".into());
                    let uptime = format_uptime(s.uptime_secs);
                    println!(
                        "{:<20} {:<12} {:<10} {:<8} {:<6} {:<10}",
                        s.id, name, s.status, ram, cpus, uptime
                    );
                    // Defunct rows: show the tail of process.log inline so
                    // the user doesn't need a separate `capsem logs` call
                    // to see why boot failed.
                    if s.status == VmLifecycleState::Defunct {
                        if let Some(err) = &s.last_error {
                            let last = err
                                .lines()
                                .rev()
                                .find(|line| !line.trim().is_empty())
                                .unwrap_or("(log empty)");
                            println!("  ! {}", last);
                            println!("  (`capsem logs {}` for full context)", s.id);
                        }
                    } else if s.status == VmLifecycleState::Incompatible {
                        if let Some(reason) = &s.resume_blocked_reason {
                            println!("  ! {}", reason);
                        }
                    }
                }
                let defunct = resp
                    .sessions
                    .iter()
                    .filter(|s| s.status == VmLifecycleState::Defunct)
                    .count();
                if defunct > 0 {
                    println!();
                    println!(
                        "{} defunct sandbox(es). Run `capsem logs <name>` to debug.",
                        defunct
                    );
                }
            }
        }
        Commands::Session(SessionCommands::Exec {
            session,
            command,
            timeout,
        }) => {
            client::validate_id(session)?;
            let req = ExecRequest {
                command: command.clone(),
                timeout_secs: *timeout,
            };
            let resp: ApiResponse<ExecResponse> =
                client.post(&format!("/vms/{}/exec", session), req).await?;
            let resp = resp.into_result()?;
            if !resp.stdout.is_empty() {
                print!("{}", resp.stdout);
            }
            if !resp.stderr.is_empty() {
                eprint!("{}", resp.stderr);
            }
            std::process::exit(resp.exit_code);
        }
        Commands::Session(SessionCommands::Run {
            command,
            timeout,
            env,
        }) => {
            let req = RunRequest {
                command: command.clone(),
                profile_id: DEFAULT_PROFILE_ID.to_string(),
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
        Commands::Session(SessionCommands::Cp { src, dst }) => {
            handle_cp(&client, src, dst).await?;
        }
        Commands::Session(SessionCommands::Delete { session }) => {
            client::validate_id(session)?;
            println!("Deleting session: {}", session);
            let resp: ApiResponse<serde_json::Value> =
                client.delete(&format!("/vms/{}/delete", session)).await?;
            resp.into_result()?;
            println!("Session deleted.");
        }
        Commands::Session(SessionCommands::Persist { session, name }) => {
            client::validate_id(session)?;
            let req = PersistRequest { name: name.clone() };
            let resp: ApiResponse<serde_json::Value> =
                client.post(&format!("/vms/{}/save", session), &req).await?;
            resp.into_result()?;
            println!(
                "[*] Session \"{}\" is now persistent as \"{}\"",
                session, name
            );
        }
        Commands::Session(SessionCommands::Purge { all }) => {
            if *all {
                // Confirmation prompt
                use std::io::Write;
                let list_resp: ApiResponse<ListResponse> = client.get("/vms/list").await?;
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
            println!("{}", purge_summary_message(&result, *all));
        }
        Commands::Session(SessionCommands::Info { session, json }) => {
            client::validate_id(session)?;
            let resp: ApiResponse<SessionInfo> =
                client.get(&format!("/vms/{}/info", session)).await?;
            let info = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                print_session_info(&info);
            }
        }
        Commands::Session(SessionCommands::Logs { session, tail }) => {
            client::validate_id(session)?;
            let resp: ApiResponse<LogsResponse> =
                client.get(&format!("/vms/{}/logs", session)).await?;
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
        Commands::Session(SessionCommands::History {
            session,
            tail,
            all,
            search,
            layer,
            json,
        }) => {
            client::validate_id(session)?;
            let limit = if *all { 100_000 } else { *tail };
            let mut url = format!("/vms/{}/history?limit={}&layer={}", session, limit, layer);
            if let Some(q) = search {
                url.push_str(&format!(
                    "&search={}",
                    q.replace(' ', "%20").replace('&', "%26")
                ));
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
                    let exit = entry
                        .exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "-".into());
                    let process = match entry.layer.as_str() {
                        "exec" => entry
                            .details
                            .get("process_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("api")
                            .to_string(),
                        "audit" => {
                            let parent = entry
                                .details
                                .get("parent_exe")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let exe = entry
                                .details
                                .get("exe")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if parent.is_empty() {
                                exe.rsplit('/').next().unwrap_or(exe).to_string()
                            } else {
                                format!(
                                    "{}>{}",
                                    parent.rsplit('/').next().unwrap_or(parent),
                                    exe.rsplit('/').next().unwrap_or(exe)
                                )
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
                    println!(
                        " {:<22} {:<7} {:<5} {:<10} {}",
                        entry.timestamp, entry.layer, exit, process, cmd
                    );
                }
                if history.has_more {
                    println!(
                        " Showing {} of {} commands. Use --all for full history.",
                        history.commands.len(),
                        history.total
                    );
                }
            }
        }
        Commands::Session(SessionCommands::Restart { name }) => {
            client::validate_id(name)?;
            let info_resp: ApiResponse<SessionInfo> =
                client.get(&format!("/vms/{}/info", name)).await?;
            let info = info_resp.into_result()?;
            if !info.persistent {
                anyhow::bail!("Cannot restart ephemeral session \"{}\". Only persistent sessions support restart.", name);
            }

            // Stop, then resume
            let stop_resp: ApiResponse<serde_json::Value> = client
                .post(&format!("/vms/{}/stop", name), &serde_json::json!({}))
                .await?;
            stop_resp
                .into_result()
                .context("failed to stop session during restart")?;
            let resp: ApiResponse<ProvisionResponse> = client
                .post(&format!("/vms/{}/resume", name), &serde_json::json!({}))
                .await?;
            let resumed = resp.into_result()?;
            println!("{}", resumed.id);
        }
        Commands::Mcp(McpCommands::Servers) => {
            let resp: ApiResponse<Vec<serde_json::Value>> = client
                .get(&format!(
                    "/profiles/{}/mcp/servers/list",
                    DEFAULT_PROFILE_ID
                ))
                .await?;
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
                        if s["enabled"].as_bool().unwrap_or(false) {
                            "yes"
                        } else {
                            "no"
                        },
                        s["source"].as_str().unwrap_or("-"),
                        s["tool_count"].as_u64().unwrap_or(0),
                        s["url"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
        Commands::Mcp(McpCommands::Tools { server }) => {
            let server_names: Vec<String> = if let Some(server_filter) = server {
                vec![server_filter.clone()]
            } else {
                let resp: ApiResponse<Vec<serde_json::Value>> = client
                    .get(&format!(
                        "/profiles/{}/mcp/servers/list",
                        DEFAULT_PROFILE_ID
                    ))
                    .await?;
                resp.into_result()?
                    .into_iter()
                    .filter_map(|server| server["name"].as_str().map(ToOwned::to_owned))
                    .collect()
            };
            let mut tools = Vec::new();
            for server_name in server_names {
                let resp: ApiResponse<Vec<serde_json::Value>> = client
                    .get(&format!(
                        "/profiles/{}/mcp/servers/{}/tools/list",
                        DEFAULT_PROFILE_ID, server_name
                    ))
                    .await?;
                tools.extend(resp.into_result()?);
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
                        if t["approved"].as_bool().unwrap_or(false) {
                            "yes"
                        } else {
                            "no"
                        },
                        short_desc,
                    );
                }
            }
        }
        Commands::Mcp(McpCommands::Refresh) => {
            let resp: ApiResponse<Vec<serde_json::Value>> = client
                .get(&format!(
                    "/profiles/{}/mcp/servers/list",
                    DEFAULT_PROFILE_ID
                ))
                .await?;
            for server in resp.into_result()? {
                if let Some(server_name) = server["name"].as_str() {
                    let refresh: ApiResponse<serde_json::Value> = client
                        .post(
                            &format!(
                                "/profiles/{}/mcp/servers/{}/refresh",
                                DEFAULT_PROFILE_ID, server_name
                            ),
                            &serde_json::json!({}),
                        )
                        .await?;
                    refresh.into_result()?;
                }
            }
            println!("MCP tools refreshed.");
        }
        Commands::Mcp(McpCommands::Call { name, args }) => {
            let (server_name, tool_name) = name.split_once("__").ok_or_else(|| {
                anyhow!("MCP tool calls must use namespaced names like server__tool; got {name}")
            })?;
            let arguments: serde_json::Value =
                serde_json::from_str(args).context("invalid JSON arguments")?;
            let resp: ApiResponse<serde_json::Value> = client
                .post(
                    &format!(
                        "/profiles/{}/mcp/servers/{}/tools/{}/call",
                        DEFAULT_PROFILE_ID, server_name, tool_name
                    ),
                    &arguments,
                )
                .await?;
            let result = resp.into_result()?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Misc(
            MiscCommands::Version
            | MiscCommands::Update { .. }
            | MiscCommands::Completions { .. }
            | MiscCommands::Uninstall { .. }
            | MiscCommands::Install
            | MiscCommands::Status
            | MiscCommands::Start
            | MiscCommands::Stop
            | MiscCommands::SupportBundle { .. }, /* handled before UDS */
        ) => {
            unreachable!("handled before UdsClient creation")
        }
        Commands::Misc(MiscCommands::Doctor { bundle }) => {
            use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
            use tokio_unix_ipc::channel_from_std;

            // Log file: ~/.capsem/run/doctor-latest.log (always overwritten)
            let log_path = run_dir.join("doctor-latest.log");
            let mut log_file = std::fs::File::create(&log_path).ok();

            println!("Running capsem-doctor...");
            println!("Log: {}", log_path.display());

            let mut mock_server = spawn_doctor_mock_server().with_context(|| {
                format!(
                    "start local mock server for capsem-doctor at {DOCTOR_MOCK_SERVER_ADDR}; \
                     this address is required so guest traffic proves the iptables-nft redirect rail"
                )
            })?;
            let mock_base_url = mock_server.base_url().to_string();
            println!("Local mock server: {mock_base_url}");

            let mut doctor_env = std::collections::HashMap::new();
            doctor_env.insert(
                "CAPSEM_MOCK_SERVER_BASE_URL".to_string(),
                mock_base_url.clone(),
            );

            let req = ProvisionRequest {
                name: None,
                profile_id: DEFAULT_PROFILE_ID.to_string(),
                ram_mb: 2048,
                cpus: 2,
                persistent: false,
                env: Some(doctor_env),
                from: None,
            };
            let resp: ApiResponse<ProvisionResponse> = client.post("/vms/create", req).await?;
            let provisioned = resp.into_result()?;
            let vm_id = provisioned.id;

            // Helper: always delete the session, even on Ctrl-C or error
            async fn delete_vm(client: &UdsClient, vm_id: &str) {
                let _: Result<ApiResponse<serde_json::Value>, _> =
                    client.delete(&format!("/vms/{}/delete", vm_id)).await;
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
                        let mut std_stream = stream.into_std().ok()?;
                        capsem_core::ipc_handshake::negotiate_initiator(
                            &mut std_stream,
                            "capsem-cli",
                            capsem_core::telemetry::current_parent_traceparent(),
                        )
                        .ok()?;
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
            capsem_core::try_send!(
                "cli_doctor_start_stream",
                tx.send(ServiceToProcess::StartTerminalStream).await
            );

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

            // Type the doctor command into the shell. T4: when --bundle
            // is set, append `--bundle /shared/doctor-bundle.tar` so the
            // in-VM doctor packages its diagnostic surface to virtiofs.
            // The host-side reader (after the doctor exits) copies that
            // tar into ~/.capsem/run/doctor-latest.tar so capsem
            // support-bundle picks it up.
            let bundle_arg = if *bundle {
                " --bundle /shared/doctor-bundle.tar"
            } else {
                ""
            };
            let cmd: Vec<u8> = format!("capsem-doctor --durations=10{bundle_arg}\n").into_bytes();
            capsem_core::try_send!(
                "cli_doctor_terminal_input",
                tx.send(ServiceToProcess::TerminalInput { data: cmd }).await
            );

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

            // T4: copy the in-VM bundle out of virtiofs BEFORE delete_vm
            // tears down the session dir. The bundle path inside the
            // guest is /shared/doctor-bundle.tar which maps to
            // <session_dir>/guest/doctor-bundle.tar on the host.
            if *bundle {
                let session_dir = run_dir.join("instances").join(&vm_id);
                let candidates = [
                    session_dir.join("guest").join("doctor-bundle.tar"),
                    session_dir.join("workspace").join("doctor-bundle.tar"),
                ];
                let dest = run_dir.join("doctor-latest.tar");
                let mut copied = false;
                for src in &candidates {
                    if src.exists() {
                        if let Err(e) = std::fs::copy(src, &dest) {
                            eprintln!(
                                "warning: failed to copy doctor bundle from {} -> {}: {e}",
                                src.display(),
                                dest.display()
                            );
                        } else {
                            eprintln!(
                                "Doctor bundle: {} ({} bytes)",
                                dest.display(),
                                std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0)
                            );
                            copied = true;
                        }
                        break;
                    }
                }
                if !copied {
                    eprintln!("warning: no doctor bundle found in any of {} -- the in-VM script may have failed before tar", candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
                }
            }

            delete_vm(&client, &vm_id).await;
            mock_server.shutdown();
            if exit_code != 0 {
                eprintln!("Full log: {}", log_path.display());
                std::process::exit(exit_code);
            }
        }
    }

    Ok(())
}

/// Parse `SESSION:PATH` style argument. Returns `Some((session, path))`
/// or `None` if no `:` is present (i.e., a plain local path).
///
/// Treats the first `:` as the separator. SESSION may not contain `:`,
/// but PATH may (e.g., `vm:/root/file:0001`).
fn parse_session_arg(arg: &str) -> Option<(&str, &str)> {
    arg.split_once(':')
}

async fn handle_cp(client: &client::UdsClient, src: &str, dst: &str) -> Result<()> {
    use std::io::Write;
    let src_remote = parse_session_arg(src);
    let dst_remote = parse_session_arg(dst);

    match (src_remote, dst_remote) {
        (Some(_), Some(_)) => Err(anyhow::anyhow!(
            "guest-to-guest copy not supported -- only one of <src>, <dst> may be a SESSION:PATH"
        )),
        (None, None) => Err(anyhow::anyhow!(
            "neither argument is `SESSION:PATH`; use `cp` for host-to-host copies"
        )),
        // Download: SESSION:PATH -> local
        (Some((session, guest_path)), None) => {
            client::validate_id(session)?;
            let url = format!(
                "/vms/{session}/files/content?path={}",
                urlencoding::encode(guest_path)
            );
            let (bytes, _ct) = client.request_bytes("GET", &url, None, None).await?;
            if dst == "-" {
                std::io::stdout().write_all(&bytes)?;
            } else {
                std::fs::write(dst, &bytes).with_context(|| format!("write {dst}"))?;
                eprintln!(
                    "[cp] {} bytes  {}:{}  ->  {}",
                    bytes.len(),
                    session,
                    guest_path,
                    dst,
                );
            }
            Ok(())
        }
        // Upload: local -> SESSION:PATH
        (None, Some((session, guest_path))) => {
            client::validate_id(session)?;
            let bytes = if src == "-" {
                use std::io::Read;
                let mut buf = Vec::new();
                std::io::stdin().read_to_end(&mut buf)?;
                buf
            } else {
                std::fs::read(src).with_context(|| format!("read {src}"))?
            };
            let url = format!(
                "/vms/{session}/files/content?path={}",
                urlencoding::encode(guest_path)
            );
            let (resp_body, _ct) = client
                .request_bytes(
                    "POST",
                    &url,
                    Some(bytes.clone()),
                    Some("application/octet-stream"),
                )
                .await?;
            // POST handler returns JSON `{success, size}`; surface for sanity.
            let _ = resp_body;
            eprintln!(
                "[cp] {} bytes  {}  ->  {}:{}",
                bytes.len(),
                src,
                session,
                guest_path,
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_runtime_paths_are_derived_from_one_run_dir() {
        let run_dir = tempfile::tempdir().unwrap();
        let paths = cli_runtime_paths_from_run_dir(run_dir.path());

        assert_eq!(paths.service_socket, run_dir.path().join("service.sock"));
        assert_eq!(paths.gateway_port, run_dir.path().join("gateway.port"));
        assert_eq!(paths.gateway_token, run_dir.path().join("gateway.token"));
    }

    // -----------------------------------------------------------------------
    // CLI parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_no_subcommand() {
        let cli = Cli::try_parse_from(["capsem"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn parse_create_with_name() {
        let cli = Cli::parse_from(["capsem", "create", "-n", "my-vm"]);
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Create { name, .. }) => {
                assert_eq!(name, None);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_with_resources() {
        let cli = Cli::parse_from(["capsem", "create", "--ram", "8", "--cpu", "2"]);
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Resume { name }) => assert_eq!(name, "mydev"),
            _ => panic!("expected Resume"),
        }
    }

    #[test]
    fn parse_attach_alias_for_resume() {
        let cli = Cli::parse_from(["capsem", "attach", "mydev"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Resume { name }) => assert_eq!(name, "mydev"),
            _ => panic!("expected Resume via attach alias"),
        }
    }

    #[test]
    fn parse_suspend() {
        let cli = Cli::parse_from(["capsem", "suspend", "vm-123"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Suspend { session }) => {
                assert_eq!(session, "vm-123")
            }
            _ => panic!("expected Suspend"),
        }
    }

    #[test]
    fn parse_shell_positional() {
        let cli = Cli::parse_from(["capsem", "shell", "my-vm"]);
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Purge { all }) => assert!(!all),
            _ => panic!("expected Purge"),
        }
    }

    #[test]
    fn parse_purge_all() {
        let cli = Cli::parse_from(["capsem", "purge", "--all"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Purge { all }) => assert!(all),
            _ => panic!("expected Purge --all"),
        }
    }

    #[test]
    fn purge_summary_mentions_broken_persistent_for_default_purge() {
        let result = PurgeResponse {
            purged: 2,
            persistent_purged: 1,
            ephemeral_purged: 1,
        };
        assert_eq!(
            purge_summary_message(&result, false),
            "[*] Purged 2 sessions (1 broken persistent, 1 temporary)."
        );
    }

    #[test]
    fn purge_summary_keeps_temporary_only_message_when_no_defunct_persistent() {
        let result = PurgeResponse {
            purged: 3,
            persistent_purged: 0,
            ephemeral_purged: 3,
        };
        assert_eq!(
            purge_summary_message(&result, false),
            "[*] Purged 3 temporary sessions."
        );
    }

    #[test]
    fn parse_run() {
        let cli = Cli::parse_from(["capsem", "run", "echo hello"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Run {
                command,
                timeout,
                env,
            }) => {
                assert_eq!(command, "echo hello");
                assert_eq!(timeout, None);
                assert!(env.is_empty());
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn parse_run_with_timeout() {
        let cli = Cli::parse_from(["capsem", "run", "--timeout", "120", "ls -la"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Run {
                command,
                timeout,
                env,
            }) => {
                assert_eq!(command, "ls -la");
                assert_eq!(timeout, Some(120));
                assert!(env.is_empty());
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn parse_list() {
        let cli = Cli::parse_from(["capsem", "list"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Session(SessionCommands::List { quiet: false })
        ));
    }

    #[test]
    fn parse_list_quiet() {
        let cli = Cli::parse_from(["capsem", "list", "-q"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::List { quiet }) => assert!(quiet),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_list_quiet_long() {
        let cli = Cli::parse_from(["capsem", "list", "--quiet"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::List { quiet }) => assert!(quiet),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_status() {
        // `capsem status` is now the service status command
        let cli = Cli::parse_from(["capsem", "status"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Status)
        ));
    }

    #[test]
    fn service_control_commands_do_not_start_background_update_work() {
        for args in [
            &["capsem", "install"][..],
            &["capsem", "status"][..],
            &["capsem", "start"][..],
            &["capsem", "stop"][..],
            &["capsem", "version"][..],
            &["capsem", "debug"][..],
            &["capsem", "completions", "zsh"][..],
            &["capsem", "uninstall", "--yes"][..],
        ] {
            let cli = Cli::parse_from(args);
            let command = cli.command.as_ref().expect("parsed command");
            assert!(
                !should_refresh_update_cache_for_command(command),
                "{args:?} must stay a pure local control command"
            );
        }
    }

    #[test]
    fn session_commands_may_refresh_update_cache() {
        let cli = Cli::parse_from(["capsem", "list"]);
        let command = cli.command.as_ref().expect("parsed command");
        assert!(should_refresh_update_cache_for_command(command));
    }

    #[test]
    fn parse_debug_aliases_support_bundle() {
        let cli = Cli::parse_from(["capsem", "debug"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::SupportBundle { .. })
        ));
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
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Exec {
                session,
                command,
                timeout,
            }) => {
                assert_eq!(session, "my-vm");
                assert_eq!(command, "echo hello");
                assert_eq!(timeout, None);
            }
            _ => panic!("expected Exec"),
        }
    }

    #[test]
    fn parse_exec_with_timeout() {
        let cli = Cli::parse_from(["capsem", "exec", "--timeout", "120", "my-vm", "make build"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Exec {
                session,
                command,
                timeout,
            }) => {
                assert_eq!(session, "my-vm");
                assert_eq!(command, "make build");
                assert_eq!(timeout, Some(120));
            }
            _ => panic!("expected Exec"),
        }
    }

    #[test]
    fn parse_delete() {
        let cli = Cli::parse_from(["capsem", "delete", "vm-123"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Delete { session }) => assert_eq!(session, "vm-123"),
            _ => panic!("expected Delete"),
        }
    }

    #[test]
    fn parse_info() {
        let cli = Cli::parse_from(["capsem", "info", "vm-1"]);
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Restart { name }) => assert_eq!(name, "mydev"),
            _ => panic!("expected Restart"),
        }
    }

    #[test]
    fn parse_version() {
        let cli = Cli::parse_from(["capsem", "version"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Version)
        ));
    }

    #[test]
    fn parse_create_with_env() {
        let cli = Cli::parse_from(["capsem", "create", "-e", "FOO=bar", "-e", "BAZ=qux"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Create { env, .. }) => {
                assert_eq!(env, vec!["FOO=bar", "BAZ=qux"]);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_with_env_long() {
        let cli = Cli::parse_from(["capsem", "create", "--env", "API_KEY=secret123"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Create { env, .. }) => {
                assert_eq!(env, vec!["API_KEY=secret123"]);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_create_no_env() {
        let cli = Cli::parse_from(["capsem", "create"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Create { env, .. }) => {
                assert!(env.is_empty());
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::parse_from(["capsem", "doctor"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Doctor { bundle: false })
        ));
    }

    #[test]
    fn parse_doctor_bundle_flag() {
        let cli = Cli::parse_from(["capsem", "doctor", "--bundle"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Doctor { bundle: true })
        ));
    }

    #[test]
    fn parse_doctor_rejects_fast_escape_hatch() {
        let err = match Cli::try_parse_from(["capsem", "doctor", "--fast"]) {
            Ok(_) => panic!("doctor --fast must not be accepted"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("--fast"),
            "error should identify the retired flag: {err}"
        );
    }

    #[test]
    fn doctor_mock_server_addr_is_iptables_redirect_target() {
        assert_eq!(DOCTOR_MOCK_SERVER_ADDR, "127.0.0.1:3713");
    }

    #[test]
    fn parse_install() {
        let cli = Cli::parse_from(["capsem", "install"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Install)
        ));
    }

    #[test]
    fn parse_start() {
        let cli = Cli::parse_from(["capsem", "start"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Start)
        ));
    }

    #[test]
    fn parse_stop() {
        let cli = Cli::parse_from(["capsem", "stop"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Stop)
        ));
    }

    #[test]
    fn parse_setup_is_removed() {
        let err = match Cli::try_parse_from(["capsem", "setup", "--non-interactive"]) {
            Ok(_) => panic!("setup command must not parse after T5 removal"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn parse_assets_status() {
        let cli = Cli::parse_from(["capsem", "assets", "status"]);
        match cli.command.unwrap() {
            Commands::Assets(AssetsCommands::Status { profile, json }) => {
                assert_eq!(profile, "code");
                assert!(!json);
            }
            _ => panic!("expected assets status"),
        }
    }

    #[test]
    fn cli_default_profile_is_primary_profile() {
        assert_eq!(DEFAULT_PROFILE_ID, "code");
    }

    #[test]
    fn status_asset_lines_are_derived_from_profiles_status_payload() {
        let payload = serde_json::json!({
            "source": "installed",
            "profile_count": 1,
            "ready_count": 1,
            "asset_manifest": {
                "origin": "package",
                "path": "/tmp/manifest.json",
                "blake3": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "assets_current": "2026.0609.1",
                "binaries_current": "1.3.0"
            },
            "profiles": [
                {
                    "id": "code",
                    "name": "Code",
                    "ready": true,
                    "current_arch": "arm64",
                    "profile_payload_hash": "bbbbbbbbbbbb",
                    "missing_assets": []
                }
            ]
        });

        let lines = profile_status_summary_lines(&payload);

        assert!(lines
            .iter()
            .any(|line| line == "Profiles:  1/1 ready (installed)"));
        assert!(lines
            .iter()
            .any(|line| line == "Manifest:  package (/tmp/manifest.json)"));
        assert!(lines.iter().any(|line| line == "  assets:  2026.0609.1"));
        assert!(lines
            .iter()
            .any(|line| line == "  - code: Code (ready, arch arm64, hash bbbbbbbbbbbb)"));
    }

    #[test]
    fn health_issues_are_derived_from_profiles_status_payload() {
        let payload = serde_json::json!({
            "profile_count": 1,
            "profiles": [
                {
                    "id": "code",
                    "ready": false,
                    "missing_assets": ["initrd.img"],
                    "invalid_assets": ["rootfs.erofs"],
                    "invalid_files": ["profiles/code/enforcement.toml"]
                }
            ]
        });

        let issues = profile_status_issues(&payload);

        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("Profile code is not ready"));
        assert!(issues[0].contains("missing assets: initrd.img"));
        assert!(issues[0].contains("invalid assets: rootfs.erofs"));
        assert!(issues[0].contains("invalid profile files: profiles/code/enforcement.toml"));
    }

    #[test]
    fn parse_assets_ensure_json() {
        let cli = Cli::parse_from(["capsem", "assets", "ensure", "--json"]);
        match cli.command.unwrap() {
            Commands::Assets(AssetsCommands::Ensure { profile, json }) => {
                assert_eq!(profile, "code");
                assert!(json);
            }
            _ => panic!("expected assets ensure"),
        }
    }

    #[test]
    fn parse_assets_status_profile() {
        let cli = Cli::parse_from(["capsem", "assets", "status", "--profile", "analysis"]);
        match cli.command.unwrap() {
            Commands::Assets(AssetsCommands::Status { profile, json }) => {
                assert_eq!(profile, "analysis");
                assert!(!json);
            }
            _ => panic!("expected assets status"),
        }
    }

    #[test]
    fn parse_completions_bash() {
        let cli = Cli::parse_from(["capsem", "completions", "bash"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Completions {
                shell: clap_complete::Shell::Bash
            })
        ));
    }

    #[test]
    fn parse_uninstall() {
        let cli = Cli::parse_from(["capsem", "uninstall"]);
        match cli.command.unwrap() {
            Commands::Misc(MiscCommands::Uninstall { yes }) => assert!(!yes),
            _ => panic!("expected Uninstall"),
        }
    }

    #[test]
    fn parse_uninstall_yes() {
        let cli = Cli::parse_from(["capsem", "uninstall", "--yes"]);
        match cli.command.unwrap() {
            Commands::Misc(MiscCommands::Uninstall { yes }) => assert!(yes),
            _ => panic!("expected Uninstall"),
        }
    }

    #[test]
    fn parse_update() {
        let cli = Cli::parse_from(["capsem", "update"]);
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Fork {
                session,
                name,
                description,
            }) => {
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
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Fork {
                session,
                name,
                description,
            }) => {
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
        match cli.command.unwrap() {
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
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Create { from, .. }) => {
                assert_eq!(from, Some("old-img".into()));
            }
            _ => panic!("expected Create with --image alias"),
        }
    }

    #[test]
    fn parse_create_with_name_and_from() {
        let cli = Cli::parse_from(["capsem", "create", "-n", "my-session", "--from", "my-src"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Create { name, from, .. }) => {
                assert_eq!(name, Some("my-session".into()));
                assert_eq!(from, Some("my-src".into()));
            }
            _ => panic!("expected Create with name and --from"),
        }
    }

    #[test]
    fn shell_without_session_launches_tui_home() {
        assert_eq!(capsem_shell_tui_args(None), Vec::<String>::new());
    }

    #[test]
    fn shell_with_session_focuses_tui_session() {
        assert_eq!(
            capsem_shell_tui_args(Some("profile-v2")),
            vec!["--session".to_string(), "profile-v2".to_string()]
        );
    }
}

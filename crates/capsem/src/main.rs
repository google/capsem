mod client;
mod completions;
mod paths;
mod platform;
mod profile_catalog_source;
mod service_install;
mod setup;
mod shell_exit;
mod status;
mod support;
mod support_bundle;
mod uninstall;
mod update;

use anyhow::{Context, Result};
use clap::builder::styling::{AnsiColor, Color, Style, Styles};
use clap::{Parser, Subcommand, ValueEnum};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use client::{
    ApiResponse, ExecRequest, ExecResponse, ForkRequest, ForkResponse, HistoryResponse,
    ListResponse, LogsResponse, PersistRequest, ProvisionRequest, ProvisionResponse, PurgeRequest,
    PurgeResponse, RunRequest, SessionInfo, SessionProfileStatus, UdsClient,
};
use profile_catalog_source::read_profile_catalog_manifest;

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
  \x1b[32;1mpurge\x1b[0m        Destroy temporary sessions or reset product state

\x1b[36;1;4mService:\x1b[0m
  \x1b[32;1minstall\x1b[0m      Install as a system service (LaunchAgent / systemd)
  \x1b[32;1mstatus\x1b[0m       Show installed Capsem health and readiness
  \x1b[32;1mstart\x1b[0m        Start the background service
  \x1b[32;1mstop\x1b[0m         Stop the background service

\x1b[36;1;4mMCP:\x1b[0m
  \x1b[32;1mmcp list\x1b[0m       List Profile V2 MCP servers
  \x1b[32;1mmcp show\x1b[0m       Show one Profile V2 MCP server
  \x1b[32;1mmcp connectors\x1b[0m List Profile V2 MCP servers
  \x1b[32;1mmcp add\x1b[0m        Add a Profile V2 MCP server
  \x1b[32;1mmcp delete\x1b[0m     Delete a Profile V2 MCP server

\x1b[36;1;4mSecurity Rules:\x1b[0m
  \x1b[32;1menforcement list\x1b[0m    List runtime enforcement rules
  \x1b[32;1menforcement compile\x1b[0m Compile a runtime enforcement rule
  \x1b[32;1menforcement install\x1b[0m Install a runtime enforcement rule
  \x1b[32;1menforcement backtest\x1b[0m Backtest one enforcement rule against events
  \x1b[32;1mdetection list\x1b[0m      List runtime detection rules
  \x1b[32;1mdetection compile\x1b[0m   Compile a runtime detection rule
  \x1b[32;1mdetection backtest\x1b[0m  Backtest one detection rule against events
  \x1b[32;1mdetection hunt\x1b[0m      Hunt detection rules against events
  \x1b[32;1mdetection hunt-session\x1b[0m Backtest one detection rule against a session
  \x1b[32;1mconfirm list\x1b[0m       Show ask/confirm resolver state

\x1b[36;1;4mProfiles:\x1b[0m
  \x1b[32;1mprofile list\x1b[0m      List typed Profile V2 profiles
  \x1b[32;1mprofile create\x1b[0m    Create a user Profile V2 profile from a typed file
  \x1b[32;1mprofile show\x1b[0m      Show one typed Profile V2 profile
  \x1b[32;1mprofile resolve\x1b[0m   Resolve one profile to effective settings
  \x1b[32;1mprofile fork\x1b[0m      Fork a profile into a user profile
  \x1b[32;1mprofile delete\x1b[0m    Delete a user Profile V2 profile
  \x1b[32;1mprofile reconcile-catalog\x1b[0m Apply a signed profile catalog manifest
  \x1b[32;1mskills list\x1b[0m       List resolved Profile V2 skills
  \x1b[32;1mskills add\x1b[0m        Add a direct Profile V2 skill

\x1b[36;1;4mMisc:\x1b[0m
  \x1b[32;1msetup\x1b[0m        Run the first-time setup wizard
  \x1b[32;1mupdate\x1b[0m       Check for updates and install the latest version
  \x1b[32;1mdoctor\x1b[0m       Run diagnostic tests in a fresh session
  \x1b[32;1mdebug\x1b[0m        Print a redacted JSON debug report for bug reports
  \x1b[32;1mcompletions\x1b[0m  Generate shell completions (bash, zsh, fish, powershell)
  \x1b[32;1mversion\x1b[0m      Show version and build information
  \x1b[32;1muninstall\x1b[0m    Uninstall Capsem runtime, preserving user state";

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

    /// Manage runtime enforcement rules
    #[command(subcommand)]
    Enforcement(EnforcementCommands),

    /// Manage runtime detection rules
    #[command(subcommand)]
    Detection(DetectionCommands),

    /// Manage ask/confirm prompts
    #[command(subcommand)]
    Confirm(ConfirmCommands),

    /// Manage Profile V2 catalogs and installed revisions
    #[command(subcommand)]
    Profile(ProfileCommands),

    /// Manage Profile V2 skills
    #[command(subcommand)]
    Skills(SkillsCommands),

    #[command(flatten)]
    Misc(MiscCommands),
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum McpCommands {
    /// List Profile V2 MCP servers
    List {
        /// Profile id to inspect
        #[arg(long)]
        profile: Option<String>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Show one Profile V2 MCP server
    Show {
        /// MCP server id
        id: String,
        /// Profile id to inspect
        #[arg(long)]
        profile: Option<String>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// List Profile V2 MCP servers
    Connectors {
        /// Profile id to inspect
        #[arg(long)]
        profile: Option<String>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Add a Profile V2 MCP server to a user profile
    Add {
        /// MCP server id
        id: String,
        /// Profile id to mutate; defaults to the selected profile
        #[arg(long)]
        profile: Option<String>,
        /// Store the server disabled
        #[arg(long)]
        disabled: bool,
        /// MCP server transport type: stdio, http, or sse
        #[arg(long = "type")]
        server_type: Option<String>,
        /// Stdio MCP server command
        #[arg(long)]
        command: Option<String>,
        /// Stdio MCP server argument; repeat for multiple args
        #[arg(long = "arg", allow_hyphen_values = true)]
        args: Vec<String>,
        /// Stdio MCP server env var; repeat as KEY=VALUE
        #[arg(long = "env")]
        env: Vec<String>,
        /// HTTP/SSE MCP server URL
        #[arg(long)]
        url: Option<String>,
        /// HTTP/SSE MCP server header; repeat as KEY=VALUE
        #[arg(long = "header")]
        headers: Vec<String>,
        /// Bearer token for HTTP/SSE MCP server auth
        #[arg(long = "bearer-token")]
        bearer_token: Option<String>,
        /// Credential reference id; repeat for multiple credentials
        #[arg(long = "credential-ref")]
        credential_refs: Vec<String>,
        /// Allowed tool id; repeat for multiple tools
        #[arg(long = "allowed-tool")]
        allowed_tools: Vec<String>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Delete a direct user Profile V2 MCP server
    Delete {
        /// MCP server id
        id: String,
        /// Profile id to mutate; defaults to the selected profile
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliSecurityDecision {
    Allow,
    Ask,
    Block,
    Rewrite,
    Throttle,
}

impl CliSecurityDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Block => "block",
            Self::Rewrite => "rewrite",
            Self::Throttle => "throttle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl CliSeverity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliConfidence {
    Low,
    Medium,
    High,
}

impl CliConfidence {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliSkillKind {
    Group,
    Enabled,
    Disabled,
}

impl CliSkillKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Subcommand)]
enum EnforcementCommands {
    /// List installed runtime enforcement rules
    List {
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// List runtime enforcement rule match counters
    Stats {
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Validate and compile an enforcement rule without installing it
    Validate {
        /// Runtime rule id
        id: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Enforcement decision to return when the rule matches
        #[arg(long, value_enum)]
        decision: CliSecurityDecision,
        /// Optional pack id
        #[arg(long = "pack-id")]
        pack_id: Option<String>,
        /// Optional operator-facing reason
        #[arg(long)]
        reason: Option<String>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Compile an enforcement rule without installing it
    Compile {
        /// Runtime rule id
        id: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Enforcement decision to return when the rule matches
        #[arg(long, value_enum)]
        decision: CliSecurityDecision,
        /// Optional pack id
        #[arg(long = "pack-id")]
        pack_id: Option<String>,
        /// Optional operator-facing reason
        #[arg(long)]
        reason: Option<String>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Install or replace a runtime enforcement rule
    #[command(visible_alias = "add")]
    Install {
        /// Runtime rule id
        id: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Enforcement decision to return when the rule matches
        #[arg(long, value_enum)]
        decision: CliSecurityDecision,
        /// Optional pack id
        #[arg(long = "pack-id")]
        pack_id: Option<String>,
        /// Optional operator-facing reason
        #[arg(long)]
        reason: Option<String>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Update an installed runtime enforcement rule
    Update {
        /// Runtime rule id
        id: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Enforcement decision to return when the rule matches
        #[arg(long, value_enum)]
        decision: CliSecurityDecision,
        /// Optional pack id
        #[arg(long = "pack-id")]
        pack_id: Option<String>,
        /// Optional operator-facing reason
        #[arg(long)]
        reason: Option<String>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Backtest one enforcement rule against a JSON/JSONL event file
    Backtest {
        /// Runtime rule id
        id: String,
        /// JSON or JSONL file containing backtest events
        #[arg(long)]
        events: PathBuf,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Enforcement decision to return when the rule matches
        #[arg(long, value_enum)]
        decision: CliSecurityDecision,
        /// Optional pack id
        #[arg(long = "pack-id")]
        pack_id: Option<String>,
        /// Optional operator-facing reason
        #[arg(long)]
        reason: Option<String>,
        /// Maximum diverse matches to return
        #[arg(long)]
        limit: Option<usize>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Delete a runtime enforcement rule
    Delete {
        /// Runtime rule id
        id: String,
    },
}

#[derive(Subcommand)]
enum DetectionCommands {
    /// List installed runtime detection rules
    List {
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// List runtime detection rule match counters
    Stats {
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Validate and compile a detection rule without installing it
    Validate {
        /// Runtime rule id
        id: String,
        /// Runtime pack id
        #[arg(long = "pack-id")]
        pack_id: String,
        /// Detection title
        #[arg(long)]
        title: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Severity for emitted findings
        #[arg(long, value_enum)]
        severity: CliSeverity,
        /// Confidence for emitted findings
        #[arg(long, value_enum)]
        confidence: CliConfidence,
        /// Optional Sigma rule id
        #[arg(long = "sigma-id")]
        sigma_id: Option<String>,
        /// Finding tag; repeat for multiple tags
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Compile a detection rule without installing it
    Compile {
        /// Runtime rule id
        id: String,
        /// Runtime pack id
        #[arg(long = "pack-id")]
        pack_id: String,
        /// Detection title
        #[arg(long)]
        title: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Severity for emitted findings
        #[arg(long, value_enum)]
        severity: CliSeverity,
        /// Confidence for emitted findings
        #[arg(long, value_enum)]
        confidence: CliConfidence,
        /// Optional Sigma rule id
        #[arg(long = "sigma-id")]
        sigma_id: Option<String>,
        /// Finding tag; repeat for multiple tags
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Install or replace a runtime detection rule
    #[command(visible_alias = "add")]
    Install {
        /// Runtime rule id
        id: String,
        /// Runtime pack id
        #[arg(long = "pack-id")]
        pack_id: String,
        /// Detection title
        #[arg(long)]
        title: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Severity for emitted findings
        #[arg(long, value_enum)]
        severity: CliSeverity,
        /// Confidence for emitted findings
        #[arg(long, value_enum)]
        confidence: CliConfidence,
        /// Optional Sigma rule id
        #[arg(long = "sigma-id")]
        sigma_id: Option<String>,
        /// Finding tag; repeat for multiple tags
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Update an installed runtime detection rule
    Update {
        /// Runtime rule id
        id: String,
        /// Runtime pack id
        #[arg(long = "pack-id")]
        pack_id: String,
        /// Detection title
        #[arg(long)]
        title: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Severity for emitted findings
        #[arg(long, value_enum)]
        severity: CliSeverity,
        /// Confidence for emitted findings
        #[arg(long, value_enum)]
        confidence: CliConfidence,
        /// Optional Sigma rule id
        #[arg(long = "sigma-id")]
        sigma_id: Option<String>,
        /// Finding tag; repeat for multiple tags
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Backtest one detection rule against a JSON/JSONL event file
    Backtest {
        /// Runtime rule id
        id: String,
        /// JSON or JSONL file containing backtest events
        #[arg(long)]
        events: PathBuf,
        /// Runtime pack id
        #[arg(long = "pack-id")]
        pack_id: String,
        /// Detection title
        #[arg(long)]
        title: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Severity for emitted findings
        #[arg(long, value_enum)]
        severity: CliSeverity,
        /// Confidence for emitted findings
        #[arg(long, value_enum)]
        confidence: CliConfidence,
        /// Optional Sigma rule id
        #[arg(long = "sigma-id")]
        sigma_id: Option<String>,
        /// Finding tag; repeat for multiple tags
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Maximum diverse matches to return
        #[arg(long)]
        limit: Option<usize>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Hunt detection rules against a JSON/JSONL event file
    Hunt {
        /// Runtime rule id
        id: String,
        /// JSON or JSONL file containing backtest events
        #[arg(long)]
        events: PathBuf,
        /// Runtime pack id
        #[arg(long = "pack-id")]
        pack_id: String,
        /// Detection title
        #[arg(long)]
        title: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Severity for emitted findings
        #[arg(long, value_enum)]
        severity: CliSeverity,
        /// Confidence for emitted findings
        #[arg(long, value_enum)]
        confidence: CliConfidence,
        /// Optional Sigma rule id
        #[arg(long = "sigma-id")]
        sigma_id: Option<String>,
        /// Finding tag; repeat for multiple tags
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Maximum diverse matches to return
        #[arg(long)]
        limit: Option<usize>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Backtest one detection rule against a session database
    HuntSession {
        /// Session id/name
        session: String,
        /// Runtime rule id
        id: String,
        /// Runtime pack id
        #[arg(long = "pack-id")]
        pack_id: String,
        /// Detection title
        #[arg(long)]
        title: String,
        /// CEL condition using policy-context roots
        #[arg(long)]
        condition: String,
        /// Severity for emitted findings
        #[arg(long, value_enum)]
        severity: CliSeverity,
        /// Confidence for emitted findings
        #[arg(long, value_enum)]
        confidence: CliConfidence,
        /// Optional Sigma rule id
        #[arg(long = "sigma-id")]
        sigma_id: Option<String>,
        /// Finding tag; repeat for multiple tags
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Maximum diverse matches to return
        #[arg(long)]
        limit: Option<usize>,
        /// Store the rule disabled
        #[arg(long)]
        disabled: bool,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Delete a runtime detection rule
    Delete {
        /// Runtime rule id
        id: String,
    },
}

#[derive(Subcommand)]
enum ConfirmCommands {
    /// Show pending ask/confirm prompts or the disabled resolver state
    List {
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ProfileCommands {
    /// List typed Profile V2 profiles
    List {
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Create a user-owned Profile V2 profile from a typed TOML or JSON file
    Create {
        /// Profile document to parse and validate
        #[arg(long)]
        file: PathBuf,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Show one typed Profile V2 profile
    Show {
        /// Profile id to inspect
        profile_id: String,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Resolve one profile to VM-effective settings
    Resolve {
        /// Profile id to resolve
        profile_id: String,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Fork a profile into a user-owned Profile V2 profile
    Fork {
        /// Source profile id
        source_profile_id: String,
        /// New profile id
        #[arg(long)]
        id: String,
        /// New profile display name
        #[arg(long)]
        name: String,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Delete a user-owned Profile V2 profile
    Delete {
        /// Profile id to delete
        profile_id: String,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Show signed profile catalog and installed revision state
    Catalog {
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Show signed revisions for one catalog profile
    Revisions {
        /// Profile id to inspect
        profile_id: String,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Install an active signed catalog revision
    Install {
        /// Profile id to install
        profile_id: String,
        /// Specific revision to install; defaults to catalog current_revision
        #[arg(long)]
        revision: Option<String>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Reconcile a signed catalog revision lifecycle
    Update {
        /// Profile id to update
        profile_id: String,
        /// Profile document to parse, validate, and write through PUT /profiles/{id}
        #[arg(long, conflicts_with = "revision")]
        file: Option<PathBuf>,
        /// Specific revision to reconcile; defaults to catalog current_revision
        #[arg(long)]
        revision: Option<String>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Remove local launchable state for an installed profile revision
    Remove {
        /// Profile id to remove
        profile_id: String,
        /// Specific revision to remove; defaults to the installed revision
        #[arg(long)]
        revision: Option<String>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Apply a signed profile catalog manifest through the service
    ReconcileCatalog {
        /// Profile catalog manifest JSON file.
        #[arg(
            long,
            conflicts_with = "manifest_url",
            required_unless_present = "manifest_url"
        )]
        manifest: Option<PathBuf>,
        /// HTTPS profile catalog manifest URL (http:// is accepted only for loopback development).
        #[arg(
            long,
            conflicts_with = "manifest",
            required_unless_present = "manifest"
        )]
        manifest_url: Option<String>,
        /// Minisign public key file used to verify profile payloads
        #[arg(long)]
        pubkey: PathBuf,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum SkillsCommands {
    /// List resolved Profile V2 skills
    List {
        /// Profile id to inspect; defaults to selected profile
        #[arg(long)]
        profile: Option<String>,
        /// Restrict results to one skill list
        #[arg(long, value_enum)]
        kind: Option<CliSkillKind>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Show one resolved Profile V2 skill
    Show {
        /// Skill id
        id: String,
        /// Profile id to inspect; defaults to selected profile
        #[arg(long)]
        profile: Option<String>,
        /// Restrict lookup to one skill list
        #[arg(long, value_enum)]
        kind: Option<CliSkillKind>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Add a direct Profile V2 skill entry to a user profile
    Add {
        /// Skill id
        id: String,
        /// Profile id to mutate; defaults to selected profile
        #[arg(long)]
        profile: Option<String>,
        /// Skill list to mutate
        #[arg(long, value_enum, default_value = "enabled")]
        kind: CliSkillKind,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
    /// Delete a direct Profile V2 skill entry from a user profile
    Delete {
        /// Skill id
        id: String,
        /// Profile id to mutate; defaults to selected profile
        #[arg(long)]
        profile: Option<String>,
        /// Skill list to mutate; defaults to enabled
        #[arg(long, value_enum)]
        kind: Option<CliSkillKind>,
        /// Print the raw JSON response
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    /// Create and boot a new session
    ///
    /// Sessions are ephemeral by default and destroyed on delete. Pass a
    /// positional name to create a persistent session that survives
    /// suspend/resume cycles.
    Create {
        /// Name for the session (makes it persistent -- "if you name it, you keep it")
        #[arg(value_name = "NAME")]
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
        #[arg(long)]
        from: Option<String>,
        /// Profile id for a fresh VM
        #[arg(long)]
        profile: Option<String>,
        /// Exact installed profile revision for a fresh VM
        #[arg(long = "profile-revision")]
        profile_revision: Option<String>,
    },
    /// Open an interactive shell in a session
    ///
    /// With no arguments, creates a temporary session (destroyed on exit).
    /// Pass a session name/ID to attach to an existing running session.
    Shell {
        /// Name or ID of the session (positional)
        #[arg(value_name = "SESSION")]
        session: Option<String>,
    },
    /// Resume a suspended session or attach to a running one
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
        /// Profile id for the temporary VM
        #[arg(long)]
        profile: Option<String>,
        /// Exact installed profile revision for the temporary VM
        #[arg(long = "profile-revision")]
        profile_revision: Option<String>,
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
    /// Export session security events as policy-context fixture JSONL
    ExportPolicyContexts {
        /// Name or ID of the session
        #[arg(value_name = "SESSION")]
        session: String,
        /// Output the full JSON export envelope instead of JSONL fixtures
        #[arg(long)]
        json: bool,
    },
    /// Delete a session and all its state
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
    /// Destroy temporary sessions or reset product state
    ///
    /// Use --all to also destroy persistent sessions (requires confirmation).
    /// Use --product for a destructive whole-product reset.
    Purge {
        /// Also destroy persistent sessions (requires confirmation)
        #[arg(long, default_value_t = false)]
        all: bool,
        /// Remove runtime and all durable user state. Requires confirmation unless --yes is passed.
        #[arg(long, default_value_t = false)]
        product: bool,
        /// Skip confirmation prompt for --product.
        #[arg(long, short, default_value_t = false)]
        yes: bool,
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
        /// Tell the in-VM doctor to package its diagnostic surface
        /// (pytest output + junit, /var/log, dmesg, /proc/{mounts,cmdline},
        /// session.db) into a tar that capsem support-bundle picks up
        /// at `~/.capsem/run/doctor-latest.tar`.
        #[arg(long)]
        bundle: bool,
    },
    /// Print a redacted JSON debug report for bug reports
    Debug,
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
    /// Secrets in service.toml/profile TOML and bearer tokens in log lines are
    /// stripped by default. The bundle excludes rootfs.img unless
    /// `--include-rootfs` is passed.
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
    /// Uninstall Capsem runtime, preserving user state
    Uninstall {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
    /// Install capsem as a system service (LaunchAgent on macOS, systemd on Linux)
    Install,
    /// Show installed Capsem health and readiness
    Status {
        /// Output a machine-readable status report
        #[arg(long)]
        json: bool,
    },
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

fn format_session_profile_for_list(session: &client::SessionInfo) -> String {
    match (
        session.profile_id.as_deref(),
        session.profile_revision.as_deref(),
        session.profile_status,
    ) {
        (_, _, Some(SessionProfileStatus::Corrupted)) => "corrupted".to_string(),
        (Some(profile_id), Some(revision), Some(status)) => {
            format!("{profile_id}@{revision}:{}", status.as_str())
        }
        (Some(profile_id), Some(revision), None) => format!("{profile_id}@{revision}"),
        (Some(profile_id), None, Some(status)) => format!("{profile_id}:{}", status.as_str()),
        (Some(profile_id), None, None) => profile_id.to_string(),
        (None, None, Some(status)) => status.as_str().to_string(),
        _ => "-".to_string(),
    }
}

fn tail_log_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= n {
        text.to_string()
    } else {
        lines[lines.len() - n..].join("\n")
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SecurityLogSummary {
    event_count: usize,
    blocked_count: usize,
    detection_count: u64,
    families: std::collections::BTreeMap<String, usize>,
    rules: std::collections::BTreeMap<String, usize>,
}

fn security_log_summary(security_logs: &str) -> SecurityLogSummary {
    let mut summary = SecurityLogSummary::default();
    for line in security_logs.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(fields) = value.get("fields").and_then(|fields| fields.as_object()) else {
            continue;
        };
        if fields.get("message").and_then(|value| value.as_str()) != Some("resolved_security_event")
        {
            continue;
        }
        summary.event_count += 1;
        if let Some(family) = fields.get("event_family").and_then(|value| value.as_str()) {
            *summary.families.entry(family.to_string()).or_default() += 1;
        }
        if fields.get("final_action").and_then(|value| value.as_str()) == Some("block") {
            summary.blocked_count += 1;
        }
        if let Some(finding_count) = fields.get("finding_count").and_then(|value| value.as_u64()) {
            summary.detection_count += finding_count;
        }
        if let Some(rule_id) = fields.get("rule_id").and_then(|value| value.as_str()) {
            *summary.rules.entry(rule_id.to_string()).or_default() += 1;
        }
        if let Some(rule_ids) = fields
            .get("detection_rule_ids")
            .and_then(|value| value.as_str())
        {
            for rule_id in rule_ids.split(',').filter(|rule_id| !rule_id.is_empty()) {
                *summary.rules.entry(rule_id.to_string()).or_default() += 1;
            }
        }
    }
    summary
}

fn format_security_log_summary(summary: &SecurityLogSummary) -> Option<String> {
    if summary.event_count == 0 {
        return None;
    }
    let families = summary
        .families
        .iter()
        .map(|(family, count)| format!("{family}={count}"))
        .collect::<Vec<_>>()
        .join(",");
    let rules = summary
        .rules
        .iter()
        .take(5)
        .map(|(rule_id, count)| format!("{rule_id}={count}"))
        .collect::<Vec<_>>()
        .join(",");
    Some(format!(
        "summary: events={} blocked={} detections={} families={} rules={}",
        summary.event_count,
        summary.blocked_count,
        summary.detection_count,
        if families.is_empty() { "-" } else { &families },
        if rules.is_empty() { "-" } else { &rules },
    ))
}

fn format_session_logs(session: &str, logs: LogsResponse, tail: Option<usize>) -> String {
    let mut output = String::new();

    if let Some(security_logs) = logs.security_logs {
        output.push_str(&format!("--- Security Events ({session}) ---\n"));
        if let Some(summary) = format_security_log_summary(&security_log_summary(&security_logs)) {
            output.push_str(&summary);
            output.push('\n');
        }
        output.push_str(&match tail {
            Some(n) => tail_log_lines(&security_logs, n),
            None => security_logs,
        });
        output.push('\n');
    }

    if let Some(process_logs) = logs.process_logs {
        output.push_str(&format!("--- Process Logs ({session}) ---\n"));
        output.push_str(&match tail {
            Some(n) => tail_log_lines(&process_logs, n),
            None => process_logs,
        });
        output.push('\n');
    }

    if let Some(serial_logs) = logs.serial_logs {
        output.push_str(&format!("--- Serial Logs ({session}) ---\n"));
        output.push_str(&match tail {
            Some(n) => tail_log_lines(&serial_logs, n),
            None => serial_logs,
        });
        output.push('\n');
    } else if !logs.logs.is_empty() {
        output.push_str(&format!("--- Serial Logs ({session}) ---\n"));
        output.push_str(&match tail {
            Some(n) => tail_log_lines(&logs.logs, n),
            None => logs.logs,
        });
        output.push('\n');
    }

    output
}

fn enforcement_rule_body(
    id: &str,
    condition: &str,
    decision: CliSecurityDecision,
    pack_id: &Option<String>,
    reason: &Option<String>,
    disabled: bool,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "pack_id": pack_id,
        "condition": condition,
        "decision": decision.as_str(),
        "reason": reason,
        "enabled": !disabled,
    })
}

#[allow(clippy::too_many_arguments)]
fn detection_rule_body(
    id: &str,
    pack_id: &str,
    title: &str,
    condition: &str,
    severity: CliSeverity,
    confidence: CliConfidence,
    sigma_id: &Option<String>,
    tags: &[String],
    disabled: bool,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "pack_id": pack_id,
        "sigma_id": sigma_id,
        "title": title,
        "condition": condition,
        "severity": severity.as_str(),
        "confidence": confidence.as_str(),
        "tags": tags,
        "enabled": !disabled,
    })
}

fn read_runtime_backtest_events(path: &Path) -> Result<Vec<serde_json::Value>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read runtime backtest events {}", path.display()))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        anyhow::bail!("runtime backtest events file is empty: {}", path.display());
    }

    if trimmed.starts_with('[') {
        return serde_json::from_str(trimmed)
            .with_context(|| format!("parse runtime backtest events array {}", path.display()));
    }

    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(events) = value.get("events").and_then(serde_json::Value::as_array) {
                return Ok(events.clone());
            }
            return Ok(vec![value]);
        }
    }

    let mut events = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).with_context(|| {
            format!(
                "parse runtime backtest JSONL event {} in {}",
                index + 1,
                path.display()
            )
        })?;
        events.push(value);
    }
    if events.is_empty() {
        anyhow::bail!(
            "runtime backtest events file had no JSON events: {}",
            path.display()
        );
    }
    Ok(events)
}

fn read_profile_document(path: &Path) -> Result<capsem_core::settings_profiles::Profile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read Profile V2 document {}", path.display()))?;
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') {
        let profile = serde_json::from_str::<capsem_core::settings_profiles::Profile>(&text)
            .with_context(|| format!("parse Profile V2 JSON {}", path.display()))?;
        profile
            .validate()
            .with_context(|| format!("validate Profile V2 JSON {}", path.display()))?;
        return Ok(profile);
    }
    capsem_core::settings_profiles::Profile::from_toml_str(&text)
        .with_context(|| format!("parse Profile V2 TOML {}", path.display()))
}

fn mcp_connectors_path(profile: Option<&String>) -> String {
    let mut path = "/mcp/connectors".to_string();
    if let Some(profile) = profile {
        path.push_str(&format!("?profile={}", urlencoding::encode(profile)));
    }
    path
}

fn format_mcp_connectors_summary(result: &serde_json::Value) -> String {
    let mut output = String::new();
    let servers = result["servers"].as_array().cloned().unwrap_or_default();
    if servers.is_empty() {
        output.push_str("No MCP servers configured.\n");
        return output;
    }
    writeln!(
        output,
        "{:<24} {:<8} {:<8} {:<18} {:<10} ALLOWED_TOOLS",
        "ID", "ENABLED", "TYPE", "TARGET", "SOURCE"
    )
    .expect("write to string");
    for server in servers {
        let config = &server["server"];
        let allowed = config["capsem"]["allowed_tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        let target = config["command"]
            .as_str()
            .or_else(|| config["url"].as_str())
            .unwrap_or("-");
        writeln!(
            output,
            "{:<24} {:<8} {:<8} {:<18} {:<10} {}",
            server["id"].as_str().unwrap_or("-"),
            if config["enabled"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            config["type"].as_str().unwrap_or("-"),
            target,
            server["source_profile"].as_str().unwrap_or("-"),
            allowed,
        )
        .expect("write to string");
    }
    output
}

fn mcp_server_matches(result: &serde_json::Value, id: &str) -> Vec<serde_json::Value> {
    result["servers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|server| server["id"].as_str() == Some(id))
        .cloned()
        .collect()
}

fn print_runtime_rule_list_summary(kind: &str, result: &serde_json::Value) {
    let rules = result["rules"].as_array().cloned().unwrap_or_default();
    if rules.is_empty() {
        println!("No runtime {kind} rules installed.");
        return;
    }
    #[allow(clippy::print_literal)]
    {
        println!(
            "{:<28} {:<8} {:<8} {:<8} CONDITION",
            "ID", "ENABLED", "MATCHES", "PLAN"
        );
    }
    for rule in rules {
        let plan = rule["compiled_plan"].as_str().unwrap_or("-");
        println!(
            "{:<28} {:<8} {:<8} {:<8} {}",
            rule["id"].as_str().unwrap_or("-"),
            if rule["enabled"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            rule["match_count"].as_u64().unwrap_or(0),
            plan,
            rule["condition"].as_str().unwrap_or("-"),
        );
    }
}

fn print_runtime_compile_summary(kind: &str, result: &serde_json::Value) {
    println!(
        "{} rule compiled: {} ({})",
        kind,
        result["id"].as_str().unwrap_or("-"),
        result["compiled_plan"].as_str().unwrap_or("-"),
    );
}

fn print_runtime_install_summary(kind: &str, result: &serde_json::Value) {
    let rule = &result["rule"];
    println!(
        "{} rule installed: {} ({})",
        kind,
        rule["id"].as_str().unwrap_or("-"),
        rule["compiled_plan"].as_str().unwrap_or("-"),
    );
}

fn print_runtime_hunt_summary(result: &serde_json::Value) {
    print!("{}", format_runtime_hunt_summary(result));
}

fn format_runtime_hunt_summary(result: &serde_json::Value) -> String {
    format_runtime_match_summary("Detection hunt", result)
}

fn print_runtime_backtest_summary(kind: &str, result: &serde_json::Value) {
    print!("{}", format_runtime_match_summary(kind, result));
}

fn format_runtime_match_summary(kind: &str, result: &serde_json::Value) -> String {
    let mut output = String::new();
    let truncated = if result["truncated"].as_bool().unwrap_or(false) {
        " (truncated)"
    } else {
        ""
    };
    writeln!(
        output,
        "{} matched {} event(s), {} unique evidence signature(s){}.",
        kind,
        result["total_matches"].as_u64().unwrap_or(0),
        result["unique_evidence_matches"].as_u64().unwrap_or(0),
        truncated
    )
    .expect("write to string");

    let Some(rows) = result["rows"].as_array() else {
        return output;
    };
    if rows.is_empty() {
        return output;
    }

    writeln!(output, "Matches:").expect("write to string");
    for row in rows {
        let event_ref = &row["event_ref"];
        let event_id = event_ref["event_id"].as_str().unwrap_or("-");
        let corpus = event_ref["corpus"].as_str().unwrap_or("-");
        let session = event_ref["session_id"].as_str().unwrap_or("-");
        let rule_id = row["rule_id"].as_str().unwrap_or("-");
        let pack_id = row["pack_id"].as_str().unwrap_or("-");
        let outcome = runtime_hunt_outcome_text(&row["outcome"]);
        writeln!(
            output,
            "- event={} session={} corpus={} rule={} pack={} outcome={}",
            event_id, session, corpus, rule_id, pack_id, outcome
        )
        .expect("write to string");
        if let Some(fields) = row["matched_fields"].as_array() {
            for field in fields.iter().take(8) {
                let path = field["path"].as_str().unwrap_or("-");
                writeln!(
                    output,
                    "  {}={}",
                    path,
                    runtime_hunt_field_value_text(&field["value"])
                )
                .expect("write to string");
            }
            if fields.len() > 8 {
                writeln!(output, "  ... {} more field(s)", fields.len() - 8)
                    .expect("write to string");
            }
        }
    }
    output
}

fn runtime_hunt_outcome_text(value: &serde_json::Value) -> String {
    if let Some(outcome) = value.as_str() {
        return outcome.to_owned();
    }
    value
        .get("outcome")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn runtime_hunt_field_value_text(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn skills_path(profile: Option<&String>, kind: Option<CliSkillKind>) -> String {
    let mut params = Vec::new();
    if let Some(profile) = profile {
        params.push(format!("profile={}", urlencoding::encode(profile)));
    }
    if let Some(kind) = kind {
        params.push(format!("kind={}", kind.as_str()));
    }
    if params.is_empty() {
        "/skills".to_string()
    } else {
        format!("/skills?{}", params.join("&"))
    }
}

fn format_skills_summary(result: &serde_json::Value) -> String {
    let mut output = String::new();
    let skills = result["skills"].as_array().cloned().unwrap_or_default();
    if skills.is_empty() {
        writeln!(
            output,
            "No skills configured for profile {}.",
            result["profile_id"].as_str().unwrap_or("-")
        )
        .expect("write to string");
        return output;
    }
    writeln!(
        output,
        "{:<32} {:<9} {:<18} {:<7} EDITABLE",
        "ID", "KIND", "SOURCE_PROFILE", "DIRECT"
    )
    .expect("write to string");
    for skill in skills {
        writeln!(
            output,
            "{:<32} {:<9} {:<18} {:<7} {}",
            skill["id"].as_str().unwrap_or("-"),
            skill["kind"].as_str().unwrap_or("-"),
            skill["source_profile"].as_str().unwrap_or("-"),
            if skill["direct"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            if skill["editable"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
        )
        .expect("write to string");
    }
    output
}

fn skill_matches(result: &serde_json::Value, id: &str) -> Vec<serde_json::Value> {
    result["skills"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|skill| skill["id"].as_str() == Some(id))
        .cloned()
        .collect()
}

fn format_confirm_list_summary(result: &serde_json::Value) -> String {
    let resolve_available = result["resolve_available"].as_bool().unwrap_or(false);
    let pending_count = result["pending_count"].as_u64().unwrap_or(0);
    if !resolve_available {
        return format!(
            "Ask/confirm resolver unavailable; owner={} pending={pending_count}",
            result["resolve_owner"].as_str().unwrap_or("-")
        );
    }
    format!("Pending confirmations: {pending_count}")
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
    let profile = format_session_profile_for_list(info);
    if profile != "-" {
        println!("Profile: {}", profile);
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

async fn run_shell(id: &str, run_dir: &std::path::Path) -> Result<()> {
    use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
    use nix::sys::termios::{tcgetattr, tcsetattr, SetArg};
    use std::sync::Arc;
    use tokio_unix_ipc::{channel_from_std, Receiver, Sender};

    client::validate_id(id)?;
    let sock_path = run_dir.join("instances").join(format!("{}.sock", id));
    if !sock_path.exists() {
        anyhow::bail!("Session socket not found at: {}", sock_path.display());
    }

    let stream = tokio::net::UnixStream::connect(&sock_path)
        .await
        .context("failed to connect to VM session")?;
    let mut std_stream = stream.into_std()?;
    capsem_core::ipc_handshake::negotiate_initiator(
        &mut std_stream,
        "capsem-cli",
        capsem_core::telemetry::current_parent_traceparent(),
    )
    .context("IPC handshake failed")?;
    #[allow(unused_variables)]
    let (tx, rx): (Sender<ServiceToProcess>, Receiver<ProcessToService>) =
        channel_from_std(std_stream)?;
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
            capsem_core::try_send!(
                "cli_terminal_resize_init",
                tx.send(ServiceToProcess::TerminalResize { cols, rows })
                    .await
            );
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

    let _guard = RawModeGuard {
        fd: stdin_fd,
        original: original_termios,
    };

    let mut stdin = tokio::io::stdin();
    let mut buf = vec![0u8; 65536];

    // Spawn a task to read from IPC and write to stdout
    let mut output_task = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Ok(msg) = rx.recv().await {
            match msg {
                ProcessToService::TerminalOutput { data } => {
                    // Smoking-gun trace mirrored from capsem-process. If a
                    // payload prefix looks like an IPC frame, dump the
                    // first 16 bytes to stderr (visible to the user, also
                    // capturable via `capsem shell 2>shell.log`). Catches
                    // the leak even when process.log isn't being tailed.
                    if shell_exit::looks_like_msgpack_ipc_frame(&data) {
                        let preview: Vec<String> =
                            data.iter().take(16).map(|b| format!("{:02x}", b)).collect();
                        eprintln!(
                            "\r\n[capsem-shell] WARN: PTY stream starts with IPC-frame-shaped bytes \
                             (len={}, first16={})\r",
                            data.len(),
                            preview.join(" "),
                        );
                    }
                    let _ = stdout.write_all(&data).await;
                    let _ = stdout.flush().await;
                }
                ProcessToService::Pong => {}
                ProcessToService::ReloadConfigResult { .. } => {}
                ProcessToService::StateChanged { .. } => {}
                ProcessToService::ExecResult { .. } => {}
                ProcessToService::WriteFileResult { .. } => {}
                ProcessToService::ReadFileResult { .. } => {}
                ProcessToService::ShutdownRequested { .. }
                | ProcessToService::SuspendRequested { .. }
                | ProcessToService::SnapshotReady { .. }
                | ProcessToService::MetricsSnapshot { .. }
                | ProcessToService::RuntimeRuleMatches { .. } => {}
            }
        }
    });

    let mut sigwinch =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;

    // Read from stdin and send over IPC.
    // Also watch for output_task completion (VM connection closed).
    loop {
        tokio::select! {
            _ = sigwinch.recv() => {
                if is_tty {
                    if let Some((cols, rows)) = get_terminal_size() {
                        capsem_core::try_send!("cli_terminal_resize", tx.send(ServiceToProcess::TerminalResize { cols, rows }).await);
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
                        capsem_core::try_send!("cli_terminal_input", tx.send(ServiceToProcess::TerminalInput { data: buf[..n].to_vec() }).await);
                    }
                    Err(_) => break,
                }
            }
        }
    }

    // ---- Clean shell exit ----
    // Order matters and is asserted by tests in shell_exit::tests:
    //  1. Tell the host to stop streaming so no new TerminalOutput frames
    //     get queued for this connection.
    //  2. Abort the local output task. tokio JoinHandle drop does NOT
    //     cancel; without abort the task lives on, holds stdout, and any
    //     in-flight TerminalOutput frame will write to the user's parent
    //     shell after raw mode is restored. This is the symptom that
    //     manifested as "MessagePack-shaped garbage in my terminal after
    //     `capsem shell`".
    //  3. Drop tx to close the IPC writer half (defensive; the next read
    //     loop will hit ECONNRESET and the connection winds down cleanly).
    //  4. Reset the terminal: SGR reset + show cursor + move to col 0.
    //     RawModeGuard restores termios on Drop right after this, but
    //     in-flight escape sequences from the guest can leave the terminal
    //     in a weird state (alt screen, scroll region, cursor hidden).
    capsem_core::try_send!(
        "cli_stop_terminal_stream",
        tx.send(ServiceToProcess::StopTerminalStream).await
    );
    output_task.abort();
    drop(tx);
    shell_exit::reset_user_terminal(is_tty).await;
    Ok(())
}

fn command_refreshes_update_cache(command: Option<&Commands>) -> bool {
    !matches!(
        command,
        Some(Commands::Misc(MiscCommands::Uninstall { .. }))
            | Some(Commands::Session(SessionCommands::Purge {
                product: true,
                ..
            }))
    )
}

fn print_profile_catalog_reconcile_summary(result: &serde_json::Value) {
    println!("{}", profile_catalog_reconcile_summary_line(result));
    if let Some(outcomes) = result["outcomes"].as_array() {
        for outcome in outcomes {
            let profile_id = outcome["profile_id"].as_str().unwrap_or("-");
            let revision = outcome["revision"].as_str().unwrap_or("-");
            let status = outcome["outcome"].as_str().unwrap_or("unknown");
            if let Some(error) = outcome["error"].as_str() {
                println!("  {profile_id}@{revision}: {status} ({error})");
            } else {
                println!("  {profile_id}@{revision}: {status}");
            }
        }
    }
}

fn print_profile_catalog_summary(result: &serde_json::Value) {
    println!("{}", profile_catalog_summary_line(result));
    if let Some(profiles) = result["profiles"].as_array() {
        for profile in profiles {
            let profile_id = profile["profile_id"].as_str().unwrap_or("-");
            let current = profile["current_revision"].as_str().unwrap_or("-");
            let installed = profile["installed_revision"].as_str().unwrap_or("-");
            println!("  {profile_id}: current={current} installed={installed}");
            if let Some(revisions) = profile["revisions"].as_array() {
                for revision in revisions {
                    let revision_id = revision["revision"].as_str().unwrap_or("-");
                    let status = revision["status"].as_str().unwrap_or("unknown");
                    let marker = if revision["installed"].as_bool().unwrap_or(false) {
                        " installed"
                    } else if revision["current"].as_bool().unwrap_or(false) {
                        " current"
                    } else {
                        ""
                    };
                    println!("    {revision_id}: {status}{marker}");
                }
            }
        }
    }
}

fn print_profile_revisions_summary(result: &serde_json::Value) {
    println!("{}", profile_revisions_summary_line(result));
    if let Some(revisions) = result["revisions"].as_array() {
        for revision in revisions {
            let revision_id = revision["revision"].as_str().unwrap_or("-");
            let status = revision["status"].as_str().unwrap_or("unknown");
            let marker = if revision["installed"].as_bool().unwrap_or(false) {
                " installed"
            } else if revision["current"].as_bool().unwrap_or(false) {
                " current"
            } else {
                ""
            };
            println!("  {revision_id}: {status}{marker}");
        }
    }
}

fn print_profile_revision_action_summary(result: &serde_json::Value) {
    println!("{}", profile_revision_action_summary_line(result));
}

fn format_profile_list_summary(result: &serde_json::Value) -> String {
    let mut output = String::new();
    let profiles = result["profiles"].as_array().cloned().unwrap_or_default();
    if profiles.is_empty() {
        writeln!(output, "No Profile V2 profiles discovered.").expect("write to string");
        return output;
    }
    writeln!(
        output,
        "{:<24} {:<20} {:<8} {:<7} EXTENDS",
        "ID", "NAME", "SOURCE", "LOCKED"
    )
    .expect("write to string");
    for record in profiles {
        let profile = &record["profile"];
        writeln!(
            output,
            "{:<24} {:<20} {:<8} {:<7} {}",
            profile["id"].as_str().unwrap_or("-"),
            profile["name"].as_str().unwrap_or("-"),
            record["source"].as_str().unwrap_or("-"),
            if record["locked"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            profile["extends_profile_id"].as_str().unwrap_or("-"),
        )
        .expect("write to string");
    }
    output
}

fn format_profile_record_summary(record: &serde_json::Value) -> String {
    let profile = &record["profile"];
    let mut output = String::new();
    writeln!(
        output,
        "Profile: {} ({})",
        profile["id"].as_str().unwrap_or("-"),
        profile["name"].as_str().unwrap_or("-")
    )
    .expect("write to string");
    writeln!(
        output,
        "Source: {} locked={}",
        record["source"].as_str().unwrap_or("-"),
        record["locked"].as_bool().unwrap_or(false)
    )
    .expect("write to string");
    if let Some(parent) = profile["extends_profile_id"].as_str() {
        writeln!(output, "Extends: {parent}").expect("write to string");
    }
    writeln!(
        output,
        "UI: {} type={}",
        profile["ui"].as_str().unwrap_or("-"),
        profile["profile_type"].as_str().unwrap_or("-")
    )
    .expect("write to string");
    write_profile_contract_summary(&mut output, &profile["packages"], &profile["tools"]);
    writeln!(
        output,
        "MCP: servers={}",
        profile["mcpServers"]
            .as_object()
            .map(|items| items.len())
            .unwrap_or(0)
    )
    .expect("write to string");
    write_profile_vm_summary(&mut output, &profile["vm"]);
    output
}

fn format_profile_resolve_summary(result: &serde_json::Value) -> String {
    let effective = &result["effective"];
    let rules = effective["rules"]
        .as_array()
        .map(|rules| rules.len())
        .unwrap_or(0);
    let mcp_servers = effective["mcp"]["value"]
        .as_object()
        .map(|servers| servers.len())
        .unwrap_or(0);
    let skills = ["groups", "enabled", "disabled"]
        .iter()
        .map(|key| {
            effective["skills"]["value"][*key]
                .as_array()
                .map(|items| items.len())
                .unwrap_or(0)
        })
        .sum::<usize>();
    let mut output = format!(
        "Profile resolved: profile={} name={} ui={} rules={} mcp_servers={} skills={} tools={}",
        result["profile_id"].as_str().unwrap_or("-"),
        effective["profile_name"].as_str().unwrap_or("-"),
        effective["profile_ui"].as_str().unwrap_or("-"),
        rules,
        mcp_servers,
        skills,
        effective["tools"]["value"]
            .as_object()
            .map(|tools| tools.len())
            .unwrap_or(0),
    );
    output.push('\n');
    write_profile_contract_summary(
        &mut output,
        &effective["packages"]["value"],
        &effective["tools"]["value"],
    );
    write_profile_vm_summary(&mut output, &effective["vm"]["value"]);
    output
}

fn write_profile_contract_summary(
    output: &mut String,
    packages: &serde_json::Value,
    tools: &serde_json::Value,
) {
    let system = &packages["system"];
    let distro = system["distro"].as_str().unwrap_or("-");
    let release = system["release"].as_str().unwrap_or("-");
    writeln!(
        output,
        "Packages: runtimes={} python={} node={} apt={} distro={} release={}",
        packages["runtimes"]
            .as_object()
            .map(|items| items.len())
            .unwrap_or(0),
        packages["python_modules"]
            .as_object()
            .map(|items| items.len())
            .unwrap_or(0),
        packages["node_packages"]
            .as_object()
            .map(|items| items.len())
            .unwrap_or(0),
        system["apt"]
            .as_object()
            .map(|items| items.len())
            .unwrap_or(0),
        if distro.is_empty() { "-" } else { distro },
        if release.is_empty() { "-" } else { release },
    )
    .expect("write to string");
    writeln!(
        output,
        "Tools: {}",
        tools.as_object().map(|items| items.len()).unwrap_or(0)
    )
    .expect("write to string");
}

fn write_profile_vm_summary(output: &mut String, vm: &serde_json::Value) {
    let assets = &vm["assets"];
    writeln!(
        output,
        "VM: memory_mib={} cpus={} network={} asset_arches={}",
        vm["memory_mib"].as_u64().unwrap_or(0),
        vm["cpus"].as_u64().unwrap_or(0),
        vm["network"].as_str().unwrap_or("-"),
        assets.as_object().map(|items| items.len()).unwrap_or(0),
    )
    .expect("write to string");
    if let Some(assets) = assets.as_object() {
        for (arch, asset_set) in assets.iter().take(4) {
            writeln!(
                output,
                "  assets.{arch}: kernel={} initrd={} rootfs={}",
                short_hash(asset_set["kernel"]["hash"].as_str().unwrap_or("-")),
                short_hash(asset_set["initrd"]["hash"].as_str().unwrap_or("-")),
                short_hash(asset_set["rootfs"]["hash"].as_str().unwrap_or("-")),
            )
            .expect("write to string");
        }
        if assets.len() > 4 {
            writeln!(output, "  ... {} more asset arch(es)", assets.len() - 4)
                .expect("write to string");
        }
    }
}

fn short_hash(hash: &str) -> String {
    if hash.len() <= 18 {
        return hash.to_string();
    }
    format!("{}...", &hash[..18])
}

fn profile_revision_action_summary_line(result: &serde_json::Value) -> String {
    let action = result["action"].as_str().unwrap_or("-");
    let profile_id = result["profile_id"].as_str().unwrap_or("-");
    let revision = result["selected_revision"].as_str().unwrap_or("-");
    let outcome = result["outcome"]["outcome"].as_str().unwrap_or("unknown");
    format!("Profile revision {action}: {profile_id}@{revision} {outcome}")
}

fn profile_revisions_summary_line(result: &serde_json::Value) -> String {
    let profile_id = result["profile_id"].as_str().unwrap_or("-");
    let current = result["current_revision"].as_str().unwrap_or("-");
    let installed = result["installed_revision"].as_str().unwrap_or("-");
    let revisions = result["revisions"]
        .as_array()
        .map(|revisions| revisions.len())
        .unwrap_or(0);
    format!(
        "Profile revisions: profile={profile_id} current={current} installed={installed} revisions={revisions}"
    )
}

fn profile_catalog_summary_line(result: &serde_json::Value) -> String {
    let profiles = result["profiles"]
        .as_array()
        .map(|profiles| profiles.len())
        .unwrap_or(0);
    let configured = result["configured"].as_bool().unwrap_or(false);
    let manifest_present = result["manifest_present"].as_bool().unwrap_or(false);
    format!(
        "Profile catalog: configured={configured} manifest_present={manifest_present} profiles={profiles}"
    )
}

fn profile_catalog_reconcile_summary_line(result: &serde_json::Value) -> String {
    let summary = &result["summary"];
    format!(
        "Profile catalog reconciled: installed={} unchanged={} deprecated_kept={} revoked_removed={} absent_removed={} errors={}",
        summary["installed"].as_u64().unwrap_or(0),
        summary["unchanged"].as_u64().unwrap_or(0),
        summary["deprecated_kept"].as_u64().unwrap_or(0),
        summary["revoked_removed"].as_u64().unwrap_or(0),
        summary["absent_removed"].as_u64().unwrap_or(0),
        summary["errors"].as_u64().unwrap_or(0),
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

    // Background update check (fire-and-forget). Skip destructive cleanup
    // commands so `capsem uninstall` cannot recreate state it just removed.
    if command_refreshes_update_cache(cli.command.as_ref()) {
        tokio::spawn(update::refresh_update_cache_if_stale());
    }

    if cli.command.is_none() {
        let issues = status::check_service_health().await?;
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

    // Commands that don't need the service
    match cli.command.as_ref().unwrap() {
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
        Commands::Misc(MiscCommands::Status { json }) => {
            status::run(*json).await?;
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
            update::run_update(*yes, *assets, Some(uds_path.clone())).await?;
            return Ok(());
        }
        Commands::Misc(MiscCommands::Setup {
            non_interactive,
            preset,
            force,
            accept_detected,
            corp_config,
            force_onboarding,
        }) => {
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

    if let Some(Commands::Session(SessionCommands::Purge {
        all,
        product: true,
        yes,
    })) = cli.command.as_ref()
    {
        if *all {
            anyhow::bail!("`capsem purge --product` cannot be combined with --all");
        }
        uninstall::run_purge(*yes).await?;
        return Ok(());
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
            })
            .await?;
        }
    }

    let client = UdsClient::new(uds_path, auto_launch);

    match cli.command.as_ref().unwrap() {
        Commands::Session(SessionCommands::Create {
            name,
            ram,
            cpu,
            env,
            from,
            profile,
            profile_revision,
        }) => {
            let persistent = name.is_some() || from.is_some();
            let req = ProvisionRequest {
                name: name.clone(),
                ram_mb: ram * 1024,
                cpus: *cpu,
                persistent,
                env: client::parse_env_vars(env)?,
                from: from.clone(),
                profile_id: profile.clone(),
                profile_revision: profile_revision.clone(),
            };

            let resp: ApiResponse<ProvisionResponse> = client.post("/provision", &req).await?;
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
                client.post(&format!("/fork/{}", session), &req).await?;
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
                .post(&format!("/resume/{}", name), &serde_json::json!({}))
                .await?;
            let info = resp.into_result()?;
            println!("{}", info.id);
        }
        Commands::Session(SessionCommands::Suspend { session }) => {
            client::validate_id(session)?;
            println!("Suspending session: {}", session);
            let resp: ApiResponse<serde_json::Value> = client
                .post(&format!("/suspend/{}", session), &serde_json::json!({}))
                .await?;
            resp.into_result()?;
            println!("Session suspended.");
        }
        Commands::Session(SessionCommands::Shell { session }) => {
            match session {
                Some(t) => {
                    client::validate_id(t.as_str())?;
                    run_shell(t.as_str(), &run_dir).await?;
                }
                None => {
                    // No args: create ephemeral session, attach, destroy on exit
                    println!("[!] Temporary session. Use `capsem create <name>` for persistent.");
                    let req = ProvisionRequest {
                        name: None,
                        ram_mb: 4 * 1024,
                        cpus: 4,
                        persistent: false,
                        env: None,
                        from: None,
                        profile_id: None,
                        profile_revision: None,
                    };
                    let resp: ApiResponse<ProvisionResponse> =
                        client.post("/provision", &req).await?;
                    let info = resp.into_result()?;

                    // Poll until the socket is connectable (not just present on disk).
                    let socket_path = run_dir.join("instances").join(format!("{}.sock", info.id));
                    let sp = socket_path.clone();
                    let _ = capsem_core::poll::poll_until(
                        capsem_core::poll::PollOpts::new(
                            "shell-socket",
                            std::time::Duration::from_secs(10),
                        ),
                        || {
                            let sp = sp.clone();
                            async move {
                                match tokio::net::UnixStream::connect(&sp).await {
                                    Ok(_) => Some(()),
                                    Err(_) => None,
                                }
                            }
                        },
                    )
                    .await;

                    let shell_result = run_shell(&info.id, &run_dir).await;
                    // Ephemeral: auto-destroy on disconnect
                    let _: Result<ApiResponse<serde_json::Value>, _> =
                        client.delete(&format!("/delete/{}", info.id)).await;
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
                println!(
                    "{:<20} {:<12} {:<10} {:<8} {:<6} {:<10} PROFILE",
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
                    let profile = format_session_profile_for_list(s);
                    println!(
                        "{:<20} {:<12} {:<10} {:<8} {:<6} {:<10} {}",
                        s.id, name, s.status, ram, cpus, uptime, profile
                    );
                    // Defunct rows: show the tail of process.log inline so
                    // the user doesn't need a separate `capsem logs` call
                    // to see why boot failed.
                    if s.status == "Defunct" {
                        if let Some(err) = &s.last_error {
                            let last = err
                                .lines()
                                .rev()
                                .find(|line| !line.trim().is_empty())
                                .unwrap_or("(log empty)");
                            println!("  ! {}", last);
                            println!("  (`capsem logs {}` for full context)", s.id);
                        }
                    }
                }
                let defunct = resp
                    .sessions
                    .iter()
                    .filter(|s| s.status == "Defunct")
                    .count();
                if defunct > 0 {
                    println!();
                    println!(
                        "{} defunct session(s). Run `capsem logs <name>` to debug.",
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
                client.post(&format!("/exec/{}", session), req).await?;
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
            profile,
            profile_revision,
            env,
        }) => {
            let req = RunRequest {
                command: command.clone(),
                timeout_secs: *timeout,
                profile_id: profile.clone(),
                profile_revision: profile_revision.clone(),
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
                client.delete(&format!("/delete/{}", session)).await?;
            resp.into_result()?;
            println!("Session deleted.");
        }
        Commands::Session(SessionCommands::Persist { session, name }) => {
            client::validate_id(session)?;
            let req = PersistRequest { name: name.clone() };
            let resp: ApiResponse<serde_json::Value> =
                client.post(&format!("/persist/{}", session), &req).await?;
            resp.into_result()?;
            println!(
                "[*] Session \"{}\" is now persistent as \"{}\"",
                session, name
            );
        }
        Commands::Session(SessionCommands::Purge {
            all,
            product,
            yes: _,
        }) => {
            if *product {
                anyhow::bail!(
                    "internal error: product purge should be handled before service startup"
                );
            }
            if *all {
                // Confirmation prompt
                use std::io::Write;
                let list_resp: ApiResponse<ListResponse> = client.get("/list").await?;
                let resp = list_resp.into_result()?;
                let persistent_count = resp.sessions.iter().filter(|s| s.persistent).count();
                let ephemeral_count = resp.sessions.iter().filter(|s| !s.persistent).count();
                print!(
                    "[!] This will destroy {} persistent and {} temporary sessions. Continue? [y/N] ",
                    persistent_count, ephemeral_count
                );
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
                println!(
                    "[*] Purged {} sessions ({} persistent, {} temporary).",
                    result.purged, result.persistent_purged, result.ephemeral_purged
                );
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
            print!("{}", format_session_logs(session, logs, *tail));
        }
        Commands::Session(SessionCommands::ExportPolicyContexts { session, json }) => {
            client::validate_id(session)?;
            let resp: ApiResponse<serde_json::Value> = client
                .get(&format!(
                    "/sessions/{}/policy-contexts",
                    urlencoding::encode(session)
                ))
                .await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                let fixtures = result["fixtures"]
                    .as_array()
                    .context("policy-context export response did not contain fixtures")?;
                for fixture in fixtures {
                    println!("{}", serde_json::to_string(fixture)?);
                }
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
            let mut url = format!("/history/{}?limit={}&layer={}", session, limit, layer);
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
                client.get(&format!("/info/{}", name)).await?;
            let info = info_resp.into_result()?;
            if !info.persistent {
                anyhow::bail!(
                    "Cannot restart ephemeral session \"{}\". Only persistent sessions support restart.",
                    name
                );
            }

            // Stop, then resume
            let stop_resp: ApiResponse<serde_json::Value> = client
                .post(&format!("/stop/{}", name), &serde_json::json!({}))
                .await?;
            stop_resp
                .into_result()
                .context("failed to stop session during restart")?;
            let resp: ApiResponse<ProvisionResponse> = client
                .post(&format!("/resume/{}", name), &serde_json::json!({}))
                .await?;
            let resumed = resp.into_result()?;
            println!("{}", resumed.id);
        }
        Commands::Skills(SkillsCommands::List {
            profile,
            kind,
            json,
        }) => {
            let path = skills_path(profile.as_ref(), *kind);
            let resp: ApiResponse<serde_json::Value> = client.get(&path).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", format_skills_summary(&result));
            }
        }
        Commands::Skills(SkillsCommands::Show {
            id,
            profile,
            kind,
            json,
        }) => {
            let path = skills_path(profile.as_ref(), *kind);
            let resp: ApiResponse<serde_json::Value> = client.get(&path).await?;
            let result = resp.into_result()?;
            let matches = skill_matches(&result, id);
            if matches.is_empty() {
                anyhow::bail!("skill '{}' not found", id);
            }
            let result = serde_json::json!({
                "mode": result["mode"].clone(),
                "profile_id": result["profile_id"].clone(),
                "skills": matches,
            });
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", format_skills_summary(&result));
            }
        }
        Commands::Skills(SkillsCommands::Add {
            id,
            profile,
            kind,
            json,
        }) => {
            let body = serde_json::json!({
                "profile": profile,
                "id": id,
                "kind": kind.as_str(),
            });
            let resp: ApiResponse<serde_json::Value> = client.post("/skills", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "Skill added: {} ({})",
                    result["id"].as_str().unwrap_or(id),
                    result["kind"].as_str().unwrap_or(kind.as_str()),
                );
            }
        }
        Commands::Skills(SkillsCommands::Delete {
            id,
            profile,
            kind,
            json,
        }) => {
            let mut path = format!("/skills/{}", urlencoding::encode(id));
            let mut params = Vec::new();
            if let Some(profile) = profile {
                params.push(format!("profile={}", urlencoding::encode(profile)));
            }
            if let Some(kind) = kind {
                params.push(format!("kind={}", kind.as_str()));
            }
            if !params.is_empty() {
                path.push_str(&format!("?{}", params.join("&")));
            }
            let resp: ApiResponse<serde_json::Value> = client.delete(&path).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "Skill deleted: {} ({})",
                    result["skill_id"].as_str().unwrap_or(id),
                    result["kind"].as_str().unwrap_or("-"),
                );
            }
        }
        Commands::Mcp(McpCommands::List { profile, json })
        | Commands::Mcp(McpCommands::Connectors { profile, json }) => {
            let path = mcp_connectors_path(profile.as_ref());
            let resp: ApiResponse<serde_json::Value> = client.get(&path).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", format_mcp_connectors_summary(&result));
            }
        }
        Commands::Mcp(McpCommands::Show { id, profile, json }) => {
            let path = mcp_connectors_path(profile.as_ref());
            let resp: ApiResponse<serde_json::Value> = client.get(&path).await?;
            let result = resp.into_result()?;
            let matches = mcp_server_matches(&result, id);
            if matches.is_empty() {
                anyhow::bail!("MCP server '{}' not found", id);
            }
            let result = serde_json::json!({
                "mode": result["mode"].clone(),
                "profile_id": result["profile_id"].clone(),
                "servers": matches,
            });
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", format_mcp_connectors_summary(&result));
            }
        }
        Commands::Mcp(McpCommands::Add {
            id,
            profile,
            disabled,
            server_type,
            command,
            args,
            env,
            url,
            headers,
            bearer_token,
            credential_refs,
            allowed_tools,
            json,
        }) => {
            let mut body = serde_json::json!({
                "id": id,
                "enabled": !*disabled,
                "capsem": {
                    "credential_refs": credential_refs,
                    "allowed_tools": allowed_tools,
                },
            });
            if let Some(server_type) = server_type {
                body["type"] = serde_json::json!(server_type);
            }
            if let Some(command) = command {
                body["command"] = serde_json::json!(command);
            }
            if !args.is_empty() {
                body["args"] = serde_json::json!(args);
            }
            if let Some(env) = client::parse_env_vars(env)? {
                body["env"] = serde_json::json!(env);
            }
            if let Some(url) = url {
                body["url"] = serde_json::json!(url);
            }
            if let Some(headers) = client::parse_env_vars(headers)? {
                body["headers"] = serde_json::json!(headers);
            }
            if let Some(bearer_token) = bearer_token {
                body["bearerToken"] = serde_json::json!(bearer_token);
            }
            if let Some(profile) = profile {
                body["profile"] = serde_json::json!(profile);
            }
            let resp: ApiResponse<serde_json::Value> =
                client.post("/mcp/connectors", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("MCP server added: {}", result["id"].as_str().unwrap_or("-"));
            }
        }
        Commands::Mcp(McpCommands::Delete { id, profile }) => {
            let mut path = format!("/mcp/connectors/{}", urlencoding::encode(id));
            if let Some(profile) = profile {
                path.push_str(&format!("?profile={}", urlencoding::encode(profile)));
            }
            let resp: ApiResponse<serde_json::Value> = client.delete(&path).await?;
            let result = resp.into_result()?;
            println!(
                "MCP server deleted: {}",
                result["server_id"].as_str().unwrap_or(id)
            );
        }
        Commands::Enforcement(EnforcementCommands::List { json }) => {
            let resp: ApiResponse<serde_json::Value> = client.get("/enforcement").await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_rule_list_summary("enforcement", &result);
            }
        }
        Commands::Enforcement(EnforcementCommands::Stats { json }) => {
            let resp: ApiResponse<serde_json::Value> = client.get("/enforcement/stats").await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_rule_list_summary("enforcement", &result);
            }
        }
        Commands::Enforcement(EnforcementCommands::Validate {
            id,
            condition,
            decision,
            pack_id,
            reason,
            disabled,
            json,
        }) => {
            let body = enforcement_rule_body(id, condition, *decision, pack_id, reason, *disabled);
            let resp: ApiResponse<serde_json::Value> =
                client.post("/enforcement/validate", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_compile_summary("Enforcement", &result);
            }
        }
        Commands::Enforcement(EnforcementCommands::Compile {
            id,
            condition,
            decision,
            pack_id,
            reason,
            disabled,
            json,
        }) => {
            let body = enforcement_rule_body(id, condition, *decision, pack_id, reason, *disabled);
            let resp: ApiResponse<serde_json::Value> =
                client.post("/enforcement/compile", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_compile_summary("Enforcement", &result);
            }
        }
        Commands::Enforcement(EnforcementCommands::Install {
            id,
            condition,
            decision,
            pack_id,
            reason,
            disabled,
            json,
        }) => {
            let body = enforcement_rule_body(id, condition, *decision, pack_id, reason, *disabled);
            let resp: ApiResponse<serde_json::Value> = client.post("/enforcement", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_install_summary("Enforcement", &result);
            }
        }
        Commands::Enforcement(EnforcementCommands::Update {
            id,
            condition,
            decision,
            pack_id,
            reason,
            disabled,
            json,
        }) => {
            let body = enforcement_rule_body(id, condition, *decision, pack_id, reason, *disabled);
            let path = format!("/enforcement/{}", urlencoding::encode(id));
            let resp: ApiResponse<serde_json::Value> = client.put(&path, &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_install_summary("Enforcement", &result);
            }
        }
        Commands::Enforcement(EnforcementCommands::Backtest {
            id,
            events,
            condition,
            decision,
            pack_id,
            reason,
            limit,
            disabled,
            json,
        }) => {
            let body = serde_json::json!({
                "rule": enforcement_rule_body(id, condition, *decision, pack_id, reason, *disabled),
                "events": read_runtime_backtest_events(events)?,
                "limit": limit,
            });
            let resp: ApiResponse<serde_json::Value> =
                client.post("/enforcement/backtest", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_backtest_summary("Enforcement backtest", &result);
            }
        }
        Commands::Enforcement(EnforcementCommands::Delete { id }) => {
            let path = format!("/enforcement/{}", urlencoding::encode(id));
            let resp: ApiResponse<serde_json::Value> = client.delete(&path).await?;
            let result = resp.into_result()?;
            println!(
                "Enforcement rule deleted: {}",
                result["id"].as_str().unwrap_or(id)
            );
        }
        Commands::Detection(DetectionCommands::List { json }) => {
            let resp: ApiResponse<serde_json::Value> = client.get("/detection").await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_rule_list_summary("detection", &result);
            }
        }
        Commands::Detection(DetectionCommands::Stats { json }) => {
            let resp: ApiResponse<serde_json::Value> = client.get("/detection/stats").await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_rule_list_summary("detection", &result);
            }
        }
        Commands::Detection(DetectionCommands::Validate {
            id,
            pack_id,
            title,
            condition,
            severity,
            confidence,
            sigma_id,
            tags,
            disabled,
            json,
        }) => {
            let body = detection_rule_body(
                id,
                pack_id,
                title,
                condition,
                *severity,
                *confidence,
                sigma_id,
                tags,
                *disabled,
            );
            let resp: ApiResponse<serde_json::Value> =
                client.post("/detection/validate", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_compile_summary("Detection", &result);
            }
        }
        Commands::Detection(DetectionCommands::Compile {
            id,
            pack_id,
            title,
            condition,
            severity,
            confidence,
            sigma_id,
            tags,
            disabled,
            json,
        }) => {
            let body = detection_rule_body(
                id,
                pack_id,
                title,
                condition,
                *severity,
                *confidence,
                sigma_id,
                tags,
                *disabled,
            );
            let resp: ApiResponse<serde_json::Value> =
                client.post("/detection/compile", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_compile_summary("Detection", &result);
            }
        }
        Commands::Detection(DetectionCommands::Install {
            id,
            pack_id,
            title,
            condition,
            severity,
            confidence,
            sigma_id,
            tags,
            disabled,
            json,
        }) => {
            let body = detection_rule_body(
                id,
                pack_id,
                title,
                condition,
                *severity,
                *confidence,
                sigma_id,
                tags,
                *disabled,
            );
            let resp: ApiResponse<serde_json::Value> = client.post("/detection", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_install_summary("Detection", &result);
            }
        }
        Commands::Detection(DetectionCommands::Update {
            id,
            pack_id,
            title,
            condition,
            severity,
            confidence,
            sigma_id,
            tags,
            disabled,
            json,
        }) => {
            let body = detection_rule_body(
                id,
                pack_id,
                title,
                condition,
                *severity,
                *confidence,
                sigma_id,
                tags,
                *disabled,
            );
            let path = format!("/detection/{}", urlencoding::encode(id));
            let resp: ApiResponse<serde_json::Value> = client.put(&path, &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_install_summary("Detection", &result);
            }
        }
        Commands::Detection(DetectionCommands::Backtest {
            id,
            events,
            pack_id,
            title,
            condition,
            severity,
            confidence,
            sigma_id,
            tags,
            limit,
            disabled,
            json,
        }) => {
            let body = serde_json::json!({
                "rule": detection_rule_body(
                    id,
                    pack_id,
                    title,
                    condition,
                    *severity,
                    *confidence,
                    sigma_id,
                    tags,
                    *disabled,
                ),
                "events": read_runtime_backtest_events(events)?,
                "limit": limit,
            });
            let resp: ApiResponse<serde_json::Value> =
                client.post("/detection/backtest", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_backtest_summary("Detection backtest", &result);
            }
        }
        Commands::Detection(DetectionCommands::Hunt {
            id,
            events,
            pack_id,
            title,
            condition,
            severity,
            confidence,
            sigma_id,
            tags,
            limit,
            disabled,
            json,
        }) => {
            let rule = detection_rule_body(
                id,
                pack_id,
                title,
                condition,
                *severity,
                *confidence,
                sigma_id,
                tags,
                *disabled,
            );
            let body = serde_json::json!({
                "rules": [rule],
                "events": read_runtime_backtest_events(events)?,
                "limit": limit,
            });
            let resp: ApiResponse<serde_json::Value> =
                client.post("/detection/hunt", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_hunt_summary(&result);
            }
        }
        Commands::Detection(DetectionCommands::HuntSession {
            session,
            id,
            pack_id,
            title,
            condition,
            severity,
            confidence,
            sigma_id,
            tags,
            limit,
            disabled,
            json,
        }) => {
            client::validate_id(session)?;
            let rule = detection_rule_body(
                id,
                pack_id,
                title,
                condition,
                *severity,
                *confidence,
                sigma_id,
                tags,
                *disabled,
            );
            let body = serde_json::json!({
                "rules": [rule],
                "limit": limit,
            });
            let resp: ApiResponse<serde_json::Value> = client
                .post(
                    &format!("/sessions/{}/detection/hunt", urlencoding::encode(session)),
                    &body,
                )
                .await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_runtime_hunt_summary(&result);
            }
        }
        Commands::Detection(DetectionCommands::Delete { id }) => {
            let path = format!("/detection/{}", urlencoding::encode(id));
            let resp: ApiResponse<serde_json::Value> = client.delete(&path).await?;
            let result = resp.into_result()?;
            println!(
                "Detection rule deleted: {}",
                result["id"].as_str().unwrap_or(id)
            );
        }
        Commands::Confirm(ConfirmCommands::List { json }) => {
            let resp: ApiResponse<serde_json::Value> = client.get("/confirm/pending").await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("{}", format_confirm_list_summary(&result));
            }
        }
        Commands::Profile(ProfileCommands::List { json }) => {
            let resp: ApiResponse<serde_json::Value> = client.get("/profiles").await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", format_profile_list_summary(&result));
            }
        }
        Commands::Profile(ProfileCommands::Create { file, json }) => {
            let profile = read_profile_document(file)?;
            let resp: ApiResponse<serde_json::Value> = client.post("/profiles", &profile).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", format_profile_record_summary(&result));
            }
        }
        Commands::Profile(ProfileCommands::Show { profile_id, json }) => {
            let path = format!("/profiles/{}", urlencoding::encode(profile_id));
            let resp: ApiResponse<serde_json::Value> = client.get(&path).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", format_profile_record_summary(&result));
            }
        }
        Commands::Profile(ProfileCommands::Resolve { profile_id, json }) => {
            let path = format!("/profiles/{}/effective", urlencoding::encode(profile_id));
            let resp: ApiResponse<serde_json::Value> = client.get(&path).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("{}", format_profile_resolve_summary(&result));
            }
        }
        Commands::Profile(ProfileCommands::Fork {
            source_profile_id,
            id,
            name,
            json,
        }) => {
            let path = format!("/profiles/{}/fork", urlencoding::encode(source_profile_id));
            let body = serde_json::json!({
                "id": id,
                "name": name,
            });
            let resp: ApiResponse<serde_json::Value> = client.post(&path, &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", format_profile_record_summary(&result));
            }
        }
        Commands::Profile(ProfileCommands::Delete { profile_id, json }) => {
            let path = format!("/profiles/{}", urlencoding::encode(profile_id));
            let resp: ApiResponse<serde_json::Value> = client.delete(&path).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "Profile deleted: {}",
                    result["deleted"].as_str().unwrap_or(profile_id)
                );
            }
        }
        Commands::Profile(ProfileCommands::ReconcileCatalog {
            manifest,
            manifest_url,
            pubkey,
            json,
        }) => {
            let manifest_json =
                read_profile_catalog_manifest(manifest.clone(), manifest_url.clone()).await?;
            let profile_payload_pubkey = std::fs::read_to_string(pubkey)
                .with_context(|| format!("read profile payload pubkey {}", pubkey.display()))?;
            let body = serde_json::json!({
                "manifest_json": manifest_json,
                "profile_payload_pubkey": profile_payload_pubkey,
            });
            let resp: ApiResponse<serde_json::Value> =
                client.post("/profiles/catalog/reconcile", &body).await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_profile_catalog_reconcile_summary(&result);
            }
        }
        Commands::Profile(ProfileCommands::Catalog { json }) => {
            let resp: ApiResponse<serde_json::Value> = client.get("/profiles/catalog").await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_profile_catalog_summary(&result);
            }
        }
        Commands::Profile(ProfileCommands::Revisions { profile_id, json }) => {
            let resp: ApiResponse<serde_json::Value> = client
                .get(&format!("/profiles/{profile_id}/revisions"))
                .await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_profile_revisions_summary(&result);
            }
        }
        Commands::Profile(ProfileCommands::Install {
            profile_id,
            revision,
            json,
        }) => {
            let body = serde_json::json!({ "revision": revision });
            let resp: ApiResponse<serde_json::Value> = client
                .post(&format!("/profiles/{profile_id}/revisions/install"), &body)
                .await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_profile_revision_action_summary(&result);
            }
        }
        Commands::Profile(ProfileCommands::Update {
            profile_id,
            file,
            revision,
            json,
        }) => {
            if let Some(file) = file {
                let profile = read_profile_document(file)?;
                let path = format!("/profiles/{}", urlencoding::encode(profile_id));
                let resp: ApiResponse<serde_json::Value> = client.put(&path, &profile).await?;
                let result = resp.into_result()?;
                if *json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    print!("{}", format_profile_record_summary(&result));
                }
                return Ok(());
            }
            let body = serde_json::json!({ "revision": revision });
            let resp: ApiResponse<serde_json::Value> = client
                .post(&format!("/profiles/{profile_id}/revisions/update"), &body)
                .await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_profile_revision_action_summary(&result);
            }
        }
        Commands::Profile(ProfileCommands::Remove {
            profile_id,
            revision,
            json,
        }) => {
            let body = serde_json::json!({ "revision": revision });
            let resp: ApiResponse<serde_json::Value> = client
                .post(&format!("/profiles/{profile_id}/revisions/remove"), &body)
                .await?;
            let result = resp.into_result()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_profile_revision_action_summary(&result);
            }
        }
        Commands::Misc(MiscCommands::Debug) => {
            status::debug_report(&client).await?;
        }
        Commands::Misc(
            MiscCommands::Version
            | MiscCommands::Setup { .. }
            | MiscCommands::Update { .. }
            | MiscCommands::Completions { .. }
            | MiscCommands::Uninstall { .. }
            | MiscCommands::Install
            | MiscCommands::Status { .. }
            | MiscCommands::Start
            | MiscCommands::Stop
            | MiscCommands::SupportBundle { .. }, /* handled before UDS */
        ) => {
            unreachable!("handled before UdsClient creation")
        }
        Commands::Misc(MiscCommands::Doctor { fast, bundle }) => {
            use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
            use tokio_unix_ipc::channel_from_std;

            // Log file: ~/.capsem/run/doctor-latest.log (always overwritten)
            let log_path = run_dir.join("doctor-latest.log");
            let mut log_file = std::fs::File::create(&log_path).ok();

            println!("Running capsem-doctor...");
            println!("Log: {}", log_path.display());

            // Preflight checks the default host install layout and service
            // manager state. When the user targets a custom socket via
            // --uds-path, those checks are unrelated to the selected
            // service instance and can false-fail (for example in e2e
            // harnesses that run against an ephemeral service).
            if auto_launch {
                status::doctor_preflight().await?;
            }

            let req = ProvisionRequest {
                name: None,
                ram_mb: 2048,
                cpus: 2,
                persistent: false,
                env: None,
                from: None,
                profile_id: None,
                profile_revision: None,
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
            let cmd: Vec<u8> = if *fast {
                format!("capsem-doctor --durations=10 -k 'not throughput'{bundle_arg}\n")
                    .into_bytes()
            } else {
                format!("capsem-doctor --durations=10{bundle_arg}\n").into_bytes()
            };
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
                    eprintln!(
                        "warning: no doctor bundle found in any of {} -- the in-VM script may have failed before tar",
                        candidates
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }

            delete_vm(&client, &vm_id).await;
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
                "/files/{session}/content?path={}",
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
                "/files/{session}/content?path={}",
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
        let cli = Cli::parse_from(["capsem", "create", "my-vm"]);
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
    fn parse_create_with_profile_selection() {
        let cli = Cli::parse_from([
            "capsem",
            "create",
            "--profile",
            "coding",
            "--profile-revision",
            "2026.0520.1",
        ]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Create {
                profile,
                profile_revision,
                ..
            }) => {
                assert_eq!(profile.as_deref(), Some("coding"));
                assert_eq!(profile_revision.as_deref(), Some("2026.0520.1"));
            }
            _ => panic!("expected Create with profile selection"),
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
    fn parse_attach_alias_rejected() {
        let cli = Cli::try_parse_from(["capsem", "attach", "mydev"]);
        assert!(cli.is_err(), "attach alias should be rejected");
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
            Commands::Session(SessionCommands::Shell { session }) => {
                assert_eq!(session, Some("my-vm".into()));
            }
            _ => panic!("expected Shell"),
        }
    }

    #[test]
    fn parse_shell_with_name_flag_rejected() {
        let cli = Cli::try_parse_from(["capsem", "shell", "-n", "mydev"]);
        assert!(cli.is_err(), "shell -n should be rejected");
    }

    #[test]
    fn parse_shell_bare() {
        // Bare `capsem shell` = temp session + auto-destroy
        let cli = Cli::parse_from(["capsem", "shell"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Shell { session }) => {
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
            Commands::Session(SessionCommands::Purge { all, product, yes }) => {
                assert!(!all);
                assert!(!product);
                assert!(!yes);
            }
            _ => panic!("expected Purge"),
        }
    }

    #[test]
    fn parse_purge_all() {
        let cli = Cli::parse_from(["capsem", "purge", "--all"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Purge { all, product, yes }) => {
                assert!(all);
                assert!(!product);
                assert!(!yes);
            }
            _ => panic!("expected Purge --all"),
        }
    }

    #[test]
    fn parse_purge_product_yes() {
        let cli = Cli::parse_from(["capsem", "purge", "--product", "--yes"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Purge { all, product, yes }) => {
                assert!(!all);
                assert!(product);
                assert!(yes);
            }
            _ => panic!("expected Purge --product --yes"),
        }
    }

    #[test]
    fn parse_run() {
        let cli = Cli::parse_from(["capsem", "run", "echo hello"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Run {
                command,
                timeout,
                profile,
                profile_revision,
                env,
            }) => {
                assert_eq!(command, "echo hello");
                assert_eq!(timeout, None);
                assert_eq!(profile, None);
                assert_eq!(profile_revision, None);
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
                profile,
                profile_revision,
                env,
            }) => {
                assert_eq!(command, "ls -la");
                assert_eq!(timeout, Some(120));
                assert_eq!(profile, None);
                assert_eq!(profile_revision, None);
                assert!(env.is_empty());
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn parse_run_with_profile_selection() {
        let cli = Cli::parse_from([
            "capsem",
            "run",
            "--profile",
            "coding",
            "--profile-revision",
            "2026.0520.1",
            "echo hello",
        ]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Run {
                command,
                timeout,
                profile,
                profile_revision,
                env,
            }) => {
                assert_eq!(command, "echo hello");
                assert_eq!(timeout, None);
                assert_eq!(profile.as_deref(), Some("coding"));
                assert_eq!(profile_revision.as_deref(), Some("2026.0520.1"));
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
    fn parse_ls_alias_rejected() {
        let cli = Cli::try_parse_from(["capsem", "ls"]);
        assert!(cli.is_err(), "ls alias should be rejected");
    }

    #[test]
    fn parse_status() {
        // `capsem status` is now the service status command
        let cli = Cli::parse_from(["capsem", "status"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Status { json: false })
        ));
    }

    #[test]
    fn parse_status_json() {
        let cli = Cli::parse_from(["capsem", "status", "--json"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Status { json: true })
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
    fn parse_rm_alias_rejected() {
        let cli = Cli::try_parse_from(["capsem", "rm", "vm-123"]);
        assert!(cli.is_err(), "rm alias should be rejected");
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
    fn parse_export_policy_contexts() {
        let cli = Cli::parse_from(["capsem", "export-policy-contexts", "vm-1", "--json"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::ExportPolicyContexts { session, json }) => {
                assert_eq!(session, "vm-1");
                assert!(json);
            }
            _ => panic!("expected export-policy-contexts"),
        }
    }

    #[test]
    fn format_session_logs_preserves_structured_process_security_line() {
        let process_security_line = serde_json::json!({
            "target": "security.process",
            "fields": {
                "message": "process_exec_security_decision",
                "event_type": "process.exec",
                "final_action": "block",
                "vm_id": "vm-cli-logs",
                "profile_id": "coding",
                "user_id": "elie",
                "rule_id": "runtime.block-shell",
                "reason": "shell exec blocked"
            }
        })
        .to_string();
        let output = format_session_logs(
            "vm-cli-logs",
            LogsResponse {
                logs: String::new(),
                serial_logs: Some("serial booted\n".into()),
                process_logs: Some(format!("old line\n{process_security_line}\n")),
                security_logs: Some(
                    serde_json::json!({
                        "target": "security.event",
                        "fields": {
                            "message": "resolved_security_event",
                            "event_type": "process.exec",
                            "final_action": "block",
                            "vm_id": "vm-cli-logs",
                            "profile_id": "coding",
                            "user_id": "elie",
                            "rule_id": "runtime.block-shell",
                            "reason": "shell exec blocked"
                        }
                    })
                    .to_string(),
                ),
            },
            Some(1),
        );

        assert!(output.contains("--- Security Events (vm-cli-logs) ---"));
        assert!(output.contains(r#""target":"security.event""#));
        assert!(output.contains(r#""message":"resolved_security_event""#));
        assert!(output.contains("--- Process Logs (vm-cli-logs) ---"));
        assert!(output.contains(r#""target":"security.process""#));
        assert!(output.contains(r#""message":"process_exec_security_decision""#));
        assert!(output.contains(r#""event_type":"process.exec""#));
        assert!(output.contains(r#""final_action":"block""#));
        assert!(output.contains(r#""profile_id":"coding""#));
        assert!(output.contains(r#""user_id":"elie""#));
        assert!(output.contains(r#""rule_id":"runtime.block-shell""#));
        assert!(!output.contains("old line"));
        assert!(output.contains("--- Serial Logs (vm-cli-logs) ---"));
        assert!(output.contains("serial booted"));
    }

    #[test]
    fn format_session_logs_adds_resolved_security_summary() {
        let process_event = serde_json::json!({
            "target": "security.event",
            "fields": {
                "message": "resolved_security_event",
                "event_family": "process",
                "event_type": "process.exec",
                "final_action": "block",
                "rule_id": "runtime.block-shell",
                "finding_count": 1,
                "detection_rule_ids": "detect.shell"
            }
        })
        .to_string();
        let dns_event = serde_json::json!({
            "target": "security.event",
            "fields": {
                "message": "resolved_security_event",
                "event_family": "dns",
                "event_type": "dns.request",
                "final_action": "allow",
                "rule_id": "runtime.allow-dns",
                "finding_count": 0
            }
        })
        .to_string();

        let output = format_session_logs(
            "vm-cli-logs",
            LogsResponse {
                logs: String::new(),
                serial_logs: None,
                process_logs: None,
                security_logs: Some(format!("{process_event}\n{dns_event}\n")),
            },
            None,
        );

        assert!(output.contains("--- Security Events (vm-cli-logs) ---"));
        assert!(
            output.contains("summary: events=2 blocked=1 detections=1 families=dns=1,process=1")
        );
        assert!(output.contains("detect.shell=1"));
        assert!(output.contains("runtime.block-shell=1"));
        assert!(output.contains("runtime.allow-dns=1"));
        assert!(output.contains(r#""event_type":"process.exec""#));
        assert!(output.contains(r#""event_type":"dns.request""#));
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
            Commands::Misc(MiscCommands::Doctor {
                fast: false,
                bundle: false
            })
        ));
    }

    #[test]
    fn parse_doctor_bundle_flag() {
        let cli = Cli::parse_from(["capsem", "doctor", "--bundle"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Doctor {
                fast: false,
                bundle: true
            })
        ));
    }

    #[test]
    fn parse_debug() {
        let cli = Cli::parse_from(["capsem", "debug"]);
        assert!(matches!(
            cli.command.unwrap(),
            Commands::Misc(MiscCommands::Debug)
        ));
    }

    #[test]
    fn parse_profile_reconcile_catalog() {
        let cli = Cli::parse_from([
            "capsem",
            "profile",
            "reconcile-catalog",
            "--manifest",
            "manifest.json",
            "--pubkey",
            "profile.pub",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::ReconcileCatalog {
                manifest,
                manifest_url,
                pubkey,
                json,
            }) => {
                assert_eq!(manifest, Some(PathBuf::from("manifest.json")));
                assert_eq!(manifest_url, None);
                assert_eq!(pubkey, PathBuf::from("profile.pub"));
                assert!(json);
            }
            _ => panic!("expected profile reconcile-catalog"),
        }
    }

    #[test]
    fn parse_profile_catalog() {
        let cli = Cli::parse_from(["capsem", "profile", "catalog", "--json"]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::Catalog { json }) => assert!(json),
            _ => panic!("expected profile catalog"),
        }
    }

    #[test]
    fn parse_profile_revisions() {
        let cli = Cli::parse_from(["capsem", "profile", "revisions", "everyday-work", "--json"]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::Revisions { profile_id, json }) => {
                assert_eq!(profile_id, "everyday-work");
                assert!(json);
            }
            _ => panic!("expected profile revisions"),
        }
    }

    #[test]
    fn parse_profile_list_show_resolve() {
        let cli = Cli::parse_from(["capsem", "profile", "list", "--json"]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::List { json }) => assert!(json),
            _ => panic!("expected profile list"),
        }

        let cli = Cli::parse_from(["capsem", "profile", "create", "--file", "profile.toml"]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::Create { file, json }) => {
                assert_eq!(file, PathBuf::from("profile.toml"));
                assert!(!json);
            }
            _ => panic!("expected profile create"),
        }

        let cli = Cli::parse_from(["capsem", "profile", "show", "coding", "--json"]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::Show { profile_id, json }) => {
                assert_eq!(profile_id, "coding");
                assert!(json);
            }
            _ => panic!("expected profile show"),
        }

        let cli = Cli::parse_from(["capsem", "profile", "resolve", "coding"]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::Resolve { profile_id, json }) => {
                assert_eq!(profile_id, "coding");
                assert!(!json);
            }
            _ => panic!("expected profile resolve"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "profile",
            "update",
            "coding",
            "--file",
            "profile.json",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::Update {
                profile_id,
                file,
                revision,
                json,
            }) => {
                assert_eq!(profile_id, "coding");
                assert_eq!(file, Some(PathBuf::from("profile.json")));
                assert!(revision.is_none());
                assert!(json);
            }
            _ => panic!("expected profile update --file"),
        }
    }

    #[test]
    fn parse_profile_fork_delete() {
        let cli = Cli::parse_from([
            "capsem",
            "profile",
            "fork",
            "coding",
            "--id",
            "my-coding",
            "--name",
            "My Coding",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::Fork {
                source_profile_id,
                id,
                name,
                json,
            }) => {
                assert_eq!(source_profile_id, "coding");
                assert_eq!(id, "my-coding");
                assert_eq!(name, "My Coding");
                assert!(json);
            }
            _ => panic!("expected profile fork"),
        }

        let cli = Cli::parse_from(["capsem", "profile", "delete", "my-coding", "--json"]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::Delete { profile_id, json }) => {
                assert_eq!(profile_id, "my-coding");
                assert!(json);
            }
            _ => panic!("expected profile delete"),
        }
    }

    #[test]
    fn parse_mcp_connectors_add_delete() {
        let cli = Cli::parse_from(["capsem", "mcp", "list", "--profile", "coding"]);
        match cli.command.unwrap() {
            Commands::Mcp(McpCommands::List { profile, json }) => {
                assert_eq!(profile.as_deref(), Some("coding"));
                assert!(!json);
            }
            _ => panic!("expected mcp list"),
        }

        let cli = Cli::parse_from(["capsem", "mcp", "show", "github", "--json"]);
        match cli.command.unwrap() {
            Commands::Mcp(McpCommands::Show { id, json, .. }) => {
                assert_eq!(id, "github");
                assert!(json);
            }
            _ => panic!("expected mcp show"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "mcp",
            "connectors",
            "--profile",
            "coding",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Mcp(McpCommands::Connectors { profile, json }) => {
                assert_eq!(profile.as_deref(), Some("coding"));
                assert!(json);
            }
            _ => panic!("expected mcp connectors"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "mcp",
            "add",
            "github",
            "--profile",
            "coding",
            "--type",
            "stdio",
            "--command",
            "npx",
            "--arg",
            "-y",
            "--arg",
            "@modelcontextprotocol/server-github",
            "--env",
            "GITHUB_TOKEN=env:CAPSEM_GITHUB_TOKEN",
            "--credential-ref",
            "github-token",
            "--allowed-tool",
            "repo.read",
            "--disabled",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Mcp(McpCommands::Add {
                id,
                profile,
                disabled,
                server_type,
                command,
                args,
                env,
                url,
                headers,
                bearer_token,
                credential_refs,
                allowed_tools,
                json,
            }) => {
                assert_eq!(id, "github");
                assert_eq!(profile.as_deref(), Some("coding"));
                assert!(disabled);
                assert_eq!(server_type.as_deref(), Some("stdio"));
                assert_eq!(command.as_deref(), Some("npx"));
                assert_eq!(args, vec!["-y", "@modelcontextprotocol/server-github"]);
                assert_eq!(env, vec!["GITHUB_TOKEN=env:CAPSEM_GITHUB_TOKEN"]);
                assert!(url.is_none());
                assert!(headers.is_empty());
                assert!(bearer_token.is_none());
                assert_eq!(credential_refs, vec!["github-token"]);
                assert_eq!(allowed_tools, vec!["repo.read"]);
                assert!(json);
            }
            _ => panic!("expected mcp add"),
        }

        let cli = Cli::parse_from(["capsem", "mcp", "delete", "github", "--profile", "coding"]);
        match cli.command.unwrap() {
            Commands::Mcp(McpCommands::Delete { id, profile }) => {
                assert_eq!(id, "github");
                assert_eq!(profile.as_deref(), Some("coding"));
            }
            _ => panic!("expected mcp delete"),
        }
    }

    #[test]
    fn parse_skills_list_show_add_delete() {
        let cli = Cli::parse_from([
            "capsem",
            "skills",
            "list",
            "--profile",
            "coding",
            "--kind",
            "enabled",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Skills(SkillsCommands::List {
                profile,
                kind,
                json,
            }) => {
                assert_eq!(profile.as_deref(), Some("coding"));
                assert_eq!(kind, Some(CliSkillKind::Enabled));
                assert!(json);
            }
            _ => panic!("expected skills list"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "skills",
            "show",
            "admin-profile",
            "--kind",
            "group",
        ]);
        match cli.command.unwrap() {
            Commands::Skills(SkillsCommands::Show { id, kind, .. }) => {
                assert_eq!(id, "admin-profile");
                assert_eq!(kind, Some(CliSkillKind::Group));
            }
            _ => panic!("expected skills show"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "skills",
            "add",
            "admin-image",
            "--profile",
            "coding",
            "--kind",
            "disabled",
        ]);
        match cli.command.unwrap() {
            Commands::Skills(SkillsCommands::Add {
                id, profile, kind, ..
            }) => {
                assert_eq!(id, "admin-image");
                assert_eq!(profile.as_deref(), Some("coding"));
                assert_eq!(kind, CliSkillKind::Disabled);
            }
            _ => panic!("expected skills add"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "skills",
            "delete",
            "admin-image",
            "--profile",
            "coding",
        ]);
        match cli.command.unwrap() {
            Commands::Skills(SkillsCommands::Delete {
                id, profile, kind, ..
            }) => {
                assert_eq!(id, "admin-image");
                assert_eq!(profile.as_deref(), Some("coding"));
                assert_eq!(kind, None);
            }
            _ => panic!("expected skills delete"),
        }
    }

    #[test]
    fn parse_runtime_security_rule_commands() {
        let cli = Cli::parse_from(["capsem", "enforcement", "list", "--json"]);
        match cli.command.unwrap() {
            Commands::Enforcement(EnforcementCommands::List { json }) => assert!(json),
            _ => panic!("expected enforcement list"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "enforcement",
            "compile",
            "block-admin",
            "--condition",
            "http.request.path.startsWith('/admin')",
            "--decision",
            "block",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Enforcement(EnforcementCommands::Compile {
                id,
                condition,
                decision,
                json,
                ..
            }) => {
                assert_eq!(id, "block-admin");
                assert_eq!(condition, "http.request.path.startsWith('/admin')");
                assert_eq!(decision, CliSecurityDecision::Block);
                assert!(json);
            }
            _ => panic!("expected enforcement compile"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "enforcement",
            "install",
            "block-admin",
            "--condition",
            "http.request.path.startsWith('/admin')",
            "--decision",
            "block",
            "--pack-id",
            "runtime",
            "--reason",
            "admin path",
            "--disabled",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Enforcement(EnforcementCommands::Install {
                id,
                condition,
                decision,
                pack_id,
                reason,
                disabled,
                json,
            }) => {
                assert_eq!(id, "block-admin");
                assert_eq!(condition, "http.request.path.startsWith('/admin')");
                assert_eq!(decision, CliSecurityDecision::Block);
                assert_eq!(pack_id.as_deref(), Some("runtime"));
                assert_eq!(reason.as_deref(), Some("admin path"));
                assert!(disabled);
                assert!(json);
            }
            _ => panic!("expected enforcement install"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "enforcement",
            "update",
            "block-admin",
            "--condition",
            "http.request.path.startsWith('/admin')",
            "--decision",
            "block",
            "--pack-id",
            "runtime",
        ]);
        match cli.command.unwrap() {
            Commands::Enforcement(EnforcementCommands::Update {
                id,
                condition,
                decision,
                pack_id,
                ..
            }) => {
                assert_eq!(id, "block-admin");
                assert_eq!(condition, "http.request.path.startsWith('/admin')");
                assert_eq!(decision, CliSecurityDecision::Block);
                assert_eq!(pack_id.as_deref(), Some("runtime"));
            }
            _ => panic!("expected enforcement update"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "enforcement",
            "backtest",
            "block-admin",
            "--events",
            "events.jsonl",
            "--condition",
            "http.request.path.startsWith('/admin')",
            "--decision",
            "block",
            "--limit",
            "25",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Enforcement(EnforcementCommands::Backtest {
                id,
                events,
                limit,
                json,
                ..
            }) => {
                assert_eq!(id, "block-admin");
                assert_eq!(events, PathBuf::from("events.jsonl"));
                assert_eq!(limit, Some(25));
                assert!(json);
            }
            _ => panic!("expected enforcement backtest"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "detection",
            "compile",
            "detect-tool-result",
            "--pack-id",
            "runtime-detection",
            "--title",
            "Tool result",
            "--condition",
            "model.response.tool_results[0].returned_to_model == true",
            "--severity",
            "medium",
            "--confidence",
            "high",
        ]);
        match cli.command.unwrap() {
            Commands::Detection(DetectionCommands::Compile {
                id,
                pack_id,
                severity,
                confidence,
                ..
            }) => {
                assert_eq!(id, "detect-tool-result");
                assert_eq!(pack_id, "runtime-detection");
                assert_eq!(severity, CliSeverity::Medium);
                assert_eq!(confidence, CliConfidence::High);
            }
            _ => panic!("expected detection compile"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "detection",
            "backtest",
            "detect-tool-result",
            "--events",
            "events.json",
            "--pack-id",
            "runtime-detection",
            "--title",
            "Tool result",
            "--condition",
            "model.response.tool_results[0].returned_to_model == true",
            "--severity",
            "medium",
            "--confidence",
            "high",
            "--limit",
            "50",
        ]);
        match cli.command.unwrap() {
            Commands::Detection(DetectionCommands::Backtest {
                id, events, limit, ..
            }) => {
                assert_eq!(id, "detect-tool-result");
                assert_eq!(events, PathBuf::from("events.json"));
                assert_eq!(limit, Some(50));
            }
            _ => panic!("expected detection backtest"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "detection",
            "hunt",
            "detect-tool-result",
            "--events",
            "events.json",
            "--pack-id",
            "runtime-detection",
            "--title",
            "Tool result",
            "--condition",
            "model.response.tool_results[0].returned_to_model == true",
            "--severity",
            "medium",
            "--confidence",
            "high",
        ]);
        match cli.command.unwrap() {
            Commands::Detection(DetectionCommands::Hunt { id, events, .. }) => {
                assert_eq!(id, "detect-tool-result");
                assert_eq!(events, PathBuf::from("events.json"));
            }
            _ => panic!("expected detection hunt"),
        }

        let cli = Cli::parse_from([
            "capsem",
            "detection",
            "hunt-session",
            "vm-1",
            "detect-tool-result",
            "--pack-id",
            "runtime-detection",
            "--title",
            "Tool result",
            "--condition",
            "model.response.tool_results[0].returned_to_model == true",
            "--severity",
            "medium",
            "--confidence",
            "high",
            "--tag",
            "model",
            "--limit",
            "50",
            "--json",
        ]);
        match cli.command.unwrap() {
            Commands::Detection(DetectionCommands::HuntSession {
                session,
                id,
                pack_id,
                title,
                condition,
                severity,
                confidence,
                tags,
                limit,
                json,
                ..
            }) => {
                assert_eq!(session, "vm-1");
                assert_eq!(id, "detect-tool-result");
                assert_eq!(pack_id, "runtime-detection");
                assert_eq!(title, "Tool result");
                assert_eq!(
                    condition,
                    "model.response.tool_results[0].returned_to_model == true"
                );
                assert_eq!(severity, CliSeverity::Medium);
                assert_eq!(confidence, CliConfidence::High);
                assert_eq!(tags, vec!["model"]);
                assert_eq!(limit, Some(50));
                assert!(json);
            }
            _ => panic!("expected detection hunt-session"),
        }
    }

    #[test]
    fn parse_confirm_list() {
        let cli = Cli::parse_from(["capsem", "confirm", "list", "--json"]);
        match cli.command.unwrap() {
            Commands::Confirm(ConfirmCommands::List { json }) => assert!(json),
            _ => panic!("expected confirm list"),
        }
    }

    #[test]
    fn format_runtime_hunt_summary_includes_event_and_evidence_rows() {
        let summary = format_runtime_hunt_summary(&serde_json::json!({
            "total_matches": 1,
            "unique_evidence_matches": 1,
            "truncated": false,
            "rows": [{
                "event_ref": {
                    "corpus": "session_db",
                    "session_id": "vm-1",
                    "event_id": "evt-1",
                    "timestamp_unix_ms": 1700000000000_i64
                },
                "rule_id": "detect-google",
                "pack_id": "runtime-detection",
                "matched_fields": [{
                    "path": "http.request.host",
                    "value": "google.example.test"
                }],
                "outcome": "matched"
            }]
        }));

        assert!(summary.contains("Detection hunt matched 1 event(s)"));
        assert!(summary.contains("detect-google"));
        assert!(summary.contains("evt-1"));
        assert!(summary.contains("http.request.host=google.example.test"));
    }

    #[test]
    fn format_runtime_backtest_summary_uses_requested_label() {
        let summary = format_runtime_match_summary(
            "Enforcement backtest",
            &serde_json::json!({
                "total_matches": 1,
                "unique_evidence_matches": 1,
                "truncated": true,
                "rows": []
            }),
        );

        assert!(summary.contains("Enforcement backtest matched 1 event(s)"));
        assert!(summary.contains("(truncated)"));
    }

    #[test]
    fn confirm_summary_renders_disabled_resolver_state() {
        let summary = format_confirm_list_summary(&serde_json::json!({
            "pending_count": 0,
            "resolve_available": false,
            "resolve_owner": "S15-confirm-ux"
        }));
        assert!(summary.contains("unavailable"));
        assert!(summary.contains("S15-confirm-ux"));
        assert!(summary.contains("pending=0"));
    }

    #[test]
    fn profile_list_show_and_resolve_summaries_use_typed_fields() {
        let list = format_profile_list_summary(&serde_json::json!({
            "profiles": [
                {
                    "source": "built-in",
                    "locked": true,
                    "profile": {
                        "id": "coding",
                        "name": "Coding",
                        "extends_profile_id": "root"
                    }
                }
            ]
        }));
        assert!(list.contains("coding"));
        assert!(list.contains("built-in"));
        assert!(list.contains("root"));

        let show = format_profile_record_summary(&serde_json::json!({
            "source": "user",
            "locked": false,
            "profile": {
                "id": "everyday",
                "name": "Everyday",
                "ui": "everyday",
                "profile_type": "user",
                "packages": {
                    "runtimes": { "python": "3.12" },
                    "python_modules": { "requests": "2" },
                    "node_packages": {},
                    "system": {
                        "distro": "debian",
                        "release": "bookworm",
                        "apt": { "curl": "latest" }
                    }
                },
                "tools": {
                    "python": { "version": "3.12", "required": true, "source": "guest" }
                },
                "mcpServers": {
                    "github": { "enabled": true }
                },
                "vm": {
                    "memory_mib": 4096,
                    "cpus": 4,
                    "network": "proxied",
                    "assets": {
                        "arm64": {
                            "kernel": { "hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
                            "initrd": { "hash": "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
                            "rootfs": { "hash": "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" }
                        }
                    }
                }
            }
        }));
        assert!(show.contains("Profile: everyday"));
        assert!(show.contains("locked=false"));
        assert!(show.contains("Packages: runtimes=1 python=1 node=0 apt=1"));
        assert!(show.contains("Tools: 1"));
        assert!(show.contains("MCP: servers=1"));
        assert!(show.contains("asset_arches=1"));
        assert!(show.contains("assets.arm64"));

        let resolved = format_profile_resolve_summary(&serde_json::json!({
            "profile_id": "coding",
            "effective": {
                "profile_name": "Coding",
                "profile_ui": "coding",
                "rules": [{ "id": "rule-1" }],
                "mcp": { "value": { "github": {} } },
                "skills": { "value": {
                    "groups": ["admin"],
                    "enabled": ["admin-profile"],
                    "disabled": []
                }},
                "packages": { "value": {
                    "runtimes": { "node": "22" },
                    "python_modules": {},
                    "node_packages": { "typescript": "latest" },
                    "system": { "distro": "", "release": "", "apt": {} }
                }},
                "tools": { "value": { "python": {} } },
                "vm": { "value": {
                    "memory_mib": 8192,
                    "cpus": 6,
                    "network": "proxied",
                    "assets": {}
                }}
            }
        }));
        assert!(resolved.contains("profile=coding"));
        assert!(resolved.contains("rules=1"));
        assert!(resolved.contains("mcp_servers=1"));
        assert!(resolved.contains("skills=2"));
        assert!(resolved.contains("Packages: runtimes=1 python=0 node=1 apt=0"));
        assert!(resolved.contains("VM: memory_mib=8192 cpus=6"));
    }

    #[test]
    fn mcp_path_summary_and_show_filter_preserve_server_identity() {
        assert_eq!(
            mcp_connectors_path(Some(&"coding profile".to_string())),
            "/mcp/connectors?profile=coding%20profile"
        );
        let result = serde_json::json!({
            "profile_id": "coding",
            "servers": [
                {
                    "id": "github",
                    "source_profile": "coding",
                    "server": {
                        "enabled": true,
                        "type": "stdio",
                        "command": "npx",
                        "capsem": { "allowed_tools": ["repo.read"] }
                    }
                },
                {
                    "id": "browser",
                    "source_profile": "corp-root",
                    "server": {
                        "enabled": false,
                        "type": "http",
                        "url": "https://mcp.example.test",
                        "capsem": { "allowed_tools": [] }
                    }
                }
            ]
        });
        let summary = format_mcp_connectors_summary(&result);
        assert!(summary.contains("github"));
        assert!(summary.contains("repo.read"));
        assert!(summary.contains("corp-root"));

        let matches = mcp_server_matches(&result, "github");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["id"], "github");
    }

    #[test]
    fn skills_path_and_summary_preserve_profile_kind_and_ownership() {
        assert_eq!(
            skills_path(
                Some(&"coding profile".to_string()),
                Some(CliSkillKind::Disabled)
            ),
            "/skills?profile=coding%20profile&kind=disabled"
        );

        let summary = format_skills_summary(&serde_json::json!({
            "profile_id": "coding",
            "skills": [
                {
                    "id": "admin-profile",
                    "kind": "enabled",
                    "source_profile": "coding",
                    "direct": true,
                    "editable": true
                },
                {
                    "id": "corp-skill",
                    "kind": "group",
                    "source_profile": "corp-root",
                    "direct": false,
                    "editable": false
                }
            ]
        }));

        assert!(summary.contains("admin-profile"));
        assert!(summary.contains("corp-skill"));
        assert!(summary.contains("corp-root"));
    }

    #[test]
    fn read_runtime_backtest_events_accepts_envelope_array_and_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let envelope = dir.path().join("events-envelope.json");
        std::fs::write(
            &envelope,
            r#"{"events":[{"event":{"event_id":"evt-1"}},{"event":{"event_id":"evt-2"}}]}"#,
        )
        .unwrap();
        assert_eq!(read_runtime_backtest_events(&envelope).unwrap().len(), 2);

        let array = dir.path().join("events-array.json");
        std::fs::write(
            &array,
            r#"[{"event":{"event_id":"evt-1"}},{"event":{"event_id":"evt-2"}}]"#,
        )
        .unwrap();
        assert_eq!(read_runtime_backtest_events(&array).unwrap().len(), 2);

        let jsonl = dir.path().join("events.jsonl");
        std::fs::write(
            &jsonl,
            "{\"event\":{\"event_id\":\"evt-1\"}}\n{\"event\":{\"event_id\":\"evt-2\"}}\n",
        )
        .unwrap();
        assert_eq!(read_runtime_backtest_events(&jsonl).unwrap().len(), 2);
    }

    #[test]
    fn read_profile_document_parses_toml_and_json_with_validation() {
        let dir = tempfile::tempdir().unwrap();

        let toml_path = dir.path().join("profile.toml");
        std::fs::write(
            &toml_path,
            r#"
id = "typed-toml"
name = "Typed TOML"
best_for = "Testing typed profile TOML parsing."
"#,
        )
        .unwrap();
        let profile = read_profile_document(&toml_path).unwrap();
        assert_eq!(profile.id, "typed-toml");

        let json_path = dir.path().join("profile.json");
        std::fs::write(
            &json_path,
            r#"{"id":"typed-json","name":"Typed JSON","best_for":"Testing typed profile JSON parsing."}"#,
        )
        .unwrap();
        let profile = read_profile_document(&json_path).unwrap();
        assert_eq!(profile.id, "typed-json");

        let bad_path = dir.path().join("bad.json");
        std::fs::write(&bad_path, r#"{"id":"bad","name":"","best_for":"nope"}"#).unwrap();
        assert!(read_profile_document(&bad_path).is_err());
    }

    #[test]
    fn parse_profile_install_update_remove() {
        for (verb, expected_revision) in [
            ("install", Some("2026.0520.2")),
            ("update", Some("2026.0520.3")),
            ("remove", None),
        ] {
            let mut args = vec!["capsem", "profile", verb, "everyday-work", "--json"];
            if let Some(revision) = expected_revision {
                args.push("--revision");
                args.push(revision);
            }
            let cli = Cli::parse_from(args);
            match (verb, cli.command.unwrap()) {
                (
                    "install",
                    Commands::Profile(ProfileCommands::Install {
                        profile_id,
                        revision,
                        json,
                    }),
                ) => {
                    assert_eq!(profile_id, "everyday-work");
                    assert_eq!(revision.as_deref(), expected_revision);
                    assert!(json);
                }
                (
                    "update",
                    Commands::Profile(ProfileCommands::Update {
                        profile_id,
                        file,
                        revision,
                        json,
                    }),
                ) => {
                    assert_eq!(profile_id, "everyday-work");
                    assert!(file.is_none());
                    assert_eq!(revision.as_deref(), expected_revision);
                    assert!(json);
                }
                (
                    "remove",
                    Commands::Profile(ProfileCommands::Remove {
                        profile_id,
                        revision,
                        json,
                    }),
                ) => {
                    assert_eq!(profile_id, "everyday-work");
                    assert_eq!(revision.as_deref(), expected_revision);
                    assert!(json);
                }
                _ => panic!("expected profile {verb}"),
            }
        }
    }

    #[test]
    fn parse_profile_reconcile_catalog_url() {
        let cli = Cli::parse_from([
            "capsem",
            "profile",
            "reconcile-catalog",
            "--manifest-url",
            "https://profiles.example.test/catalog.json",
            "--pubkey",
            "profile.pub",
        ]);
        match cli.command.unwrap() {
            Commands::Profile(ProfileCommands::ReconcileCatalog {
                manifest,
                manifest_url,
                pubkey,
                json,
            }) => {
                assert_eq!(manifest, None);
                assert_eq!(
                    manifest_url.as_deref(),
                    Some("https://profiles.example.test/catalog.json")
                );
                assert_eq!(pubkey, PathBuf::from("profile.pub"));
                assert!(!json);
            }
            _ => panic!("expected profile reconcile-catalog"),
        }
    }

    #[test]
    fn parse_profile_reconcile_catalog_rejects_missing_source() {
        let err = match Cli::try_parse_from([
            "capsem",
            "profile",
            "reconcile-catalog",
            "--pubkey",
            "profile.pub",
        ]) {
            Ok(_) => panic!("expected missing source parse error"),
            Err(err) => err,
        };

        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn profile_catalog_reconcile_summary_line_includes_absent_removed() {
        let result = serde_json::json!({
            "summary": {
                "installed": 1,
                "unchanged": 2,
                "deprecated_kept": 3,
                "revoked_removed": 4,
                "absent_removed": 5,
                "errors": 6
            }
        });

        assert_eq!(
            profile_catalog_reconcile_summary_line(&result),
            "Profile catalog reconciled: installed=1 unchanged=2 deprecated_kept=3 revoked_removed=4 absent_removed=5 errors=6"
        );
    }

    #[test]
    fn profile_catalog_summary_line_counts_profiles() {
        let result = serde_json::json!({
            "configured": true,
            "manifest_present": true,
            "profiles": [
                {
                    "profile_id": "everyday-work",
                    "current_revision": "2026.0520.2",
                    "installed_revision": "2026.0520.2",
                    "revisions": []
                }
            ]
        });

        assert_eq!(
            profile_catalog_summary_line(&result),
            "Profile catalog: configured=true manifest_present=true profiles=1"
        );
    }

    #[test]
    fn profile_revisions_summary_line_counts_revisions() {
        let result = serde_json::json!({
            "profile_id": "everyday-work",
            "current_revision": "2026.0520.2",
            "installed_revision": "2026.0520.1",
            "revisions": [
                {"revision": "2026.0520.1", "status": "deprecated"},
                {"revision": "2026.0520.2", "status": "active"},
                {"revision": "2026.0520.3", "status": "revoked"}
            ]
        });

        assert_eq!(
            profile_revisions_summary_line(&result),
            "Profile revisions: profile=everyday-work current=2026.0520.2 installed=2026.0520.1 revisions=3"
        );
    }

    #[test]
    fn profile_revision_action_summary_line_reports_outcome() {
        let result = serde_json::json!({
            "action": "install",
            "profile_id": "everyday-work",
            "selected_revision": "2026.0520.2",
            "outcome": {
                "outcome": "installed"
            }
        });

        assert_eq!(
            profile_revision_action_summary_line(&result),
            "Profile revision install: everyday-work@2026.0520.2 installed"
        );
    }

    #[test]
    fn format_session_profile_for_list_shows_revision_and_status() {
        let mut session = SessionInfo {
            id: "vm".into(),
            name: None,
            pid: 0,
            status: "Stopped".into(),
            persistent: true,
            ram_mb: None,
            cpus: None,
            version: None,
            forked_from: None,
            description: None,
            profile_id: Some("everyday-work".into()),
            profile_revision: Some("2026.0520.2".into()),
            profile_status: Some(SessionProfileStatus::Current),
            created_at: None,
            uptime_secs: None,
            total_input_tokens: None,
            total_output_tokens: None,
            total_estimated_cost: None,
            total_tool_calls: None,
            total_mcp_calls: None,
            total_requests: None,
            allowed_requests: None,
            denied_requests: None,
            total_file_events: None,
            model_call_count: None,
            last_error: None,
        };

        assert_eq!(
            format_session_profile_for_list(&session),
            "everyday-work@2026.0520.2:current"
        );

        session.profile_id = None;
        session.profile_revision = None;
        session.profile_status = Some(SessionProfileStatus::Corrupted);
        assert_eq!(format_session_profile_for_list(&session), "corrupted");
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
    fn parse_setup_non_interactive() {
        let cli = Cli::parse_from(["capsem", "setup", "--non-interactive"]);
        match cli.command.unwrap() {
            Commands::Misc(MiscCommands::Setup {
                non_interactive,
                preset,
                force,
                ..
            }) => {
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
        match cli.command.unwrap() {
            Commands::Misc(MiscCommands::Setup { preset, force, .. }) => {
                assert_eq!(preset, Some("high".into()));
                assert!(force);
            }
            _ => panic!("expected Setup"),
        }
    }

    #[test]
    fn parse_setup_with_corp_config() {
        let cli = Cli::parse_from([
            "capsem",
            "setup",
            "--corp-config",
            "https://example.com/corp-profile.toml",
            "--non-interactive",
        ]);
        match cli.command.unwrap() {
            Commands::Misc(MiscCommands::Setup {
                corp_config,
                non_interactive,
                ..
            }) => {
                assert_eq!(
                    corp_config,
                    Some("https://example.com/corp-profile.toml".into())
                );
                assert!(non_interactive);
            }
            _ => panic!("expected Setup"),
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
    fn uninstall_does_not_refresh_update_cache() {
        let cli = Cli::parse_from(["capsem", "uninstall", "--yes"]);
        assert!(!command_refreshes_update_cache(cli.command.as_ref()));
    }

    #[test]
    fn product_purge_does_not_refresh_update_cache() {
        let cli = Cli::parse_from(["capsem", "purge", "--product", "--yes"]);
        assert!(!command_refreshes_update_cache(cli.command.as_ref()));
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
    fn parse_create_with_image_alias_rejected() {
        let cli = Cli::try_parse_from(["capsem", "create", "--image", "old-img"]);
        assert!(cli.is_err(), "--image alias should be rejected");
    }

    #[test]
    fn parse_create_with_name_and_from() {
        let cli = Cli::parse_from(["capsem", "create", "my-session", "--from", "my-src"]);
        match cli.command.unwrap() {
            Commands::Session(SessionCommands::Create { name, from, .. }) => {
                assert_eq!(name, Some("my-session".into()));
                assert_eq!(from, Some("my-src".into()));
            }
            _ => panic!("expected Create with name and --from"),
        }
    }

    #[test]
    fn parse_create_with_name_flag_rejected() {
        let cli = Cli::try_parse_from(["capsem", "create", "-n", "my-vm"]);
        assert!(cli.is_err(), "create -n should be rejected");
    }
}

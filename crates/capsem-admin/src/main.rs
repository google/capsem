use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use anyhow::{anyhow, Context, Result};
use capsem_core::asset_manager::{BinaryExecutable, BinaryFile, ManifestV2};
use capsem_core::net::policy_config::{
    resolve_profile_rule_file_path, validate_corp_toml_contract, CompiledSecurityRule,
    ProfileCatalog, ProfileConfigFile, ProfileObomConfig, ProfileObomDescriptor,
    SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource, SettingsFile,
};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

mod channel_bootstrap;
#[allow(dead_code)]
mod release_graph;
mod source_commit;

use source_commit::SourceCommit;

#[derive(Debug, Parser)]
#[command(name = "capsem-admin")]
#[command(version)]
#[command(about = "Capsem profile and asset administration")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate a channel/profile release selection without publishing it.
    Validate(ReleaseValidateArgs),
    /// Publish one channel/profile through the serialized release workflow.
    Release(ReleaseArgs),
    Profile(ProfileCommand),
    Settings(SettingsCommand),
    Enforcement(RuleFileCommand),
    Detection(RuleFileCommand),
    Manifest(ManifestCommand),
    Assets(AssetsCommand),
    Image(ImageCommand),
}

#[derive(Debug, Parser)]
struct ProfileCommand {
    #[command(subcommand)]
    command: ProfileSubcommand,
}

#[derive(Debug, Subcommand)]
enum ProfileSubcommand {
    Validate(ProfileValidateArgs),
    Check(ProfileCheckArgs),
    Materialize(ProfileMaterializeArgs),
}

#[derive(Debug, Parser)]
struct SettingsCommand {
    #[command(subcommand)]
    command: SettingsSubcommand,
}

#[derive(Debug, Subcommand)]
enum SettingsSubcommand {
    Validate(SettingsValidateArgs),
}

#[derive(Debug, Parser)]
struct RuleFileCommand {
    #[command(subcommand)]
    command: RuleFileSubcommand,
}

#[derive(Debug, Subcommand)]
enum RuleFileSubcommand {
    Validate(RuleFileArgs),
}

#[derive(Debug, Parser)]
struct ManifestCommand {
    #[command(subcommand)]
    command: ManifestSubcommand,
}

#[derive(Debug, Subcommand)]
enum ManifestSubcommand {
    Check(ManifestCheckArgs),
    Generate(ManifestGenerateArgs),
    /// Author a corporation-owned manifest from official packages and owned profiles.
    Corporate(ManifestCorporateArgs),
}

#[derive(Debug, Parser)]
struct AssetsCommand {
    #[command(subcommand)]
    command: AssetsSubcommand,
}

#[derive(Debug, Subcommand)]
enum AssetsSubcommand {
    Channel(AssetsChannelCommand),
}

#[derive(Debug, Parser)]
struct AssetsChannelCommand {
    #[command(subcommand)]
    command: AssetsChannelSubcommand,
}

#[derive(Debug, Subcommand)]
enum AssetsChannelSubcommand {
    Build(AssetsChannelBuildArgs),
    Check(AssetsChannelCheckArgs),
    RecordBinary(AssetsChannelRecordBinaryArgs),
}

#[derive(Debug, Parser)]
struct ImageCommand {
    #[command(subcommand)]
    command: ImageSubcommand,
}

#[derive(Debug, Subcommand)]
enum ImageSubcommand {
    Build(ImageBuildArgs),
    Workspace(ImageWorkspaceArgs),
}

#[derive(Debug, Parser)]
struct ProfileValidateArgs {
    /// Profile TOML to validate.
    path: PathBuf,
    /// Config root used to resolve profile rule files.
    #[arg(long)]
    config_root: Option<PathBuf>,
    /// Require signed runtime pins instead of source-profile placeholders.
    #[arg(long)]
    materialized: bool,
    /// Emit a machine-readable validation report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ProfileCheckArgs {
    /// Profile TOML to check.
    path: PathBuf,
    /// Config root used to resolve profile rule files.
    #[arg(long)]
    config_root: Option<PathBuf>,
    /// Restrict file:// asset verification to one profile arch.
    #[arg(long)]
    arch: Option<String>,
    /// Emit a machine-readable check report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ProfileMaterializeArgs {
    /// Source profile TOML to materialize.
    #[arg(long)]
    profile: PathBuf,
    /// Source config root containing settings, corp, profiles, and rule files.
    #[arg(long, default_value = "config")]
    config_root: PathBuf,
    /// Generated asset manifest URL to use for current build hashes.
    #[arg(long)]
    manifest: String,
    /// Built asset root containing per-arch logical asset files.
    #[arg(long, default_value = "assets")]
    assets_dir: PathBuf,
    /// Generated runtime config output root.
    #[arg(long, default_value = "target/config")]
    output_root: PathBuf,
    /// Restrict materialization to one architecture.
    #[arg(long)]
    arch: Option<String>,
    /// Remove output root before materializing.
    #[arg(long)]
    clean: bool,
    /// Emit a machine-readable materialization report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args, Clone)]
struct ReleaseValidateArgs {
    /// Channel that owns the profile.
    #[arg(long)]
    channel: String,
    /// Profile id to validate.
    #[arg(long)]
    profile: String,
    /// Source config root containing the profile definition.
    #[arg(long, default_value = "config")]
    config_root: PathBuf,
    /// Emit a machine-readable validation report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args, Clone)]
struct ReleaseArgs {
    /// Channel that owns this independently releasable profile instance.
    #[arg(long)]
    channel: String,
    /// Profile id to publish.
    #[arg(long)]
    profile: String,
    /// Exact committed source qualified before this release dispatch.
    #[arg(long)]
    source_commit: SourceCommit,
    /// Source config root containing the profile definition.
    #[arg(long, default_value = "config")]
    config_root: PathBuf,
    /// Manifest JSON file to update inside the serialized workflow.
    #[arg(long, hide = true, requires_all = ["manifest_version", "profile_version"])]
    manifest_path: Option<PathBuf>,
    /// Candidate manifest containing the newly built selected profile.
    #[arg(long, hide = true, requires = "manifest_path")]
    candidate_manifest: Option<PathBuf>,
    /// Immutable release base containing the selected profile's published files.
    #[arg(long, hide = true, requires = "candidate_manifest")]
    publication_base: Option<String>,
    /// Manifest version expected in the JSON file.
    #[arg(long, hide = true, requires = "manifest_path")]
    manifest_version: Option<String>,
    /// Profile revision/version expected in the manifest.
    #[arg(long, hide = true, requires = "manifest_path")]
    profile_version: Option<String>,
    /// Publication state written by the serialized workflow.
    #[arg(long, value_enum, default_value_t = ProfileReleaseStatusArg::Current, hide = true)]
    status: ProfileReleaseStatusArg,
    /// Existing first-party channel source used only to initialize a missing channel.
    #[arg(
        long,
        hide = true,
        requires = "bootstrap_output",
        conflicts_with = "manifest_path"
    )]
    bootstrap_from_manifest: Option<PathBuf>,
    /// Selected-channel source manifest created by the serialized workflow.
    #[arg(
        long,
        hide = true,
        requires = "bootstrap_from_manifest",
        conflicts_with = "manifest_path"
    )]
    bootstrap_output: Option<PathBuf>,
    /// Validate and print the workflow dispatch without executing it.
    #[arg(long, conflicts_with = "bootstrap_from_manifest")]
    dry_run: bool,
    /// Emit a machine-readable release report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct SettingsValidateArgs {
    /// Settings TOML to validate.
    path: PathBuf,
    /// Emit a machine-readable validation report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct RuleFileArgs {
    /// Enforcement TOML or Sigma YAML file to validate.
    path: PathBuf,
    /// Treat the rules as this source when resolving priority.
    #[arg(long, value_enum, default_value_t = RuleFileSourceArg::User)]
    source: RuleFileSourceArg,
    /// Emit a machine-readable validation report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ManifestCheckArgs {
    /// Manifest JSON file to validate.
    path: PathBuf,
    /// Emit a machine-readable manifest report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ManifestGenerateArgs {
    /// Asset directory containing built per-arch assets.
    #[arg(default_value = "assets")]
    assets_dir: PathBuf,
    /// Binary version to record. Defaults to capsem-builder's project version.
    #[arg(long)]
    version: Option<String>,
    /// Emit the generated manifest after writing it.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ManifestCorporateArgs {
    /// Corporation namespace that owns the generated manifest.
    #[arg(long)]
    corporation: String,
    /// Corporation-owned channel name.
    #[arg(long)]
    channel: String,
    /// Read-only official Capsem release manifest containing selectable packages.
    #[arg(long)]
    official_manifest: PathBuf,
    /// Read-only capsem-admin-generated manifest containing corporation-owned profiles.
    #[arg(long)]
    profile_manifest: PathBuf,
    /// HTTPS base that must own every profile config, image, inventory, and evidence URL.
    #[arg(long)]
    profile_base: String,
    /// Official Capsem version to pin, or "latest" for the highest selectable version.
    #[arg(long)]
    binary: String,
    /// Exact source commit that built the corporation-owned profiles.
    #[arg(long)]
    source_commit: SourceCommit,
    /// Root below which capsem-admin owns corporation/channel manifest destinations.
    #[arg(long)]
    output_root: PathBuf,
    /// Version written to the generated corporate manifest.
    #[arg(long, default_value = "1.0.0")]
    manifest_version: String,
    /// Emit a machine-readable authoring report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct AssetsChannelBuildArgs {
    /// Source asset manifest URL to publish into the channel.
    #[arg(long)]
    manifest: String,
    /// Built asset root containing per-arch logical asset files.
    #[arg(long, default_value = "assets")]
    assets_dir: PathBuf,
    /// Optional published asset base for immutable VM blobs. Use a stable base
    /// or a template containing {asset_version}; when set, the release channel
    /// records external blob URLs instead of copying blobs into the Pages dist.
    #[arg(long)]
    asset_source_base: Option<String>,
    /// Source profile directory to publish in the channel manifest.
    #[arg(long, default_value = "config/profiles")]
    profiles_dir: PathBuf,
    /// Channel name to publish under assets/<channel>/manifest.json.
    #[arg(long, default_value = "stable")]
    channel: String,
    /// Revision validation for the profile bytes being assembled. Release
    /// authoring is always strict; the sealed install proof may explicitly
    /// import an already-published legacy profile into its local graph.
    #[arg(
        long,
        value_enum,
        default_value_t = ProfileRevisionPolicyArg::Strict,
        hide = true
    )]
    profile_revision_policy: ProfileRevisionPolicyArg,
    /// Release graph manifest version for this channel pointer.
    #[arg(long, default_value = "1.0.0")]
    manifest_version: String,
    /// Static output directory for Cloudflare Pages.
    #[arg(long, default_value = "target/release-channel")]
    out_dir: PathBuf,
    /// Channel generation timestamp. Defaults to current UTC time.
    #[arg(long)]
    generated_at: Option<String>,
    /// Emit a machine-readable build report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct AssetsChannelCheckArgs {
    /// Static output directory to validate.
    #[arg(long, default_value = "target/release-channel")]
    dist: PathBuf,
    /// Channel name expected under assets/<channel>/manifest.json.
    #[arg(long, default_value = "stable")]
    channel: String,
    /// Emit a machine-readable validation report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct AssetsChannelRecordBinaryArgs {
    /// Local channel manifest to update in place.
    #[arg(long)]
    manifest_path: PathBuf,
    /// Binary version being published, without the leading v.
    #[arg(long)]
    version: String,
    /// Exact committed source whose packages are being recorded.
    #[arg(long)]
    source_commit: SourceCommit,
    /// Oldest asset version compatible with this binary. Defaults to assets.current.
    #[arg(long)]
    min_assets: Option<String>,
    /// Release artifact to record. Repeat for .pkg, .deb, and SBOM files.
    #[arg(long = "artifact", required = true)]
    artifacts: Vec<PathBuf>,
    /// Release date (YYYY-MM-DD). Defaults to current UTC date.
    #[arg(long)]
    date: Option<String>,
    /// Emit a machine-readable update report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ImageBuildArgs {
    /// Profile TOML that owns the asset build.
    #[arg(long)]
    profile: PathBuf,
    /// Config root used to validate profile rule files.
    #[arg(long, default_value = "config")]
    config_root: PathBuf,
    /// Guest image source directory consumed by capsem-builder.
    #[arg(long, default_value = "guest")]
    guest_dir: PathBuf,
    /// Output directory for built assets.
    #[arg(long, default_value = "assets")]
    output: PathBuf,
    /// Restrict the build to one profile architecture.
    #[arg(long)]
    arch: Option<String>,
    /// Build only kernel, only rootfs, or both.
    #[arg(long, value_enum, default_value_t = ImageBuildTemplate::All)]
    template: ImageBuildTemplate,
    /// Remove selected output assets before building.
    #[arg(long)]
    clean: bool,
    /// Emit a machine-readable build plan/report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ImageWorkspaceArgs {
    /// Profile TOML that owns the image workspace.
    #[arg(long)]
    profile: PathBuf,
    /// Config root used to resolve profile rule files.
    #[arg(long, default_value = "config")]
    config_root: PathBuf,
    /// Guest image source directory consumed by capsem-builder.
    #[arg(long, default_value = "guest")]
    guest_dir: PathBuf,
    /// Directory to materialize the image workspace into.
    #[arg(long)]
    output: PathBuf,
    /// Restrict the workspace build plan to one profile architecture.
    #[arg(long)]
    arch: Option<String>,
    /// Emit a machine-readable workspace report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ImageBuildTemplate {
    All,
    Kernel,
    Rootfs,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum RuleFileSourceArg {
    User,
    Corp,
    BuiltinDefault,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ProfileReleaseStatusArg {
    Current,
    Supported,
    Deprecated,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ProfileRevisionPolicyArg {
    Strict,
    SelectedInput,
}

impl ProfileReleaseStatusArg {
    const fn into_status(self) -> release_graph::Status {
        match self {
            Self::Current => release_graph::Status::Current,
            Self::Supported => release_graph::Status::Supported,
            Self::Deprecated => release_graph::Status::Deprecated,
            Self::Revoked => release_graph::Status::Revoked,
        }
    }
}

impl RuleFileSourceArg {
    const fn into_security_rule_source(self) -> SecurityRuleSource {
        match self {
            Self::User => SecurityRuleSource::User,
            Self::Corp => SecurityRuleSource::Corp,
            Self::BuiltinDefault => SecurityRuleSource::BuiltinDefault,
        }
    }
}

#[derive(Debug, Serialize)]
struct ProfileValidationReport {
    schema: &'static str,
    ok: bool,
    profile_id: String,
    path: String,
    config_root: String,
    compiled_rules: usize,
}

#[derive(Debug, Serialize)]
struct ProfileCheckReport {
    schema: &'static str,
    ok: bool,
    validation: ProfileValidationReport,
    assets: Vec<LocalAssetCheckReport>,
    profile_files: Vec<LocalAssetCheckReport>,
}

#[derive(Debug, Serialize)]
struct ConfigRootCheckReport {
    schema: &'static str,
    ok: bool,
    config_root: String,
    settings: SettingsValidationReport,
    corp_rules: usize,
    profiles: Vec<ProfileCheckReport>,
}

#[derive(Debug, Serialize)]
struct ProfileMaterializeReport {
    schema: &'static str,
    ok: bool,
    profile_id: String,
    profile_revision: String,
    source_config_root: String,
    output_config_root: String,
    profile_path: String,
    manifest: String,
    asset_version: String,
    materialized_assets: Vec<ProfileMaterializedAssetReport>,
    materialized_obom: Vec<ProfileMaterializedObomReport>,
}

#[derive(Debug, Serialize)]
struct ProfileReleaseReport {
    schema: &'static str,
    ok: bool,
    action: &'static str,
    channel: String,
    manifest: String,
    manifest_version: String,
    profile: String,
    profile_version: String,
    publication_identity: String,
    status: release_graph::Status,
    changed_channels: Vec<String>,
    changed_manifests: Vec<String>,
    changed_profiles: Vec<String>,
    changed_config_refs: usize,
    changed_image_artifacts: usize,
    compatible_with_current_binary: bool,
}

#[derive(Debug, Serialize)]
struct ReleaseSelectionReport {
    schema: &'static str,
    ok: bool,
    channel: String,
    profile: String,
    profile_revision: String,
    publication_identity: String,
    profile_path: String,
}

#[derive(Debug, Serialize)]
struct ReleaseDispatchReport {
    schema: &'static str,
    ok: bool,
    channel: String,
    profile: String,
    profile_revision: String,
    publication_identity: String,
    source_commit: SourceCommit,
    workflow: &'static str,
    dispatched: bool,
    run_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ProfileWorkflowRun {
    #[serde(rename = "databaseId")]
    database_id: u64,
    #[serde(rename = "displayTitle")]
    display_title: String,
    #[serde(rename = "headSha")]
    head_sha: String,
    #[serde(rename = "headBranch")]
    head_branch: String,
    status: String,
    conclusion: String,
}

trait ProfileWorkflowRunner {
    fn run(&mut self, args: &[String]) -> Result<()>;
    fn output(&mut self, args: &[String]) -> Result<String>;
    fn wait_before_poll(&mut self);
}

struct GhProfileWorkflowRunner;

impl ProfileWorkflowRunner for GhProfileWorkflowRunner {
    fn run(&mut self, args: &[String]) -> Result<()> {
        let status = Command::new("gh")
            .args(args)
            .status()
            .with_context(|| format!("run gh {}", args.join(" ")))?;
        if !status.success() {
            return Err(anyhow!("gh {} failed with {}", args.join(" "), status));
        }
        Ok(())
    }

    fn output(&mut self, args: &[String]) -> Result<String> {
        let output = Command::new("gh")
            .args(args)
            .output()
            .with_context(|| format!("run gh {}", args.join(" ")))?;
        if !output.status.success() {
            return Err(anyhow!(
                "gh {} failed with {}: {}",
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        String::from_utf8(output.stdout).context("gh workflow listing was not UTF-8")
    }

    fn wait_before_poll(&mut self) {
        thread::sleep(std::time::Duration::from_secs(2));
    }
}

fn dispatch_profile_workflow<R: ProfileWorkflowRunner>(
    runner: &mut R,
    workflow: &str,
    channel: &str,
    profile: &str,
    source_commit: &SourceCommit,
    dispatch_id: &str,
) -> Result<u64> {
    if dispatch_id.is_empty()
        || !dispatch_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
    {
        return Err(anyhow!("profile workflow dispatch id is unsafe"));
    }
    let title = format!("Release profile {channel}/{profile} {dispatch_id}");
    let source_ref = format!("capsem-source-{source_commit}");
    runner.run(&[
        "workflow".to_string(),
        "run".to_string(),
        workflow.to_string(),
        "--ref".to_string(),
        source_ref.clone(),
        "-f".to_string(),
        format!("channel={channel}"),
        "-f".to_string(),
        format!("profile={profile}"),
        "-f".to_string(),
        "dry_run=false".to_string(),
        "-f".to_string(),
        format!("dispatch_id={dispatch_id}"),
        "-f".to_string(),
        format!("source_commit={source_commit}"),
    ])?;

    for poll in 0..60 {
        let raw = runner.output(&[
            "run".to_string(),
            "list".to_string(),
            "--workflow".to_string(),
            workflow.to_string(),
            "--branch".to_string(),
            source_ref.clone(),
            "--commit".to_string(),
            source_commit.to_string(),
            "--event".to_string(),
            "workflow_dispatch".to_string(),
            "--limit".to_string(),
            "100".to_string(),
            "--json".to_string(),
            "databaseId,displayTitle,headSha,headBranch,status,conclusion".to_string(),
        ])?;
        let runs: Vec<ProfileWorkflowRun> = serde_json::from_str(&raw)
            .context("GitHub returned invalid profile workflow run JSON")?;
        let matches = runs
            .into_iter()
            .filter(|run| run.display_title == title)
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(anyhow!(
                "GitHub returned multiple profile workflow runs for correlation {dispatch_id}"
            ));
        }
        if let Some(run) = matches.first() {
            if run.head_sha != source_commit.as_str() || run.head_branch != source_ref {
                return Err(anyhow!(
                    "profile workflow correlation matched the wrong source: {run:?}"
                ));
            }
            if run.status == "completed" && run.conclusion != "success" {
                return Err(anyhow!(
                    "profile workflow run {} completed with {}",
                    run.database_id,
                    run.conclusion
                ));
            }
            runner.run(&[
                "run".to_string(),
                "watch".to_string(),
                run.database_id.to_string(),
                "--exit-status".to_string(),
            ])?;
            let viewed = runner.output(&[
                "run".to_string(),
                "view".to_string(),
                run.database_id.to_string(),
                "--json".to_string(),
                "databaseId,displayTitle,headSha,headBranch,status,conclusion".to_string(),
            ])?;
            let completed: ProfileWorkflowRun = serde_json::from_str(&viewed)
                .context("GitHub returned invalid completed profile workflow JSON")?;
            if completed.database_id != run.database_id
                || completed.display_title != title
                || completed.head_sha != source_commit.as_str()
                || completed.head_branch != source_ref
                || completed.status != "completed"
                || completed.conclusion != "success"
            {
                return Err(anyhow!(
                    "completed profile workflow identity changed: {completed:?}"
                ));
            }
            return Ok(run.database_id);
        }
        if poll < 59 {
            runner.wait_before_poll();
        }
    }
    Err(anyhow!(
        "timed out waiting for {workflow} run correlated by {dispatch_id}"
    ))
}

#[derive(Debug, Serialize)]
struct CorporateManifestReport {
    schema: &'static str,
    ok: bool,
    corporation: String,
    channel: String,
    binary_policy: String,
    resolved_binary_version: String,
    official_manifest: String,
    profile_manifest: String,
    output_manifest: String,
    profiles: Vec<String>,
    packages: usize,
}

#[derive(Debug, Serialize)]
struct ProfileMaterializedAssetReport {
    arch: String,
    logical_name: String,
    url: String,
    hash: String,
    size: u64,
}

#[derive(Debug, Serialize)]
struct ProfileMaterializedObomReport {
    arch: String,
    url: String,
    hash: String,
    size: u64,
    generator: String,
    generator_version: String,
    rootfs_hash: String,
    scope: &'static str,
}

#[derive(Debug, Serialize)]
struct SettingsValidationReport {
    schema: &'static str,
    ok: bool,
    path: String,
    app: SettingsAppReport,
    appearance: SettingsAppearanceReport,
}

#[derive(Debug, Serialize)]
struct SettingsAppReport {
    auto_update: bool,
    notifications: bool,
    start_service_at_login: bool,
}

#[derive(Debug, Serialize)]
struct SettingsAppearanceReport {
    theme: String,
    font_size: u32,
    reduced_motion: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsConfigFile {
    app: SettingsApp,
    appearance: SettingsAppearance,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsApp {
    auto_update: bool,
    notifications: bool,
    start_service_at_login: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsAppearance {
    theme: String,
    font_size: u32,
    reduced_motion: bool,
}

#[derive(Debug, Serialize)]
struct RuleFileReport {
    schema: &'static str,
    ok: bool,
    kind: &'static str,
    source: &'static str,
    path: String,
    compiled_rules: usize,
    rules: Vec<CompiledRuleReport>,
}

#[derive(Debug, Serialize)]
struct CompiledRuleReport {
    rule_id: String,
    provider: String,
    namespace: String,
    rule_key: String,
    default_rule: bool,
    name: String,
    action: &'static str,
    detection_level: Option<&'static str>,
    priority: i32,
    condition: String,
    reason: Option<String>,
    corp_locked: bool,
}

#[derive(Debug, Serialize)]
struct ManifestReport {
    schema: &'static str,
    ok: bool,
    path: String,
    blake3: String,
    refresh_policy: String,
    asset_version: String,
    binary_version: String,
    releases: usize,
    arches: Vec<ManifestArchReport>,
}

#[derive(Debug, Serialize)]
struct ManifestArchReport {
    asset_version: String,
    arch: String,
    assets: Vec<ManifestAssetReport>,
}

#[derive(Debug, Serialize)]
struct ManifestAssetReport {
    logical_name: String,
    hash: String,
    size: u64,
    path: Option<String>,
    present: bool,
    size_ok: Option<bool>,
    blake3_ok: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ImageBuildPlan {
    schema: &'static str,
    profile_id: String,
    profile_revision: String,
    guest_dir: String,
    output: String,
    clean: bool,
    template: &'static str,
    arches: Vec<ImageBuildArchPlan>,
    commands: Vec<CommandReport>,
}

#[cfg(test)]
#[derive(Debug, Serialize)]
struct ImageVerifyReport {
    schema: &'static str,
    ok: bool,
    profile_id: String,
    profile_revision: String,
    output: String,
    manifest: String,
    arches: Vec<ImageVerifyArchReport>,
}

#[derive(Debug, Serialize)]
struct ImageWorkspaceReport {
    schema: &'static str,
    ok: bool,
    profile_id: String,
    profile_revision: String,
    workspace: String,
    config_root: String,
    profile_path: String,
    profile_blake3: String,
    build_plan_path: String,
    rule_files: Vec<ImageWorkspaceRuleFileReport>,
    arches: Vec<ImageBuildArchPlan>,
}

#[derive(Debug, Serialize)]
struct ImageWorkspaceRuleFileReport {
    kind: &'static str,
    source: String,
    path: String,
    blake3: String,
    size: u64,
}

#[cfg(test)]
#[derive(Debug, Serialize)]
struct ImageVerifyArchReport {
    arch: String,
    assets: Vec<LocalAssetCheckReport>,
}

#[derive(Debug, Serialize)]
struct LocalAssetCheckReport {
    arch: String,
    logical_name: String,
    expected_hash: String,
    expected_size: u64,
    path: Option<String>,
    present: bool,
    size_ok: Option<bool>,
    blake3_ok: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ImageBuildArchPlan {
    arch: String,
    kernel: String,
    initrd: String,
    rootfs: String,
}

#[derive(Debug, Serialize, Clone)]
struct CommandReport {
    step: String,
    arch: Option<String>,
    env: BTreeMap<String, String>,
    argv: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AssetsChannelIndex {
    schema_version: u64,
    channel: String,
    state: String,
    generated_at: String,
    release_site: String,
    summary: String,
    manifest: String,
    asset_base: String,
    manifest_blake3: String,
    binary_version: String,
    asset_version: String,
    asset_state: String,
    asset_min_binary: Option<String>,
    binary_state: String,
    asset_releases: usize,
    asset_release_history: Vec<AssetsChannelAssetRelease>,
    binary_releases: usize,
    arches: Vec<String>,
    current_asset_files: Vec<AssetsChannelAssetFile>,
    binary_files: Vec<AssetsChannelBinaryFile>,
    host_sboms: Vec<AssetsChannelBinaryFile>,
    attestations: Vec<AssetsChannelAttestation>,
    vm_oboms: Vec<AssetsChannelAssetFile>,
    profiles: AssetsChannelProfilesSummary,
    image_update_state: String,
}

#[derive(Debug, Serialize, Clone)]
struct AssetsChannelAssetRelease {
    version: String,
    date: String,
    state: String,
    deprecated: bool,
    deprecated_date: Option<String>,
    min_binary: String,
    arches: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AssetsChannelProfilesSummary {
    revision: String,
    profile_count: usize,
    profile_ids: Vec<String>,
    refresh_policy: String,
    min_binary: String,
    requires_newer_binary: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AssetsChannelsCatalog {
    version: u64,
    generated_at: String,
    release_site: String,
    channels: BTreeMap<String, AssetsChannelsCatalogChannel>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AssetsChannelsCatalogChannel {
    label: String,
    manifests: Vec<AssetsChannelsCatalogManifest>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AssetsChannelsCatalogManifest {
    version: String,
    status: String,
    url: String,
    digest: AssetsChannelsCatalogDigest,
}

#[derive(Debug, Serialize, Deserialize)]
struct AssetsChannelsCatalogDigest {
    sha256: String,
    blake3: String,
}

struct PublishableProfiles {
    summary: AssetsChannelProfilesSummary,
    profiles: Vec<serde_json::Value>,
    file_copies: Vec<ProfileReleaseFileCopy>,
}

struct ProfileReleaseFileCopy {
    source: PathBuf,
    url: String,
}

#[derive(Debug, Serialize, Clone)]
struct AssetsChannelAssetFile {
    arch: String,
    logical_name: String,
    url: String,
    hash: String,
    size: u64,
}

#[derive(Debug, Serialize, Clone)]
struct AssetsChannelBinaryFile {
    name: String,
    url: String,
    sha256: String,
    blake3: String,
    size: u64,
    binaries: Vec<capsem_core::asset_manager::BinaryExecutable>,
}

#[derive(Debug, Serialize, Clone)]
struct AssetsChannelAttestation {
    name: String,
    scope: String,
    workflow: String,
    predicate_type: String,
    predicate_url: Option<String>,
    verify_command: String,
    subjects: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AssetsChannelBuildReport {
    schema: &'static str,
    channel: String,
    generated_at: String,
    out_dir: String,
    human_site_source: &'static str,
    channels_json: String,
    manifest: String,
    health_json: String,
    copied_assets: usize,
}

#[derive(Debug, Serialize)]
struct AssetsChannelRecordBinaryReport {
    schema: &'static str,
    manifest: String,
    version: String,
    min_assets: String,
    files: Vec<BinaryFile>,
}

#[derive(Debug, Serialize)]
struct AssetsChannelCheckReport {
    schema: &'static str,
    ok: bool,
    channel: String,
    state: String,
    dist: String,
    manifest: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Validate(args) => release_validate_command(args),
        Commands::Release(args) => release_command(args),
        Commands::Profile(command) => match command.command {
            ProfileSubcommand::Validate(args) => validate_profile_command(args),
            ProfileSubcommand::Check(args) => profile_check_command(args),
            ProfileSubcommand::Materialize(args) => profile_materialize_command(args),
        },
        Commands::Settings(command) => match command.command {
            SettingsSubcommand::Validate(args) => validate_settings_command(args),
        },
        Commands::Enforcement(command) => match command.command {
            RuleFileSubcommand::Validate(args) => validate_rule_file_command("enforcement", args),
        },
        Commands::Detection(command) => match command.command {
            RuleFileSubcommand::Validate(args) => validate_rule_file_command("detection", args),
        },
        Commands::Manifest(command) => match command.command {
            ManifestSubcommand::Check(args) => manifest_check_command(args),
            ManifestSubcommand::Generate(args) => manifest_generate_command(args),
            ManifestSubcommand::Corporate(args) => corporate_manifest_command(args),
        },
        Commands::Assets(command) => match command.command {
            AssetsSubcommand::Channel(command) => match command.command {
                AssetsChannelSubcommand::Build(args) => assets_channel_build_command(args),
                AssetsChannelSubcommand::Check(args) => assets_channel_check_command(args),
                AssetsChannelSubcommand::RecordBinary(args) => {
                    assets_channel_record_binary_command(args)
                }
            },
        },
        Commands::Image(command) => match command.command {
            ImageSubcommand::Build(args) => image_build_command(args),
            ImageSubcommand::Workspace(args) => image_workspace_command(args),
        },
    }
}

fn validate_profile_command(args: ProfileValidateArgs) -> Result<()> {
    let report = if args.materialized {
        validate_materialized_profile(&args.path, args.config_root.as_deref())?
    } else {
        validate_profile(&args.path, args.config_root.as_deref())?
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "valid: profile {} ({} compiled rules)",
            report.profile_id, report.compiled_rules
        );
    }
    Ok(())
}

fn profile_check_command(args: ProfileCheckArgs) -> Result<()> {
    let report = check_profile(&args)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "valid: profile {} ({} compiled rules)",
            report.validation.profile_id, report.validation.compiled_rules
        );
        if !report.assets.is_empty() {
            println!(
                "valid: profile file assets ({} assets)",
                report.assets.len()
            );
        }
    }
    Ok(())
}

fn profile_materialize_command(args: ProfileMaterializeArgs) -> Result<()> {
    let report = materialize_profile_config(&args)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "materialized: profile {} at {}",
            report.profile_id, report.output_config_root
        );
    }
    Ok(())
}

fn validate_release_selection(args: &ReleaseValidateArgs) -> Result<ReleaseSelectionReport> {
    validate_channel_name(&args.channel)?;
    let profiles_dir = args.config_root.join("profiles");
    let catalog = ProfileCatalog::load_from_dir(&profiles_dir)
        .map_err(|error| anyhow!("load profile directory {}: {error}", profiles_dir.display()))?;
    let profile = catalog.get(&args.profile).ok_or_else(|| {
        anyhow!(
            "profile {} does not exist below {}",
            args.profile,
            profiles_dir.display()
        )
    })?;
    let profile_path = profiles_dir.join(&profile.id).join("profile.toml");
    validate_profile(&profile_path, Some(&args.config_root))?;
    let publication_identity =
        profile_publication_identity(&args.channel, &profile.id, &profile.revision)?;
    Ok(ReleaseSelectionReport {
        schema: "capsem.admin.release_validate.v1",
        ok: true,
        channel: args.channel.clone(),
        profile: profile.id.clone(),
        profile_revision: profile.revision.clone(),
        publication_identity,
        profile_path: profile_path.display().to_string(),
    })
}

fn release_validate_command(args: ReleaseValidateArgs) -> Result<()> {
    let report = validate_release_selection(&args)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "valid: {}/{} revision {}",
            report.channel, report.profile, report.profile_revision
        );
    }
    Ok(())
}

fn release_command(args: ReleaseArgs) -> Result<()> {
    let selection = validate_release_selection(&ReleaseValidateArgs {
        channel: args.channel.clone(),
        profile: args.profile.clone(),
        config_root: args.config_root.clone(),
        json: args.json,
    })?;
    if let (Some(donor_path), Some(output_path)) = (
        args.bootstrap_from_manifest.as_deref(),
        args.bootstrap_output.as_deref(),
    ) {
        let donor: serde_json::Value = serde_json::from_slice(
            &fs::read(donor_path)
                .with_context(|| format!("read bootstrap donor {}", donor_path.display()))?,
        )
        .with_context(|| format!("parse bootstrap donor {}", donor_path.display()))?;
        let donor_channel = donor
            .get("channel")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("bootstrap donor is missing its channel"))?;
        validate_assets_channel_graph_manifest(&donor, donor_channel)?;
        let bootstrapped =
            channel_bootstrap::bootstrap_first_party_channel_source(&args.channel, &donor)?;
        validate_assets_channel_graph_manifest(&bootstrapped, &args.channel)?;
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let mut bytes =
            serde_json::to_vec_pretty(&bootstrapped).context("serialize bootstrap manifest")?;
        bytes.push(b'\n');
        fs::write(output_path, bytes)
            .with_context(|| format!("write {}", output_path.display()))?;
        let report = serde_json::json!({
            "schema": "capsem.admin.release_bootstrap.v1",
            "ok": true,
            "channel": args.channel,
            "profile": selection.profile,
            "profile_revision": selection.profile_revision,
            "publication_identity": selection.publication_identity,
            "donor_channel": donor_channel,
            "package_count": bootstrapped["packages"].as_array().map_or(0, Vec::len),
            "output": output_path.display().to_string(),
        });
        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "bootstrapped {}/{} source manifest from verified {} packages",
                report["channel"].as_str().unwrap_or("channel"),
                report["profile"].as_str().unwrap_or("profile"),
                donor_channel
            );
        }
        return Ok(());
    }
    if args.manifest_path.is_some() {
        let report = apply_profile_release_status(&args)?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "release: profile {} {} in channel {} manifest {}",
                report.profile,
                serde_json::to_value(report.status)?
                    .as_str()
                    .unwrap_or("status"),
                report.channel,
                report.manifest_version
            );
        }
        return Ok(());
    }

    let workflow = "release-assets.yaml";
    let run_id = if args.dry_run {
        None
    } else {
        let dispatch_id = format!(
            "capsem-admin-{}-{}",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        );
        Some(dispatch_profile_workflow(
            &mut GhProfileWorkflowRunner,
            workflow,
            &args.channel,
            &args.profile,
            &args.source_commit,
            &dispatch_id,
        )?)
    };
    let report = ReleaseDispatchReport {
        schema: "capsem.admin.release_dispatch.v1",
        ok: true,
        channel: args.channel,
        profile: args.profile,
        profile_revision: selection.profile_revision,
        publication_identity: selection.publication_identity,
        source_commit: args.source_commit.clone(),
        workflow,
        dispatched: !args.dry_run,
        run_id,
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "{}: {}/{} revision {} via {}{}",
            if report.dispatched {
                "dispatched"
            } else {
                "validated"
            },
            report.channel,
            report.profile,
            report.profile_revision,
            report.workflow,
            report
                .run_id
                .map(|run_id| format!(" run {run_id}"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

fn apply_profile_release_status(args: &ReleaseArgs) -> Result<ProfileReleaseReport> {
    let manifest_path = args
        .manifest_path
        .as_ref()
        .ok_or_else(|| anyhow!("internal profile publication requires --manifest-path"))?;
    let manifest_version = args
        .manifest_version
        .as_deref()
        .ok_or_else(|| anyhow!("internal profile publication requires --manifest-version"))?;
    let profile_version = args
        .profile_version
        .as_deref()
        .ok_or_else(|| anyhow!("internal profile publication requires --profile-version"))?;
    let status = args.status.into_status();
    if let Some(candidate_manifest) = args.candidate_manifest.as_deref() {
        return merge_graph_profile_release(
            args,
            manifest_path,
            candidate_manifest,
            manifest_version,
            profile_version,
            status,
        );
    }
    let bytes = fs::read(manifest_path)
        .with_context(|| format!("read release manifest {}", manifest_path.display()))?;
    let mut manifest: release_graph::ReleaseManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse release manifest {}", manifest_path.display()))?;
    if manifest.version != manifest_version {
        return Err(anyhow!(
            "manifest {} has version {}, expected {}",
            manifest_path.display(),
            manifest.version,
            manifest_version
        ));
    }
    let profile = manifest.profiles.get_mut(&args.profile).ok_or_else(|| {
        anyhow!(
            "manifest {} does not list profile {}",
            manifest_path.display(),
            args.profile
        )
    })?;
    if profile.revision != profile_version {
        return Err(anyhow!(
            "profile {} has revision {}, expected {}",
            args.profile,
            profile.revision,
            profile_version
        ));
    }

    profile.validate_profile_ownership()?;
    profile.status = status;
    let mut changed_config_refs = 0;
    let mut changed_image_artifacts = 0;
    for architecture in &mut profile.architectures {
        for config in &mut architecture.config {
            if config.status != status {
                changed_config_refs += 1;
            }
            config.status = status;
        }
        for artifact in &mut architecture.artifacts {
            if artifact.status != status {
                changed_image_artifacts += 1;
            }
            artifact.status = status;
        }
    }
    profile.validate_profile_ownership()?;

    let updated = serde_json::to_vec_pretty(&manifest)?;
    fs::write(manifest_path, [&updated[..], b"\n"].concat())
        .with_context(|| format!("write release manifest {}", manifest_path.display()))?;

    Ok(ProfileReleaseReport {
        schema: "capsem.admin.profile_release.v1",
        ok: true,
        action: "release",
        channel: args.channel.clone(),
        manifest: manifest_path.display().to_string(),
        manifest_version: manifest_version.to_string(),
        profile: args.profile.clone(),
        profile_version: profile_version.to_string(),
        publication_identity: profile_publication_identity(
            &args.channel,
            &args.profile,
            profile_version,
        )?,
        status,
        changed_channels: vec![args.channel.clone()],
        changed_manifests: vec![manifest_version.to_string()],
        changed_profiles: vec![args.profile.clone()],
        changed_config_refs,
        changed_image_artifacts,
        compatible_with_current_binary: true,
    })
}

fn merge_graph_profile_release(
    args: &ReleaseArgs,
    manifest_path: &Path,
    candidate_manifest: &Path,
    manifest_version: &str,
    profile_version: &str,
    status: release_graph::Status,
) -> Result<ProfileReleaseReport> {
    let mut base: serde_json::Value = serde_json::from_slice(
        &fs::read(manifest_path)
            .with_context(|| format!("read release manifest {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parse release manifest {}", manifest_path.display()))?;
    let candidate: serde_json::Value =
        serde_json::from_slice(&fs::read(candidate_manifest).with_context(|| {
            format!("read candidate manifest {}", candidate_manifest.display())
        })?)
        .with_context(|| format!("parse candidate manifest {}", candidate_manifest.display()))?;
    validate_assets_channel_graph_manifest(&base, &args.channel)?;
    validate_assets_channel_graph_manifest(&candidate, &args.channel)?;
    let mut profile = candidate
        .get("profiles")
        .and_then(|value| value.get(&args.profile))
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "candidate manifest {} does not list profile {}",
                candidate_manifest.display(),
                args.profile
            )
        })?;
    if let Some(publication_base) = args.publication_base.as_deref() {
        rewrite_profile_publication_urls(&mut profile, publication_base)?;
    }
    let revision = profile
        .get("revision")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("candidate profile {} has no revision", args.profile))?;
    if revision != profile_version {
        return Err(anyhow!(
            "profile {} has revision {}, expected {}",
            args.profile,
            revision,
            profile_version
        ));
    }
    if let Some(existing) = profile.get("source_commit") {
        let existing = existing
            .as_str()
            .ok_or_else(|| anyhow!("candidate profile source_commit must be a string"))?
            .parse::<SourceCommit>()?;
        if existing.as_str() != args.source_commit.as_str() {
            return Err(anyhow!(
                "candidate profile {} was built from {}, not selected source {}",
                args.profile,
                existing,
                args.source_commit
            ));
        }
    }
    let compatible = graph_profile_matches_current_binary(&profile, &base)?;
    profile["source_commit"] = serde_json::to_value(&args.source_commit)?;
    let status_value = serde_json::to_value(status)?;
    profile["status"] = status_value.clone();
    let mut changed_config_refs = 0;
    let mut changed_image_artifacts = 0;
    if let Some(architectures) = profile
        .get_mut("architectures")
        .and_then(serde_json::Value::as_array_mut)
    {
        for architecture in architectures {
            for (field, changed) in [
                ("config", &mut changed_config_refs),
                ("images", &mut changed_image_artifacts),
            ] {
                if let Some(rows) = architecture
                    .get_mut(field)
                    .and_then(serde_json::Value::as_array_mut)
                {
                    for row in rows {
                        if row.get("status") != Some(&status_value) {
                            *changed += 1;
                        }
                        row["status"] = status_value.clone();
                    }
                }
            }
        }
    }
    base["version"] = serde_json::Value::String(manifest_version.to_string());
    base["profiles"]
        .as_object_mut()
        .ok_or_else(|| anyhow!("base manifest profiles must be an object"))?
        .insert(args.profile.clone(), profile);
    validate_assets_channel_graph_manifest(&base, &args.channel)?;
    let mut bytes =
        serde_json::to_vec_pretty(&base).context("serialize merged profile manifest")?;
    bytes.push(b'\n');
    fs::write(manifest_path, bytes)
        .with_context(|| format!("write release manifest {}", manifest_path.display()))?;
    Ok(ProfileReleaseReport {
        schema: "capsem.admin.profile_release.v1",
        ok: true,
        action: "release",
        channel: args.channel.clone(),
        manifest: manifest_path.display().to_string(),
        manifest_version: manifest_version.to_string(),
        profile: args.profile.clone(),
        profile_version: profile_version.to_string(),
        publication_identity: profile_publication_identity(
            &args.channel,
            &args.profile,
            profile_version,
        )?,
        status,
        changed_channels: vec![args.channel.clone()],
        changed_manifests: vec![manifest_version.to_string()],
        changed_profiles: vec![args.profile.clone()],
        changed_config_refs,
        changed_image_artifacts,
        compatible_with_current_binary: compatible,
    })
}

fn graph_profile_matches_current_binary(
    profile: &serde_json::Value,
    manifest: &serde_json::Value,
) -> Result<bool> {
    let minimum = profile
        .get("min_capsem_version")
        .and_then(serde_json::Value::as_str)
        .map(|minimum| {
            semver::Version::parse(minimum)
                .with_context(|| format!("profile minimum Capsem version is invalid: {minimum}"))
        })
        .transpose()?;
    let maximum = profile
        .get("max_capsem_version")
        .and_then(serde_json::Value::as_str)
        .map(|maximum| {
            semver::Version::parse(maximum)
                .with_context(|| format!("profile maximum Capsem version is invalid: {maximum}"))
        })
        .transpose()?;
    if let (Some(minimum), Some(maximum)) = (&minimum, &maximum) {
        if minimum > maximum {
            return Err(anyhow!(
                "profile minimum Capsem version {minimum} exceeds maximum {maximum}"
            ));
        }
    }
    let versions = manifest
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("base manifest packages must be an array"))?
        .iter()
        .filter(|package| {
            package.get("status").and_then(serde_json::Value::as_str) == Some("current")
        })
        .map(|package| {
            let version = package
                .get("version")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow!("current package has no version"))?;
            semver::Version::parse(version)
                .with_context(|| format!("current package version is invalid: {version}"))
        })
        .collect::<Result<Vec<_>>>()?;
    if versions.is_empty() {
        return Ok(false);
    }
    Ok(versions.iter().all(|version| {
        minimum.as_ref().is_none_or(|minimum| version >= minimum)
            && maximum.as_ref().is_none_or(|maximum| version <= maximum)
    }))
}

fn validate_graph_profiles_match_current_binary(manifest: &serde_json::Value) -> Result<()> {
    let profiles = manifest
        .get("profiles")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow!("graph manifest profiles must be an object"))?;
    for (profile_id, profile) in profiles {
        if profile.get("status").and_then(serde_json::Value::as_str) == Some("revoked") {
            continue;
        }
        if !graph_profile_matches_current_binary(profile, manifest)? {
            let minimum = profile
                .get("min_capsem_version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unbounded");
            let maximum = profile
                .get("max_capsem_version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unbounded");
            return Err(anyhow!(
                "profile {profile_id} is incompatible with current packages \
                 (minimum Capsem {minimum}, maximum Capsem {maximum})"
            ));
        }
    }
    Ok(())
}

fn rewrite_profile_publication_urls(
    profile: &mut serde_json::Value,
    publication_base: &str,
) -> Result<()> {
    let parsed = reqwest::Url::parse(publication_base)
        .with_context(|| format!("profile publication base is not a URL: {publication_base}"))?;
    if parsed.scheme() != "https" {
        return Err(anyhow!("profile publication base must use HTTPS"));
    }
    let architectures = profile
        .get_mut("architectures")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| anyhow!("candidate profile architectures must be an array"))?;
    for architecture in architectures {
        let arch = architecture
            .get("architecture")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("candidate profile architecture has no name"))?
            .to_string();
        for field in ["config", "images", "evidence"] {
            let rows = architecture
                .get_mut(field)
                .and_then(serde_json::Value::as_array_mut)
                .ok_or_else(|| anyhow!("candidate profile architecture has no {field} array"))?;
            for row in rows {
                let file_name = row
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        row.get("url")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|url| url.rsplit('/').next())
                    })
                    .or_else(|| {
                        row.get("path")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|path| Path::new(path).file_name())
                            .and_then(|name| name.to_str())
                    })
                    .ok_or_else(|| {
                        anyhow!("candidate profile {field} row has no publication file name")
                    })?;
                if file_name.is_empty()
                    || !file_name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
                {
                    return Err(anyhow!(
                        "candidate profile {field} file name is unsafe: {file_name}"
                    ));
                }
                let publication_name = if file_name.starts_with(&format!("{arch}-")) {
                    file_name.to_string()
                } else {
                    format!("{arch}-{file_name}")
                };
                row["url"] = serde_json::Value::String(format!(
                    "{}/{}",
                    publication_base.trim_end_matches('/'),
                    publication_name
                ));
            }
        }
        let software_inventory_urls = architecture
            .get("evidence")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow!("candidate profile architecture has no evidence array"))?
            .iter()
            .filter(|row| {
                row.get("kind").and_then(serde_json::Value::as_str) == Some("software_inventory")
            })
            .map(|row| {
                row.get("url")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| {
                        anyhow!(
                            "candidate profile software_inventory evidence has no publication URL"
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let software = architecture
            .get_mut("software")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| anyhow!("candidate profile architecture has no software array"))?;
        if !software.is_empty() {
            let [software_inventory_url] = software_inventory_urls.as_slice() else {
                return Err(anyhow!(
                    "candidate profile architecture must have exactly one software_inventory \
                     evidence URL for its software rows"
                ));
            };
            for row in software {
                if !row.is_object()
                    || row
                        .get("evidence")
                        .and_then(serde_json::Value::as_str)
                        .is_none()
                {
                    return Err(anyhow!(
                        "candidate profile software row has no evidence URL"
                    ));
                }
                row["evidence"] = serde_json::Value::String(software_inventory_url.to_string());
            }
        }
    }
    Ok(())
}

fn check_config_root(config_root: &Path, arch: Option<&str>) -> Result<ConfigRootCheckReport> {
    let settings = validate_settings(&config_root.join("settings/settings.toml"))?;
    let corp_rules = validate_corp_config(&config_root.join("corp/corp.toml"), config_root)?;
    let catalog =
        ProfileCatalog::load_from_dir(&config_root.join("profiles")).map_err(|error| {
            anyhow!(
                "load profile directory {}: {error}",
                config_root.join("profiles").display()
            )
        })?;
    let mut profiles = Vec::new();
    for profile in catalog.profiles() {
        profiles.push(check_profile(&ProfileCheckArgs {
            path: config_root
                .join("profiles")
                .join(&profile.id)
                .join("profile.toml"),
            config_root: Some(config_root.to_path_buf()),
            arch: arch.map(ToOwned::to_owned),
            json: true,
        })?);
    }
    Ok(ConfigRootCheckReport {
        schema: "capsem.admin.config_root_check.v1",
        ok: true,
        config_root: config_root.display().to_string(),
        settings,
        corp_rules,
        profiles,
    })
}

fn validate_corp_config(path: &Path, config_root: &Path) -> Result<usize> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read corp {}", path.display()))?;
    let file: SettingsFile =
        toml::from_str(&content).with_context(|| format!("parse corp {}", path.display()))?;
    file.validate_metadata_contract()
        .map_err(|error| anyhow!("validate corp {}: {error}", path.display()))?;
    validate_corp_toml_contract(&file)
        .map_err(|error| anyhow!("validate corp ownership {}: {error}", path.display()))?;

    let inline_profile = SecurityRuleProfile {
        default: file.default.clone(),
        corp: file.corp.clone(),
        profiles: file.profiles.clone(),
        ai: file.ai.clone(),
        plugins: file.plugins.clone(),
    };
    let mut compiled = inline_profile
        .compile(SecurityRuleSource::Corp)
        .map_err(|error| anyhow!("compile corp inline rules {}: {error}", path.display()))?
        .len();
    if let Some(enforcement) = file.corp_rule_files.enforcement.as_deref() {
        compiled += compile_rule_file(
            "enforcement",
            &config_root.join(enforcement),
            RuleFileSourceArg::Corp,
        )?
        .compiled_rules;
    }
    if let Some(sigma) = file.corp_rule_files.sigma.as_deref() {
        compiled += compile_rule_file(
            "detection",
            &config_root.join(sigma),
            RuleFileSourceArg::Corp,
        )?
        .compiled_rules;
    }
    Ok(compiled)
}

fn validate_settings_command(args: SettingsValidateArgs) -> Result<()> {
    let report = validate_settings(&args.path)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("valid: settings {}", args.path.display());
    }
    Ok(())
}

fn validate_rule_file_command(kind: &'static str, args: RuleFileArgs) -> Result<()> {
    let report = compile_rule_file(kind, &args.path, args.source)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "valid: {kind} {} ({} compiled rules)",
            args.path.display(),
            report.compiled_rules
        );
    }
    Ok(())
}

fn manifest_check_command(args: ManifestCheckArgs) -> Result<()> {
    let manifest = load_manifest(&args.path)?;
    let report = manifest_report(&args.path, &manifest, None, None)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "valid: manifest {} ({} asset releases)",
            args.path.display(),
            report.releases
        );
    }
    Ok(())
}

fn manifest_generate_command(args: ManifestGenerateArgs) -> Result<()> {
    let command = manifest_generate_command_report(&args);
    run_command(&command)?;
    let manifest_path = args.assets_dir.join("manifest.json");
    if args.json {
        let manifest = load_manifest(&manifest_path)?;
        let report = manifest_report(&manifest_path, &manifest, None, None)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "generated manifest {}",
            args.assets_dir.join("manifest.json").display()
        );
    }
    Ok(())
}

fn corporate_manifest_command(args: ManifestCorporateArgs) -> Result<()> {
    let report = author_corporate_manifest(&args)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "authored corporate manifest {}/{} at {} using Capsem {}",
            report.corporation,
            report.channel,
            report.output_manifest,
            report.resolved_binary_version
        );
    }
    Ok(())
}

fn author_corporate_manifest(args: &ManifestCorporateArgs) -> Result<CorporateManifestReport> {
    validate_corporate_namespace(&args.corporation, &args.channel)?;
    validate_corporate_profile_base(&args.profile_base)?;

    let official_bytes = fs::read(&args.official_manifest).with_context(|| {
        format!(
            "read official Capsem manifest {}",
            args.official_manifest.display()
        )
    })?;
    let official: serde_json::Value =
        serde_json::from_slice(&official_bytes).with_context(|| {
            format!(
                "parse official Capsem manifest {}",
                args.official_manifest.display()
            )
        })?;

    let profile_bytes = fs::read(&args.profile_manifest).with_context(|| {
        format!(
            "read corporate profile manifest {}",
            args.profile_manifest.display()
        )
    })?;
    let mut profile_source: serde_json::Value = serde_json::from_slice(&profile_bytes)
        .with_context(|| {
            format!(
                "parse corporate profile manifest {}",
                args.profile_manifest.display()
            )
        })?;
    let (resolved_version, packages) = select_official_packages(&official, &args.binary)?;
    let referenced_packages = profile_source
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("corporate profile manifest packages must be an array"))?;
    if !referenced_packages.is_empty() && referenced_packages != &packages {
        return Err(anyhow!(
            "corporate profile manifest may reference only the selected official packages"
        ));
    }
    let profiles = profile_source
        .get_mut("profiles")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| anyhow!("corporate profile manifest profiles must be an object"))?;
    if profiles.is_empty() {
        return Err(anyhow!(
            "corporate profile manifest must contain at least one profile"
        ));
    }
    for (profile_id, profile) in profiles.iter_mut() {
        profile["source_commit"] = serde_json::to_value(&args.source_commit)?;
        validate_corporate_profile_document(
            profile_id,
            profile,
            &args.profile_base,
            &resolved_version,
        )?;
    }

    let manifest = serde_json::json!({
        "version": args.manifest_version,
        "channel": args.channel,
        "status": "current",
        "packages": packages,
        "profiles": profiles,
    });
    validate_assets_channel_graph_manifest(&manifest, &args.channel)?;
    let output_dir = corporate_manifest_output_dir(args)?;
    let output_path = output_dir.join("manifest.json");
    let official_canonical = fs::canonicalize(&args.official_manifest).with_context(|| {
        format!(
            "resolve official Capsem manifest {}",
            args.official_manifest.display()
        )
    })?;
    let profile_canonical = fs::canonicalize(&args.profile_manifest).with_context(|| {
        format!(
            "resolve corporate profile manifest {}",
            args.profile_manifest.display()
        )
    })?;
    if output_path == official_canonical || output_path == profile_canonical {
        return Err(anyhow!(
            "corporate output must not overwrite an authoring input"
        ));
    }

    let mut encoded = serde_json::to_vec_pretty(&manifest)?;
    encoded.push(b'\n');
    let temporary = output_dir.join(format!(".manifest.json.tmp-{}", std::process::id()));
    fs::write(&temporary, &encoded).with_context(|| {
        format!(
            "write corporate manifest staging file {}",
            temporary.display()
        )
    })?;
    fs::rename(&temporary, &output_path)
        .with_context(|| format!("publish corporate manifest {}", output_path.display()))?;

    Ok(CorporateManifestReport {
        schema: "capsem.admin.corporate_manifest.v1",
        ok: true,
        corporation: args.corporation.clone(),
        channel: args.channel.clone(),
        binary_policy: args.binary.clone(),
        resolved_binary_version: resolved_version.to_string(),
        official_manifest: args.official_manifest.display().to_string(),
        profile_manifest: args.profile_manifest.display().to_string(),
        output_manifest: output_path.display().to_string(),
        profiles: profiles.keys().cloned().collect(),
        packages: packages.len(),
    })
}

fn validate_corporate_namespace(corporation: &str, channel: &str) -> Result<()> {
    validate_channel_name(corporation)
        .with_context(|| format!("invalid corporation namespace {corporation:?}"))?;
    validate_channel_name(channel)
        .with_context(|| format!("invalid corporate channel {channel:?}"))?;
    if corporation == "capsem" || matches!(channel, "stable" | "nightly") {
        return Err(anyhow!(
            "corporate authoring cannot target a first-party namespace"
        ));
    }
    Ok(())
}

fn validate_corporate_profile_base(profile_base: &str) -> Result<()> {
    if !profile_base.starts_with("https://") || !profile_base.ends_with('/') {
        return Err(anyhow!(
            "corporate profile base must be an HTTPS directory URL ending in '/'"
        ));
    }
    Ok(())
}

fn validate_corporate_profile_document(
    profile_id: &str,
    profile: &serde_json::Value,
    profile_base: &str,
    selected_version: &semver::Version,
) -> Result<()> {
    let embedded_id = require_json_string(profile, &["id"])?;
    if embedded_id != profile_id {
        return Err(anyhow!(
            "corporate profile key {profile_id} does not match profile id {embedded_id}"
        ));
    }
    require_json_string(profile, &["revision"])?;
    let architectures = profile
        .get("architectures")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("corporate profile {profile_id} architectures must be an array"))?;
    if architectures.is_empty() {
        return Err(anyhow!(
            "corporate profile {profile_id} must list architectures"
        ));
    }
    if let Some(minimum) = profile
        .get("min_capsem_version")
        .and_then(serde_json::Value::as_str)
    {
        let minimum = semver::Version::parse(minimum).with_context(|| {
            format!("corporate profile {profile_id} minimum Capsem version is invalid: {minimum}")
        })?;
        if selected_version < &minimum {
            return Err(anyhow!(
                "corporate profile {profile_id} requires Capsem {minimum} or newer, selected {selected_version}"
            ));
        }
    }
    if let Some(maximum) = profile
        .get("max_capsem_version")
        .and_then(serde_json::Value::as_str)
    {
        let maximum = semver::Version::parse(maximum).with_context(|| {
            format!("corporate profile {profile_id} maximum Capsem version is invalid: {maximum}")
        })?;
        if selected_version > &maximum {
            return Err(anyhow!(
                "corporate profile {profile_id} supports at most Capsem {maximum}, selected {selected_version}"
            ));
        }
    }
    validate_corporate_reference_tree(profile_id, profile, None, profile_base)?;
    Ok(())
}

fn validate_corporate_reference_tree(
    profile_id: &str,
    value: &serde_json::Value,
    key: Option<&str>,
    profile_base: &str,
) -> Result<()> {
    if matches!(key, Some("url" | "evidence")) {
        if let Some(reference) = value.as_str() {
            if !reference.starts_with(profile_base) {
                return Err(anyhow!(
                    "corporate profile {profile_id} reference is outside the owned profile base: {reference}"
                ));
            }
            return Ok(());
        }
    }
    match value {
        serde_json::Value::Array(rows) => {
            for row in rows {
                validate_corporate_reference_tree(profile_id, row, key, profile_base)?;
            }
        }
        serde_json::Value::Object(fields) => {
            for (field, child) in fields {
                validate_corporate_reference_tree(profile_id, child, Some(field), profile_base)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn select_official_packages(
    manifest: &serde_json::Value,
    policy: &str,
) -> Result<(semver::Version, Vec<serde_json::Value>)> {
    let package_rows = manifest
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("official manifest packages must be an array"))?;
    let mut selectable_versions = BTreeMap::<semver::Version, String>::new();
    for package in package_rows {
        let status = require_json_string(package, &["status"])?;
        if status == "revoked" {
            continue;
        }
        if !matches!(status.as_str(), "current" | "supported" | "deprecated") {
            return Err(anyhow!("official package has invalid status {status:?}"));
        }
        let package_name = require_json_string(package, &["name"])?;
        let package_version = require_json_string(package, &["version"])?;
        let parsed = semver::Version::parse(&package_version).with_context(|| {
            format!(
                "official package {} has invalid Capsem version {}",
                package_name, package_version
            )
        })?;
        selectable_versions.entry(parsed).or_insert(package_version);
    }
    let resolved = if policy == "latest" {
        selectable_versions
            .last_key_value()
            .map(|(version, _)| version.clone())
            .ok_or_else(|| anyhow!("official manifest has no selectable Capsem packages"))?
    } else {
        let pinned = semver::Version::parse(policy)
            .with_context(|| format!("invalid corporate Capsem binary pin {policy:?}"))?;
        if !selectable_versions.contains_key(&pinned) {
            return Err(anyhow!(
                "official manifest does not publish Capsem {policy}"
            ));
        }
        pinned
    };
    let packages = package_rows
        .iter()
        .filter(|package| {
            package.get("status").and_then(serde_json::Value::as_str) != Some("revoked")
                && package
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|version| semver::Version::parse(version).ok())
                    .is_some_and(|version| version == resolved)
        })
        .cloned()
        .collect::<Vec<_>>();
    if packages.is_empty() {
        return Err(anyhow!(
            "official manifest does not publish Capsem {resolved}"
        ));
    }
    Ok((resolved, packages))
}

fn corporate_manifest_output_dir(args: &ManifestCorporateArgs) -> Result<PathBuf> {
    fs::create_dir_all(&args.output_root).with_context(|| {
        format!(
            "create corporate manifest output root {}",
            args.output_root.display()
        )
    })?;
    let output_root = fs::canonicalize(&args.output_root).with_context(|| {
        format!(
            "resolve corporate manifest output root {}",
            args.output_root.display()
        )
    })?;
    let output_dir = output_root.join(&args.corporation).join(&args.channel);
    fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "create corporate manifest destination {}",
            output_dir.display()
        )
    })?;
    let output_dir = fs::canonicalize(&output_dir).with_context(|| {
        format!(
            "resolve corporate manifest destination {}",
            output_dir.display()
        )
    })?;
    if !output_dir.starts_with(&output_root) {
        return Err(anyhow!(
            "corporate manifest destination escapes its owned output root"
        ));
    }
    Ok(output_dir)
}

fn assets_channel_build_command(args: AssetsChannelBuildArgs) -> Result<()> {
    let generated_at = args.generated_at.unwrap_or(current_utc_rfc3339()?);
    let report = build_assets_channel_with_policy(
        &args.manifest,
        &args.assets_dir,
        &args.profiles_dir,
        &args.channel,
        &args.manifest_version,
        &args.out_dir,
        &generated_at,
        args.asset_source_base.as_deref(),
        args.profile_revision_policy,
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "generated assets channel {} at {}",
            report.channel, report.out_dir
        );
    }
    Ok(())
}

fn assets_channel_check_command(args: AssetsChannelCheckArgs) -> Result<()> {
    let report = check_assets_channel(&args.dist, &args.channel)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "valid: assets channel {} ({})",
            report.channel,
            args.dist.display()
        );
    }
    Ok(())
}

fn assets_channel_record_binary_command(args: AssetsChannelRecordBinaryArgs) -> Result<()> {
    let date = args.date.unwrap_or(current_utc_date()?);
    let report = record_binary_release_metadata(
        &args.manifest_path,
        &args.version,
        &args.source_commit,
        args.min_assets.as_deref(),
        &args.artifacts,
        &date,
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("recorded binary {} in {}", report.version, report.manifest);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn build_assets_channel(
    manifest_url: &str,
    assets_dir: &Path,
    profiles_dir: &Path,
    channel: &str,
    manifest_version: &str,
    out_dir: &Path,
    generated_at: &str,
    asset_source_base: Option<&str>,
) -> Result<AssetsChannelBuildReport> {
    build_assets_channel_with_policy(
        manifest_url,
        assets_dir,
        profiles_dir,
        channel,
        manifest_version,
        out_dir,
        generated_at,
        asset_source_base,
        ProfileRevisionPolicyArg::Strict,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_assets_channel_with_policy(
    manifest_url: &str,
    assets_dir: &Path,
    profiles_dir: &Path,
    channel: &str,
    manifest_version: &str,
    out_dir: &Path,
    generated_at: &str,
    asset_source_base: Option<&str>,
    profile_revision_policy: ProfileRevisionPolicyArg,
) -> Result<AssetsChannelBuildReport> {
    validate_channel_name(channel)?;
    let manifest_bytes = read_manifest_url(manifest_url)?;
    let manifest_content = std::str::from_utf8(&manifest_bytes)
        .with_context(|| format!("manifest URL did not return UTF-8 JSON: {manifest_url}"))?;
    let manifest_value: serde_json::Value = serde_json::from_str(manifest_content)
        .with_context(|| format!("parse manifest from {manifest_url}"))?;
    if is_release_graph_manifest_value(&manifest_value) {
        return build_assets_channel_from_graph(
            manifest_value,
            channel,
            manifest_version,
            out_dir,
            generated_at,
        );
    }
    let manifest = ManifestV2::from_json(manifest_content)
        .with_context(|| format!("parse manifest from {manifest_url}"))?;
    let asset_base_override = asset_source_base;
    let asset_base = asset_base_override
        .or(manifest.asset_base.as_deref())
        .unwrap_or("/assets/releases");
    let mut channel_manifest_doc = manifest.clone();
    channel_manifest_doc.asset_base = if asset_base == "/assets/releases" {
        None
    } else {
        Some(asset_base.to_string())
    };
    let channel_dir = out_dir.join("assets").join(channel);
    let copy_vm_blobs = asset_base == "/assets/releases";
    let current_asset_version = channel_manifest_doc.assets.current.clone();
    let release_dir = out_dir
        .join("assets")
        .join("releases")
        .join(&current_asset_version);
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    if channel_dir.exists() {
        fs::remove_dir_all(&channel_dir)
            .with_context(|| format!("remove {}", channel_dir.display()))?;
    }
    let graph_channel_dir = out_dir.join("manifests").join(channel);
    if graph_channel_dir.exists() {
        fs::remove_dir_all(&graph_channel_dir)
            .with_context(|| format!("remove {}", graph_channel_dir.display()))?;
    }
    if copy_vm_blobs && release_dir.exists() {
        fs::remove_dir_all(&release_dir)
            .with_context(|| format!("remove {}", release_dir.display()))?;
    }
    fs::create_dir_all(&channel_dir)
        .with_context(|| format!("create {}", channel_dir.display()))?;
    if copy_vm_blobs {
        fs::create_dir_all(&release_dir)
            .with_context(|| format!("create {}", release_dir.display()))?;
    }
    let mut asset_digest_cache = AssetDigestCache::new();
    let copied_assets = if copy_vm_blobs {
        let current_release = channel_manifest_doc
            .assets
            .releases
            .get_mut(&current_asset_version)
            .ok_or_else(|| anyhow!("manifest current asset release is missing"))?;
        copy_assets_channel_release_assets(
            assets_dir,
            &release_dir,
            current_release,
            &mut asset_digest_cache,
        )?
    } else {
        hydrate_current_asset_entry_sha256(
            &mut channel_manifest_doc,
            assets_dir,
            &mut asset_digest_cache,
        )?;
        0
    };
    let publishable_profiles = publishable_profiles(
        &channel_manifest_doc,
        profiles_dir,
        channel,
        asset_base,
        assets_dir,
        &mut asset_digest_cache,
        profile_revision_policy,
    )?;
    copy_profile_release_files(out_dir, &publishable_profiles.file_copies)?;
    validate_graph_manifest_version(manifest_version)?;
    let graph_manifest_version = manifest_version.to_string();
    let graph_manifest_url = format!("/assets/{channel}/manifest.json");
    let graph_manifest = render_graph_release_manifest(
        &channel_manifest_doc,
        channel,
        &publishable_profiles.profiles,
        asset_base,
        &graph_manifest_version,
    )?;
    let channel_manifest = channel_dir.join("manifest.json");
    fs::write(&channel_manifest, &graph_manifest)
        .with_context(|| format!("write {}", channel_manifest.display()))?;
    let graph_manifest_sha256 = format!("{:x}", Sha256::digest(graph_manifest.as_bytes()));
    let graph_manifest_blake3 = blake3::hash(graph_manifest.as_bytes()).to_hex().to_string();
    let index = assets_channel_index(
        &channel_manifest_doc,
        channel,
        generated_at,
        &graph_manifest_blake3,
        publishable_profiles.summary.clone(),
        asset_base,
    );
    fs::write(
        out_dir.join("channels.json"),
        render_assets_channels_catalog(
            &out_dir.join("channels.json"),
            &index,
            &graph_manifest_version,
            &graph_manifest_url,
            &graph_manifest_sha256,
            &graph_manifest_blake3,
        )?,
    )
    .with_context(|| format!("write {}", out_dir.join("channels.json").display()))?;
    let health_json = render_assets_channel_health(&index)?;
    fs::write(out_dir.join("health.json"), &health_json)
        .with_context(|| format!("write {}", out_dir.join("health.json").display()))?;
    fs::write(channel_dir.join("health.json"), &health_json)
        .with_context(|| format!("write {}", channel_dir.join("health.json").display()))?;
    fs::write(
        out_dir.join("_headers"),
        render_assets_channel_headers_for_dist(out_dir, channel)?,
    )
    .with_context(|| format!("write {}", out_dir.join("_headers").display()))?;
    fs::write(out_dir.join("robots.txt"), "User-agent: *\nDisallow:\n")
        .with_context(|| format!("write {}", out_dir.join("robots.txt").display()))?;
    Ok(AssetsChannelBuildReport {
        schema: "capsem.admin.assets_channel_build.v1",
        channel: channel.to_string(),
        generated_at: generated_at.to_string(),
        out_dir: out_dir.display().to_string(),
        human_site_source: "release-site",
        channels_json: out_dir.join("channels.json").display().to_string(),
        manifest: channel_manifest.display().to_string(),
        health_json: out_dir.join("health.json").display().to_string(),
        copied_assets,
    })
}

fn record_binary_release_metadata(
    manifest_path: &Path,
    version: &str,
    source_commit: &SourceCommit,
    min_assets: Option<&str>,
    artifacts: &[PathBuf],
    date: &str,
) -> Result<AssetsChannelRecordBinaryReport> {
    if artifacts.is_empty() {
        return Err(anyhow!("at least one binary release artifact is required"));
    }
    validate_binary_version(version)?;
    validate_release_date(date)?;
    // Validate the candidate bytes independently of the manifest shape. A
    // legacy graph is no longer writable because it cannot carry package
    // provenance, but that must not turn a malformed package into a plausible
    // provenance-only failure.
    let files = binary_files_from_artifacts(artifacts)?;
    validate_binary_release_files(version, &files)?;
    let manifest_content = fs::read_to_string(manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest_value: serde_json::Value = serde_json::from_str(&manifest_content)
        .with_context(|| format!("parse manifest {}", manifest_path.display()))?;
    if is_release_graph_manifest_value(&manifest_value) {
        return record_graph_binary_release_metadata(
            manifest_path,
            manifest_value,
            version,
            source_commit,
            min_assets,
            &files,
        );
    }
    Err(anyhow!(
        "record-binary requires a release graph manifest so every package row records source_commit"
    ))
}

fn build_assets_channel_from_graph(
    mut graph_manifest: serde_json::Value,
    channel: &str,
    manifest_version: &str,
    out_dir: &Path,
    generated_at: &str,
) -> Result<AssetsChannelBuildReport> {
    validate_assets_channel_graph_manifest(&graph_manifest, channel)?;
    validate_graph_profiles_match_current_binary(&graph_manifest)?;
    graph_manifest["version"] = serde_json::Value::String(manifest_version.to_string());
    graph_manifest["channel"] = serde_json::Value::String(channel.to_string());
    graph_manifest["status"] = serde_json::Value::String("current".to_string());
    let channel_dir = out_dir.join("assets").join(channel);
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    if channel_dir.exists() {
        fs::remove_dir_all(&channel_dir)
            .with_context(|| format!("remove {}", channel_dir.display()))?;
    }
    fs::create_dir_all(&channel_dir)
        .with_context(|| format!("create {}", channel_dir.display()))?;
    let graph_manifest = format!(
        "{}\n",
        serde_json::to_string_pretty(&graph_manifest).context("serialize graph manifest")?
    );
    let channel_manifest = channel_dir.join("manifest.json");
    fs::write(&channel_manifest, &graph_manifest)
        .with_context(|| format!("write {}", channel_manifest.display()))?;
    let graph_manifest_sha256 = format!("{:x}", Sha256::digest(graph_manifest.as_bytes()));
    let graph_manifest_blake3 = blake3::hash(graph_manifest.as_bytes()).to_hex().to_string();
    let graph_value: serde_json::Value =
        serde_json::from_str(&graph_manifest).context("parse rendered graph manifest")?;
    let index = assets_channel_index_from_graph(
        &graph_value,
        channel,
        generated_at,
        &graph_manifest_blake3,
    )?;
    let graph_manifest_url = format!("/assets/{channel}/manifest.json");
    fs::write(
        out_dir.join("channels.json"),
        render_assets_channels_catalog(
            &out_dir.join("channels.json"),
            &index,
            manifest_version,
            &graph_manifest_url,
            &graph_manifest_sha256,
            &graph_manifest_blake3,
        )?,
    )
    .with_context(|| format!("write {}", out_dir.join("channels.json").display()))?;
    let health_json = render_assets_channel_health(&index)?;
    fs::write(out_dir.join("health.json"), &health_json)
        .with_context(|| format!("write {}", out_dir.join("health.json").display()))?;
    fs::write(channel_dir.join("health.json"), &health_json)
        .with_context(|| format!("write {}", channel_dir.join("health.json").display()))?;
    fs::write(
        out_dir.join("_headers"),
        render_assets_channel_headers_for_dist(out_dir, channel)?,
    )
    .with_context(|| format!("write {}", out_dir.join("_headers").display()))?;
    fs::write(out_dir.join("robots.txt"), "User-agent: *\nDisallow:\n")
        .with_context(|| format!("write {}", out_dir.join("robots.txt").display()))?;
    Ok(AssetsChannelBuildReport {
        schema: "capsem.admin.assets_channel_build.v1",
        channel: channel.to_string(),
        generated_at: generated_at.to_string(),
        out_dir: out_dir.display().to_string(),
        human_site_source: "release-site",
        channels_json: out_dir.join("channels.json").display().to_string(),
        manifest: channel_manifest.display().to_string(),
        health_json: out_dir.join("health.json").display().to_string(),
        copied_assets: 0,
    })
}

fn record_graph_binary_release_metadata(
    manifest_path: &Path,
    mut manifest: serde_json::Value,
    version: &str,
    source_commit: &SourceCommit,
    min_assets: Option<&str>,
    files: &[BinaryFile],
) -> Result<AssetsChannelRecordBinaryReport> {
    let profiles = manifest
        .get("profiles")
        .and_then(|value| value.as_object())
        .ok_or_else(|| anyhow!("graph manifest profiles must be an object"))?;
    if profiles.is_empty() {
        return Err(anyhow!("graph manifest profiles must not be empty"));
    }
    let min_assets = min_assets
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| graph_profile_revision_summary(profiles));
    let packages = graph_packages_from_binary_files(version, source_commit, files)?;
    manifest["packages"] = serde_json::Value::Array(packages);
    validate_graph_profiles_match_current_binary(&manifest)?;
    let mut bytes = serde_json::to_vec_pretty(&manifest).context("serialize updated manifest")?;
    bytes.push(b'\n');
    fs::write(manifest_path, &bytes)
        .with_context(|| format!("write {}", manifest_path.display()))?;
    Ok(AssetsChannelRecordBinaryReport {
        schema: "capsem.admin.assets_channel_record_binary.v1",
        manifest: manifest_path.display().to_string(),
        version: version.to_string(),
        min_assets,
        files: files.to_vec(),
    })
}

fn validate_binary_release_files(version: &str, files: &[BinaryFile]) -> Result<()> {
    if !files.iter().any(|file| is_host_sbom_file(&file.name)) {
        return Err(anyhow!(
            "binary release metadata must include capsem-sbom.spdx.json"
        ));
    }
    if !files.iter().any(|file| !is_host_sbom_file(&file.name)) {
        return Err(anyhow!(
            "binary release metadata must include a host package artifact"
        ));
    }
    if !files.iter().any(|file| is_host_package_file(&file.name)) {
        return Err(anyhow!(
            "binary release metadata must include a .pkg or .deb artifact"
        ));
    }
    if let Some(file) = files.iter().find(|file| {
        is_host_package_file(&file.name) && !host_package_name_matches_version(&file.name, version)
    }) {
        return Err(anyhow!(
            "binary release package artifact name must match version {version}: {}",
            file.name
        ));
    }
    Ok(())
}

fn graph_packages_from_binary_files(
    version: &str,
    source_commit: &SourceCommit,
    files: &[BinaryFile],
) -> Result<Vec<serde_json::Value>> {
    let host_sbom = files
        .iter()
        .find(|file| is_host_sbom_file(&file.name))
        .ok_or_else(|| anyhow!("binary release metadata must include capsem-sbom.spdx.json"))?;
    let mut packages = files
        .iter()
        .filter(|file| is_host_package_file(&file.name))
        .map(|file| graph_package_from_binary_file(version, source_commit, file, host_sbom))
        .collect::<Result<Vec<_>>>()?;
    packages.sort_by(|left, right| {
        let left_name = left
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let right_name = right
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        left_name.cmp(right_name)
    });
    Ok(packages)
}

fn graph_package_from_binary_file(
    version: &str,
    source_commit: &SourceCommit,
    file: &BinaryFile,
    host_sbom: &BinaryFile,
) -> Result<serde_json::Value> {
    let package_kind = package_kind_for_name(&file.name);
    let platform = package_platform_for_kind(package_kind);
    let architecture = release_graph::PackageArchitecture::from_package_name(&file.name)?;
    let package_id = release_graph_id(&file.name);
    let package_url = capsem_core::asset_manager::release_url(version);
    let package_url = format!("{}/{}", package_url.trim_end_matches('/'), file.name);
    let host_sbom_url = capsem_core::asset_manager::release_url(version);
    let host_sbom_url = format!("{}/{}", host_sbom_url.trim_end_matches('/'), host_sbom.name);
    let binaries = file
        .binaries
        .iter()
        .map(|binary| {
            serde_json::json!({
                "name": binary.name,
                "description": binary.description,
                "version": version,
                "installed_path": binary.installed_path,
                "platform": platform,
                "architecture": architecture,
                "bytes": binary.size,
                "digest": {
                    "sha256": binary.sha256,
                    "blake3": binary.blake3,
                },
                "status": "current",
                "sbom_component_ref": binary.sbom_component_ref,
            })
        })
        .collect::<Vec<_>>();
    if binaries.is_empty() {
        return Err(anyhow!(
            "binary release package artifact must contain executable inventory: {}",
            file.name
        ));
    }
    Ok(serde_json::json!({
        "id": package_id,
        "kind": package_kind,
        "name": file.name,
        "version": version,
        "source_commit": source_commit,
        "platform": platform,
        "architecture": architecture,
        "url": package_url,
        "bytes": file.size,
        "digest": {
            "sha256": file.sha256,
            "blake3": file.blake3,
        },
        "binaries": binaries,
        "evidence": [
            {
                "kind": "sbom",
                "name": host_sbom.name,
                "url": host_sbom_url,
                "bytes": host_sbom.size,
                "digest": {
                    "sha256": host_sbom.sha256,
                    "blake3": host_sbom.blake3,
                },
                "status": "current",
            }
        ],
        "status": "current",
    }))
}

fn graph_profile_revision_summary(profiles: &serde_json::Map<String, serde_json::Value>) -> String {
    let revisions = profiles
        .values()
        .filter_map(|profile| profile.get("revision").and_then(|value| value.as_str()))
        .collect::<BTreeSet<_>>();
    if revisions.len() == 1 {
        revisions
            .into_iter()
            .next()
            .unwrap_or("unknown")
            .to_string()
    } else {
        "mixed".to_string()
    }
}

fn binary_files_from_artifacts(artifacts: &[PathBuf]) -> Result<Vec<BinaryFile>> {
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    for path in artifacts {
        let metadata = fs::metadata(path)
            .with_context(|| format!("stat binary release artifact {}", path.display()))?;
        if !metadata.is_file() {
            return Err(anyhow!(
                "binary release artifact is not a file: {}",
                path.display()
            ));
        }
        if metadata.len() == 0 {
            return Err(anyhow!(
                "binary release artifact is empty: {}",
                path.display()
            ));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("artifact path has no UTF-8 file name: {}", path.display()))?
            .to_string();
        if !names.insert(name.clone()) {
            return Err(anyhow!("duplicate binary release artifact name: {name}"));
        }
        let bytes = fs::read(path)
            .with_context(|| format!("read binary release artifact {}", path.display()))?;
        if name.ends_with(".deb") {
            let filename_architecture =
                release_graph::PackageArchitecture::from_package_name(&name)?;
            let control_architecture = deb_control_architecture(&bytes)
                .with_context(|| format!("read Debian control metadata from {}", path.display()))?;
            if filename_architecture != control_architecture {
                return Err(anyhow!(
                    "Debian package filename architecture {} does not match control Architecture {}: {}",
                    filename_architecture.as_str(),
                    control_architecture.as_str(),
                    name
                ));
            }
        } else if name.ends_with(".pkg") {
            release_graph::PackageArchitecture::from_package_name(&name)?;
        }
        if is_host_sbom_file(&name) || is_package_sbom_file(&name) {
            validate_host_spdx_sbom_bytes(&bytes, path)
                .with_context(|| format!("validate host SBOM artifact {}", path.display()))?;
        }
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let blake3 = blake3::hash(&bytes).to_hex().to_string();
        files.push(BinaryFile {
            name,
            size: bytes.len() as u64,
            sha256,
            blake3,
            binaries: packaged_executable_inventory(path, &bytes)?,
        });
    }
    files.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(files)
}

fn packaged_executable_inventory(path: &Path, bytes: &[u8]) -> Result<Vec<BinaryExecutable>> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("artifact path has no UTF-8 file name: {}", path.display()))?;
    if name.ends_with(".deb") {
        return deb_executable_inventory(bytes)
            .with_context(|| format!("extract executable inventory from {}", path.display()));
    }
    if name.ends_with(".pkg") {
        return pkg_executable_inventory(path)
            .with_context(|| format!("extract executable inventory from {}", path.display()));
    }
    Ok(Vec::new())
}

fn pkg_executable_inventory(path: &Path) -> Result<Vec<BinaryExecutable>> {
    let temp = std::env::temp_dir().join(format!(
        "capsem-admin-pkg-expand-{}-{}",
        std::process::id(),
        blake3::hash(path.to_string_lossy().as_bytes()).to_hex()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).with_context(|| format!("remove {}", temp.display()))?;
    }
    let output = match Command::new("pkgutil")
        .arg("--expand-full")
        .arg(path)
        .arg(&temp)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return pkg_xar_payload_executable_inventory(path)
                .or_else(|_| pkg_payload_tar_executable_inventory(path))
        }
        Err(error) => return Err(error).context("run pkgutil --expand-full"),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_dir_all(&temp);
        return Err(anyhow!("pkgutil --expand-full failed: {stderr}"));
    }
    let result = collect_pkg_payload_executables(&temp);
    let _ = fs::remove_dir_all(&temp);
    result
}

fn pkg_xar_payload_executable_inventory(path: &Path) -> Result<Vec<BinaryExecutable>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.len() < 28 || &bytes[..4] != b"xar!" {
        return Err(anyhow!("{} is not a xar pkg archive", path.display()));
    }
    let header_size = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    if header_size < 28 {
        return Err(anyhow!("{} has an invalid xar header", path.display()));
    }
    let compressed_toc_size = u64::from_be_bytes(
        bytes[8..16]
            .try_into()
            .expect("xar compressed TOC size width"),
    ) as usize;
    let toc_end = header_size
        .checked_add(compressed_toc_size)
        .ok_or_else(|| anyhow!("{} xar TOC size overflow", path.display()))?;
    if toc_end > bytes.len() {
        return Err(anyhow!(
            "{} xar TOC extends past end of file",
            path.display()
        ));
    }
    let mut toc_decoder = flate2::read::ZlibDecoder::new(&bytes[header_size..toc_end]);
    let mut toc = String::new();
    toc_decoder
        .read_to_string(&mut toc)
        .with_context(|| format!("decompress xar TOC {}", path.display()))?;
    let mut binaries = Vec::new();
    let mut search_from = 0;
    while let Some(relative_name) = toc[search_from..].find("<name>Payload</name>") {
        let name_index = search_from + relative_name;
        let block_start = toc[..name_index]
            .rfind("<file")
            .ok_or_else(|| anyhow!("{} xar Payload entry missing file start", path.display()))?;
        let block_end = name_index
            + toc[name_index..]
                .find("</file>")
                .ok_or_else(|| anyhow!("{} xar Payload entry missing file end", path.display()))?
            + "</file>".len();
        let block = &toc[block_start..block_end];
        let offset = xml_tag_u64(block, "offset")? as usize;
        let length = xml_tag_u64(block, "length")? as usize;
        let payload_start = toc_end
            .checked_add(offset)
            .ok_or_else(|| anyhow!("{} xar Payload offset overflow", path.display()))?;
        let payload_end = payload_start
            .checked_add(length)
            .ok_or_else(|| anyhow!("{} xar Payload length overflow", path.display()))?;
        if payload_end > bytes.len() {
            return Err(anyhow!(
                "{} xar Payload extends past end of file",
                path.display()
            ));
        }
        let mut payload = Vec::new();
        if block.contains("application/x-gzip")
            || bytes[payload_start..payload_end].starts_with(&[0x1f, 0x8b])
        {
            let mut decoder = flate2::read::GzDecoder::new(&bytes[payload_start..payload_end]);
            decoder
                .read_to_end(&mut payload)
                .with_context(|| format!("decompress xar Payload {}", path.display()))?;
        } else {
            payload.extend_from_slice(&bytes[payload_start..payload_end]);
        }
        collect_newc_cpio_executables(&payload, &mut binaries)
            .with_context(|| format!("read xar Payload cpio {}", path.display()))?;
        search_from = block_end;
    }
    if binaries.is_empty() {
        return Err(anyhow!(
            "{} xar Payload contained no Capsem executables",
            path.display()
        ));
    }
    binaries.sort_by(|left, right| left.installed_path.cmp(&right.installed_path));
    Ok(binaries)
}

fn xml_tag_u64(block: &str, tag: &str) -> Result<u64> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = block
        .find(&open)
        .ok_or_else(|| anyhow!("xar XML missing <{tag}>"))?
        + open.len();
    let end = start
        + block[start..]
            .find(&close)
            .ok_or_else(|| anyhow!("xar XML missing </{tag}>"))?;
    block[start..end]
        .trim()
        .parse::<u64>()
        .with_context(|| format!("parse xar XML <{tag}>"))
}

fn collect_newc_cpio_executables(bytes: &[u8], binaries: &mut Vec<BinaryExecutable>) -> Result<()> {
    if bytes.starts_with(b"070707") {
        return collect_odc_cpio_executables(bytes, binaries);
    }
    let mut offset = 0usize;
    while offset < bytes.len() {
        if offset + 110 > bytes.len() {
            return Err(anyhow!("newc cpio header truncated"));
        }
        let header = &bytes[offset..offset + 110];
        if &header[..6] != b"070701" && &header[..6] != b"070702" {
            return Err(anyhow!("newc cpio header magic mismatch"));
        }
        let mode = cpio_hex_field(header, 14)?;
        let file_size = cpio_hex_field(header, 54)? as usize;
        let name_size = cpio_hex_field(header, 94)? as usize;
        let name_start = offset + 110;
        let name_end = name_start
            .checked_add(name_size)
            .ok_or_else(|| anyhow!("newc cpio name size overflow"))?;
        if name_end > bytes.len() || name_size == 0 {
            return Err(anyhow!("newc cpio name truncated"));
        }
        let name_bytes = &bytes[name_start..name_end - 1];
        let name = std::str::from_utf8(name_bytes).context("newc cpio path is not UTF-8")?;
        let data_start = align4(name_end);
        let data_end = data_start
            .checked_add(file_size)
            .ok_or_else(|| anyhow!("newc cpio file size overflow"))?;
        if data_end > bytes.len() {
            return Err(anyhow!("newc cpio file data truncated"));
        }
        if name == "TRAILER!!!" {
            break;
        }
        let normalized = name.trim_start_matches("./");
        let is_regular = mode & 0o170000 == 0o100000;
        if is_regular && mode & 0o111 != 0 {
            let mut contents = &bytes[data_start..data_end];
            push_pkg_payload_executable(normalized, &mut contents, binaries)?;
        }
        offset = align4(data_end);
    }
    Ok(())
}

fn collect_odc_cpio_executables(bytes: &[u8], binaries: &mut Vec<BinaryExecutable>) -> Result<()> {
    let mut offset = 0usize;
    while offset < bytes.len() {
        if offset + 76 > bytes.len() {
            return Err(anyhow!("odc cpio header truncated"));
        }
        let header = &bytes[offset..offset + 76];
        if &header[..6] != b"070707" {
            return Err(anyhow!("odc cpio header magic mismatch"));
        }
        let mode = cpio_octal_field(header, 18, 6)?;
        let file_size = cpio_octal_field(header, 65, 11)? as usize;
        let name_size = cpio_octal_field(header, 59, 6)? as usize;
        let name_start = offset + 76;
        let name_end = name_start
            .checked_add(name_size)
            .ok_or_else(|| anyhow!("odc cpio name size overflow"))?;
        if name_end > bytes.len() || name_size == 0 {
            return Err(anyhow!("odc cpio name truncated"));
        }
        let name_bytes = &bytes[name_start..name_end - 1];
        let name = std::str::from_utf8(name_bytes).context("odc cpio path is not UTF-8")?;
        let data_start = name_end;
        let data_end = data_start
            .checked_add(file_size)
            .ok_or_else(|| anyhow!("odc cpio file size overflow"))?;
        if data_end > bytes.len() {
            return Err(anyhow!("odc cpio file data truncated"));
        }
        if name == "TRAILER!!!" {
            break;
        }
        let normalized = name.trim_start_matches("./");
        let is_regular = mode & 0o170000 == 0o100000;
        if is_regular && mode & 0o111 != 0 {
            let mut contents = &bytes[data_start..data_end];
            push_pkg_payload_executable(normalized, &mut contents, binaries)?;
        }
        offset = data_end;
    }
    Ok(())
}

fn cpio_hex_field(header: &[u8], start: usize) -> Result<u64> {
    let end = start + 8;
    let value = std::str::from_utf8(&header[start..end]).context("newc cpio hex field UTF-8")?;
    u64::from_str_radix(value, 16).with_context(|| format!("parse newc cpio field {value}"))
}

fn cpio_octal_field(header: &[u8], start: usize, width: usize) -> Result<u64> {
    let end = start + width;
    let value = std::str::from_utf8(&header[start..end]).context("odc cpio octal field UTF-8")?;
    u64::from_str_radix(value, 8).with_context(|| format!("parse odc cpio field {value}"))
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn pkg_payload_tar_executable_inventory(path: &Path) -> Result<Vec<BinaryExecutable>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(Vec::new());
    }
    let decoder = flate2::read::GzDecoder::new(bytes.as_slice());
    let mut archive = tar::Archive::new(decoder);
    let mut binaries = Vec::new();
    for entry in archive
        .entries()
        .context("read synthetic pkg payload tar")?
    {
        let mut entry = entry.context("read synthetic pkg payload entry")?;
        let header = entry.header().clone();
        if !header.entry_type().is_file() || header.mode().unwrap_or(0) & 0o111 == 0 {
            continue;
        }
        let path = entry
            .path()
            .context("read synthetic pkg payload entry path")?;
        let normalized = path.to_string_lossy().to_string();
        let Some((_, installed_path)) = normalized.split_once("/Payload/") else {
            continue;
        };
        push_pkg_payload_executable(installed_path, &mut entry, &mut binaries)?;
    }
    binaries.sort_by(|left, right| left.installed_path.cmp(&right.installed_path));
    Ok(binaries)
}

fn collect_pkg_payload_executables(root: &Path) -> Result<Vec<BinaryExecutable>> {
    let mut binaries = Vec::new();
    collect_pkg_payload_executables_from(root, &mut binaries)?;
    binaries.sort_by(|left, right| left.installed_path.cmp(&right.installed_path));
    Ok(binaries)
}

fn collect_pkg_payload_executables_from(
    path: &Path,
    binaries: &mut Vec<BinaryExecutable>,
) -> Result<()> {
    for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", path.display()))?;
        let path = entry.path();
        let metadata = entry
            .metadata()
            .with_context(|| format!("stat {}", path.display()))?;
        if metadata.is_dir() {
            collect_pkg_payload_executables_from(&path, binaries)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        let normalized = path.to_string_lossy();
        let Some((_, installed_path)) = normalized.split_once("/Payload/") else {
            continue;
        };
        let mut contents =
            fs::File::open(&path).with_context(|| format!("open {}", path.display()))?;
        push_pkg_payload_executable(installed_path, &mut contents, binaries)?;
    }
    Ok(())
}

fn push_pkg_payload_executable(
    installed_path: &str,
    reader: &mut dyn Read,
    binaries: &mut Vec<BinaryExecutable>,
) -> Result<()> {
    if !installed_path.starts_with("usr/local/share/capsem/bin/")
        && !installed_path.starts_with("Applications/Capsem.app/Contents/MacOS/")
    {
        return Ok(());
    }
    let name = Path::new(installed_path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("pkg executable has no file name: {installed_path}"))?
        .to_string();
    let mut contents = Vec::new();
    reader
        .read_to_end(&mut contents)
        .with_context(|| format!("read pkg executable {installed_path}"))?;
    binaries.push(BinaryExecutable {
        sbom_component_ref: format!("SPDXRef-File-{}", spdx_ref_fragment(&name)),
        description: binary_description_for_name(&name).to_string(),
        installed_path: format!("/{installed_path}"),
        name,
        size: contents.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&contents)),
        blake3: blake3::hash(&contents).to_hex().to_string(),
    });
    Ok(())
}

fn deb_executable_inventory(bytes: &[u8]) -> Result<Vec<BinaryExecutable>> {
    let mut reader: Box<dyn Read> = if let Ok(data_tar) = deb_member(bytes, "data.tar.gz") {
        Box::new(flate2::read::GzDecoder::new(data_tar))
    } else {
        let data_tar = deb_member(bytes, "data.tar.zst")?;
        Box::new(zstd::stream::read::Decoder::new(data_tar).context("decode data.tar.zst")?)
    };
    let mut archive = tar::Archive::new(&mut reader);
    let mut binaries = Vec::new();
    for entry in archive.entries().context("read data.tar.gz entries")? {
        let mut entry = entry.context("read data.tar.gz entry")?;
        let header = entry.header().clone();
        if !header.entry_type().is_file() || header.mode().unwrap_or(0) & 0o111 == 0 {
            continue;
        }
        let path = entry.path().context("read data.tar.gz entry path")?;
        let normalized = path.to_string_lossy().trim_start_matches("./").to_string();
        if !normalized.starts_with("usr/bin/") && !normalized.starts_with("usr/local/bin/") {
            continue;
        }
        let name = Path::new(&normalized)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("deb executable has no file name: {normalized}"))?
            .to_string();
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .with_context(|| format!("read deb executable {normalized}"))?;
        binaries.push(BinaryExecutable {
            sbom_component_ref: format!("SPDXRef-File-{}", spdx_ref_fragment(&name)),
            description: binary_description_for_name(&name).to_string(),
            installed_path: format!("/{normalized}"),
            name,
            size: contents.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&contents)),
            blake3: blake3::hash(&contents).to_hex().to_string(),
        });
    }
    binaries.sort_by(|left, right| left.installed_path.cmp(&right.installed_path));
    Ok(binaries)
}

fn deb_control_architecture(bytes: &[u8]) -> Result<release_graph::PackageArchitecture> {
    let mut reader: Box<dyn Read> = if let Ok(control_tar) = deb_member(bytes, "control.tar.gz") {
        Box::new(flate2::read::GzDecoder::new(control_tar))
    } else {
        let control_tar = deb_member(bytes, "control.tar.zst")?;
        Box::new(zstd::stream::read::Decoder::new(control_tar).context("decode control.tar.zst")?)
    };
    let mut archive = tar::Archive::new(&mut reader);
    let mut architecture = None;
    for entry in archive.entries().context("read Debian control archive")? {
        let mut entry = entry.context("read Debian control entry")?;
        let path = entry.path().context("read Debian control entry path")?;
        if path.to_string_lossy().trim_start_matches("./") != "control" {
            continue;
        }
        let mut control = String::new();
        entry
            .read_to_string(&mut control)
            .context("read Debian control file")?;
        for line in control.lines() {
            let Some(value) = line.strip_prefix("Architecture:") else {
                continue;
            };
            if architecture.is_some() {
                return Err(anyhow!(
                    "Debian control file contains duplicate Architecture fields"
                ));
            }
            architecture = Some(match value.trim() {
                "amd64" => release_graph::PackageArchitecture::Amd64,
                "arm64" => release_graph::PackageArchitecture::Arm64,
                value => {
                    return Err(anyhow!("unsupported Debian control Architecture: {value}"));
                }
            });
        }
    }
    architecture.ok_or_else(|| anyhow!("Debian control file is missing Architecture"))
}

fn deb_member<'a>(bytes: &'a [u8], member_name: &str) -> Result<&'a [u8]> {
    if !bytes.starts_with(b"!<arch>\n") {
        return Err(anyhow!("deb archive missing ar global header"));
    }
    let mut offset = 8usize;
    while offset + 60 <= bytes.len() {
        let header = &bytes[offset..offset + 60];
        offset += 60;
        if &header[58..60] != b"`\n" {
            return Err(anyhow!("deb archive member header is malformed"));
        }
        let raw_name = std::str::from_utf8(&header[0..16])
            .context("deb archive member name is not UTF-8")?
            .trim();
        let name = raw_name.trim_end_matches('/');
        let size_text = std::str::from_utf8(&header[48..58])
            .context("deb archive member size is not UTF-8")?
            .trim();
        let size = size_text
            .parse::<usize>()
            .with_context(|| format!("deb archive member {name} has invalid size"))?;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| anyhow!("deb archive member {name} size overflows"))?;
        if end > bytes.len() {
            return Err(anyhow!(
                "deb archive member {name} extends past end of file"
            ));
        }
        if name == member_name {
            return Ok(&bytes[offset..end]);
        }
        offset = end + (size % 2);
    }
    Err(anyhow!("deb archive missing {member_name}"))
}

fn spdx_ref_fragment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn validate_binary_version(version: &str) -> Result<()> {
    if version.is_empty()
        || version.starts_with('v')
        || !version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(anyhow!(
            "binary version must be a URL-safe version without a leading v: {version}"
        ));
    }
    Ok(())
}

fn validate_release_date(date: &str) -> Result<()> {
    let valid = date.len() == 10
        && date.as_bytes()[4] == b'-'
        && date.as_bytes()[7] == b'-'
        && date
            .bytes()
            .enumerate()
            .all(|(idx, byte)| idx == 4 || idx == 7 || byte.is_ascii_digit());
    if !valid {
        return Err(anyhow!("release date must be YYYY-MM-DD: {date}"));
    }
    Ok(())
}

fn copy_assets_channel_release_assets(
    assets_dir: &Path,
    release_dir: &Path,
    release: &mut capsem_core::asset_manager::AssetRelease,
    cache: &mut AssetDigestCache,
) -> Result<usize> {
    let mut copied = 0;
    for (arch, assets) in &mut release.arches {
        for (logical_name, entry) in assets {
            let dst = release_dir.join(format!("{arch}-{logical_name}"));
            let src = assets_dir.join(arch).join(logical_name);
            let (bytes, digest) = copy_file_with_digest(&src, &dst)?;
            validate_asset_digest(arch, logical_name, entry, bytes, &digest)?;
            if entry.sha256.is_empty() {
                entry.sha256 = digest["sha256"].as_str().unwrap_or_default().to_string();
            }
            cache.insert((arch.clone(), logical_name.clone()), (bytes, digest));
            copied += 1;
        }
    }
    Ok(copied)
}

fn hydrate_current_asset_entry_sha256(
    manifest: &mut ManifestV2,
    assets_dir: &Path,
    cache: &mut AssetDigestCache,
) -> Result<()> {
    let asset_version = manifest.assets.current.clone();
    let release = manifest
        .assets
        .releases
        .get_mut(&asset_version)
        .ok_or_else(|| anyhow!("manifest current asset release is missing"))?;
    for (arch, assets) in &mut release.arches {
        for (logical_name, entry) in assets {
            if !entry.sha256.is_empty() {
                continue;
            }
            let source = assets_dir.join(arch).join(logical_name);
            let (bytes, digest) = file_digest(&source).with_context(|| {
                format!(
                    "hydrate current asset {asset_version} {arch}/{logical_name} from {}",
                    source.display()
                )
            })?;
            validate_asset_digest(arch, logical_name, entry, bytes, &digest)?;
            entry.sha256 = digest["sha256"].as_str().unwrap_or_default().to_string();
            cache.insert((arch.clone(), logical_name.clone()), (bytes, digest));
        }
    }
    Ok(())
}

fn check_assets_channel(dist: &Path, channel: &str) -> Result<AssetsChannelCheckReport> {
    validate_channel_name(channel)?;
    let index_path = dist.join("index.html");
    let channel_index_path = dist.join("channels").join(channel).join("index.html");
    let manifest_path = dist.join("assets").join(channel).join("manifest.json");
    let channel_health_path = dist.join("assets").join(channel).join("health.json");
    let root_health_path = dist.join("health.json");
    let health_path = if channel_health_path.exists()
        && root_health_path.exists()
        && root_health_belongs_to_other_channel(&root_health_path, channel)
    {
        channel_health_path
    } else {
        root_health_path
    };
    let headers_path = dist.join("_headers");

    #[cfg(test)]
    if !index_path.exists() {
        write_test_assets_channel_index_fixture(dist, channel)
            .with_context(|| format!("write test {}", index_path.display()))?;
    }

    let index_html = fs::read_to_string(&index_path)
        .with_context(|| format!("read {}", index_path.display()))?;
    let channel_index_html = fs::read_to_string(&channel_index_path)
        .with_context(|| format!("read {}", channel_index_path.display()))?;
    if !index_html.contains("Capsem Release Channels") {
        return Err(anyhow!(
            "{} is not a Capsem release channel page",
            index_path.display()
        ));
    }
    validate_assets_channel_index_html(&index_html, channel)?;
    validate_assets_channel_page_html(&channel_index_html, channel)?;
    let manifest_content = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest_json: serde_json::Value =
        serde_json::from_str(&manifest_content).context("parse channel manifest JSON")?;
    let headers = fs::read_to_string(&headers_path)
        .with_context(|| format!("read {}", headers_path.display()))?;
    validate_assets_channel_headers(&headers, channel)?;
    validate_assets_channel_catalog_manifest_digest(dist, channel, &manifest_content)?;
    if is_release_graph_manifest_value(&manifest_json) {
        validate_assets_channel_graph_manifest(&manifest_json, channel)?;
        let health_content = fs::read_to_string(&health_path)
            .with_context(|| format!("read {}", health_path.display()))?;
        let health: serde_json::Value =
            serde_json::from_str(&health_content).context("parse asset channel health.json")?;
        validate_assets_channel_graph_health(dist, channel, &manifest_json, &health)?;
        validate_assets_channel_graph_index_state(&index_html, channel, &manifest_json, &health)?;
        validate_assets_channel_graph_page_state(
            &channel_index_html,
            channel,
            &manifest_json,
            &health,
        )?;
        return Ok(AssetsChannelCheckReport {
            schema: "capsem.admin.assets_channel_check.v1",
            ok: true,
            channel: channel.to_string(),
            state: "published".to_string(),
            dist: dist.display().to_string(),
            manifest: manifest_path.display().to_string(),
        });
    }
    let manifest: ManifestV2 =
        serde_json::from_value(manifest_json).context("parse legacy asset manifest")?;
    let health_content = fs::read_to_string(&health_path)
        .with_context(|| format!("read {}", health_path.display()))?;
    let health: serde_json::Value =
        serde_json::from_str(&health_content).context("parse asset channel health.json")?;
    validate_assets_channel_health(dist, channel, &manifest, &health)?;
    validate_assets_channel_index_state(&index_html, channel, &health)?;
    validate_assets_channel_page_state(&channel_index_html, channel, &manifest, &health)?;
    validate_assets_channel_headers(&headers, channel)?;
    Ok(AssetsChannelCheckReport {
        schema: "capsem.admin.assets_channel_check.v1",
        ok: true,
        channel: channel.to_string(),
        state: "published".to_string(),
        dist: dist.display().to_string(),
        manifest: manifest_path.display().to_string(),
    })
}

fn validate_assets_channel_headers(headers: &str, channel: &str) -> Result<()> {
    let channel_manifest_header =
        format!("/assets/{channel}/*\n  Cache-Control: no-cache, must-revalidate");
    if !headers.contains(&channel_manifest_header) {
        return Err(anyhow!("_headers must keep asset channel manifests fresh"));
    }
    if !headers.contains("/channels.json\n  Cache-Control: no-cache, must-revalidate") {
        return Err(anyhow!("_headers must keep channels.json fresh"));
    }
    if !headers.contains("/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable")
    {
        return Err(anyhow!("_headers must cache immutable asset releases"));
    }
    if !headers
        .contains("/profiles/releases/*\n  Cache-Control: public, max-age=31536000, immutable")
    {
        return Err(anyhow!("_headers must cache immutable profile releases"));
    }
    Ok(())
}

fn is_release_graph_manifest_value(manifest: &serde_json::Value) -> bool {
    manifest.get("packages").is_some() && manifest.get("profiles").is_some()
}

fn validate_assets_channel_graph_manifest(
    manifest: &serde_json::Value,
    channel: &str,
) -> Result<()> {
    require_json_string(manifest, &["version"])?;
    require_json_str(
        manifest,
        &["channel"],
        channel,
        "graph manifest channel mismatch",
    )?;
    require_json_str(
        manifest,
        &["status"],
        "current",
        "graph manifest status mismatch",
    )?;
    let packages = manifest
        .get("packages")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("graph manifest packages must be an array"))?;
    for package in packages {
        require_json_string(package, &["name"])?;
        require_json_string(package, &["url"])?;
        require_json_string(package, &["digest", "sha256"])?;
        require_json_string(package, &["digest", "blake3"])?;
        let binaries = package
            .get("binaries")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("graph package must list binaries"))?;
        if binaries.is_empty() {
            return Err(anyhow!("graph package must list at least one binary"));
        }
        for binary in binaries {
            require_json_string(binary, &["name"])?;
            require_json_string(binary, &["version"])?;
            require_json_string(binary, &["installed_path"])?;
            require_json_string(binary, &["digest", "sha256"])?;
            require_json_string(binary, &["digest", "blake3"])?;
            require_json_string(binary, &["sbom_component_ref"])?;
        }
    }
    manifest
        .get("profiles")
        .and_then(|value| value.as_object())
        .ok_or_else(|| anyhow!("graph manifest profiles must be an object"))?;
    Ok(())
}

fn validate_assets_channel_graph_health(
    dist: &Path,
    channel: &str,
    manifest: &serde_json::Value,
    health: &serde_json::Value,
) -> Result<()> {
    require_json_str(
        health,
        &["schema"],
        "capsem.assets_channel.health.v1",
        "health.json schema mismatch",
    )?;
    require_json_str(
        health,
        &["channel"],
        channel,
        "health.json channel mismatch",
    )?;
    require_json_bool(health, &["ok"], true, "health.json ok mismatch")?;
    require_json_str(
        health,
        &["state"],
        "published",
        "health.json state mismatch",
    )?;
    require_json_str(
        health,
        &["urls", "index"],
        "/index.html",
        "health.json index URL mismatch",
    )?;
    require_json_str(
        health,
        &["urls", "health"],
        "/health.json",
        "health.json health URL mismatch",
    )?;
    require_json_str(
        health,
        &["urls", "manifest"],
        &format!("/assets/{channel}/manifest.json"),
        "health.json manifest URL does not match channel",
    )?;
    let expected_asset_base = require_json_string(health, &["urls", "asset_base"])?;
    if json_path(health, &["urls", "profile_catalog"]).is_some() {
        return Err(anyhow!("health.json profile catalog URL mismatch"));
    }
    let current_assets = require_json_string(health, &["current", "assets"])?;
    let current_binary = require_json_string(health, &["current", "binary"])?;
    require_json_str(
        health,
        &["assets", "version"],
        &current_assets,
        "health.json assets value does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["binary", "version"],
        &current_binary,
        "health.json binary value does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "latest"],
        &current_assets,
        "health.json asset update latest target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "current"],
        &current_assets,
        "health.json asset update target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "source"],
        "manifest.assets.current",
        "health.json asset update source mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "manifest"],
        &format!("/assets/{channel}/manifest.json"),
        "health.json asset update manifest mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "asset_base"],
        &expected_asset_base,
        "health.json asset update base mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "latest"],
        &current_binary,
        "health.json binary update latest target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "current"],
        &current_binary,
        "health.json binary update target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "source"],
        "manifest.binaries.current",
        "health.json binary update source mismatch",
    )?;
    let profile_revision = require_json_string(health, &["profiles", "revision"])?;
    require_json_str(
        health,
        &["profiles", "state"],
        "current",
        "health.json profile state mismatch",
    )?;
    require_json_str(
        health,
        &["profiles", "source"],
        "manifest.profiles",
        "health.json profile source mismatch",
    )?;
    require_json_absent(
        health,
        &["profiles", "hash"],
        "health.json profiles must not publish detached catalog hash",
    )?;
    require_json_absent(
        health,
        &["profiles", "compatibility"],
        "health.json profiles must not publish channel compatibility",
    )?;
    require_json_absent(
        health,
        &["profiles", "requires_newer"],
        "health.json profiles must not publish channel requirements",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "latest"],
        &profile_revision,
        "health.json profile update latest target does not match manifest profiles",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "current"],
        &profile_revision,
        "health.json profile update current target does not match manifest profiles",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "source"],
        "manifest.profiles",
        "health.json profile update source mismatch",
    )?;
    require_json_absent(
        health,
        &["updates", "profiles", "hash"],
        "health.json profile updates must not publish detached catalog hash",
    )?;
    let asset_files = require_json_array(health, &["assets", "files"])?;
    let asset_releases = require_json_array(health, &["asset_releases"])?;
    for release in asset_releases {
        require_json_string(release, &["date"])
            .map_err(|_| anyhow!("health.json asset release date mismatch"))?;
    }
    let vm_oboms = require_json_array(health, &["evidence", "vm_oboms"])?;
    let host_sboms = require_json_array(health, &["evidence", "host_sboms"])?;
    let host_binary_files = require_json_array(health, &["evidence", "host_binary_files"])?;
    let attestations = require_json_array(health, &["evidence", "attestations"])?;
    let packages = manifest
        .get("packages")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("graph manifest packages must be an array"))?;

    let mut package_urls = BTreeSet::new();
    let mut expected_host_files = BTreeMap::new();
    let mut package_versions = BTreeSet::new();
    for package in packages {
        let package_url = package
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("graph package url missing"))?;
        package_urls.insert(package_url.to_string());
        let package_version = package
            .get("version")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("graph package version missing"))?;
        package_versions.insert(package_version.to_string());
        expected_host_files.insert(package_url.to_string(), package);
        let binaries = package
            .get("binaries")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("graph package must list binaries"))?;
        if binaries.is_empty() {
            return Err(anyhow!("graph package must list at least one binary"));
        }
        for evidence in package
            .get("evidence")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
        {
            let url = evidence
                .get("url")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("graph package evidence url missing"))?;
            expected_host_files.insert(url.to_string(), evidence);
        }
    }
    if package_versions.len() == 1 {
        let expected = package_versions.iter().next().expect("one package version");
        if expected != &current_binary {
            return Err(anyhow!(
                "health.json current binary value does not match graph package version"
            ));
        }
    }
    if !packages.is_empty() && host_binary_files.is_empty() {
        return Err(anyhow!("health.json host binary files missing"));
    }
    let has_host_sbom_attestation = attestations.iter().any(|item| {
        item.get("name").and_then(|value| value.as_str()) == Some("github_attestations_host_sbom")
    });
    if has_host_sbom_attestation && host_sboms.is_empty() {
        return Err(anyhow!("health.json host SBOM evidence missing"));
    }
    for host_file in host_binary_files {
        let url = host_file
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json host binary file url missing"))?;
        let Some(expected) = expected_host_files.get(url) else {
            continue;
        };
        let expected_name = expected
            .get("name")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("graph host binary name missing for {url}"))?;
        if host_file.get("name").and_then(|value| value.as_str()) != Some(expected_name) {
            return Err(anyhow!("health.json host binary name mismatch for {url}"));
        }
        let expected_sha256 = require_json_string(expected, &["digest", "sha256"])?;
        if host_file.get("sha256").and_then(|value| value.as_str())
            != Some(expected_sha256.as_str())
        {
            return Err(anyhow!("health.json host binary sha256 mismatch for {url}"));
        }
        let expected_blake3 = require_json_string(expected, &["digest", "blake3"])?;
        if host_file.get("blake3").and_then(|value| value.as_str())
            != Some(expected_blake3.as_str())
        {
            return Err(anyhow!("health.json host binary blake3 mismatch for {url}"));
        }
        let expected_bytes = expected
            .get("bytes")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| anyhow!("graph host binary bytes missing for {url}"))?;
        if host_file.get("size").and_then(|value| value.as_u64()) != Some(expected_bytes) {
            return Err(anyhow!("health.json host binary size mismatch for {url}"));
        }
    }
    for sbom in host_sboms {
        let sbom_url = sbom
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json host SBOM evidence missing url"))?;
        if sbom.get("name").and_then(|value| value.as_str()) != Some("capsem-sbom.spdx.json") {
            return Err(anyhow!(
                "health.json host SBOM evidence name mismatch for {sbom_url}"
            ));
        }
        let Some(host_binary) = host_binary_files
            .iter()
            .find(|item| item.get("url").and_then(|value| value.as_str()) == Some(sbom_url))
        else {
            return Err(anyhow!(
                "health.json host SBOM evidence {sbom_url} missing from host binary files"
            ));
        };
        if host_binary.get("name").and_then(|value| value.as_str()) != Some("capsem-sbom.spdx.json")
        {
            return Err(anyhow!(
                "health.json host SBOM evidence binary name mismatch for {sbom_url}"
            ));
        }
    }

    let mut current_asset_subjects = BTreeSet::new();
    for file in asset_files {
        let url = file
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json asset file url missing"))?;
        current_asset_subjects.insert(url.to_string());
        let size = file
            .get("size")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| anyhow!("health.json asset file size missing for {url}"))?;
        let hash = file
            .get("hash")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json asset file hash missing for {url}"))?;
        if url.starts_with('/') {
            let local_path = dist.join(url.trim_start_matches('/'));
            let bytes = fs::read(&local_path)
                .with_context(|| format!("read asset channel blob {}", local_path.display()))?;
            if bytes.len() as u64 != size {
                return Err(anyhow!(
                    "asset channel blob {} size mismatch",
                    local_path.display()
                ));
            }
            if blake3::hash(&bytes).to_hex().as_str() != hash {
                return Err(anyhow!(
                    "asset channel blob {} hash mismatch",
                    local_path.display()
                ));
            }
            if file.get("logical_name").and_then(|value| value.as_str()) == Some("obom.cdx.json") {
                validate_vm_cyclonedx_obom_bytes(&bytes, &local_path)?;
            }
        }
    }
    if asset_files.iter().any(|item| {
        item.get("logical_name").and_then(|value| value.as_str()) == Some("obom.cdx.json")
    }) && vm_oboms.is_empty()
    {
        return Err(anyhow!("health.json missing VM OBOM evidence"));
    }

    let mut saw_host_sbom_attestation = false;
    let mut saw_vm_asset_attestation = false;
    let mut host_package_attestation_subjects = BTreeSet::new();
    for attestation in attestations {
        let attestation_name = attestation
            .get("name")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json attestation name missing"))?;
        if let Some((expected_scope, expected_workflow)) =
            expected_attestation_rail(attestation_name)
        {
            let scope = attestation
                .get("scope")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json attestation scope missing"))?;
            if scope != expected_scope {
                return Err(anyhow!(
                    "health.json {} scope mismatch",
                    attestation_rail_label(attestation_name)
                ));
            }
            let workflow = attestation
                .get("workflow")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json attestation workflow missing"))?;
            if workflow != expected_workflow {
                return Err(anyhow!(
                    "health.json {} workflow mismatch",
                    attestation_rail_label(attestation_name)
                ));
            }
        }
        attestation
            .get("predicate_type")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("health.json attestation predicate_type missing"))?;
        let verify_command = attestation
            .get("verify_command")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json attestation verify_command missing"))?;
        if !verify_command.contains("gh attestation verify") {
            return Err(anyhow!(
                "health.json attestation verify_command must use gh attestation verify"
            ));
        }
        if attestation_name == "github_attestations_host_sbom" {
            saw_host_sbom_attestation = true;
            let predicate_url = attestation
                .get("predicate_url")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    anyhow!("health.json host SBOM attestation predicate_url missing")
                })?;
            if !host_sboms
                .iter()
                .any(|item| item.get("url").and_then(|value| value.as_str()) == Some(predicate_url))
            {
                return Err(anyhow!(
                    "health.json host SBOM attestation predicate {predicate_url} missing from host SBOM evidence"
                ));
            }
        }
        if attestation_name == "github_attestations_vm_assets" {
            let predicate_url = attestation
                .get("predicate_url")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json VM asset attestation predicate_url missing"))?;
            if !vm_oboms
                .iter()
                .any(|item| item.get("url").and_then(|value| value.as_str()) == Some(predicate_url))
            {
                return Err(anyhow!(
                    "health.json VM asset attestation predicate {predicate_url} missing from VM OBOM evidence"
                ));
            }
        }
        let subjects = attestation
            .get("subjects")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("health.json attestation subjects missing"))?;
        if subjects.is_empty() {
            return Err(anyhow!("health.json attestation subjects empty"));
        }
        for subject in subjects {
            let subject_url = subject
                .as_str()
                .ok_or_else(|| anyhow!("health.json attestation subject is not a string"))?;
            if attestation_name == "github_attestations_host" {
                host_package_attestation_subjects.insert(subject_url.to_string());
            }
            if current_asset_subjects.contains(subject_url) {
                saw_vm_asset_attestation = true;
            }
        }
    }
    if !host_sboms.is_empty() && !saw_host_sbom_attestation {
        return Err(anyhow!(
            "health.json host SBOM attestation evidence missing"
        ));
    }
    for subject in &package_urls {
        if !host_package_attestation_subjects.contains(subject) {
            return Err(anyhow!(
                "health.json host package attestation subjects missing {subject}"
            ));
        }
    }
    if !current_asset_subjects.is_empty() && !saw_vm_asset_attestation {
        return Err(anyhow!("health.json VM asset attestation evidence missing"));
    }
    Ok(())
}

fn validate_assets_channel_graph_index_state(
    index_html: &str,
    channel: &str,
    manifest: &serde_json::Value,
    health: &serde_json::Value,
) -> Result<()> {
    let generated_at = require_json_string(health, &["generated_at"])?;
    let manifest_version = require_json_string(manifest, &["version"])?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let expected = [
        ("generated timestamp", generated_at.as_str()),
        ("manifest version", manifest_version.as_str()),
        ("channel manifest", channel_manifest.as_str()),
    ];
    for (label, value) in expected {
        if !index_html.contains(&escape_html(value)) {
            return Err(anyhow!("asset channel index missing {label} {value}"));
        }
    }
    Ok(())
}

fn validate_assets_channel_catalog_manifest_digest(
    dist: &Path,
    channel: &str,
    manifest_content: &str,
) -> Result<()> {
    let channels_path = dist.join("channels.json");
    let channels: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&channels_path)
            .with_context(|| format!("read {}", channels_path.display()))?,
    )
    .with_context(|| format!("parse {}", channels_path.display()))?;
    let manifest_url = format!("/assets/{channel}/manifest.json");
    let records = channels
        .get("channels")
        .and_then(|value| value.get(channel))
        .and_then(|value| value.get("manifests"))
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("channels.json missing {channel} manifest records"))?;
    let record = records
        .iter()
        .find(|record| {
            record.get("status").and_then(|value| value.as_str()) == Some("current")
                && record.get("url").and_then(|value| value.as_str()) == Some(manifest_url.as_str())
        })
        .ok_or_else(|| anyhow!("channels.json missing current manifest record for {channel}"))?;
    let expected_sha256 = require_json_string(record, &["digest", "sha256"])?;
    let actual_sha256 = format!("{:x}", Sha256::digest(manifest_content.as_bytes()));
    if actual_sha256 != expected_sha256 {
        return Err(anyhow!("channels.json manifest sha256 mismatch"));
    }
    let expected_blake3 = require_json_string(record, &["digest", "blake3"])?;
    let actual_blake3 = blake3::hash(manifest_content.as_bytes())
        .to_hex()
        .to_string();
    if actual_blake3 != expected_blake3 {
        return Err(anyhow!("channels.json manifest blake3 mismatch"));
    }
    Ok(())
}

fn validate_assets_channel_graph_page_state(
    channel_html: &str,
    channel: &str,
    manifest: &serde_json::Value,
    health: &serde_json::Value,
) -> Result<()> {
    let generated_at = require_json_string(health, &["generated_at"])?;
    let manifest_version = require_json_string(manifest, &["version"])?;
    let current_binary = require_json_string(health, &["current", "binary"])?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let mut expected = vec![
        ("generated timestamp", generated_at.as_str()),
        ("manifest version", manifest_version.as_str()),
        ("channel manifest", channel_manifest.as_str()),
    ];
    if !require_json_array(health, &["evidence", "host_binary_files"])?.is_empty() {
        expected.push(("current binary", current_binary.as_str()));
    }
    for (label, value) in expected {
        if !channel_html.contains(&escape_html(value)) {
            return Err(anyhow!("asset channel page missing {label} {value}"));
        }
    }
    let profiles = manifest
        .get("profiles")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow!("graph manifest profiles must be an object"))?;
    for (profile_id, profile) in profiles {
        let revision = require_json_string(profile, &["revision"])?;
        if !channel_html.contains(&escape_html(profile_id))
            || !channel_html.contains(&escape_html(&revision))
        {
            return Err(anyhow!(
                "asset channel page missing profile revision {profile_id} {revision}"
            ));
        }
    }
    Ok(())
}

fn root_health_belongs_to_other_channel(root_health_path: &Path, channel: &str) -> bool {
    let Ok(content) = fs::read_to_string(root_health_path) else {
        return false;
    };
    let Ok(health) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    health
        .get("channel")
        .and_then(|value| value.as_str())
        .is_some_and(|root_channel| root_channel != channel)
}

fn validate_assets_channel_index_html(index_html: &str, channel: &str) -> Result<()> {
    let expected = [
        "Channels",
        "Manifest revision",
        "Updated",
        "Coverage",
        "/channels.json",
        "Manifest URL",
    ];
    for needle in expected {
        if !index_html.contains(needle) {
            return Err(anyhow!("asset channel index missing {needle}"));
        }
    }
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    if !index_html.contains(&channel_manifest) {
        return Err(anyhow!("asset channel index missing {channel_manifest}"));
    }
    for forbidden in ["Selected manifest", ">Status<", ">Records<"] {
        if index_html.contains(forbidden) {
            return Err(anyhow!("asset channel index still contains {forbidden}"));
        }
    }
    if index_html.contains(&format!("/manifests/{channel}/")) {
        return Err(anyhow!(
            "asset channel index must not publish legacy graph manifest URLs"
        ));
    }
    Ok(())
}

fn validate_assets_channel_page_html(channel_html: &str, channel: &str) -> Result<()> {
    let expected = [
        "Current Manifest",
        "Manifest History",
        "Capsem Packages",
        "Profile References",
    ];
    for needle in expected {
        if !channel_html.contains(needle) {
            return Err(anyhow!("asset channel page missing {needle}"));
        }
    }
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    if !channel_html.contains(&channel_manifest) {
        return Err(anyhow!("asset channel page missing {channel_manifest}"));
    }
    if channel_html.contains("Capsem Binaries") {
        return Err(anyhow!(
            "asset channel page must not flatten package-owned binaries"
        ));
    }
    Ok(())
}

#[cfg(test)]
fn write_test_assets_channel_index_fixture(dist: &Path, channel: &str) -> Result<()> {
    let health: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dist.join("health.json")).context("read test health.json")?,
    )
    .context("parse test health.json")?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let manifest_path = dist.join(channel_manifest.trim_start_matches('/'));
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("read test {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parse test {}", manifest_path.display()))?;
    let manifest_version = require_json_string(&manifest, &["version"])?;
    let generated_at = require_json_string(&health, &["generated_at"])?;
    let profile_revision = require_json_string(&health, &["profiles", "revision"])?;
    let profile_revisions = manifest
        .get("profiles")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow!("test graph manifest profiles must be an object"))?
        .iter()
        .map(|(profile_id, profile)| {
            Ok(format!(
                "{} {}",
                escape_html(profile_id),
                escape_html(&require_json_string(profile, &["revision"])?)
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join(" ");
    let asset_base = require_json_string(&health, &["urls", "asset_base"])?;
    let binary = require_json_string(&health, &["current", "binary"])?;
    let assets = require_json_string(&health, &["current", "assets"])?;
    let date = health
        .get("asset_releases")
        .and_then(|value| value.as_array())
        .and_then(|releases| releases.first())
        .and_then(|release| release.get("date"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let html = format!(
        "<!doctype html><html><body><main><h1>Capsem Release Channels</h1>\
        <h2>Channels</h2><h2>Manifest revision</h2><h2>Updated</h2><h2>Coverage</h2>\
        <a href=\"/channels.json\">/channels.json</a>\
        <p>Manifest URL <a href=\"{channel_manifest}\">{channel_manifest}</a></p>\
        <p>{manifest_version} {binary} {assets} {generated_at} {date}</p>\
        <p>Current asset base {asset_base}</p>\
        <p>{profile_revision}</p><h2>Binaries</h2><h2>Profiles</h2>\
        <h2>Capsem Binaries</h2><h2>Asset Release History</h2></main></body></html>",
        channel_manifest = escape_html(&channel_manifest),
        manifest_version = escape_html(&manifest_version),
        binary = escape_html(&binary),
        assets = escape_html(&assets),
        generated_at = escape_html(&generated_at),
        date = escape_html(date),
        asset_base = escape_html(&asset_base),
        profile_revision = escape_html(&profile_revision),
    );
    fs::write(dist.join("index.html"), html).context("write test release index fixture")?;
    let channel_dir = dist.join("channels").join(channel);
    fs::create_dir_all(&channel_dir)
        .with_context(|| format!("create test channel page {}", channel_dir.display()))?;
    let channel_html = format!(
        "<!doctype html><html><body><main><h1>{channel}</h1>\
        <h2>Current Manifest</h2><h2>Manifest History</h2><h2>Capsem Packages</h2>\
        <h3>Package target Linux arm64</h3>\
        <a href=\"/channels/{channel}/packages/capsem-test-arm64-deb/\">Package detail</a>\
        <h2>Profile References</h2><p>SBOM</p>\
        <p>{generated_at}</p><p>{manifest_version}</p><p>{binary}</p><p>{assets}</p>\
        <a href=\"{channel_manifest}\">{channel_manifest}</a>\
        <p>{profile_revision}</p><p>{profile_revisions}</p>\
        </main></body></html>",
        channel = escape_html(channel),
        generated_at = escape_html(&generated_at),
        manifest_version = escape_html(&manifest_version),
        binary = escape_html(&binary),
        assets = escape_html(&assets),
        channel_manifest = escape_html(&channel_manifest),
        profile_revision = escape_html(&profile_revision),
        profile_revisions = profile_revisions,
    );
    fs::write(channel_dir.join("index.html"), channel_html)
        .context("write test release channel page fixture")
}

fn validate_assets_channel_index_state(
    index_html: &str,
    channel: &str,
    health: &serde_json::Value,
) -> Result<()> {
    let generated_at = require_json_string(health, &["generated_at"])?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let expected = [
        ("generated timestamp", generated_at.as_str()),
        ("channel manifest", channel_manifest.as_str()),
    ];
    for (label, value) in expected {
        if !index_html.contains(&escape_html(value)) {
            return Err(anyhow!("asset channel index missing {label} {value}"));
        }
    }
    Ok(())
}

fn validate_assets_channel_page_state(
    channel_html: &str,
    channel: &str,
    manifest: &ManifestV2,
    health: &serde_json::Value,
) -> Result<()> {
    let generated_at = require_json_string(health, &["generated_at"])?;
    let profile_revision = require_json_string(health, &["profiles", "revision"])?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let mut expected = vec![
        ("generated timestamp", generated_at.as_str()),
        ("channel manifest", channel_manifest.as_str()),
        ("profile revision", profile_revision.as_str()),
    ];
    if manifest
        .binaries
        .releases
        .get(&manifest.binaries.current)
        .is_some_and(|release| !release.files.is_empty())
    {
        expected.push(("current binary", manifest.binaries.current.as_str()));
    }
    for (label, value) in expected {
        if !channel_html.contains(&escape_html(value)) {
            return Err(anyhow!("asset channel page missing {label} {value}"));
        }
    }
    Ok(())
}

fn validate_assets_channel_health(
    dist: &Path,
    channel: &str,
    manifest: &ManifestV2,
    health: &serde_json::Value,
) -> Result<()> {
    require_json_str(
        health,
        &["schema"],
        "capsem.assets_channel.health.v1",
        "health.json schema mismatch",
    )?;
    require_json_bool(health, &["ok"], true, "health.json ok mismatch")?;
    require_json_str(
        health,
        &["channel"],
        channel,
        "health.json channel mismatch",
    )?;
    require_json_str(
        health,
        &["state"],
        "published",
        "health.json state mismatch",
    )?;
    require_json_str(
        health,
        &["urls", "index"],
        "/index.html",
        "health.json index URL mismatch",
    )?;
    require_json_str(
        health,
        &["urls", "health"],
        "/health.json",
        "health.json health URL mismatch",
    )?;
    require_json_str(
        health,
        &["urls", "manifest"],
        &format!("/assets/{channel}/manifest.json"),
        "health.json manifest URL does not match channel",
    )?;
    let expected_asset_base = manifest.asset_base.as_deref().unwrap_or("/assets/releases");
    require_json_str(
        health,
        &["urls", "asset_base"],
        expected_asset_base,
        "health.json asset base mismatch",
    )?;
    require_json_str(
        health,
        &["current", "assets"],
        &manifest.assets.current,
        "health.json current assets value does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["assets", "version"],
        &manifest.assets.current,
        "health.json assets value does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["current", "binary"],
        &manifest.binaries.current,
        "health.json current binary value does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["binary", "version"],
        &manifest.binaries.current,
        "health.json binary value does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "latest"],
        &manifest.binaries.current,
        "health.json binary update latest target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "current"],
        &manifest.binaries.current,
        "health.json binary update target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "state"],
        health
            .get("binary")
            .and_then(|binary| binary.get("state"))
            .and_then(|state| state.as_str())
            .unwrap_or(""),
        "health.json binary update state mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "source"],
        "manifest.binaries.current",
        "health.json binary update source mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "latest"],
        &manifest.assets.current,
        "health.json asset update latest target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "current"],
        &manifest.assets.current,
        "health.json asset update target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "state"],
        health
            .get("assets")
            .and_then(|assets| assets.get("state"))
            .and_then(|state| state.as_str())
            .unwrap_or(""),
        "health.json asset update state mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "source"],
        "manifest.assets.current",
        "health.json asset update source mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "manifest"],
        &format!("/assets/{channel}/manifest.json"),
        "health.json asset update manifest mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "asset_base"],
        expected_asset_base,
        "health.json asset update base mismatch",
    )?;
    let current_release = manifest
        .assets
        .releases
        .get(&manifest.assets.current)
        .ok_or_else(|| anyhow!("channel manifest current asset release is missing"))?;
    let expected_profile_revision = require_json_string(health, &["profiles", "revision"])?;
    require_json_str(
        health,
        &["profiles", "state"],
        "current",
        "health.json profile state mismatch",
    )?;
    require_json_str(
        health,
        &["profiles", "source"],
        "manifest.profiles",
        "health.json profile source mismatch",
    )?;
    require_json_absent(
        health,
        &["profiles", "hash"],
        "health.json profiles must not publish detached catalog hash",
    )?;
    require_json_absent(
        health,
        &["profiles", "compatibility"],
        "health.json profiles must not publish channel compatibility",
    )?;
    require_json_absent(
        health,
        &["profiles", "requires_newer"],
        "health.json profiles must not publish channel requirements",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "latest"],
        &expected_profile_revision,
        "health.json profile update latest target does not match manifest profiles",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "current"],
        &expected_profile_revision,
        "health.json profile update current target does not match manifest profiles",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "state"],
        "current",
        "health.json profile update state mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "source"],
        "manifest.profiles",
        "health.json profile update source mismatch",
    )?;
    require_json_absent(
        health,
        &["updates", "profiles", "hash"],
        "health.json profile updates must not publish detached catalog hash",
    )?;
    require_json_absent(
        health,
        &["updates", "profiles", "compatibility"],
        "health.json profile update must not publish channel compatibility",
    )?;
    require_json_absent(
        health,
        &["updates", "profiles", "requires_newer"],
        "health.json profile update must not publish channel requirements",
    )?;
    require_json_str(
        health,
        &["updates", "images", "state"],
        "not_published",
        "health.json image update state mismatch",
    )?;
    require_json_null(
        health,
        &["updates", "images", "latest"],
        "health.json image update latest should be null while unpublished",
    )?;
    require_json_str(
        health,
        &["updates", "images", "source"],
        "manifest.profiles.images",
        "health.json image update source mismatch",
    )?;

    let asset_releases = require_json_array(health, &["asset_releases"])?;
    for (version, release) in &manifest.assets.releases {
        let public_release = asset_releases.iter().find(|item| {
            item.get("version").and_then(|value| value.as_str()) == Some(version.as_str())
        });
        let Some(public_release) = public_release else {
            return Err(anyhow!("health.json missing asset release {version}"));
        };
        if public_release.get("date").and_then(|value| value.as_str())
            != Some(release.date.as_str())
        {
            return Err(anyhow!(
                "health.json asset release date mismatch for {version}"
            ));
        }
    }
    let asset_files = require_json_array(health, &["assets", "files"])?;
    let asset_base = manifest.asset_base.as_deref().unwrap_or("/assets/releases");
    let current_asset_files =
        current_asset_file_refs(asset_base, &manifest.assets.current, current_release);
    let current_asset_subjects = current_asset_files
        .iter()
        .map(|file| file.url.as_str())
        .collect::<BTreeSet<_>>();
    let vm_oboms = require_json_array(health, &["evidence", "vm_oboms"])?;
    let host_sboms = require_json_array(health, &["evidence", "host_sboms"])?;
    let host_binary_files = require_json_array(health, &["evidence", "host_binary_files"])?;
    let attestations = require_json_array(health, &["evidence", "attestations"])?;
    let binary_files = manifest
        .binaries
        .releases
        .get(&manifest.binaries.current)
        .map(|release| binary_package_file_refs(&manifest.binaries.current, release))
        .unwrap_or_default();
    let host_package_subjects = binary_files
        .iter()
        .filter(|file| !is_host_sbom_file(&file.name))
        .map(|file| file.url.clone())
        .collect::<BTreeSet<_>>();
    if !binary_files.is_empty() {
        if host_binary_files.is_empty() {
            return Err(anyhow!("health.json host binary files missing"));
        }
        let expects_canonical_host_sbom = attestations.iter().any(|item| {
            item.get("name").and_then(|value| value.as_str())
                == Some("github_attestations_host_sbom")
        });
        if expects_canonical_host_sbom && host_sboms.is_empty() {
            return Err(anyhow!("health.json host SBOM evidence missing"));
        }
        if attestations.is_empty() {
            return Err(anyhow!("health.json binary attestation evidence missing"));
        }
    }
    for expected in &binary_files {
        let public_file = host_binary_files.iter().find(|item| {
            item.get("url").and_then(|value| value.as_str()) == Some(expected.url.as_str())
        });
        let Some(public_file) = public_file else {
            return Err(anyhow!(
                "health.json missing host binary file {}",
                expected.url
            ));
        };
        if public_file.get("name").and_then(|value| value.as_str()) != Some(expected.name.as_str())
        {
            return Err(anyhow!(
                "health.json host binary name mismatch for {}",
                expected.url
            ));
        }
        if public_file.get("sha256").and_then(|value| value.as_str())
            != Some(expected.sha256.as_str())
        {
            return Err(anyhow!(
                "health.json host binary sha256 mismatch for {}",
                expected.url
            ));
        }
        if public_file.get("blake3").and_then(|value| value.as_str())
            != Some(expected.blake3.as_str())
        {
            return Err(anyhow!(
                "health.json host binary blake3 mismatch for {}",
                expected.url
            ));
        }
        if public_file.get("size").and_then(|value| value.as_u64()) != Some(expected.size) {
            return Err(anyhow!(
                "health.json host binary size mismatch for {}",
                expected.url
            ));
        }
        if expected.sha256.len() != 64 || !expected.sha256.chars().all(|ch| ch.is_ascii_hexdigit())
        {
            return Err(anyhow!(
                "channel manifest host binary {} has malformed sha256",
                expected.name
            ));
        }
        if expected.blake3.len() != 64 || !expected.blake3.chars().all(|ch| ch.is_ascii_hexdigit())
        {
            return Err(anyhow!(
                "channel manifest host binary {} has malformed blake3",
                expected.name
            ));
        }
    }
    for sbom in host_sboms {
        let sbom_url = sbom
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json host SBOM evidence missing url"))?;
        if sbom.get("name").and_then(|value| value.as_str()) != Some("capsem-sbom.spdx.json") {
            return Err(anyhow!(
                "health.json host SBOM evidence name mismatch for {sbom_url}"
            ));
        }
        let host_binary = host_binary_files
            .iter()
            .find(|item| item.get("url").and_then(|value| value.as_str()) == Some(sbom_url));
        let Some(host_binary) = host_binary else {
            return Err(anyhow!(
                "health.json host SBOM evidence {sbom_url} missing from host binary files"
            ));
        };
        if host_binary.get("name").and_then(|value| value.as_str()) != Some("capsem-sbom.spdx.json")
        {
            return Err(anyhow!(
                "health.json host SBOM evidence binary file name mismatch for {sbom_url}"
            ));
        }
    }
    let mut saw_host_sbom_attestation = false;
    let mut saw_vm_asset_attestation = false;
    let mut host_sbom_attestation_subjects = BTreeSet::new();
    for attestation in attestations {
        let attestation_name = attestation
            .get("name")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json attestation name missing"))?;
        if let Some((expected_scope, expected_workflow)) =
            expected_attestation_rail(attestation_name)
        {
            let scope = attestation
                .get("scope")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json attestation scope missing"))?;
            if scope != expected_scope {
                return Err(anyhow!(
                    "health.json {} scope mismatch",
                    attestation_rail_label(attestation_name)
                ));
            }
            let workflow = attestation
                .get("workflow")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json attestation workflow missing"))?;
            if workflow != expected_workflow {
                return Err(anyhow!(
                    "health.json {} workflow mismatch",
                    attestation_rail_label(attestation_name)
                ));
            }
        }
        let predicate_type = attestation
            .get("predicate_type")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json attestation predicate_type missing"))?;
        if predicate_type.is_empty() {
            return Err(anyhow!("health.json attestation predicate_type empty"));
        }
        let verify_command = attestation
            .get("verify_command")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json attestation verify_command missing"))?;
        if !verify_command.contains("gh attestation verify") {
            return Err(anyhow!(
                "health.json attestation verify_command must use gh attestation verify"
            ));
        }
        if attestation_name == "github_attestations_host_sbom" {
            saw_host_sbom_attestation = true;
            let predicate_url = attestation
                .get("predicate_url")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    anyhow!("health.json host SBOM attestation predicate_url missing")
                })?;
            if !host_sboms
                .iter()
                .any(|item| item.get("url").and_then(|value| value.as_str()) == Some(predicate_url))
            {
                return Err(anyhow!(
                    "health.json host SBOM attestation predicate {predicate_url} missing from host SBOM evidence"
                ));
            }
        }
        if attestation_name == "github_attestations_vm_assets" {
            let predicate_url = attestation
                .get("predicate_url")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json VM asset attestation predicate_url missing"))?;
            if !vm_oboms.is_empty()
                && !vm_oboms.iter().any(|item| {
                    item.get("url").and_then(|value| value.as_str()) == Some(predicate_url)
                })
            {
                return Err(anyhow!(
                    "health.json VM asset attestation predicate {predicate_url} missing from VM OBOM evidence"
                ));
            }
        }
        let subjects = attestation
            .get("subjects")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("health.json attestation subjects missing"))?;
        if subjects.is_empty() {
            return Err(anyhow!("health.json attestation subjects empty"));
        }
        for subject in subjects {
            let subject_url = subject
                .as_str()
                .ok_or_else(|| anyhow!("health.json attestation subject is not a string"))?;
            if attestation_name == "github_attestations_host_sbom" {
                host_sbom_attestation_subjects.insert(subject_url.to_string());
            }
            let is_host_binary_subject = host_binary_files
                .iter()
                .any(|item| item.get("url").and_then(|value| value.as_str()) == Some(subject_url));
            let is_vm_asset_subject = current_asset_subjects.contains(subject_url);
            if is_vm_asset_subject {
                saw_vm_asset_attestation = true;
            }
            if !is_host_binary_subject && !is_vm_asset_subject {
                return Err(anyhow!(
                    "health.json attestation subject {subject_url} missing from host binary files and VM asset files"
                ));
            }
        }
    }
    if !host_sboms.is_empty() && !saw_host_sbom_attestation {
        return Err(anyhow!(
            "health.json host SBOM attestation evidence missing"
        ));
    }
    for subject in &host_package_subjects {
        if !host_sbom_attestation_subjects.contains(subject) {
            return Err(anyhow!(
                "health.json host SBOM attestation subjects missing {subject}"
            ));
        }
    }
    if !current_asset_subjects.is_empty() && !saw_vm_asset_attestation {
        return Err(anyhow!("health.json VM asset attestation evidence missing"));
    }
    let mut saw_obom = false;
    for (arch, assets) in &current_release.arches {
        for (logical_name, entry) in assets {
            let url = channel_asset_url(
                expected_asset_base,
                &manifest.assets.current,
                arch,
                logical_name,
            );
            let public_file = asset_files.iter().find(|item| {
                item.get("url").and_then(|value| value.as_str()) == Some(url.as_str())
            });
            let Some(public_file) = public_file else {
                return Err(anyhow!("health.json missing asset file {url}"));
            };
            if public_file.get("hash").and_then(|value| value.as_str()) != Some(entry.hash.as_str())
            {
                return Err(anyhow!("health.json asset hash mismatch for {url}"));
            }
            if public_file.get("size").and_then(|value| value.as_u64()) != Some(entry.size) {
                return Err(anyhow!("health.json asset size mismatch for {url}"));
            }
            if logical_name == "obom.cdx.json" {
                saw_obom = true;
                if !vm_oboms.iter().any(|item| {
                    item.get("url").and_then(|value| value.as_str()) == Some(url.as_str())
                }) {
                    return Err(anyhow!("health.json missing VM OBOM evidence {url}"));
                }
                if url.starts_with('/') {
                    let local_path = dist.join(url.trim_start_matches('/'));
                    let bytes = fs::read(&local_path).with_context(|| {
                        format!("read asset channel blob {}", local_path.display())
                    })?;
                    if bytes.len() as u64 != entry.size {
                        return Err(anyhow!(
                            "asset channel blob {} size mismatch",
                            local_path.display()
                        ));
                    }
                    if blake3::hash(&bytes).to_hex().as_str() != entry.hash {
                        return Err(anyhow!(
                            "asset channel blob {} hash mismatch",
                            local_path.display()
                        ));
                    }
                    validate_vm_cyclonedx_obom_bytes(&bytes, &local_path)?;
                }
            } else if url.starts_with('/') {
                let local_path = dist.join(url.trim_start_matches('/'));
                let bytes = fs::read(&local_path)
                    .with_context(|| format!("read asset channel blob {}", local_path.display()))?;
                if bytes.len() as u64 != entry.size {
                    return Err(anyhow!(
                        "asset channel blob {} size mismatch",
                        local_path.display()
                    ));
                }
                if blake3::hash(&bytes).to_hex().as_str() != entry.hash {
                    return Err(anyhow!(
                        "asset channel blob {} hash mismatch",
                        local_path.display()
                    ));
                }
            }
        }
    }
    if !saw_obom {
        return Err(anyhow!(
            "channel manifest current asset release has no VM OBOM evidence"
        ));
    }
    Ok(())
}

fn expected_attestation_rail(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "github_attestations_host" => Some(("host_binaries", ".github/workflows/release.yaml")),
        "github_attestations_host_sbom" => Some(("host_sbom", ".github/workflows/release.yaml")),
        "github_attestations_vm_assets" => {
            Some(("vm_assets", ".github/workflows/release-assets.yaml"))
        }
        _ => None,
    }
}

fn attestation_rail_label(name: &str) -> &'static str {
    match name {
        "github_attestations_host" => "host attestation",
        "github_attestations_host_sbom" => "host SBOM attestation",
        "github_attestations_vm_assets" => "VM asset attestation",
        _ => "attestation",
    }
}

fn require_json_str(
    root: &serde_json::Value,
    path: &[&str],
    expected: &str,
    message: &str,
) -> Result<()> {
    if json_path(root, path).and_then(|value| value.as_str()) != Some(expected) {
        return Err(anyhow!("{message}"));
    }
    Ok(())
}

fn require_json_bool(
    root: &serde_json::Value,
    path: &[&str],
    expected: bool,
    message: &str,
) -> Result<()> {
    if json_path(root, path).and_then(|value| value.as_bool()) != Some(expected) {
        return Err(anyhow!("{message}"));
    }
    Ok(())
}

fn require_json_string(root: &serde_json::Value, path: &[&str]) -> Result<String> {
    json_path(root, path)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("health.json missing {}", path.join(".")))
}

fn require_json_absent(root: &serde_json::Value, path: &[&str], message: &str) -> Result<()> {
    if json_path(root, path).is_some() {
        return Err(anyhow!("{message}"));
    }
    Ok(())
}

fn require_json_null(value: &serde_json::Value, path: &[&str], message: &str) -> Result<()> {
    let actual = value
        .pointer(&format!("/{}", path.join("/")))
        .ok_or_else(|| anyhow!("health.json missing {}", path.join(".")))?;
    if !actual.is_null() {
        return Err(anyhow!("{message}: got {actual}"));
    }
    Ok(())
}

fn require_json_array<'a>(
    root: &'a serde_json::Value,
    path: &[&str],
) -> Result<&'a Vec<serde_json::Value>> {
    json_path(root, path)
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("health.json missing {}", path.join(".")))
}

fn json_path<'a>(root: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    let mut value = root;
    for key in path {
        value = value.get(*key)?;
    }
    Some(value)
}

fn assets_channel_index(
    manifest: &ManifestV2,
    channel: &str,
    generated_at: &str,
    manifest_blake3: &str,
    profiles: AssetsChannelProfilesSummary,
    asset_base: &str,
) -> AssetsChannelIndex {
    let mut arches = BTreeSet::new();
    for release in manifest.assets.releases.values() {
        arches.extend(release.arches.keys().cloned());
    }
    let current_release = manifest.assets.releases.get(&manifest.assets.current);
    let binary_release = manifest.binaries.releases.get(&manifest.binaries.current);
    let current_asset_files = current_release
        .map(|release| current_asset_file_refs(asset_base, &manifest.assets.current, release))
        .unwrap_or_default();
    let vm_oboms = current_asset_files
        .iter()
        .filter(|file| file.logical_name == "obom.cdx.json")
        .cloned()
        .collect();
    let binary_files = binary_release
        .map(|release| binary_package_file_refs(&manifest.binaries.current, release))
        .unwrap_or_default();
    let host_sboms = binary_files
        .iter()
        .filter(|file| is_host_sbom_file(&file.name))
        .cloned()
        .collect();
    let mut attestations = binary_package_attestations(&binary_files);
    attestations.extend(current_asset_attestations(&current_asset_files));
    AssetsChannelIndex {
        schema_version: 1,
        channel: channel.to_string(),
        state: "published".to_string(),
        generated_at: generated_at.to_string(),
        release_site: "https://release.capsem.org/".to_string(),
        summary: "Capsem asset channel generated from assets/manifest.json.".to_string(),
        manifest: format!("/assets/{channel}/manifest.json"),
        asset_base: asset_base.to_string(),
        manifest_blake3: manifest_blake3.to_string(),
        binary_version: manifest.binaries.current.clone(),
        asset_version: manifest.assets.current.clone(),
        asset_state: current_release
            .map(release_state)
            .unwrap_or("missing")
            .to_string(),
        asset_min_binary: current_release.map(|release| release.min_binary.clone()),
        binary_state: binary_release
            .map(release_state)
            .unwrap_or("missing")
            .to_string(),
        asset_releases: manifest.assets.releases.len(),
        asset_release_history: summarize_asset_releases(manifest),
        binary_releases: manifest.binaries.releases.len(),
        arches: arches.into_iter().collect(),
        current_asset_files,
        binary_files,
        host_sboms,
        attestations,
        vm_oboms,
        profiles,
        image_update_state: "not_published".to_string(),
    }
}

fn assets_channel_index_from_graph(
    manifest: &serde_json::Value,
    channel: &str,
    generated_at: &str,
    manifest_blake3: &str,
) -> Result<AssetsChannelIndex> {
    let packages = manifest
        .get("packages")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("graph manifest packages must be an array"))?;
    let profiles = manifest
        .get("profiles")
        .and_then(|value| value.as_object())
        .ok_or_else(|| anyhow!("graph manifest profiles must be an object"))?;
    let binary_version = graph_binary_version(packages);
    let profiles_summary = graph_profiles_summary(profiles)?;
    let current_asset_files = graph_asset_files(profiles)?;
    let vm_oboms = current_asset_files
        .iter()
        .filter(|file| is_vm_obom_asset_file(file))
        .cloned()
        .collect::<Vec<_>>();
    let binary_files = graph_binary_files(packages)?;
    let host_sboms = binary_files
        .iter()
        .filter(|file| is_host_sbom_file(&file.name))
        .cloned()
        .collect();
    let mut attestations = binary_package_attestations(&binary_files);
    attestations.extend(current_asset_attestations(&current_asset_files));
    let arches = current_asset_files
        .iter()
        .map(|file| file.arch.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Ok(AssetsChannelIndex {
        schema_version: 1,
        channel: channel.to_string(),
        state: "published".to_string(),
        generated_at: generated_at.to_string(),
        release_site: "https://release.capsem.org/".to_string(),
        summary: "Capsem asset channel generated from release graph manifest.".to_string(),
        manifest: format!("/assets/{channel}/manifest.json"),
        asset_base: "/profiles/releases".to_string(),
        manifest_blake3: manifest_blake3.to_string(),
        binary_version,
        asset_version: profiles_summary.revision.clone(),
        asset_state: "current".to_string(),
        asset_min_binary: Some(profiles_summary.min_binary.clone()),
        binary_state: if packages.is_empty() {
            "missing"
        } else {
            "current"
        }
        .to_string(),
        asset_releases: 1,
        asset_release_history: vec![AssetsChannelAssetRelease {
            version: profiles_summary.revision.clone(),
            date: generated_at.get(..10).unwrap_or(generated_at).to_string(),
            state: "current".to_string(),
            deprecated: false,
            deprecated_date: None,
            min_binary: profiles_summary.min_binary.clone(),
            arches,
        }],
        binary_releases: if packages.is_empty() { 0 } else { 1 },
        arches: current_asset_files
            .iter()
            .map(|file| file.arch.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        current_asset_files,
        binary_files,
        host_sboms,
        attestations,
        vm_oboms,
        profiles: profiles_summary,
        image_update_state: "not_published".to_string(),
    })
}

fn graph_binary_version(packages: &[serde_json::Value]) -> String {
    packages
        .iter()
        .filter_map(|package| package.get("version").and_then(|value| value.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .next()
        .unwrap_or("not_published")
        .to_string()
}

fn graph_profiles_summary(
    profiles: &serde_json::Map<String, serde_json::Value>,
) -> Result<AssetsChannelProfilesSummary> {
    let profile_ids = profiles.keys().cloned().collect::<Vec<_>>();
    let revision = graph_profile_revision_summary(profiles);
    let min_binary = profiles
        .values()
        .filter_map(|profile| {
            profile
                .get("min_capsem_version")
                .and_then(|value| value.as_str())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .next()
        .unwrap_or("")
        .to_string();
    Ok(AssetsChannelProfilesSummary {
        revision,
        profile_count: profiles.len(),
        profile_ids,
        refresh_policy: "graph".to_string(),
        min_binary,
        requires_newer_binary: false,
    })
}

fn graph_asset_files(
    profiles: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<AssetsChannelAssetFile>> {
    let mut files = Vec::new();
    for profile in profiles.values() {
        let architectures = profile
            .get("architectures")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("graph profile architectures must be an array"))?;
        for arch_doc in architectures {
            let arch = require_json_string(arch_doc, &["architecture"])?;
            for field in ["images", "evidence"] {
                for item in arch_doc
                    .get(field)
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                {
                    let url = require_json_string(item, &["url"])?;
                    let digest = require_json_string(item, &["digest", "blake3"])?;
                    let size = item
                        .get("bytes")
                        .and_then(|value| value.as_u64())
                        .ok_or_else(|| anyhow!("graph asset file bytes missing"))?;
                    let logical_name = item
                        .get("name")
                        .and_then(|value| value.as_str())
                        .or_else(|| item.get("kind").and_then(|value| value.as_str()))
                        .unwrap_or("asset")
                        .to_string();
                    files.push(AssetsChannelAssetFile {
                        arch: arch.clone(),
                        logical_name,
                        url,
                        hash: digest,
                        size,
                    });
                }
            }
        }
    }
    files.sort_by(|left, right| left.url.cmp(&right.url));
    files.dedup_by(|left, right| left.url == right.url);
    Ok(files)
}

fn graph_binary_files(packages: &[serde_json::Value]) -> Result<Vec<AssetsChannelBinaryFile>> {
    let mut files = Vec::new();
    for package in packages {
        files.push(graph_binary_file(package)?);
        for evidence in package
            .get("evidence")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
        {
            files.push(graph_binary_file(evidence)?);
        }
    }
    files.sort_by(|left, right| left.url.cmp(&right.url));
    files.dedup_by(|left, right| left.url == right.url);
    Ok(files)
}

fn graph_binary_file(value: &serde_json::Value) -> Result<AssetsChannelBinaryFile> {
    let name = require_json_string(value, &["name"])?;
    let url = require_json_string(value, &["url"])?;
    let sha256 = require_json_string(value, &["digest", "sha256"])?;
    let blake3 = require_json_string(value, &["digest", "blake3"])?;
    let size = value
        .get("bytes")
        .and_then(|item| item.as_u64())
        .ok_or_else(|| anyhow!("graph binary file bytes missing"))?;
    let binaries = value
        .get("binaries")
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .map(graph_binary_executable)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(AssetsChannelBinaryFile {
        name,
        url,
        sha256,
        blake3,
        size,
        binaries,
    })
}

fn graph_binary_executable(value: &serde_json::Value) -> Result<BinaryExecutable> {
    Ok(BinaryExecutable {
        name: require_json_string(value, &["name"])?,
        description: value
            .get("description")
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .to_string(),
        installed_path: require_json_string(value, &["installed_path"])?,
        size: value
            .get("bytes")
            .and_then(|item| item.as_u64())
            .ok_or_else(|| anyhow!("graph binary bytes missing"))?,
        sha256: require_json_string(value, &["digest", "sha256"])?,
        blake3: require_json_string(value, &["digest", "blake3"])?,
        sbom_component_ref: require_json_string(value, &["sbom_component_ref"])?,
    })
}

fn summarize_asset_releases(manifest: &ManifestV2) -> Vec<AssetsChannelAssetRelease> {
    let mut releases = manifest
        .assets
        .releases
        .iter()
        .map(|(version, release)| AssetsChannelAssetRelease {
            version: version.clone(),
            date: release.date.clone(),
            state: release_state(release).to_string(),
            deprecated: release.deprecated,
            deprecated_date: release.deprecated_date.clone(),
            min_binary: release.min_binary.clone(),
            arches: release.arches.keys().cloned().collect(),
        })
        .collect::<Vec<_>>();
    releases.sort_by(|left, right| right.version.cmp(&left.version));
    releases
}

fn publishable_profiles(
    manifest: &ManifestV2,
    profiles_dir: &Path,
    channel: &str,
    asset_base: &str,
    assets_dir: &Path,
    asset_digest_cache: &mut AssetDigestCache,
    profile_revision_policy: ProfileRevisionPolicyArg,
) -> Result<PublishableProfiles> {
    let current_release = manifest
        .assets
        .releases
        .get(&manifest.assets.current)
        .ok_or_else(|| anyhow!("manifest current asset release is missing"))?;
    let catalog = ProfileCatalog::load_from_dir(profiles_dir)
        .map_err(|error| anyhow!("load profile directory {}: {error}", profiles_dir.display()))?;
    let config_root = profiles_dir.parent().ok_or_else(|| {
        anyhow!(
            "profile directory {} has no config root",
            profiles_dir.display()
        )
    })?;
    let mut profiles = catalog
        .profiles()
        .cloned()
        .map(|profile| {
            publishable_profile_config(profile, config_root, manifest, current_release, asset_base)
        })
        .collect::<Result<Vec<_>>>()?;
    profiles.sort_by(|left, right| left.id.cmp(&right.id));
    let profile_ids = profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    let revision = profile_release_revision(&profiles, profile_revision_policy)?;
    validate_profile_revision_path(&revision)?;
    let refresh_policy = profile_refresh_policy(&profiles);
    let min_binary = current_release.min_binary.clone();
    let mut file_copies = Vec::new();
    let mut graph_profiles = Vec::new();
    let graph_context = ProfileGraphContext {
        channel,
        manifest,
        current_release,
        asset_base,
        assets_dir,
    };
    for profile in &profiles {
        graph_profiles.push(graph_profile_document(
            profile,
            config_root,
            &graph_context,
            &mut file_copies,
            asset_digest_cache,
        )?);
    }
    Ok(PublishableProfiles {
        summary: AssetsChannelProfilesSummary {
            revision,
            profile_count: graph_profiles.len(),
            profile_ids,
            refresh_policy,
            min_binary,
            requires_newer_binary: false,
        },
        profiles: graph_profiles,
        file_copies,
    })
}

fn validate_graph_manifest_version(version: &str) -> Result<()> {
    if version.trim().is_empty() {
        return Err(anyhow!("manifest version must not be empty"));
    }
    if version.contains("+assets.") {
        return Err(anyhow!(
            "manifest version must be independent from asset and binary versions"
        ));
    }
    Ok(())
}

fn render_graph_release_manifest(
    manifest: &ManifestV2,
    channel: &str,
    profiles: &[serde_json::Value],
    _asset_base: &str,
    version: &str,
) -> Result<String> {
    let packages = graph_package_rows(manifest)?;
    let profile_map = profiles
        .iter()
        .map(|profile| {
            let id = profile
                .get("id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("graph profile missing id"))?;
            Ok((id.to_string(), profile.clone()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&serde_json::json!({
            "version": version,
            "channel": channel,
            "status": "current",
            "packages": packages,
            "profiles": profile_map,
        }))
        .context("serialize graph release manifest")?
    ))
}

struct ProfileGraphContext<'a> {
    channel: &'a str,
    manifest: &'a ManifestV2,
    current_release: &'a capsem_core::asset_manager::AssetRelease,
    asset_base: &'a str,
    assets_dir: &'a Path,
}

fn graph_package_rows(manifest: &ManifestV2) -> Result<Vec<serde_json::Value>> {
    let Some(release) = manifest.binaries.releases.get(&manifest.binaries.current) else {
        return Ok(Vec::new());
    };
    let binary_files = binary_package_file_refs(&manifest.binaries.current, release);
    let rows = binary_files
        .iter()
        .filter(|file| !is_host_sbom_file(&file.name) && !is_package_sbom_file(&file.name))
        .map(|file| -> Result<serde_json::Value> {
            let package_kind = package_kind_for_name(&file.name);
            let platform = package_platform_for_kind(package_kind);
            let architecture = release_graph::PackageArchitecture::from_package_name(&file.name)?;
            let package_id = release_graph_id(&file.name);
            let package_sboms = package_sbom_refs(&package_id, &binary_files, release);
            let binaries = file
                .binaries
                .iter()
                .map(|binary| {
                    serde_json::json!({
                        "name": binary.name,
                        "description": binary.description,
                        "version": manifest.binaries.current,
                        "installed_path": binary.installed_path,
                        "platform": platform,
                        "architecture": architecture,
                        "bytes": binary.size,
                        "digest": {
                            "sha256": binary.sha256,
                            "blake3": binary.blake3,
                        },
                        "status": release_state(release),
                        "sbom_component_ref": binary.sbom_component_ref,
                    })
                })
                .collect::<Vec<_>>();
            Ok(serde_json::json!({
                "id": package_id,
                "kind": package_kind,
                "name": file.name,
                "version": manifest.binaries.current,
                "platform": platform,
                "architecture": architecture,
                "url": file.url,
                "bytes": file.size,
                "digest": {
                    "sha256": file.sha256,
                    "blake3": file.blake3,
                },
                "binaries": binaries,
                "evidence": package_sboms,
                "status": release_state(release),
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(rows)
}

fn package_sbom_refs(
    package_id: &str,
    binary_files: &[AssetsChannelBinaryFile],
    release: &capsem_core::asset_manager::BinaryRelease,
) -> Vec<serde_json::Value> {
    let expected = package_sbom_file_name(package_id);
    binary_files
        .iter()
        .filter(|file| file.name == expected)
        .map(|file| {
            serde_json::json!({
                "kind": "sbom",
                "name": file.name,
                "url": file.url,
                "bytes": file.size,
                "digest": {
                    "sha256": file.sha256,
                    "blake3": file.blake3,
                },
                "status": release_state(release),
            })
        })
        .collect()
}

fn graph_profile_document(
    profile: &ProfileConfigFile,
    config_root: &Path,
    context: &ProfileGraphContext<'_>,
    file_copies: &mut Vec<ProfileReleaseFileCopy>,
    asset_digest_cache: &mut AssetDigestCache,
) -> Result<serde_json::Value> {
    let revision = profile.revision.clone();
    let images =
        graph_profile_images(profile, &revision, context, file_copies, asset_digest_cache)?;
    let software = graph_profile_software(profile, &revision, context, asset_digest_cache)?;
    let image_records = images
        .as_array()
        .ok_or_else(|| anyhow!("profile {} image graph is not an array", profile.id))?;
    let mut architectures = Vec::new();
    for image in image_records {
        let arch = image
            .get("architecture")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("profile {} image record missing architecture", profile.id))?;
        let config = graph_profile_config_refs(
            profile,
            config_root,
            context.channel,
            &revision,
            arch,
            file_copies,
        )?;
        let arch_software = software.get(arch).cloned().unwrap_or_default();
        let image_artifacts = image
            .get("artifacts")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let evidence = image
            .get("evidence")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        architectures.push(serde_json::json!({
            "architecture": arch,
            "package_inventory_revision": context.manifest.assets.current,
            "image_revision": context.manifest.assets.current,
            "software": arch_software,
            "config": config,
            "images": image_artifacts,
            "evidence": evidence,
        }));
    }
    Ok(serde_json::json!({
        "id": profile.id,
        "name": profile.name,
        "description": profile.description,
        "version": profile.revision,
        "revision": profile.revision,
        "status": "current",
        "min_capsem_version": context.current_release.min_binary,
        "architectures": architectures,
    }))
}

fn graph_profile_config_refs(
    profile: &ProfileConfigFile,
    config_root: &Path,
    channel: &str,
    revision: &str,
    arch: &str,
    file_copies: &mut Vec<ProfileReleaseFileCopy>,
) -> Result<Vec<serde_json::Value>> {
    let mut files = Vec::new();
    let profile_toml = format!("profiles/{}/profile.toml", profile.id);
    files.push(("profile".to_string(), profile_toml, None));
    for (kind, descriptor) in profile_file_descriptors(profile) {
        files.push((kind.to_string(), descriptor.path.clone(), None));
    }
    if let Some(root_manifest_descriptor) = profile.files.root_manifest.as_ref() {
        let manifest_path = config_root.join(&root_manifest_descriptor.path);
        check_profile_root_manifest(&manifest_path)?;
        let manifest: ProfileRootManifest =
            serde_json::from_slice(&fs::read(&manifest_path).with_context(|| {
                format!("read profile root manifest {}", manifest_path.display())
            })?)
            .with_context(|| format!("parse profile root manifest {}", manifest_path.display()))?;
        let manifest_parent = Path::new(&root_manifest_descriptor.path)
            .parent()
            .ok_or_else(|| anyhow!("profile {} root manifest has no parent path", profile.id))?;
        for entry in manifest.files {
            let relative_path = manifest_parent.join("root").join(&entry.path);
            let relative = relative_path
                .to_str()
                .ok_or_else(|| {
                    anyhow!(
                        "profile {} root payload path is not UTF-8: {}",
                        profile.id,
                        relative_path.display()
                    )
                })?
                .replace(std::path::MAIN_SEPARATOR, "/");
            validate_relative_manifest_path("profile root publication path", &relative)?;
            files.push((
                "root_payload".to_string(),
                relative,
                Some("root-payload".to_string()),
            ));
        }
    }
    files.sort_by(|left, right| left.1.cmp(&right.1));
    files.dedup_by(|left, right| left.1 == right.1);

    let mut rows = Vec::new();
    let mut urls = BTreeMap::new();
    let mut digest_urls = BTreeMap::new();
    for (kind, relative, publication_name) in files {
        let source = config_root.join(&relative);
        let (bytes, digest) = file_digest(&source)?;
        let file_name = match publication_name {
            Some(prefix) => format!(
                "{prefix}-{}",
                digest
                    .get("blake3")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        anyhow!(
                            "profile {} config path lacks BLAKE3 digest: {relative}",
                            profile.id
                        )
                    })?
            ),
            None => Path::new(&relative)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    anyhow!(
                        "profile {} config path has no file name: {relative}",
                        profile.id
                    )
                })?
                .to_string(),
        };
        let identity = (
            bytes,
            digest
                .get("sha256")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            digest
                .get("blake3")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        );
        let proposed_url = profile_release_url(channel, &profile.id, revision, arch, &file_name)?;
        let url = digest_urls
            .entry(identity.clone())
            .or_insert(proposed_url)
            .clone();
        if let Some(previous) = urls.insert(url.clone(), identity.clone()) {
            if previous != identity {
                return Err(anyhow!(
                    "profile {}/{} config publication URL collides: {}",
                    profile.id,
                    arch,
                    url
                ));
            }
        }
        file_copies.push(ProfileReleaseFileCopy {
            source,
            url: url.clone(),
        });
        rows.push(serde_json::json!({
            "kind": kind,
            "path": relative,
            "url": url,
            "bytes": bytes,
            "digest": digest,
            "status": "current",
        }));
    }
    Ok(rows)
}

fn profile_release_url(
    channel: &str,
    profile: &str,
    revision: &str,
    architecture: &str,
    file_name: &str,
) -> Result<String> {
    profile_publication_identity(channel, profile, revision)?;
    for (label, value) in [
        ("profile architecture", architecture),
        ("profile publication file", file_name),
    ] {
        let valid = !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid {
            return Err(anyhow!(
                "{label} cannot form an immutable profile path: {value}"
            ));
        }
    }
    Ok(format!(
        "/profiles/releases/{channel}/{profile}/{revision}/{architecture}/{file_name}"
    ))
}

fn graph_profile_images(
    profile: &ProfileConfigFile,
    revision: &str,
    context: &ProfileGraphContext<'_>,
    file_copies: &mut Vec<ProfileReleaseFileCopy>,
    asset_digest_cache: &mut AssetDigestCache,
) -> Result<serde_json::Value> {
    let mut images = Vec::new();
    for (arch, arch_assets) in &profile.assets.arch {
        let manifest_assets = context.current_release.arches.get(arch).ok_or_else(|| {
            anyhow!(
                "manifest current release {} does not contain profile arch {arch}",
                context.manifest.assets.current
            )
        })?;
        let artifacts = [
            ("kernel", &arch_assets.kernel),
            ("initrd", &arch_assets.initrd),
            ("rootfs", &arch_assets.rootfs),
        ]
        .into_iter()
        .map(|(kind, descriptor)| {
            let entry = manifest_assets.get(&descriptor.name).ok_or_else(|| {
                anyhow!(
                    "manifest current release arch {arch} is missing {}",
                    descriptor.name
                )
            })?;
            let (bytes, digest) = asset_entry_digest(
                context.assets_dir,
                arch,
                &descriptor.name,
                entry,
                asset_digest_cache,
            )?;
            let url = if context.asset_base == "/assets/releases" {
                let url = profile_release_url(
                    context.channel,
                    &profile.id,
                    revision,
                    arch,
                    &descriptor.name,
                )?;
                file_copies.push(ProfileReleaseFileCopy {
                    source: context.assets_dir.join(arch).join(&descriptor.name),
                    url: url.clone(),
                });
                url
            } else {
                channel_asset_url(
                    context.asset_base,
                    &context.manifest.assets.current,
                    arch,
                    &descriptor.name,
                )
            };
            Ok(serde_json::json!({
                "kind": kind,
                "name": descriptor.name,
                "url": url,
                "bytes": bytes,
                "digest": digest,
                "status": "current",
            }))
        })
        .collect::<Result<Vec<_>>>()?;

        let mut evidence = Vec::new();
        for (kind, logical_name) in [
            ("abom", "abom.cdx.json"),
            ("obom", "obom.cdx.json"),
            ("software_inventory", "software-inventory.json"),
        ] {
            if let Some(entry) = manifest_assets.get(logical_name) {
                let (bytes, digest) = asset_entry_digest(
                    context.assets_dir,
                    arch,
                    logical_name,
                    entry,
                    asset_digest_cache,
                )?;
                let url = if context.asset_base == "/assets/releases" {
                    let url = profile_release_url(
                        context.channel,
                        &profile.id,
                        revision,
                        arch,
                        logical_name,
                    )?;
                    file_copies.push(ProfileReleaseFileCopy {
                        source: context.assets_dir.join(arch).join(logical_name),
                        url: url.clone(),
                    });
                    url
                } else {
                    channel_asset_url(
                        context.asset_base,
                        &context.manifest.assets.current,
                        arch,
                        logical_name,
                    )
                };
                evidence.push(serde_json::json!({
                    "kind": kind,
                    "url": url,
                    "bytes": bytes,
                    "digest": digest,
                    "status": "current",
                }));
            }
        }
        images.push(serde_json::json!({
            "architecture": arch,
            "artifacts": artifacts,
            "evidence": evidence,
        }));
    }
    images.sort_by(|left, right| {
        left.get("architecture")
            .and_then(|value| value.as_str())
            .cmp(&right.get("architecture").and_then(|value| value.as_str()))
    });
    Ok(serde_json::Value::Array(images))
}

type AssetDigestCache = BTreeMap<(String, String), (u64, serde_json::Value)>;

fn asset_entry_digest(
    _assets_dir: &Path,
    arch: &str,
    logical_name: &str,
    entry: &capsem_core::asset_manager::AssetEntry,
    cache: &mut AssetDigestCache,
) -> Result<(u64, serde_json::Value)> {
    let cache_key = (arch.to_string(), logical_name.to_string());
    if let Some((bytes, digest)) = cache.get(&cache_key) {
        return Ok((*bytes, digest.clone()));
    }
    if entry.sha256.is_empty() {
        return Err(anyhow!(
            "asset {arch}/{logical_name} manifest entry does not carry sha256"
        ));
    }
    let result = (
        entry.size,
        serde_json::json!({
            "sha256": entry.sha256.clone(),
            "blake3": entry.hash.clone(),
        }),
    );
    cache.insert(cache_key, result.clone());
    Ok(result)
}

fn graph_profile_software(
    profile: &ProfileConfigFile,
    revision: &str,
    context: &ProfileGraphContext<'_>,
    asset_digest_cache: &mut AssetDigestCache,
) -> Result<BTreeMap<String, Vec<serde_json::Value>>> {
    let mut rows: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for arch in profile.assets.arch.keys() {
        let manifest_assets = context.current_release.arches.get(arch).ok_or_else(|| {
            anyhow!(
                "manifest current release {} does not contain profile arch {arch}",
                context.manifest.assets.current
            )
        })?;
        let logical_name = "software-inventory.json";
        let entry = manifest_assets.get(logical_name).ok_or_else(|| {
            anyhow!(
                "manifest current release {} arch {arch} missing software-inventory.json",
                context.manifest.assets.current
            )
        })?;
        asset_entry_digest(
            context.assets_dir,
            arch,
            logical_name,
            entry,
            asset_digest_cache,
        )?;
        let inventory_path = context.assets_dir.join(arch).join(logical_name);
        let inventory_bytes = fs::read(&inventory_path)
            .with_context(|| format!("read {}", inventory_path.display()))?;
        let inventory: serde_json::Value = serde_json::from_slice(&inventory_bytes)
            .with_context(|| format!("parse {}", inventory_path.display()))?;
        if inventory.get("schema").and_then(|value| value.as_str())
            != Some("capsem.profile_software_inventory.v1")
        {
            return Err(anyhow!(
                "{} schema must be capsem.profile_software_inventory.v1",
                inventory_path.display()
            ));
        }
        let packages = inventory
            .get("packages")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("{} missing packages array", inventory_path.display()))?;
        let evidence = if context.asset_base == "/assets/releases" {
            profile_release_url(context.channel, &profile.id, revision, arch, logical_name)?
        } else {
            channel_asset_url(
                context.asset_base,
                &context.manifest.assets.current,
                arch,
                logical_name,
            )
        };
        for package in packages {
            let name = require_json_string_value(package, "name")
                .with_context(|| format!("{} package missing name", inventory_path.display()))?;
            let version = require_json_string_value(package, "version").with_context(|| {
                format!("{name} missing version in {}", inventory_path.display())
            })?;
            if version == "unversioned" {
                return Err(anyhow!(
                    "{name} in {} has unversioned version",
                    inventory_path.display()
                ));
            }
            let source = require_json_string_value(package, "source").with_context(|| {
                format!("{name} missing source in {}", inventory_path.display())
            })?;
            let row_core = serde_json::json!({
                "name": name,
                "version": version,
                "source": source,
                "architecture": arch,
                "evidence": evidence,
            });
            let digest = json_digest(&row_core)?;
            rows.entry(arch.clone())
                .or_default()
                .push(serde_json::json!({
                    "name": name,
                    "version": version,
                    "source": source,
                    "architecture": arch,
                    "digest": digest,
                    "evidence": evidence,
                }));
        }
    }
    for arch_rows in rows.values_mut() {
        arch_rows.sort_by(|left, right| {
            left.get("name")
                .and_then(|value| value.as_str())
                .cmp(&right.get("name").and_then(|value| value.as_str()))
        });
    }
    Ok(rows)
}

fn require_json_string_value<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(|child| child.as_str())
        .filter(|child| !child.is_empty())
        .ok_or_else(|| anyhow!("missing string field {key}"))
}

fn json_digest(value: &serde_json::Value) -> Result<serde_json::Value> {
    let bytes = serde_json::to_vec(value).context("serialize json digest payload")?;
    Ok(serde_json::json!({
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
        "blake3": blake3::hash(&bytes).to_hex().to_string(),
    }))
}

fn profile_file_descriptors(
    profile: &ProfileConfigFile,
) -> Vec<(
    &'static str,
    &capsem_core::net::policy_config::ProfileFileDescriptor,
)> {
    let mut descriptors = Vec::new();
    if let Some(value) = profile.files.enforcement.as_ref() {
        descriptors.push(("enforcement", value));
    }
    if let Some(value) = profile.files.detection.as_ref() {
        descriptors.push(("detection", value));
    }
    if let Some(value) = profile.files.mcp.as_ref() {
        descriptors.push(("mcp", value));
    }
    if let Some(value) = profile.files.apt_packages.as_ref() {
        descriptors.push(("apt_packages", value));
    }
    if let Some(value) = profile.files.python_requirements.as_ref() {
        descriptors.push(("python_requirements", value));
    }
    if let Some(value) = profile.files.python_requirements_lock.as_ref() {
        descriptors.push(("python_requirements_lock", value));
    }
    if let Some(value) = profile.files.npm_packages.as_ref() {
        descriptors.push(("npm_packages", value));
    }
    if let Some(value) = profile.files.npm_package_lock.as_ref() {
        descriptors.push(("npm_package_lock", value));
    }
    if let Some(value) = profile.files.build.as_ref() {
        descriptors.push(("build", value));
    }
    if let Some(value) = profile.files.tips.as_ref() {
        descriptors.push(("tips", value));
    }
    if let Some(value) = profile.files.root_manifest.as_ref() {
        descriptors.push(("root_manifest", value));
    }
    descriptors
}

fn copy_profile_release_files(out_dir: &Path, copies: &[ProfileReleaseFileCopy]) -> Result<()> {
    for copy in copies {
        let dst = out_dir.join(copy.url.trim_start_matches('/'));
        fs::create_dir_all(
            dst.parent()
                .ok_or_else(|| anyhow!("profile release file path has no parent"))?,
        )
        .with_context(|| format!("create parent for {}", dst.display()))?;
        hardlink_or_copy(&copy.source, &dst)?;
    }
    Ok(())
}

fn file_digest(path: &Path) -> Result<(u64, serde_json::Value)> {
    let mut source = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut sha256 = Sha256::new();
    let mut blake3 = blake3::Hasher::new();
    let mut bytes = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = source
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        sha256.update(&buffer[..read]);
        blake3.update(&buffer[..read]);
    }
    Ok((
        bytes,
        serde_json::json!({
            "sha256": format!("{:x}", sha256.finalize()),
            "blake3": blake3.finalize().to_hex().to_string(),
        }),
    ))
}

fn copy_file_with_digest(source: &Path, destination: &Path) -> Result<(u64, serde_json::Value)> {
    hardlink_or_copy(source, destination)?;
    file_digest(destination)
}

/// Stage a file into release output.
///
/// Delegates, because the decision is not "link if you can". Linking a
/// checked-in file into published output makes them one file: this put 48
/// `config/` seeds inside the release channel sharing an inode, where a chmod
/// on the artifact rewrote tracked source and no content digest noticed. See
/// `capsem_core::auditfs`.
fn hardlink_or_copy(source: &Path, destination: &Path) -> Result<()> {
    capsem_core::auditfs::stage(source, destination, &repo_root())
}

/// The checkout this admin invocation is staging from.
fn repo_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn validate_asset_digest(
    arch: &str,
    logical_name: &str,
    entry: &capsem_core::asset_manager::AssetEntry,
    bytes: u64,
    digest: &serde_json::Value,
) -> Result<()> {
    if bytes != entry.size {
        return Err(anyhow!("asset {arch}/{logical_name} byte count mismatch"));
    }
    let actual_blake3 = digest["blake3"].as_str().unwrap_or_default();
    if actual_blake3 != entry.hash {
        return Err(anyhow!("asset {arch}/{logical_name} blake3 mismatch"));
    }
    if !entry.sha256.is_empty() {
        let actual_sha256 = digest["sha256"].as_str().unwrap_or_default();
        if actual_sha256 != entry.sha256 {
            return Err(anyhow!("asset {arch}/{logical_name} sha256 mismatch"));
        }
    }
    Ok(())
}

fn release_graph_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn package_kind_for_name(name: &str) -> &'static str {
    if name.ends_with(".pkg") {
        "macos_pkg"
    } else if name.ends_with(".deb") {
        "debian_package"
    } else {
        "package"
    }
}

fn package_platform_for_kind(kind: &str) -> &'static str {
    match kind {
        "macos_pkg" => "macos",
        "debian_package" => "linux",
        _ => "unknown",
    }
}

fn binary_description_for_name(name: &str) -> &'static str {
    match name {
        "capsem-app" => "Capsem desktop application executable",
        "capsem-tray" => "Capsem tray companion executable",
        "capsem-service" => "Capsem host service executable",
        "capsem-gateway" => "Capsem local gateway executable",
        "capsem-mcp" => "Capsem MCP server executable",
        "capsem-process" => "Capsem guest process bridge executable",
        "capsem" => "Capsem command-line executable",
        _ => "Capsem packaged executable",
    }
}

fn publishable_profile_config(
    mut profile: ProfileConfigFile,
    config_root: &Path,
    manifest: &ManifestV2,
    current_release: &capsem_core::asset_manager::AssetRelease,
    asset_base: &str,
) -> Result<ProfileConfigFile> {
    materialize_profile_file_descriptors(&mut profile, config_root)?;
    profile
        .assets
        .arch
        .retain(|arch, _| current_release.arches.contains_key(arch));
    if profile.assets.arch.is_empty() {
        return Err(anyhow!(
            "manifest current release {} does not contain any arches for profile {}",
            manifest.assets.current,
            profile.id
        ));
    }
    for (arch, arch_assets) in profile.assets.arch.iter_mut() {
        let manifest_assets = current_release.arches.get(arch).ok_or_else(|| {
            anyhow!(
                "manifest current release {} does not contain profile arch {arch}",
                manifest.assets.current
            )
        })?;
        rewrite_publishable_asset_descriptor(
            &manifest.assets.current,
            arch,
            &mut arch_assets.kernel,
            manifest_assets,
            asset_base,
        )?;
        rewrite_publishable_asset_descriptor(
            &manifest.assets.current,
            arch,
            &mut arch_assets.initrd,
            manifest_assets,
            asset_base,
        )?;
        rewrite_publishable_asset_descriptor(
            &manifest.assets.current,
            arch,
            &mut arch_assets.rootfs,
            manifest_assets,
            asset_base,
        )?;
        if let Some(entry) = manifest_assets.get("obom.cdx.json") {
            profile
                .obom
                .get_or_insert_with(|| ProfileObomConfig {
                    format: "cyclonedx-obom.v1".to_string(),
                    arch: BTreeMap::new(),
                })
                .arch
                .insert(
                    arch.clone(),
                    ProfileObomDescriptor {
                        name: "obom.cdx.json".to_string(),
                        url: profile_release_asset_url(
                            asset_base,
                            &manifest.assets.current,
                            arch,
                            "obom.cdx.json",
                        ),
                        hash: format!("blake3:{}", entry.hash),
                        size: entry.size,
                        generator: "remote".to_string(),
                        generator_version: "unknown".to_string(),
                    },
                );
        }
    }
    profile
        .validate()
        .map_err(|error| anyhow!("validate publishable profile {}: {error}", profile.id))?;
    Ok(profile)
}

fn rewrite_publishable_asset_descriptor(
    asset_version: &str,
    arch: &str,
    descriptor: &mut capsem_core::net::policy_config::ProfileAssetDescriptor,
    manifest_assets: &std::collections::HashMap<String, capsem_core::asset_manager::AssetEntry>,
    asset_base: &str,
) -> Result<()> {
    let entry = manifest_assets.get(&descriptor.name).ok_or_else(|| {
        anyhow!(
            "manifest current release arch {arch} is missing {}",
            descriptor.name
        )
    })?;
    descriptor.url = profile_release_asset_url(asset_base, asset_version, arch, &descriptor.name);
    descriptor.hash = Some(format!("blake3:{}", entry.hash));
    descriptor.size = Some(entry.size);
    Ok(())
}

fn channel_asset_url(
    asset_base: &str,
    asset_version: &str,
    arch: &str,
    logical_name: &str,
) -> String {
    if asset_base.starts_with('/') {
        return format!(
            "{}/{asset_version}/{arch}-{logical_name}",
            asset_base.trim_end_matches('/')
        );
    }
    capsem_core::asset_manager::asset_download_url_with_base(
        asset_base,
        asset_version,
        arch,
        logical_name,
    )
}

fn profile_release_asset_url(
    asset_base: &str,
    asset_version: &str,
    arch: &str,
    logical_name: &str,
) -> String {
    if asset_base.starts_with('/') {
        return format!(
            "https://release.capsem.org{}",
            channel_asset_url(asset_base, asset_version, arch, logical_name)
        );
    }
    channel_asset_url(asset_base, asset_version, arch, logical_name)
}

fn validate_profile_revision_path(revision: &str) -> Result<()> {
    if revision.is_empty()
        || !revision
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(anyhow!(
            "profile revision must be URL-path safe: {revision}"
        ));
    }
    Ok(())
}

fn profile_release_revision(
    profiles: &[ProfileConfigFile],
    policy: ProfileRevisionPolicyArg,
) -> Result<String> {
    for profile in profiles {
        validate_profile_revision_path(&profile.revision)
            .with_context(|| format!("profile {} declares an unsafe revision", profile.id))?;
        let strict = release_graph::parse_profile_revision(&profile.revision);
        if strict.is_err()
            && (policy == ProfileRevisionPolicyArg::Strict
                || !release_graph::is_legacy_profile_revision(&profile.revision))
        {
            strict
                .with_context(|| format!("profile {} declares an unusable revision", profile.id))?;
        }
    }
    let mut revisions = profiles
        .iter()
        .map(|profile| profile.revision.as_str())
        .collect::<BTreeSet<_>>();
    if revisions.len() == 1 {
        let revision = revisions
            .pop_first()
            .ok_or_else(|| anyhow!("profile revision set is empty"))?;
        return Ok(revision.to_string());
    }
    let hash = profile_config_set_hash(profiles)?;
    Ok(format!("profiles-{}", &hash[..16]))
}

fn profile_refresh_policy(profiles: &[ProfileConfigFile]) -> String {
    let policies = profiles
        .iter()
        .map(|profile| profile.refresh_policy.as_str())
        .collect::<BTreeSet<_>>();
    if policies.len() == 1 {
        policies.into_iter().next().unwrap_or("mixed").to_string()
    } else {
        "mixed".to_string()
    }
}

fn profile_config_set_hash(profiles: &[ProfileConfigFile]) -> Result<String> {
    let bytes = serde_json::to_vec(profiles).context("serialize profile set for hashing")?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn release_state<T: ReleaseDeprecated>(release: &T) -> &'static str {
    if release.is_deprecated() {
        "deprecated"
    } else {
        "current"
    }
}

trait ReleaseDeprecated {
    fn is_deprecated(&self) -> bool;
}

impl ReleaseDeprecated for capsem_core::asset_manager::AssetRelease {
    fn is_deprecated(&self) -> bool {
        self.deprecated
    }
}

impl ReleaseDeprecated for capsem_core::asset_manager::BinaryRelease {
    fn is_deprecated(&self) -> bool {
        self.deprecated
    }
}

fn current_asset_file_refs(
    asset_base: &str,
    asset_version: &str,
    release: &capsem_core::asset_manager::AssetRelease,
) -> Vec<AssetsChannelAssetFile> {
    let mut files = Vec::new();
    for (arch, assets) in &release.arches {
        for (logical_name, entry) in assets {
            files.push(AssetsChannelAssetFile {
                arch: arch.clone(),
                logical_name: logical_name.clone(),
                url: channel_asset_url(asset_base, asset_version, arch, logical_name),
                hash: entry.hash.clone(),
                size: entry.size,
            });
        }
    }
    files.sort_by(|left, right| {
        left.arch
            .cmp(&right.arch)
            .then_with(|| left.logical_name.cmp(&right.logical_name))
    });
    files
}

fn binary_package_file_refs(
    binary_version: &str,
    release: &capsem_core::asset_manager::BinaryRelease,
) -> Vec<AssetsChannelBinaryFile> {
    let base = capsem_core::asset_manager::release_url(binary_version);
    let mut files = release
        .files
        .iter()
        .map(|file| AssetsChannelBinaryFile {
            name: file.name.clone(),
            url: format!("{}/{}", base.trim_end_matches('/'), file.name),
            sha256: file.sha256.clone(),
            blake3: file.blake3.clone(),
            size: file.size,
            binaries: file.binaries.clone(),
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.name.cmp(&right.name));
    files
}

fn binary_package_attestations(files: &[AssetsChannelBinaryFile]) -> Vec<AssetsChannelAttestation> {
    if files.is_empty() {
        return Vec::new();
    }
    let host_subjects = files
        .iter()
        .filter(|file| !is_host_sbom_file(&file.name) && !is_package_sbom_file(&file.name))
        .map(|file| file.url.clone())
        .collect::<Vec<_>>();
    let sbom_subjects = files
        .iter()
        .filter(|file| is_host_sbom_file(&file.name))
        .map(|file| file.url.clone())
        .collect::<Vec<_>>();
    let mut attestations = Vec::new();
    if !host_subjects.is_empty() {
        attestations.push(AssetsChannelAttestation {
            name: "github_attestations_host".to_string(),
            scope: "host_binaries".to_string(),
            workflow: ".github/workflows/release.yaml".to_string(),
            predicate_type: "https://slsa.dev/provenance/v1".to_string(),
            predicate_url: None,
            verify_command: "gh attestation verify <subject-url> --owner google".to_string(),
            subjects: host_subjects.clone(),
        });
    }
    if let (Some(sbom_subject), false) = (sbom_subjects.first(), host_subjects.is_empty()) {
        attestations.push(AssetsChannelAttestation {
            name: "github_attestations_host_sbom".to_string(),
            scope: "host_sbom".to_string(),
            workflow: ".github/workflows/release.yaml".to_string(),
            predicate_type: "https://spdx.dev/Document/v2.3".to_string(),
            predicate_url: Some(sbom_subject.clone()),
            verify_command: "gh attestation verify <subject-url> --owner google".to_string(),
            subjects: host_subjects,
        });
    }
    attestations
}

fn current_asset_attestations(files: &[AssetsChannelAssetFile]) -> Vec<AssetsChannelAttestation> {
    if files.is_empty() {
        return Vec::new();
    }
    let subjects = files
        .iter()
        .map(|file| file.url.clone())
        .collect::<Vec<_>>();
    let predicate_url = files
        .iter()
        .find(|file| is_vm_obom_asset_file(file))
        .map(|file| file.url.clone());
    vec![AssetsChannelAttestation {
        name: "github_attestations_vm_assets".to_string(),
        scope: "vm_assets".to_string(),
        workflow: ".github/workflows/release-assets.yaml".to_string(),
        predicate_type: "https://slsa.dev/provenance/v1".to_string(),
        predicate_url,
        verify_command: "gh attestation verify <subject-url> --owner google".to_string(),
        subjects,
    }]
}

fn is_vm_obom_asset_file(file: &AssetsChannelAssetFile) -> bool {
    file.logical_name == "obom"
        || file.logical_name == "obom.cdx.json"
        || file.url.ends_with("/obom.cdx.json")
        || file.url.ends_with("-obom.cdx.json")
}

fn render_assets_channels_catalog(
    existing_catalog_path: &Path,
    index: &AssetsChannelIndex,
    manifest_version: &str,
    manifest_url: &str,
    manifest_sha256: &str,
    manifest_blake3: &str,
) -> Result<String> {
    let mut catalog = if existing_catalog_path.exists() {
        serde_json::from_str::<AssetsChannelsCatalog>(
            &fs::read_to_string(existing_catalog_path)
                .with_context(|| format!("read {}", existing_catalog_path.display()))?,
        )
        .with_context(|| format!("parse {}", existing_catalog_path.display()))?
    } else {
        AssetsChannelsCatalog {
            version: 1,
            generated_at: index.generated_at.clone(),
            release_site: index.release_site.clone(),
            channels: BTreeMap::new(),
        }
    };
    catalog.version = 1;
    catalog.generated_at = index.generated_at.clone();
    catalog.release_site = index.release_site.clone();
    catalog.channels.insert(
        index.channel.clone(),
        AssetsChannelsCatalogChannel {
            label: title_case_channel(&index.channel),
            manifests: vec![AssetsChannelsCatalogManifest {
                version: manifest_version.to_string(),
                status: "current".to_string(),
                url: manifest_url.to_string(),
                digest: AssetsChannelsCatalogDigest {
                    sha256: manifest_sha256.to_string(),
                    blake3: manifest_blake3.to_string(),
                },
            }],
        },
    );
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&catalog).context("serialize channels catalog")?
    ))
}

fn render_assets_channel_health(index: &AssetsChannelIndex) -> Result<String> {
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "capsem.assets_channel.health.v1",
            "ok": true,
            "channel": index.channel,
            "state": index.state,
            "generated_at": index.generated_at,
            "release_site": index.release_site,
            "manifest_blake3": index.manifest_blake3,
            "urls": {
                "index": "/index.html",
                "health": "/health.json",
                "manifest": index.manifest,
                "asset_base": index.asset_base,
            },
            "current": {
                "binary": index.binary_version,
                "assets": index.asset_version,
            },
            "binary": {
                "version": index.binary_version,
                "state": index.binary_state,
                "files": index.binary_files,
            },
            "assets": {
                "version": index.asset_version,
                "state": index.asset_state,
                "compatibility": {
                    "binary": index.binary_version,
                    "min_binary": index.asset_min_binary,
                },
                "requires_newer": {
                    "binary": false,
                },
                "files": index.current_asset_files,
            },
            "asset_releases": index.asset_release_history,
                "profiles": {
                    "revision": index.profiles.revision,
                    "state": "current",
                    "source": "manifest.profiles",
                    "profile_count": index.profiles.profile_count,
                    "profile_ids": index.profiles.profile_ids,
                    "refresh_policy": index.profiles.refresh_policy,
                    "min_binary": index.profiles.min_binary,
                    "requires_newer_binary": index.profiles.requires_newer_binary,
                },
            "updates": {
                "binary": {
                    "latest": index.binary_version,
                    "current": index.binary_version,
                    "state": index.binary_state,
                    "source": "manifest.binaries.current",
                    "files": index.binary_files,
                },
                "assets": {
                    "latest": index.asset_version,
                    "current": index.asset_version,
                    "state": index.asset_state,
                    "source": "manifest.assets.current",
                    "manifest": index.manifest,
                    "asset_base": index.asset_base,
                    "compatibility": {
                        "binary": index.binary_version,
                        "min_binary": index.asset_min_binary,
                    },
                    "requires_newer": {
                        "binary": false,
                    },
                },
                "profiles": {
                    "latest": index.profiles.revision,
                    "current": index.profiles.revision,
                    "state": "current",
                    "source": "manifest.profiles",
                    "profile_count": index.profiles.profile_count,
                    "profile_ids": index.profiles.profile_ids,
                    "refresh_policy": index.profiles.refresh_policy,
                    "min_binary": index.profiles.min_binary,
                    "requires_newer_binary": index.profiles.requires_newer_binary,
                },
                "images": {
                    "latest": serde_json::Value::Null,
                    "current": serde_json::Value::Null,
                    "state": index.image_update_state,
                    "source": "manifest.profiles.images",
                },
            },
            "evidence": {
                "vm_oboms": index.vm_oboms,
                "host_sboms": index.host_sboms,
                "host_binary_files": index.binary_files,
                "attestations": index.attestations,
            },
            "manifest": index.manifest,
        }))?
    ))
}

#[cfg(test)]
fn render_assets_channel_headers(channel: &str) -> String {
    render_assets_channel_headers_for_channels(&[channel.to_string()])
}

fn render_assets_channel_headers_for_dist(
    out_dir: &Path,
    fallback_channel: &str,
) -> Result<String> {
    let channels_path = out_dir.join("channels.json");
    let channels = if channels_path.exists() {
        let catalog: AssetsChannelsCatalog = serde_json::from_str(
            &fs::read_to_string(&channels_path)
                .with_context(|| format!("read {}", channels_path.display()))?,
        )
        .with_context(|| format!("parse {}", channels_path.display()))?;
        catalog.channels.keys().cloned().collect::<Vec<_>>()
    } else {
        vec![fallback_channel.to_string()]
    };
    Ok(render_assets_channel_headers_for_channels(&channels))
}

fn render_assets_channel_headers_for_channels(channels: &[String]) -> String {
    let mut lines = vec![
        "/".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
        "/index.html".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
        "/channels.json".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
        "/health.json".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
    ];
    for channel in channels {
        lines.push(format!("/assets/{channel}/*"));
        lines.push("  Cache-Control: no-cache, must-revalidate".to_string());
    }
    lines.extend([
        "/assets/releases/*".to_string(),
        "  Cache-Control: public, max-age=31536000, immutable".to_string(),
        "/profiles/releases/*".to_string(),
        "  Cache-Control: public, max-age=31536000, immutable".to_string(),
        "/robots.txt".to_string(),
        "  Cache-Control: public, max-age=3600".to_string(),
        "".to_string(),
    ]);
    lines.join("\n")
}

fn title_case_channel(channel: &str) -> String {
    let mut chars = channel.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn validate_channel_name(channel: &str) -> Result<()> {
    let valid = !channel.is_empty()
        && channel
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !valid {
        return Err(anyhow!("invalid asset channel name: {channel}"));
    }
    Ok(())
}

fn profile_publication_identity(channel: &str, profile: &str, revision: &str) -> Result<String> {
    validate_channel_name(channel)?;
    for (label, value) in [("profile", profile), ("profile revision", revision)] {
        let valid = !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid {
            return Err(anyhow!(
                "{label} cannot form an immutable publication identity: {value}"
            ));
        }
    }
    Ok(format!("profile-{channel}-{profile}-{revision}"))
}

fn current_utc_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .replace_microsecond(0)
        .context("truncate current timestamp")?
        .format(&Rfc3339)
        .context("format current timestamp")
}

fn current_utc_date() -> Result<String> {
    let timestamp = current_utc_rfc3339()?;
    timestamp
        .get(..10)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("current UTC timestamp was shorter than a date"))
}

fn is_host_sbom_file(name: &str) -> bool {
    name == "capsem-sbom.spdx.json"
}

fn is_package_sbom_file(name: &str) -> bool {
    name.ends_with("-sbom.spdx.json") && !is_host_sbom_file(name)
}

fn package_sbom_file_name(package_id: &str) -> String {
    format!("{package_id}-sbom.spdx.json")
}

fn validate_host_spdx_sbom_bytes(bytes: &[u8], path: &Path) -> Result<()> {
    let document: serde_json::Value = serde_json::from_slice(bytes)
        .with_context(|| format!("parse host SPDX SBOM {}", path.display()))?;
    let spdx_version = document
        .get("spdxVersion")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("{} spdxVersion missing", path.display()))?;
    if spdx_version != "SPDX-2.3" {
        return Err(anyhow!(
            "{} spdxVersion mismatch: expected SPDX-2.3, got {spdx_version}",
            path.display()
        ));
    }
    if let Some(files) = document.get("files") {
        let files = files
            .as_array()
            .ok_or_else(|| anyhow!("{} SPDX files must be an array", path.display()))?;
        for file in files {
            let spdx_id = file
                .get("SPDXID")
                .and_then(|value| value.as_str())
                .unwrap_or("<unknown>");
            let checksums = file
                .get("checksums")
                .and_then(|value| value.as_array())
                .ok_or_else(|| {
                    anyhow!(
                        "{} SPDX file {spdx_id} missing checksums with SHA256",
                        path.display()
                    )
                })?;
            let has_sha256 = checksums.iter().any(|checksum| {
                checksum
                    .get("algorithm")
                    .and_then(|value| value.as_str())
                    .is_some_and(|algorithm| algorithm.eq_ignore_ascii_case("SHA256"))
                    && checksum
                        .get("checksumValue")
                        .and_then(|value| value.as_str())
                        .is_some_and(|value| {
                            value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
                        })
            });
            if !has_sha256 {
                return Err(anyhow!(
                    "{} SPDX file {spdx_id} missing SHA256 checksum",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn validate_vm_cyclonedx_obom_bytes(bytes: &[u8], path: &Path) -> Result<()> {
    let document: serde_json::Value = serde_json::from_slice(bytes)
        .with_context(|| format!("parse VM CycloneDX OBOM {}", path.display()))?;
    let bom_format = document
        .get("bomFormat")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("VM OBOM evidence bomFormat missing: {}", path.display()))?;
    if bom_format != "CycloneDX" {
        return Err(anyhow!(
            "VM OBOM evidence bomFormat mismatch: expected CycloneDX, got {bom_format}"
        ));
    }
    Ok(())
}

fn is_host_package_file(name: &str) -> bool {
    name.ends_with(".pkg") || name.ends_with(".deb")
}

fn host_package_name_matches_version(name: &str, version: &str) -> bool {
    name == format!("Capsem-{version}.pkg")
        || (name.starts_with(&format!("Capsem_{version}_")) && name.ends_with(".deb"))
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn image_build_workspace_path(source_profile: &ProfileConfigFile, arch: Option<&str>) -> PathBuf {
    PathBuf::from("target")
        .join("image-workspace")
        .join(&source_profile.id)
        .join(arch.unwrap_or("all"))
}

fn image_build_command(args: ImageBuildArgs) -> Result<()> {
    let source_profile = load_profile(&args.profile)?;
    let workspace = image_build_workspace_path(&source_profile, args.arch.as_deref());
    let workspace_report = materialize_image_workspace(&ImageWorkspaceArgs {
        profile: args.profile.clone(),
        config_root: args.config_root.clone(),
        guest_dir: args.guest_dir.clone(),
        output: workspace,
        arch: args.arch.clone(),
        json: true,
    })?;
    let plan = image_build_plan(&ImageBuildArgs {
        profile: PathBuf::from(&workspace_report.profile_path),
        config_root: PathBuf::from(&workspace_report.config_root),
        guest_dir: PathBuf::from(&workspace_report.workspace).join("guest"),
        output: args.output.clone(),
        arch: args.arch.clone(),
        template: args.template,
        clean: args.clean,
        json: args.json,
    })?;
    if plan.clean {
        clean_image_outputs(&plan)?;
    }
    for command in &plan.commands {
        run_command(command)?;
    }
    print_image_build_plan(&plan, args.json)?;
    Ok(())
}

fn image_workspace_command(args: ImageWorkspaceArgs) -> Result<()> {
    let json = args.json;
    let report = materialize_image_workspace(&args)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "profile {} rev {} -> {}",
            report.profile_id, report.profile_revision, report.workspace
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfilePinMode {
    Source,
    Materialized,
}

fn validate_profile(path: &Path, config_root: Option<&Path>) -> Result<ProfileValidationReport> {
    validate_profile_with_pin_mode(path, config_root, ProfilePinMode::Source)
}

fn validate_materialized_profile(
    path: &Path,
    config_root: Option<&Path>,
) -> Result<ProfileValidationReport> {
    validate_profile_with_pin_mode(path, config_root, ProfilePinMode::Materialized)
}

fn validate_profile_with_pin_mode(
    path: &Path,
    config_root: Option<&Path>,
    pin_mode: ProfilePinMode,
) -> Result<ProfileValidationReport> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read profile {}", path.display()))?;
    let profile: ProfileConfigFile =
        toml::from_str(&content).with_context(|| format!("parse profile {}", path.display()))?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", path.display()))?;
    match pin_mode {
        ProfilePinMode::Source => ensure_source_profile_unpinned(&profile, path)?,
        ProfilePinMode::Materialized => ensure_materialized_profile_pinned(&profile, path)?,
    }

    let config_root = match config_root {
        Some(root) => root.to_path_buf(),
        None => infer_config_root(path)?,
    };
    let rules = profile
        .compile_security_rule_set_from_files(&config_root, SecurityRuleSource::User)
        .map_err(|error| {
            anyhow!(
                "compile profile rule files for {} with config root {}: {error}",
                path.display(),
                config_root.display()
            )
        })?;

    Ok(ProfileValidationReport {
        schema: "capsem.admin.profile_validation.v1",
        ok: true,
        profile_id: profile.id,
        path: path.display().to_string(),
        config_root: config_root.display().to_string(),
        compiled_rules: rules.rules().len(),
    })
}

fn ensure_source_profile_unpinned(profile: &ProfileConfigFile, path: &Path) -> Result<()> {
    let location = path.display();
    if profile.obom.is_some() {
        return Err(anyhow!(
            "source profile {location} must not contain generated obom pins"
        ));
    }
    for (arch, assets) in &profile.assets.arch {
        for (kind, descriptor) in [
            ("kernel", &assets.kernel),
            ("initrd", &assets.initrd),
            ("rootfs", &assets.rootfs),
        ] {
            if descriptor.hash.is_some() || descriptor.size.is_some() {
                return Err(anyhow!(
                    "source profile {location} must not contain hash/size pins for assets.arch.{arch}.{kind}"
                ));
            }
        }
    }
    for (kind, descriptor) in profile.files.iter() {
        if descriptor.hash.is_some() || descriptor.size.is_some() {
            return Err(anyhow!(
                "source profile {location} must not contain hash/size pins for files.{kind}"
            ));
        }
    }
    Ok(())
}

fn ensure_materialized_profile_pinned(profile: &ProfileConfigFile, path: &Path) -> Result<()> {
    let location = path.display();
    for (arch, assets) in &profile.assets.arch {
        for (kind, descriptor) in [
            ("kernel", &assets.kernel),
            ("initrd", &assets.initrd),
            ("rootfs", &assets.rootfs),
        ] {
            descriptor
                .resolved_hash(&format!("profile.assets.arch.{arch}.{kind}"))
                .map_err(|error| anyhow!("materialized profile {location}: {error}"))?;
            descriptor
                .resolved_size(&format!("profile.assets.arch.{arch}.{kind}"))
                .map_err(|error| anyhow!("materialized profile {location}: {error}"))?;
        }
    }
    for (kind, descriptor) in profile.files.iter() {
        descriptor
            .resolved_hash(&format!("profile.files.{kind}"))
            .map_err(|error| anyhow!("materialized profile {location}: {error}"))?;
        descriptor
            .resolved_size(&format!("profile.files.{kind}"))
            .map_err(|error| anyhow!("materialized profile {location}: {error}"))?;
    }
    Ok(())
}

fn check_profile(args: &ProfileCheckArgs) -> Result<ProfileCheckReport> {
    let validation = validate_profile(&args.path, args.config_root.as_deref())?;
    let profile = load_profile(&args.path)?;
    let config_root = match &args.config_root {
        Some(root) => root.clone(),
        None => infer_config_root(&args.path)?,
    };
    let assets: Vec<LocalAssetCheckReport> = Vec::new();
    let arches = selected_profile_arches(&profile, args.arch.as_deref())?;
    for arch in arches {
        let arch_assets = profile
            .assets
            .arch
            .get(&arch)
            .expect("arch came from selected_profile_arches");
        for descriptor in [
            &arch_assets.kernel,
            &arch_assets.initrd,
            &arch_assets.rootfs,
        ] {
            if descriptor.url.starts_with("file://")
                && (descriptor.hash.is_some() || descriptor.size.is_some())
            {
                return Err(anyhow!(
                    "source profile {} must not contain file:// asset pins for {arch}/{}",
                    args.path.display(),
                    descriptor.name
                ));
            }
        }
    }
    fail_if_local_asset_checks_failed("profile file:// asset pin check", &assets)?;
    let profile_files = check_profile_payload_files(&profile, &config_root)?;
    fail_if_local_asset_checks_failed("profile payload file pin check", &profile_files)?;
    Ok(ProfileCheckReport {
        schema: "capsem.admin.profile_check.v1",
        ok: true,
        validation,
        assets,
        profile_files,
    })
}

fn check_profile_payload_files(
    profile: &ProfileConfigFile,
    config_root: &Path,
) -> Result<Vec<LocalAssetCheckReport>> {
    let mut reports = Vec::new();
    for (kind, descriptor) in profile.files.iter() {
        let path = config_root.join(&descriptor.path);
        let present = path.is_file();
        reports.push(LocalAssetCheckReport {
            arch: "profile".to_string(),
            logical_name: kind.to_string(),
            expected_hash: "unpinned-source".to_string(),
            expected_size: 0,
            path: Some(path.display().to_string()),
            present,
            size_ok: None,
            blake3_ok: None,
        });
        if !present {
            continue;
        }
        validate_profile_payload_semantics(kind, &path)?;
        if kind == "root_manifest" {
            reports.extend(check_profile_root_manifest(&path)?);
        }
    }
    Ok(reports)
}

fn validate_profile_payload_semantics(kind: &str, path: &Path) -> Result<()> {
    match kind {
        "mcp" => validate_profile_mcp_file(path),
        "apt_packages" | "python_requirements" | "npm_packages" => {
            read_profile_package_lines(path).map(|_| ())
        }
        "python_requirements_lock" => validate_python_requirements_lock(path, None).map(|_| ()),
        "npm_package_lock" => validate_npm_package_lock(path, None).map(|_| ()),
        _ => Ok(()),
    }
}

fn normalized_python_name(name: &str) -> String {
    name.to_ascii_lowercase().replace(['_', '.'], "-")
}

fn exact_python_dependencies(packages: &[String]) -> Result<BTreeMap<String, String>> {
    packages
        .iter()
        .map(|package| {
            let (name, version) = package.split_once("==").ok_or_else(|| {
                anyhow!("Python requirement {package} must select one exact version")
            })?;
            if name.is_empty()
                || version.is_empty()
                || version.contains(['=', ';', '@'])
                || version.contains(char::is_whitespace)
            {
                return Err(anyhow!(
                    "Python requirement {package} must select one exact version"
                ));
            }
            Ok((normalized_python_name(name), version.to_string()))
        })
        .collect()
}

fn validate_python_requirements_lock(
    path: &Path,
    expected: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, String>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read Python requirements lock {}", path.display()))?;
    let mut dependencies = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut current_hashed = false;
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        if line.starts_with(char::is_whitespace) {
            if line.trim_start().starts_with("--hash=sha256:") {
                current_hashed = true;
            }
            continue;
        }
        if current.is_some() && !current_hashed {
            return Err(anyhow!(
                "Python requirements lock {} entry {} has no SHA-256 hash",
                path.display(),
                current.as_deref().unwrap_or_default()
            ));
        }
        let (name, version) = line
            .trim_end_matches(" \\")
            .split_once("==")
            .ok_or_else(|| anyhow!("Python lock entry {line} is not exact"))?;
        let normalized = normalized_python_name(name);
        dependencies.insert(normalized.clone(), version.to_string());
        current = Some(normalized);
        current_hashed = line.contains("--hash=sha256:");
    }
    if dependencies.is_empty() {
        return Err(anyhow!(
            "Python requirements lock {} must contain exact requirements",
            path.display()
        ));
    }
    if !current_hashed {
        return Err(anyhow!(
            "Python requirements lock {} entry {} has no SHA-256 hash",
            path.display(),
            current.as_deref().unwrap_or_default()
        ));
    }
    if expected.is_some_and(|wanted| {
        wanted
            .iter()
            .any(|(name, version)| dependencies.get(name) != Some(version))
    }) {
        return Err(anyhow!(
            "Python requirements lock {} does not match the profile's exact direct packages",
            path.display()
        ));
    }
    Ok(dependencies)
}

fn exact_npm_dependencies(packages: &[String]) -> Result<BTreeMap<String, String>> {
    packages
        .iter()
        .map(|package| {
            let (name, version) = package
                .rsplit_once('@')
                .ok_or_else(|| anyhow!("npm package {package} must select one exact version"))?;
            if name.is_empty()
                || version.is_empty()
                || version
                    .chars()
                    .next()
                    .is_some_and(|prefix| matches!(prefix, '^' | '~' | '>' | '<' | '='))
                || version.contains(char::is_whitespace)
            {
                return Err(anyhow!(
                    "npm package {package} must select one exact version"
                ));
            }
            Ok((name.to_string(), version.to_string()))
        })
        .collect()
}

fn validate_npm_package_lock(
    path: &Path,
    expected: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, String>> {
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read npm lock {}", path.display()))?,
    )
    .with_context(|| format!("parse npm lock {}", path.display()))?;
    if value
        .get("lockfileVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(3)
    {
        return Err(anyhow!(
            "npm lock {} must use lockfileVersion 3",
            path.display()
        ));
    }
    let packages = value
        .get("packages")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow!("npm lock {} has no packages object", path.display()))?;
    let root = packages
        .get("")
        .and_then(|entry| entry.get("dependencies"))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow!("npm lock {} has no root dependencies", path.display()))?;
    let dependencies: BTreeMap<String, String> = root
        .iter()
        .map(|(name, version)| {
            let version = version
                .as_str()
                .ok_or_else(|| anyhow!("npm lock dependency {name} is not a string"))?;
            Ok((name.clone(), version.to_string()))
        })
        .collect::<Result<_>>()?;
    if expected.is_some_and(|wanted| wanted != &dependencies) {
        return Err(anyhow!(
            "npm lock {} does not match the profile's exact direct packages",
            path.display()
        ));
    }
    for (name, package) in packages.iter().filter(|(name, _)| !name.is_empty()) {
        let integrity = package
            .get("integrity")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !integrity.starts_with("sha512-") {
            return Err(anyhow!(
                "npm lock {} package {name} has no SHA-512 integrity",
                path.display()
            ));
        }
    }
    Ok(dependencies)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileMcpJsonConfig {
    #[serde(rename = "mcpServers")]
    mcp_servers: BTreeMap<String, serde_json::Value>,
}

fn validate_profile_mcp_file(path: &Path) -> Result<()> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read profile MCP config {}", path.display()))?;
    let config: ProfileMcpJsonConfig = serde_json::from_str(&content)
        .with_context(|| format!("parse profile MCP config {}", path.display()))?;
    if config.mcp_servers.is_empty() {
        return Err(anyhow!(
            "profile MCP config {} must declare at least one server",
            path.display()
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRootManifest {
    format: String,
    files: Vec<ProfileRootManifestFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRootManifestFile {
    path: String,
    hash: String,
    size: u64,
}

fn check_profile_root_manifest(path: &Path) -> Result<Vec<LocalAssetCheckReport>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read profile root manifest {}", path.display()))?;
    let manifest: ProfileRootManifest = serde_json::from_str(&content)
        .with_context(|| format!("parse profile root manifest {}", path.display()))?;
    if manifest.format != "capsem.profile-root.v1" {
        return Err(anyhow!(
            "profile root manifest {} has unsupported format {}",
            path.display(),
            manifest.format
        ));
    }
    if manifest.files.is_empty() {
        return Err(anyhow!(
            "profile root manifest {} must list at least one file",
            path.display()
        ));
    }
    let root_dir = path
        .parent()
        .ok_or_else(|| anyhow!("profile root manifest has no parent: {}", path.display()))?
        .join("root");
    let mut listed_files = BTreeSet::new();
    for entry in &manifest.files {
        validate_relative_manifest_path("profile root manifest file", &entry.path)?;
        if !listed_files.insert(entry.path.clone()) {
            return Err(anyhow!(
                "profile root manifest {} lists duplicate payload file {}",
                path.display(),
                entry.path
            ));
        }
        if entry.size == 0 {
            return Err(anyhow!(
                "profile root manifest {} entry {} has zero size",
                path.display(),
                entry.path
            ));
        }
    }
    let actual_files = collect_profile_root_files(&root_dir)?;
    if let Some(unlisted) = actual_files.difference(&listed_files).next() {
        return Err(anyhow!(
            "unlisted profile root payload file {} under {}",
            unlisted,
            root_dir.display()
        ));
    }
    if let Some(missing) = listed_files.difference(&actual_files).next() {
        return Err(anyhow!(
            "profile root manifest {} lists missing payload file {}",
            path.display(),
            missing
        ));
    }
    let mut reports = Vec::new();
    for entry in manifest.files {
        validate_profile_root_payload_content(&root_dir.join(&entry.path), &entry.path)?;
        reports.push(check_exact_local_asset(
            &root_dir.join(&entry.path),
            "profile-root",
            &entry.path,
            normalized_blake3(&entry.hash)?,
            entry.size,
        )?);
    }
    Ok(reports)
}

fn validate_profile_root_payload_content(path: &Path, logical_name: &str) -> Result<()> {
    let payload =
        fs::read(path).with_context(|| format!("read profile root payload {}", path.display()))?;
    let text = String::from_utf8_lossy(&payload);
    for forbidden in [
        "127.0.0.1:11434",
        "localhost:11434",
        "CAPSEM_MOCK_SERVER",
        "\"provider\": \"ollama\"",
        "\"baseUrl\": \"http://127.0.0.1:11434\"",
    ] {
        if text.contains(forbidden) {
            return Err(anyhow!(
                "profile root provider override {} contains forbidden test/local provider fragment {}",
                logical_name,
                forbidden
            ));
        }
    }
    Ok(())
}

fn collect_profile_root_files(root_dir: &Path) -> Result<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    if !root_dir.is_dir() {
        return Err(anyhow!(
            "profile root directory {} is missing",
            root_dir.display()
        ));
    }
    collect_profile_root_files_into(root_dir, root_dir, &mut files)?;
    Ok(files)
}

fn collect_profile_root_files_into(
    root_dir: &Path,
    current: &Path,
    files: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in fs::read_dir(current)
        .with_context(|| format!("read profile root directory {}", current.display()))?
    {
        let entry = entry.with_context(|| format!("read entry in {}", current.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat profile root payload {}", path.display()))?;
        if file_type.is_dir() {
            collect_profile_root_files_into(root_dir, &path, files)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(anyhow!(
                "profile root payload {} is not a regular file",
                path.display()
            ));
        }
        let relative = path
            .strip_prefix(root_dir)
            .with_context(|| format!("strip profile root prefix for {}", path.display()))?;
        let relative = relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        validate_relative_manifest_path("profile root payload file", &relative)?;
        files.insert(relative);
    }
    Ok(())
}

fn materialize_profile_config(args: &ProfileMaterializeArgs) -> Result<ProfileMaterializeReport> {
    check_config_root(&args.config_root, args.arch.as_deref())?;
    if args.output_root == args.config_root {
        return Err(anyhow!(
            "output root {} must differ from source config root {}",
            args.output_root.display(),
            args.config_root.display()
        ));
    }
    if args.clean && args.output_root.exists() {
        fs::remove_dir_all(&args.output_root)
            .with_context(|| format!("remove {}", args.output_root.display()))?;
    }
    if !args.output_root.exists() {
        copy_dir_recursive(&args.config_root, &args.output_root)?;
    }

    let mut profile = load_profile(&args.profile)?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", args.profile.display()))?;

    let selected_arches = selected_profile_arches(&profile, args.arch.as_deref())?;
    if args.arch.is_some() {
        profile
            .assets
            .arch
            .retain(|arch, _| selected_arches.iter().any(|selected| selected == arch));
    }

    let manifest_bytes = read_manifest_url(&args.manifest)?;
    let manifest_content = std::str::from_utf8(&manifest_bytes)
        .with_context(|| format!("manifest URL did not return UTF-8 JSON: {}", args.manifest))?;
    let materialize_manifest = load_profile_materialize_manifest(
        &args.manifest,
        manifest_content,
        &manifest_bytes,
        &profile.id,
        &selected_arches,
    )
    .with_context(|| format!("parse manifest from {}", args.manifest))?;
    let manifest = materialize_manifest.manifest;
    let current_release = manifest
        .assets
        .releases
        .get(&manifest.assets.current)
        .ok_or_else(|| {
            anyhow!(
                "manifest {} current asset release {} is missing",
                args.manifest,
                manifest.assets.current
            )
        })?;

    copy_profile_descriptor_files(&profile, &args.config_root, &args.output_root)?;
    materialize_profile_file_descriptors(&mut profile, &args.output_root)?;

    let mut materialized_assets = Vec::new();
    let mut materialized_obom = Vec::new();
    for arch in selected_arches {
        let manifest_assets = current_release.arches.get(&arch).ok_or_else(|| {
            anyhow!(
                "manifest {} current release {} does not contain profile arch {arch}",
                args.manifest,
                manifest.assets.current
            )
        })?;
        let asset_inputs = ProfileAssetMaterializeInputs {
            assets_dir: &args.assets_dir,
            manifest_url: &args.manifest,
            asset_version: &manifest.assets.current,
            arch: &arch,
            manifest_assets,
            asset_urls: &materialize_manifest.asset_urls,
        };
        let rootfs_hash = {
            let profile_assets = profile
                .assets
                .arch
                .get_mut(&arch)
                .expect("arch came from selected_profile_arches");
            materialize_profile_asset_descriptor(
                asset_inputs,
                &mut profile_assets.kernel,
                &mut materialized_assets,
            )?;
            materialize_profile_asset_descriptor(
                asset_inputs,
                &mut profile_assets.initrd,
                &mut materialized_assets,
            )?;
            materialize_profile_asset_descriptor(
                asset_inputs,
                &mut profile_assets.rootfs,
                &mut materialized_assets,
            )?;
            profile_assets
                .rootfs
                .hash
                .clone()
                .ok_or_else(|| anyhow!("materialized {arch} rootfs hash is unresolved"))?
        };
        materialize_profile_obom_descriptor(
            asset_inputs,
            rootfs_hash,
            &mut profile,
            &mut materialized_obom,
        )?;
    }

    let output_profile_path = args
        .output_root
        .join("profiles")
        .join(&profile.id)
        .join("profile.toml");
    fs::create_dir_all(
        output_profile_path
            .parent()
            .ok_or_else(|| anyhow!("materialized profile path has no parent"))?,
    )
    .with_context(|| format!("create parent for {}", output_profile_path.display()))?;
    fs::write(
        &output_profile_path,
        toml::to_string_pretty(&profile).context("serialize materialized profile")?,
    )
    .with_context(|| format!("write {}", output_profile_path.display()))?;

    let manifest_output = args.output_root.join("assets/manifest.json");
    fs::create_dir_all(
        manifest_output
            .parent()
            .ok_or_else(|| anyhow!("materialized manifest path has no parent"))?,
    )
    .with_context(|| format!("create parent for {}", manifest_output.display()))?;
    fs::write(&manifest_output, &materialize_manifest.manifest_bytes)
        .with_context(|| format!("write {}", manifest_output.display()))?;

    let copied_validation =
        validate_materialized_profile(&output_profile_path, Some(&args.output_root))?;
    if copied_validation.profile_id != profile.id {
        return Err(anyhow!(
            "materialized profile id drifted: expected {}, got {}",
            profile.id,
            copied_validation.profile_id
        ));
    }

    Ok(ProfileMaterializeReport {
        schema: "capsem.admin.profile_materialize.v1",
        ok: true,
        profile_id: profile.id,
        profile_revision: profile.revision,
        source_config_root: args.config_root.display().to_string(),
        output_config_root: args.output_root.display().to_string(),
        profile_path: output_profile_path.display().to_string(),
        manifest: manifest_output.display().to_string(),
        asset_version: manifest.assets.current,
        materialized_assets,
        materialized_obom,
    })
}

struct ProfileMaterializeManifest {
    manifest: ManifestV2,
    manifest_bytes: Vec<u8>,
    asset_urls: HashMap<(String, String), String>,
}

#[derive(Debug, Deserialize)]
struct ReleaseChannelProfileManifest {
    profiles: BTreeMap<String, ReleaseChannelProfileDocument>,
}

#[derive(Debug, Deserialize)]
struct ReleaseChannelProfileDocument {
    revision: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    architectures: Vec<ReleaseChannelProfileArchitecture>,
}

#[derive(Debug, Deserialize)]
struct ReleaseChannelProfileArchitecture {
    architecture: String,
    #[serde(default)]
    images: Vec<ReleaseChannelProfileArtifact>,
    #[serde(default)]
    evidence: Vec<ReleaseChannelProfileArtifact>,
}

#[derive(Debug, Deserialize)]
struct ReleaseChannelProfileArtifact {
    kind: String,
    #[serde(default)]
    name: String,
    url: String,
    #[serde(rename = "bytes")]
    size: u64,
    digest: ReleaseChannelProfileDigest,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseChannelProfileDigest {
    sha256: String,
    blake3: String,
}

fn load_profile_materialize_manifest(
    manifest_url: &str,
    manifest_content: &str,
    manifest_bytes: &[u8],
    profile_id: &str,
    selected_arches: &[String],
) -> Result<ProfileMaterializeManifest> {
    let release_graph = serde_json::from_str::<serde_json::Value>(manifest_content)
        .ok()
        .and_then(|document| document.get("profiles").cloned())
        .is_some_and(|profiles| profiles.is_object());
    if release_graph {
        return profile_materialize_manifest_from_release_channel(
            manifest_url,
            manifest_content,
            profile_id,
            selected_arches,
        );
    }
    if let Ok(manifest) = ManifestV2::from_json(manifest_content) {
        return Ok(ProfileMaterializeManifest {
            manifest,
            manifest_bytes: manifest_bytes.to_vec(),
            asset_urls: HashMap::new(),
        });
    }

    profile_materialize_manifest_from_release_channel(
        manifest_url,
        manifest_content,
        profile_id,
        selected_arches,
    )
}

fn profile_materialize_manifest_from_release_channel(
    manifest_url: &str,
    manifest_content: &str,
    profile_id: &str,
    selected_arches: &[String],
) -> Result<ProfileMaterializeManifest> {
    let document: ReleaseChannelProfileManifest = serde_json::from_str(manifest_content)
        .context("failed to parse release channel profile manifest JSON")?;
    let profile = document
        .profiles
        .get(profile_id)
        .ok_or_else(|| anyhow!("release channel manifest does not contain profile {profile_id}"))?;
    if release_channel_status_is_revoked(&profile.status) {
        anyhow::bail!("release channel profile {profile_id} is revoked");
    }

    let mut arch_entries: HashMap<String, HashMap<String, capsem_core::asset_manager::AssetEntry>> =
        HashMap::new();
    let mut asset_urls = HashMap::new();
    for arch in selected_arches {
        let architecture = profile
            .architectures
            .iter()
            .find(|candidate| candidate.architecture == *arch)
            .ok_or_else(|| {
                anyhow!("release channel profile {profile_id} does not contain architecture {arch}")
            })?;
        let mut assets = HashMap::new();
        for artifact in architecture
            .images
            .iter()
            .chain(architecture.evidence.iter())
        {
            if release_channel_status_is_revoked(&artifact.status) {
                continue;
            }
            let Some(logical_name) = release_channel_profile_artifact_logical_name(artifact) else {
                continue;
            };
            validate_release_channel_digest(&artifact.digest)
                .with_context(|| format!("validate {arch} {logical_name} digest"))?;
            assets.insert(
                logical_name.to_string(),
                capsem_core::asset_manager::AssetEntry {
                    hash: artifact.digest.blake3.clone(),
                    sha256: artifact.digest.sha256.clone(),
                    size: artifact.size,
                },
            );
            asset_urls.insert(
                (arch.clone(), logical_name.to_string()),
                resolve_release_channel_artifact_url(manifest_url, &artifact.url)?,
            );
        }
        for required in ["vmlinuz", "initrd.img", "rootfs.erofs"] {
            if !assets.contains_key(required) {
                anyhow::bail!(
                    "release channel profile {profile_id} revision {} architecture {arch} missing {required} image",
                    profile.revision
                );
            }
        }
        arch_entries.insert(arch.clone(), assets);
    }

    let binary_version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = ManifestV2 {
        format: 2,
        refresh_policy: "24h".to_string(),
        asset_base: None,
        assets: capsem_core::asset_manager::AssetsSection {
            current: profile.revision.clone(),
            releases: HashMap::from([(
                profile.revision.clone(),
                capsem_core::asset_manager::AssetRelease {
                    date: String::new(),
                    deprecated: false,
                    deprecated_date: None,
                    min_binary: String::new(),
                    arches: arch_entries,
                },
            )]),
        },
        binaries: capsem_core::asset_manager::BinariesSection {
            current: binary_version.clone(),
            releases: HashMap::from([(
                binary_version.clone(),
                capsem_core::asset_manager::BinaryRelease {
                    date: String::new(),
                    deprecated: false,
                    deprecated_date: None,
                    min_assets: profile.revision.clone(),
                    version: binary_version,
                    files: Vec::new(),
                },
            )]),
        },
    };
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).context("serialize converted asset manifest")?;
    let manifest_json =
        std::str::from_utf8(&manifest_bytes).context("converted manifest JSON is UTF-8")?;
    ManifestV2::from_json(manifest_json).context("validate converted asset manifest")?;

    Ok(ProfileMaterializeManifest {
        manifest,
        manifest_bytes,
        asset_urls,
    })
}

fn release_channel_profile_artifact_logical_name(
    artifact: &ReleaseChannelProfileArtifact,
) -> Option<&'static str> {
    match artifact.kind.as_str() {
        "kernel" => Some("vmlinuz"),
        "initrd" => Some("initrd.img"),
        "rootfs" => Some("rootfs.erofs"),
        "abom" => Some("abom.cdx.json"),
        "obom" => Some("obom.cdx.json"),
        "software_inventory" => Some("software-inventory.json"),
        _ if artifact.name == "obom.cdx.json" => Some("obom.cdx.json"),
        _ => None,
    }
}

fn release_channel_status_is_revoked(status: &str) -> bool {
    status.eq_ignore_ascii_case("revoked")
}

fn validate_release_channel_digest(digest: &ReleaseChannelProfileDigest) -> Result<()> {
    if !is_64_hex(&digest.blake3) {
        anyhow::bail!("profile image blake3 must be a 64-character hex digest");
    }
    if !is_64_hex(&digest.sha256) {
        anyhow::bail!("profile image sha256 must be a 64-character hex digest");
    }
    Ok(())
}

fn is_64_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn resolve_release_channel_artifact_url(channel_source: &str, artifact: &str) -> Result<String> {
    let trimmed = artifact.trim();
    if trimmed.is_empty() {
        anyhow::bail!("release channel artifact URL is empty");
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("file://")
    {
        let parsed = reqwest::Url::parse(trimmed)
            .with_context(|| format!("parse release channel artifact URL {trimmed}"))?;
        return Ok(parsed.to_string());
    }

    let base = reqwest::Url::parse(channel_source)
        .with_context(|| format!("parse release channel URL {channel_source}"))?;
    if trimmed.starts_with('/') {
        let mut root = base;
        root.set_path(trimmed);
        root.set_query(None);
        root.set_fragment(None);
        return Ok(root.to_string());
    }
    base.join(trimmed)
        .with_context(|| {
            format!("resolve release channel artifact {trimmed} against {channel_source}")
        })
        .map(|url| url.to_string())
}

fn materialize_profile_asset_descriptor(
    inputs: ProfileAssetMaterializeInputs<'_>,
    descriptor: &mut capsem_core::net::policy_config::ProfileAssetDescriptor,
    reports: &mut Vec<ProfileMaterializedAssetReport>,
) -> Result<()> {
    let entry = inputs
        .manifest_assets
        .get(&descriptor.name)
        .ok_or_else(|| {
            anyhow!(
                "manifest current release arch {} is missing {}",
                inputs.arch,
                descriptor.name
            )
        })?;
    descriptor.url =
        materialized_profile_asset_url(inputs, &descriptor.name, &entry.hash, entry.size)?;
    descriptor.hash = Some(format!("blake3:{}", entry.hash));
    descriptor.size = Some(entry.size);
    reports.push(ProfileMaterializedAssetReport {
        arch: inputs.arch.to_string(),
        logical_name: descriptor.name.clone(),
        url: descriptor.url.clone(),
        hash: descriptor
            .hash
            .clone()
            .expect("materialized asset hash was just set"),
        size: descriptor
            .size
            .expect("materialized asset size was just set"),
    });
    Ok(())
}

fn materialize_profile_file_descriptors(
    profile: &mut ProfileConfigFile,
    config_root: &Path,
) -> Result<()> {
    fn pin(
        descriptor: Option<&mut capsem_core::net::policy_config::ProfileFileDescriptor>,
        config_root: &Path,
    ) -> Result<()> {
        let Some(descriptor) = descriptor else {
            return Ok(());
        };
        let path = config_root.join(&descriptor.path);
        let hash =
            hash_file(&path).with_context(|| format!("hash profile payload {}", path.display()))?;
        let size = fs::metadata(&path)
            .with_context(|| format!("stat profile payload {}", path.display()))?
            .len();
        if size == 0 {
            return Err(anyhow!(
                "profile payload {} must not be empty",
                path.display()
            ));
        }
        descriptor.hash = Some(format!("blake3:{hash}"));
        descriptor.size = Some(size);
        Ok(())
    }

    pin(profile.files.enforcement.as_mut(), config_root)?;
    pin(profile.files.detection.as_mut(), config_root)?;
    pin(profile.files.mcp.as_mut(), config_root)?;
    pin(profile.files.apt_packages.as_mut(), config_root)?;
    pin(profile.files.python_requirements.as_mut(), config_root)?;
    pin(profile.files.python_requirements_lock.as_mut(), config_root)?;
    pin(profile.files.npm_packages.as_mut(), config_root)?;
    pin(profile.files.npm_package_lock.as_mut(), config_root)?;
    pin(profile.files.build.as_mut(), config_root)?;
    pin(profile.files.tips.as_mut(), config_root)?;
    pin(profile.files.root_manifest.as_mut(), config_root)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct ProfileAssetMaterializeInputs<'a> {
    assets_dir: &'a Path,
    manifest_url: &'a str,
    asset_version: &'a str,
    arch: &'a str,
    manifest_assets: &'a std::collections::HashMap<String, capsem_core::asset_manager::AssetEntry>,
    asset_urls: &'a HashMap<(String, String), String>,
}

fn materialize_profile_obom_descriptor(
    inputs: ProfileAssetMaterializeInputs<'_>,
    rootfs_hash: String,
    profile: &mut ProfileConfigFile,
    reports: &mut Vec<ProfileMaterializedObomReport>,
) -> Result<()> {
    let Some(entry) = inputs.manifest_assets.get("obom.cdx.json") else {
        return Ok(());
    };
    let obom_url =
        materialized_profile_asset_url(inputs, "obom.cdx.json", &entry.hash, entry.size)?;
    let parsed_obom_url = reqwest::Url::parse(&obom_url)
        .with_context(|| format!("parse materialized OBOM URL {obom_url}"))?;
    let (generator, generator_version) = if parsed_obom_url.scheme() == "file" {
        let obom_path = parsed_obom_url
            .to_file_path()
            .map_err(|_| anyhow!("materialized OBOM file URL must be absolute: {obom_url}"))?;
        let obom_path = obom_path
            .canonicalize()
            .with_context(|| format!("canonicalize {}", obom_path.display()))?;
        read_obom_generator(&obom_path)?
    } else {
        ("remote".to_string(), "unknown".to_string())
    };
    let descriptor = ProfileObomDescriptor {
        name: "obom.cdx.json".to_string(),
        url: obom_url,
        hash: format!("blake3:{}", entry.hash),
        size: entry.size,
        generator: generator.clone(),
        generator_version: generator_version.clone(),
    };
    profile
        .obom
        .get_or_insert_with(|| ProfileObomConfig {
            format: "cyclonedx-obom.v1".to_string(),
            arch: BTreeMap::new(),
        })
        .arch
        .insert(inputs.arch.to_string(), descriptor.clone());
    reports.push(ProfileMaterializedObomReport {
        arch: inputs.arch.to_string(),
        url: descriptor.url,
        hash: descriptor.hash,
        size: descriptor.size,
        generator,
        generator_version,
        rootfs_hash,
        scope: "base_image",
    });
    Ok(())
}

fn materialized_profile_asset_url(
    inputs: ProfileAssetMaterializeInputs<'_>,
    logical_name: &str,
    hash: &str,
    size: u64,
) -> Result<String> {
    if let Some(url) = inputs
        .asset_urls
        .get(&(inputs.arch.to_string(), logical_name.to_string()))
    {
        return Ok(url.clone());
    }
    materialized_asset_url(
        inputs.assets_dir,
        inputs.manifest_url,
        inputs.asset_version,
        inputs.arch,
        logical_name,
        hash,
        size,
    )
}

fn materialized_asset_url(
    assets_dir: &Path,
    manifest_url: &str,
    asset_version: &str,
    arch: &str,
    logical_name: &str,
    hash: &str,
    size: u64,
) -> Result<String> {
    if let Some(asset_base_url) =
        capsem_core::asset_manager::asset_release_base_url_from_manifest_url(manifest_url)
    {
        return Ok(capsem_core::asset_manager::asset_download_url_with_base(
            &asset_base_url,
            asset_version,
            arch,
            logical_name,
        ));
    }

    let check = check_local_asset(assets_dir, arch, logical_name, hash, size)?;
    fail_if_local_asset_checks_failed("profile materialize asset check", &[check])?;
    let asset_path = assets_dir.join(arch).join(logical_name);
    let asset_path = asset_path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", asset_path.display()))?;
    Ok(format!("file://{}", asset_path.display()))
}

fn read_obom_generator(path: &Path) -> Result<(String, String)> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read CycloneDX OBOM {}", path.display()))?;
    let document: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("parse CycloneDX OBOM {}", path.display()))?;
    let metadata = document
        .get("metadata")
        .ok_or_else(|| anyhow!("CycloneDX OBOM {} is missing metadata", path.display()))?;
    let tools = metadata.get("tools").ok_or_else(|| {
        anyhow!(
            "CycloneDX OBOM {} is missing metadata.tools",
            path.display()
        )
    })?;
    let candidates: Vec<&serde_json::Value> = tools
        .get("components")
        .and_then(|components| components.as_array())
        .map(|components| components.iter().collect())
        .or_else(|| tools.as_array().map(|tools| tools.iter().collect()))
        .unwrap_or_default();
    let preferred = candidates
        .iter()
        .copied()
        .find(|candidate| {
            candidate
                .get("name")
                .and_then(|name| name.as_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("cdxgen"))
        })
        .or_else(|| {
            candidates.iter().copied().find(|candidate| {
                candidate
                    .get("name")
                    .and_then(|name| name.as_str())
                    .is_some()
                    && candidate
                        .get("version")
                        .and_then(|version| version.as_str())
                        .is_some()
            })
        })
        .ok_or_else(|| {
            anyhow!(
                "CycloneDX OBOM {} must record a generator name and version in metadata.tools",
                path.display()
            )
        })?;
    let name = preferred
        .get("name")
        .and_then(|name| name.as_str())
        .ok_or_else(|| {
            anyhow!(
                "CycloneDX OBOM {} generator is missing name",
                path.display()
            )
        })?;
    let version = preferred
        .get("version")
        .and_then(|version| version.as_str())
        .ok_or_else(|| {
            anyhow!(
                "CycloneDX OBOM {} generator is missing version",
                path.display()
            )
        })?;
    Ok((name.to_string(), version.to_string()))
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).with_context(|| format!("create {}", destination.display()))?;
    for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", source_path.display()))?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "copy {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn load_profile(path: &Path) -> Result<ProfileConfigFile> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read profile {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("parse profile {}", path.display()))
}

fn validate_settings(path: &Path) -> Result<SettingsValidationReport> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read settings {}", path.display()))?;
    let settings: SettingsConfigFile =
        toml::from_str(&content).with_context(|| format!("parse settings {}", path.display()))?;
    settings
        .validate()
        .map_err(|error| anyhow!("validate settings {}: {error}", path.display()))?;
    Ok(SettingsValidationReport {
        schema: "capsem.admin.settings_validation.v1",
        ok: true,
        path: path.display().to_string(),
        app: SettingsAppReport {
            auto_update: settings.app.auto_update,
            notifications: settings.app.notifications,
            start_service_at_login: settings.app.start_service_at_login,
        },
        appearance: SettingsAppearanceReport {
            theme: settings.appearance.theme,
            font_size: settings.appearance.font_size,
            reduced_motion: settings.appearance.reduced_motion,
        },
    })
}

impl SettingsConfigFile {
    fn validate(&self) -> Result<(), String> {
        match self.appearance.theme.as_str() {
            "system" | "light" | "dark" => {}
            other => {
                return Err(format!(
                    "appearance.theme must be system, light, or dark, got {other}"
                ));
            }
        }
        if !(8..=32).contains(&self.appearance.font_size) {
            return Err(format!(
                "appearance.font_size must be between 8 and 32, got {}",
                self.appearance.font_size
            ));
        }
        Ok(())
    }
}

fn image_build_plan(args: &ImageBuildArgs) -> Result<ImageBuildPlan> {
    let profile = load_profile(&args.profile)?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", args.profile.display()))?;
    profile
        .compile_security_rule_set_from_files(&args.config_root, SecurityRuleSource::User)
        .map_err(|error| {
            anyhow!(
                "compile profile rule files for {} with config root {}: {error}",
                args.profile.display(),
                args.config_root.display()
            )
        })?;

    let mut arches = profile.assets.arch.keys().cloned().collect::<Vec<_>>();
    arches.sort();
    if let Some(arch) = &args.arch {
        if !profile.assets.arch.contains_key(arch) {
            return Err(anyhow!(
                "profile {} does not define assets for arch {arch}",
                profile.id
            ));
        }
        arches = vec![arch.clone()];
    }
    if arches.is_empty() {
        return Err(anyhow!(
            "profile {} defines no asset architectures",
            profile.id
        ));
    }

    let mut arch_plans = Vec::new();
    let mut commands = Vec::new();
    for arch in &arches {
        let assets = profile
            .assets
            .arch
            .get(arch)
            .expect("arch came from profile asset map");
        arch_plans.push(ImageBuildArchPlan {
            arch: arch.clone(),
            kernel: assets.kernel.name.clone(),
            initrd: assets.initrd.name.clone(),
            rootfs: assets.rootfs.name.clone(),
        });
        if matches!(
            args.template,
            ImageBuildTemplate::All | ImageBuildTemplate::Kernel
        ) {
            commands.push(CommandReport {
                step: "kernel".to_string(),
                arch: Some(arch.clone()),
                env: BTreeMap::new(),
                argv: vec![
                    "uv".to_string(),
                    "run".to_string(),
                    "python".to_string(),
                    "-m".to_string(),
                    "capsem.builder.image_build_backend".to_string(),
                    args.guest_dir.display().to_string(),
                    "--arch".to_string(),
                    arch.clone(),
                    "--template".to_string(),
                    "kernel".to_string(),
                    "--output".to_string(),
                    format!("{}/", args.output.display()),
                ],
            });
        }
        if matches!(
            args.template,
            ImageBuildTemplate::All | ImageBuildTemplate::Rootfs
        ) {
            let mut env = BTreeMap::new();
            env.insert(
                "CAPSEM_BUILD_EXPERIMENTAL_EROFS".to_string(),
                "1".to_string(),
            );
            env.insert(
                "CAPSEM_BUILD_EROFS_COMPRESSION".to_string(),
                "lz4hc".to_string(),
            );
            env.insert(
                "CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL".to_string(),
                "12".to_string(),
            );
            commands.push(CommandReport {
                step: "rootfs".to_string(),
                arch: Some(arch.clone()),
                env,
                argv: vec![
                    "uv".to_string(),
                    "run".to_string(),
                    "python".to_string(),
                    "-m".to_string(),
                    "capsem.builder.image_build_backend".to_string(),
                    args.guest_dir.display().to_string(),
                    "--arch".to_string(),
                    arch.clone(),
                    "--template".to_string(),
                    "rootfs".to_string(),
                    "--output".to_string(),
                    format!("{}/", args.output.display()),
                ],
            });
        }
    }
    if !matches!(args.template, ImageBuildTemplate::Kernel) {
        commands.push(manifest_generate_command_report(&ManifestGenerateArgs {
            assets_dir: args.output.clone(),
            version: None,
            json: false,
        }));
    }

    Ok(ImageBuildPlan {
        schema: "capsem.admin.image_build_plan.v1",
        profile_id: profile.id,
        profile_revision: profile.revision,
        guest_dir: args.guest_dir.display().to_string(),
        output: args.output.display().to_string(),
        clean: args.clean,
        template: match args.template {
            ImageBuildTemplate::All => "all",
            ImageBuildTemplate::Kernel => "kernel",
            ImageBuildTemplate::Rootfs => "rootfs",
        },
        arches: arch_plans,
        commands,
    })
}

#[cfg(test)]
fn verify_image_outputs(args: &ImageVerifyArgs) -> Result<ImageVerifyReport> {
    let profile = load_profile(&args.profile)?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", args.profile.display()))?;
    profile
        .compile_security_rule_set_from_files(&args.config_root, SecurityRuleSource::User)
        .map_err(|error| {
            anyhow!(
                "compile profile rule files for {} with config root {}: {error}",
                args.profile.display(),
                args.config_root.display()
            )
        })?;

    let manifest_path = args
        .manifest
        .clone()
        .unwrap_or_else(|| args.output.join("manifest.json"));
    let manifest = load_manifest(&manifest_path)?;
    let current_release = manifest
        .assets
        .releases
        .get(&manifest.assets.current)
        .ok_or_else(|| {
            anyhow!(
                "manifest {} current asset release {} is missing",
                manifest_path.display(),
                manifest.assets.current
            )
        })?;

    let mut arches = Vec::new();
    for arch in selected_profile_arches(&profile, args.arch.as_deref())? {
        let manifest_assets = current_release.arches.get(&arch).ok_or_else(|| {
            anyhow!(
                "manifest {} current release {} does not contain profile arch {arch}",
                manifest_path.display(),
                manifest.assets.current
            )
        })?;
        let profile_assets = profile
            .assets
            .arch
            .get(&arch)
            .expect("arch came from selected_profile_arches");
        let mut asset_reports = Vec::new();
        for descriptor in [
            &profile_assets.kernel,
            &profile_assets.initrd,
            &profile_assets.rootfs,
        ] {
            let entry = manifest_assets.get(&descriptor.name).ok_or_else(|| {
                anyhow!(
                    "manifest {} current release {} arch {arch} is missing {}",
                    manifest_path.display(),
                    manifest.assets.current,
                    descriptor.name
                )
            })?;
            asset_reports.push(check_local_asset(
                &args.output,
                &arch,
                &descriptor.name,
                &entry.hash,
                entry.size,
            )?);
        }
        fail_if_local_asset_checks_failed("image output verify", &asset_reports)?;
        arches.push(ImageVerifyArchReport {
            arch,
            assets: asset_reports,
        });
    }

    Ok(ImageVerifyReport {
        schema: "capsem.admin.image_verify.v1",
        ok: true,
        profile_id: profile.id,
        profile_revision: profile.revision,
        output: args.output.display().to_string(),
        manifest: manifest_path.display().to_string(),
        arches,
    })
}

fn materialize_image_workspace(args: &ImageWorkspaceArgs) -> Result<ImageWorkspaceReport> {
    check_config_root(&args.config_root, args.arch.as_deref())?;
    check_profile(&ProfileCheckArgs {
        path: args.profile.clone(),
        config_root: Some(args.config_root.clone()),
        arch: args.arch.clone(),
        json: true,
    })?;
    let profile = load_profile(&args.profile)?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", args.profile.display()))?;
    profile
        .compile_security_rule_set_from_files(&args.config_root, SecurityRuleSource::User)
        .map_err(|error| {
            anyhow!(
                "compile profile rule files for {} with config root {}: {error}",
                args.profile.display(),
                args.config_root.display()
            )
        })?;
    let arches = selected_profile_arches(&profile, args.arch.as_deref())?;

    let workspace = &args.output;
    if workspace.exists() {
        fs::remove_dir_all(workspace)
            .with_context(|| format!("remove stale image workspace {}", workspace.display()))?;
    }
    let workspace_config_root = workspace.join("config");
    let workspace_guest_dir = workspace.join("guest");
    let workspace_profile_path = workspace_config_root
        .join("profiles")
        .join(&profile.id)
        .join("profile.toml");
    let workspace_rules_root = workspace_config_root.join("profiles").join(&profile.id);
    fs::create_dir_all(
        workspace_profile_path
            .parent()
            .expect("workspace profile path has parent"),
    )
    .with_context(|| format!("create {}", workspace_profile_path.display()))?;
    fs::create_dir_all(&workspace_rules_root)
        .with_context(|| format!("create {}", workspace_rules_root.display()))?;

    let profile_toml =
        fs::read(&args.profile).with_context(|| format!("read {}", args.profile.display()))?;
    fs::write(&workspace_profile_path, &profile_toml)
        .with_context(|| format!("write {}", workspace_profile_path.display()))?;

    let mut rule_files = Vec::new();
    copy_profile_rule_file(
        &args.config_root,
        &workspace_config_root,
        profile.rule_files.enforcement.as_deref(),
        "enforcement",
        &mut rule_files,
    )?;
    copy_profile_rule_file(
        &args.config_root,
        &workspace_config_root,
        profile.rule_files.sigma.as_deref(),
        "sigma",
        &mut rule_files,
    )?;
    copy_profile_descriptor_files(&profile, &args.config_root, &workspace_config_root)?;
    materialize_profile_guest_inputs(
        &profile,
        &args.config_root,
        &args.guest_dir,
        &workspace_guest_dir,
    )?;

    let copied_check = check_profile(&ProfileCheckArgs {
        path: workspace_profile_path.clone(),
        config_root: Some(workspace_config_root.clone()),
        arch: args.arch.clone(),
        json: true,
    })?;
    if copied_check.validation.profile_id != profile.id {
        return Err(anyhow!(
            "workspace profile id drifted: expected {}, got {}",
            profile.id,
            copied_check.validation.profile_id
        ));
    }

    let plan = image_build_plan(&ImageBuildArgs {
        profile: workspace_profile_path.clone(),
        config_root: workspace_config_root.clone(),
        guest_dir: workspace_guest_dir.clone(),
        output: workspace.join("assets"),
        arch: args.arch.clone(),
        template: ImageBuildTemplate::All,
        clean: false,
        json: true,
    })?;
    let build_plan_path = workspace.join("build-plan.json");
    fs::write(&build_plan_path, serde_json::to_vec_pretty(&plan)?)
        .with_context(|| format!("write {}", build_plan_path.display()))?;

    let report = ImageWorkspaceReport {
        schema: "capsem.admin.image_workspace.v1",
        ok: true,
        profile_id: profile.id,
        profile_revision: profile.revision,
        workspace: workspace.display().to_string(),
        config_root: workspace_config_root.display().to_string(),
        profile_path: workspace_profile_path.display().to_string(),
        profile_blake3: blake3::hash(&profile_toml).to_hex().to_string(),
        build_plan_path: build_plan_path.display().to_string(),
        rule_files,
        arches: plan
            .arches
            .into_iter()
            .filter(|arch| arches.iter().any(|selected| selected == &arch.arch))
            .collect(),
    };
    fs::write(
        workspace.join("workspace.json"),
        serde_json::to_vec_pretty(&report)?,
    )
    .with_context(|| format!("write {}", workspace.join("workspace.json").display()))?;
    Ok(report)
}

fn copy_profile_descriptor_files(
    profile: &ProfileConfigFile,
    source_config_root: &Path,
    destination_config_root: &Path,
) -> Result<()> {
    for (kind, descriptor) in profile.files.iter() {
        validate_relative_manifest_path("profile file descriptor path", &descriptor.path)?;
        let source = source_config_root.join(&descriptor.path);
        let destination = destination_config_root.join(&descriptor.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::copy(&source, &destination).with_context(|| {
            format!(
                "copy profile {kind} {} to {}",
                source.display(),
                destination.display()
            )
        })?;

        if kind == "root_manifest" {
            let source_root = source
                .parent()
                .ok_or_else(|| anyhow!("profile root manifest has no parent"))?
                .join("root");
            let destination_root = destination
                .parent()
                .ok_or_else(|| anyhow!("workspace profile root manifest has no parent"))?
                .join("root");
            if destination_root.exists() {
                fs::remove_dir_all(&destination_root)
                    .with_context(|| format!("remove {}", destination_root.display()))?;
            }
            copy_dir_recursive(&source_root, &destination_root)?;
        }
    }
    Ok(())
}

fn materialize_profile_guest_inputs(
    profile: &ProfileConfigFile,
    config_root: &Path,
    source_guest_dir: &Path,
    workspace_guest_dir: &Path,
) -> Result<()> {
    let source_config = config_root.join("docker").join("image");
    let workspace_config = workspace_guest_dir.join("config");
    fs::create_dir_all(&workspace_config)
        .with_context(|| format!("create {}", workspace_config.display()))?;
    for relative in ["build.toml", "manifest.toml"] {
        let source = source_config.join(relative);
        let destination = workspace_config.join(relative);
        fs::copy(&source, &destination)
            .with_context(|| format!("copy {} to {}", source.display(), destination.display()))?;
    }
    copy_dir_recursive(
        &source_config.join("kernel"),
        &workspace_config.join("kernel"),
    )?;
    copy_dir_recursive(
        &source_config.join("security"),
        &workspace_config.join("security"),
    )?;
    copy_dir_recursive(&source_config.join("vm"), &workspace_config.join("vm"))?;
    write_profile_vm_resources_toml(&workspace_config.join("vm").join("resources.toml"), profile)?;
    copy_dir_recursive(
        &source_guest_dir.join("artifacts"),
        &workspace_guest_dir.join("artifacts"),
    )?;

    let packages_dir = workspace_config.join("packages");
    fs::create_dir_all(&packages_dir)
        .with_context(|| format!("create {}", packages_dir.display()))?;
    if let Some(descriptor) = profile.files.apt_packages.as_ref() {
        let packages = read_profile_package_lines(&config_root.join(&descriptor.path))?;
        write_profile_package_toml(
            &packages_dir.join("apt.toml"),
            "apt",
            "System Packages",
            "apt",
            "apt-get install -y --no-install-recommends",
            &packages,
        )?;
    }
    if let (Some(descriptor), Some(lock_descriptor)) = (
        profile.files.python_requirements.as_ref(),
        profile.files.python_requirements_lock.as_ref(),
    ) {
        let packages = read_profile_package_lines(&config_root.join(&descriptor.path))?;
        write_profile_package_toml(
            &packages_dir.join("python.toml"),
            "python",
            "Python Packages",
            "uv",
            "uv pip install --system --break-system-packages",
            &packages,
        )?;
        let lock_source = config_root.join(&lock_descriptor.path);
        let expected = exact_python_dependencies(&packages)?;
        validate_python_requirements_lock(&lock_source, Some(&expected))?;
        fs::copy(&lock_source, packages_dir.join("python-requirements.lock"))
            .with_context(|| format!("copy Python requirements lock {}", lock_source.display()))?;
    }
    if let (Some(descriptor), Some(lock_descriptor)) = (
        profile.files.npm_packages.as_ref(),
        profile.files.npm_package_lock.as_ref(),
    ) {
        let packages = read_profile_package_lines(&config_root.join(&descriptor.path))?;
        write_profile_package_toml(
            &packages_dir.join("npm.toml"),
            "npm",
            "Node Packages",
            "npm",
            "npm install -g --prefix /opt/ai-clis",
            &packages,
        )?;
        let expected = exact_npm_dependencies(&packages)?;
        let lock_source = config_root.join(&lock_descriptor.path);
        validate_npm_package_lock(&lock_source, Some(&expected))?;
        fs::copy(&lock_source, packages_dir.join("npm-package-lock.json"))
            .with_context(|| format!("copy npm package lock {}", lock_source.display()))?;
        fs::write(
            packages_dir.join("npm-package.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "name": "capsem-profile-ai-clis",
                "private": true,
                "dependencies": expected,
            }))?,
        )?;
    }
    if let Some(descriptor) = profile.files.build.as_ref() {
        let source = config_root.join(&descriptor.path);
        let destination = workspace_guest_dir.join("profile-build.sh");
        fs::copy(&source, &destination)
            .with_context(|| format!("copy {} to {}", source.display(), destination.display()))?;
    }
    if let Some(descriptor) = profile.files.tips.as_ref() {
        let source = config_root.join(&descriptor.path);
        let artifacts_dir = workspace_guest_dir.join("artifacts");
        fs::create_dir_all(&artifacts_dir)
            .with_context(|| format!("create {}", artifacts_dir.display()))?;
        fs::copy(&source, artifacts_dir.join("tips.txt"))
            .with_context(|| format!("copy profile tips {}", source.display()))?;
    }
    if let Some(descriptor) = profile.files.root_manifest.as_ref() {
        let manifest_path = config_root.join(&descriptor.path);
        let source_root = manifest_path
            .parent()
            .ok_or_else(|| anyhow!("profile root manifest has no parent"))?
            .join("root");
        copy_dir_recursive(&source_root, &workspace_guest_dir.join("profile-root"))?;
    }
    Ok(())
}

fn write_profile_vm_resources_toml(path: &Path, profile: &ProfileConfigFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let content = format!(
        "[resources]\n\
         cpu_count = {}\n\
         ram_gb = {}\n\
         scratch_disk_size_gb = {}\n\
         log_bodies = false\n\
         max_body_capture = 4096\n\
         retention_days = 30\n\
         max_sessions = 100\n\
         min_content_sessions = 25\n\
         max_disk_gb = 100\n\
         terminated_retention_days = 365\n",
        profile.vm.cpu_count, profile.vm.ram_gb, profile.vm.scratch_disk_size_gb
    );
    fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

fn read_profile_package_lines(path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read package list {}", path.display()))?;
    let packages = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if packages.is_empty() {
        return Err(anyhow!("package list {} is empty", path.display()));
    }
    Ok(packages)
}

fn write_profile_package_toml(
    path: &Path,
    key: &str,
    name: &str,
    manager: &str,
    install_cmd: &str,
    packages: &[String],
) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("package TOML path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let packages = packages
        .iter()
        .map(|package| format!("    {package:?}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let content = format!(
        r#"[{key}]
name = {name:?}
manager = {manager:?}
install_cmd = {install_cmd:?}
packages = [
{packages},
]
"#
    );
    fs::write(path, content).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn copy_profile_rule_file(
    config_root: &Path,
    workspace_config_root: &Path,
    rule_file: Option<&str>,
    kind: &'static str,
    reports: &mut Vec<ImageWorkspaceRuleFileReport>,
) -> Result<()> {
    let Some(rule_file) = rule_file else {
        return Ok(());
    };
    if Path::new(rule_file).is_absolute() {
        return Err(anyhow!(
            "image workspace requires profile rule files to be relative, got {rule_file}"
        ));
    }
    let source_path = resolve_profile_rule_file_path(config_root, rule_file);
    let destination_path = workspace_config_root.join(rule_file);
    fs::create_dir_all(
        destination_path
            .parent()
            .ok_or_else(|| anyhow!("rule file destination has no parent"))?,
    )
    .with_context(|| format!("create parent for {}", destination_path.display()))?;
    let bytes = fs::read(&source_path)
        .with_context(|| format!("read rule file {}", source_path.display()))?;
    fs::write(&destination_path, &bytes)
        .with_context(|| format!("write rule file {}", destination_path.display()))?;
    reports.push(ImageWorkspaceRuleFileReport {
        kind,
        source: source_path.display().to_string(),
        path: destination_path.display().to_string(),
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        size: bytes.len() as u64,
    });
    Ok(())
}

fn manifest_generate_command_report(args: &ManifestGenerateArgs) -> CommandReport {
    let version_expr = match &args.version {
        Some(version) => format!("{version:?}"),
        None => "get_project_version(Path('.'))".to_string(),
    };
    CommandReport {
        step: "manifest".to_string(),
        arch: None,
        env: BTreeMap::new(),
        argv: vec![
            "uv".to_string(),
            "run".to_string(),
            "python3".to_string(),
            "-c".to_string(),
            format!(
                "from pathlib import Path; from capsem.builder.docker import generate_checksums, get_project_version; v = {version_expr}; generate_checksums(Path({:?}), v); print(f'manifest.json generated (v{{v}})')",
                args.assets_dir.display().to_string()
            ),
        ],
    }
}

fn selected_profile_arches(
    profile: &ProfileConfigFile,
    only_arch: Option<&str>,
) -> Result<Vec<String>> {
    let mut arches = profile.assets.arch.keys().cloned().collect::<Vec<_>>();
    arches.sort();
    if let Some(arch) = only_arch {
        if !profile.assets.arch.contains_key(arch) {
            return Err(anyhow!(
                "profile {} does not define assets for arch {arch}",
                profile.id
            ));
        }
        arches = vec![arch.to_string()];
    }
    if arches.is_empty() {
        return Err(anyhow!(
            "profile {} defines no asset architectures",
            profile.id
        ));
    }
    Ok(arches)
}

fn check_local_asset(
    assets_dir: &Path,
    arch: &str,
    logical_name: &str,
    expected_hash: &str,
    expected_size: u64,
) -> Result<LocalAssetCheckReport> {
    let path = assets_dir.join(arch).join(logical_name);
    check_exact_local_asset(&path, arch, logical_name, expected_hash, expected_size)
}

fn check_exact_local_asset(
    path: &Path,
    arch: &str,
    logical_name: &str,
    expected_hash: &str,
    expected_size: u64,
) -> Result<LocalAssetCheckReport> {
    if !path.is_file() {
        return Ok(LocalAssetCheckReport {
            arch: arch.to_string(),
            logical_name: logical_name.to_string(),
            expected_hash: expected_hash.to_string(),
            expected_size,
            path: Some(path.display().to_string()),
            present: false,
            size_ok: None,
            blake3_ok: None,
        });
    }
    let metadata =
        fs::metadata(path).with_context(|| format!("stat local asset {}", path.display()))?;
    let digest = hash_file(path)?;
    Ok(LocalAssetCheckReport {
        arch: arch.to_string(),
        logical_name: logical_name.to_string(),
        expected_hash: expected_hash.to_string(),
        expected_size,
        path: Some(path.display().to_string()),
        present: true,
        size_ok: Some(metadata.len() == expected_size),
        blake3_ok: Some(digest == expected_hash),
    })
}

fn fail_if_local_asset_checks_failed(
    context: &str,
    assets: &[LocalAssetCheckReport],
) -> Result<()> {
    let failures = assets
        .iter()
        .filter(|asset| {
            !asset.present
                || asset.size_ok.is_some_and(|ok| !ok)
                || asset.blake3_ok.is_some_and(|ok| !ok)
        })
        .map(|asset| {
            format!(
                "{}:{} present={} size_ok={} blake3_ok={} path={}",
                asset.arch,
                asset.logical_name,
                asset.present,
                asset
                    .size_ok
                    .map(|ok| ok.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                asset
                    .blake3_ok
                    .map(|ok| ok.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                asset.path.as_deref().unwrap_or("n/a"),
            )
        })
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        return Err(anyhow!("{context} failed: {}", failures.join("; ")));
    }
    Ok(())
}

fn normalized_blake3(value: &str) -> Result<&str> {
    value
        .strip_prefix("blake3:")
        .ok_or_else(|| anyhow!("expected blake3:<hash>, got {value}"))
}

fn validate_relative_manifest_path(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('/')
        || value.starts_with("file://")
        || value.contains("..")
        || value.contains('\\')
        || value.trim() != value
    {
        return Err(anyhow!(
            "{field} must be a relative path without traversal: {value}"
        ));
    }
    Ok(())
}

fn print_image_build_plan(plan: &ImageBuildPlan, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }
    println!(
        "profile {} rev {} -> {}",
        plan.profile_id, plan.profile_revision, plan.output
    );
    for arch in &plan.arches {
        println!(
            "  {}: {}, {}, {}",
            arch.arch, arch.kernel, arch.initrd, arch.rootfs
        );
    }
    for command in &plan.commands {
        let env = if command.env.is_empty() {
            String::new()
        } else {
            format!(
                "{} ",
                command
                    .env
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        println!("  {}{}", env, command.argv.join(" "));
    }
    Ok(())
}

fn clean_image_outputs(plan: &ImageBuildPlan) -> Result<()> {
    let output = PathBuf::from(&plan.output);
    for arch in &plan.arches {
        let path = output.join(&arch.arch);
        if !path.exists() {
            continue;
        }
        match plan.template {
            "all" => {
                fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
            }
            "kernel" => {
                for name in [&arch.kernel, &arch.initrd] {
                    let file = path.join(name);
                    if file.exists() {
                        fs::remove_file(&file)
                            .with_context(|| format!("remove {}", file.display()))?;
                    }
                }
            }
            "rootfs" => {
                for name in [
                    arch.rootfs.as_str(),
                    "rootfs.squashfs",
                    "obom.cdx.json",
                    "software-inventory.json",
                    "build-ledger.log",
                    "tool-versions.txt",
                ] {
                    let file = path.join(name);
                    if file.exists() {
                        fs::remove_file(&file)
                            .with_context(|| format!("remove {}", file.display()))?;
                    }
                }
            }
            other => return Err(anyhow!("unsupported image build template {other}")),
        }
    }
    if plan.arches.len() > 1 {
        for name in ["manifest.json", "B3SUMS"] {
            let path = output.join(name);
            if path.exists() {
                fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            }
        }
    }
    Ok(())
}

fn run_command(command: &CommandReport) -> Result<()> {
    let (program, args) = command
        .argv
        .split_first()
        .ok_or_else(|| anyhow!("empty command for step {}", command.step))?;
    let status = Command::new(program)
        .args(args)
        .envs(&command.env)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("run image build step {}", command.step))?;
    if !status.success() {
        return Err(anyhow!(
            "image build step {} failed with status {status}",
            command.step
        ));
    }
    Ok(())
}

fn compile_rule_file(
    kind: &'static str,
    path: &Path,
    source: RuleFileSourceArg,
) -> Result<RuleFileReport> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read {kind} {}", path.display()))?;
    let profile = match kind {
        "enforcement" => SecurityRuleProfile::parse_toml(&content)
            .map_err(|error| anyhow!("parse enforcement {}: {error}", path.display()))?,
        "detection" => SecurityRuleProfile::parse_sigma_yaml(&content)
            .map_err(|error| anyhow!("parse detection {}: {error}", path.display()))?,
        other => return Err(anyhow!("unsupported rule file kind: {other}")),
    };
    let source = source.into_security_rule_source();
    let rule_set = SecurityRuleSet::compile_profile(&profile, source)
        .map_err(|error| anyhow!("compile {kind} {}: {error}", path.display()))?;
    let rules = rule_set
        .rules()
        .iter()
        .map(compiled_rule_report)
        .collect::<Vec<_>>();
    Ok(RuleFileReport {
        schema: "capsem.admin.rule_file_report.v1",
        ok: true,
        kind,
        source: match source {
            SecurityRuleSource::User => "user",
            SecurityRuleSource::Corp => "corp",
            SecurityRuleSource::BuiltinDefault => "builtin_default",
        },
        path: path.display().to_string(),
        compiled_rules: rules.len(),
        rules,
    })
}

fn compiled_rule_report(rule: &CompiledSecurityRule) -> CompiledRuleReport {
    CompiledRuleReport {
        rule_id: rule.rule_id.clone(),
        provider: rule.provider.clone(),
        namespace: rule.namespace.clone(),
        rule_key: rule.rule_key.clone(),
        default_rule: rule.default_rule,
        name: rule.name.clone(),
        action: rule.action.as_str(),
        detection_level: rule.detection_level.map(|level| level.as_str()),
        priority: rule.priority,
        condition: rule.condition.clone(),
        reason: rule.reason.clone(),
        corp_locked: rule.corp_locked,
    }
}

fn load_manifest(path: &Path) -> Result<ManifestV2> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read manifest {}", path.display()))?;
    ManifestV2::from_json(&content).with_context(|| format!("parse manifest {}", path.display()))
}

fn read_manifest_url(source: &str) -> Result<Vec<u8>> {
    read_url_bytes(source, "manifest")
}

fn read_url_bytes(source: &str, label: &str) -> Result<Vec<u8>> {
    let url = reqwest::Url::parse(source).with_context(|| {
        format!(
            "{label} must be a URL: use https://..., http://..., or file:///absolute/path, got {source}"
        )
    })?;
    match url.scheme() {
        "http" | "https" => {
            let response = reqwest::blocking::Client::builder()
                .user_agent("capsem-admin")
                .build()
                .with_context(|| format!("build {label} HTTP client"))?
                .get(url)
                .send()
                .with_context(|| format!("fetch {label} {source}"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(anyhow!("{label} fetch failed: HTTP {status} for {source}"));
            }
            Ok(response
                .bytes()
                .with_context(|| format!("read {label} response body"))?
                .to_vec())
        }
        "file" => {
            let path = url
                .to_file_path()
                .map_err(|_| anyhow!("{label} file URL must be absolute: {source}"))?;
            fs::read(&path).with_context(|| format!("read {label} {}", path.display()))
        }
        scheme => Err(anyhow!(
            "unsupported {label} URL scheme {scheme}: use https://, http://, or file://"
        )),
    }
}

fn manifest_report(
    path: &Path,
    manifest: &ManifestV2,
    assets_dir: Option<&Path>,
    only_arch: Option<&str>,
) -> Result<ManifestReport> {
    let mut arches = Vec::new();
    for (asset_version, release) in &manifest.assets.releases {
        for (arch, assets) in &release.arches {
            if only_arch.is_some_and(|only| only != arch) {
                continue;
            }
            let mut asset_reports = Vec::new();
            let mut names = assets.keys().collect::<Vec<_>>();
            names.sort();
            for name in names {
                let entry = assets.get(name).expect("asset name from keys");
                let (path, present, size_ok, blake3_ok) = match assets_dir {
                    Some(dir) => {
                        let file_path = dir.join(arch).join(name);
                        if !file_path.is_file() {
                            (Some(file_path.display().to_string()), false, None, None)
                        } else {
                            let metadata = fs::metadata(&file_path).with_context(|| {
                                format!("stat manifest asset {}", file_path.display())
                            })?;
                            let digest = hash_file(&file_path)?;
                            (
                                Some(file_path.display().to_string()),
                                true,
                                Some(metadata.len() == entry.size),
                                Some(digest == entry.hash),
                            )
                        }
                    }
                    None => (None, false, None, None),
                };
                asset_reports.push(ManifestAssetReport {
                    logical_name: name.clone(),
                    hash: entry.hash.clone(),
                    size: entry.size,
                    path,
                    present,
                    size_ok,
                    blake3_ok,
                });
            }
            arches.push(ManifestArchReport {
                asset_version: asset_version.clone(),
                arch: arch.clone(),
                assets: asset_reports,
            });
        }
    }
    arches.sort_by(|left, right| {
        left.asset_version
            .cmp(&right.asset_version)
            .then_with(|| left.arch.cmp(&right.arch))
    });
    if let Some(only_arch) = only_arch {
        if arches.is_empty() {
            return Err(anyhow!(
                "manifest {} does not contain arch {only_arch}",
                path.display()
            ));
        }
    }
    Ok(ManifestReport {
        schema: "capsem.admin.manifest_report.v1",
        ok: true,
        path: path.display().to_string(),
        blake3: hash_file(path)?,
        refresh_policy: manifest.refresh_policy.clone(),
        asset_version: manifest.assets.current.clone(),
        binary_version: manifest.binaries.current.clone(),
        releases: manifest.assets.releases.len(),
        arches,
    })
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn infer_config_root(profile_path: &Path) -> Result<PathBuf> {
    let parent = profile_path.parent().ok_or_else(|| {
        anyhow!(
            "cannot infer config root for profile path without parent: {}",
            profile_path.display()
        )
    })?;
    if profile_path
        .file_name()
        .is_some_and(|name| name == "profile.toml")
        && parent
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "profiles")
    {
        return parent
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                anyhow!(
                    "cannot infer config root from profile path {}",
                    profile_path.display()
                )
            });
    }
    if parent.file_name().is_some_and(|name| name == "profiles") {
        return parent.parent().map(Path::to_path_buf).ok_or_else(|| {
            anyhow!(
                "cannot infer config root from profile path {}",
                profile_path.display()
            )
        });
    }
    Ok(parent.to_path_buf())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
#[derive(Debug)]
struct ImageVerifyArgs {
    profile: PathBuf,
    config_root: PathBuf,
    output: PathBuf,
    manifest: Option<PathBuf>,
    arch: Option<String>,
}

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use anyhow::{anyhow, Context, Result};
use capsem_assets::asset_manager::{BinaryExecutable, BinaryFile, ManifestV2};
use capsem_core::net::policy_config::{
    resolve_profile_rule_file_path, validate_corp_toml_contract, CompiledSecurityRule, ProfileCatalog,
    ProfileConfigFile, ProfileObomConfig, ProfileObomDescriptor, SecurityRuleProfile, SecurityRuleSet,
    SecurityRuleSource, SettingsFile,
};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

mod assets_channel_build;
mod assets_channel_render;
mod assets_channel_validation;
mod channel_bootstrap;
mod package_inspection;
mod profile_images;
#[allow(dead_code)]
mod release_graph;
mod source_commit;

use assets_channel_build::*;
use assets_channel_render::*;
use assets_channel_validation::*;
use profile_images::*;

use package_inspection::binary_files_from_artifacts;
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
    #[arg(long, default_value = "cache/target/config")]
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
    #[arg(long, hide = true, requires = "bootstrap_output", conflicts_with = "manifest_path")]
    bootstrap_from_manifest: Option<PathBuf>,
    /// Exact retired first-party graph used only for its digest-authorized replacement.
    #[arg(
        long,
        hide = true,
        requires_all = ["bootstrap_output", "bootstrap_retired_sha256"],
        conflicts_with_all = ["bootstrap_from_manifest", "manifest_path"]
    )]
    bootstrap_retired_manifest: Option<PathBuf>,
    /// Config-owned SHA-256 of the retired public graph bytes.
    #[arg(long, hide = true, requires = "bootstrap_retired_manifest")]
    bootstrap_retired_sha256: Option<channel_bootstrap::RetiredGraphSha256>,
    /// Selected-channel source manifest created by the serialized workflow.
    #[arg(long, hide = true, conflicts_with = "manifest_path")]
    bootstrap_output: Option<PathBuf>,
    /// Validate and print the workflow dispatch without executing it.
    #[arg(
        long,
        conflicts_with_all = ["bootstrap_from_manifest", "bootstrap_retired_manifest"]
    )]
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
    #[arg(long, default_value = "cache/target/release/distribution")]
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
    #[arg(long, default_value = "cache/target/release/distribution")]
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
        let runs: Vec<ProfileWorkflowRun> =
            serde_json::from_str(&raw).context("GitHub returned invalid profile workflow run JSON")?;
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
            let completed: ProfileWorkflowRun =
                serde_json::from_str(&viewed).context("GitHub returned invalid completed profile workflow JSON")?;
            if completed.database_id != run.database_id
                || completed.display_title != title
                || completed.head_sha != source_commit.as_str()
                || completed.head_branch != source_ref
                || completed.status != "completed"
                || completed.conclusion != "success"
            {
                return Err(anyhow!("completed profile workflow identity changed: {completed:?}"));
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
    binaries: Vec<capsem_assets::asset_manager::BinaryExecutable>,
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
                AssetsChannelSubcommand::RecordBinary(args) => assets_channel_record_binary_command(args),
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
            println!("valid: profile file assets ({} assets)", report.assets.len());
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
    let publication_identity = profile_publication_identity(&args.channel, &profile.id, &profile.revision)?;
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
    let bootstrap = match (
        args.bootstrap_from_manifest.as_deref(),
        args.bootstrap_retired_manifest.as_deref(),
        args.bootstrap_retired_sha256.as_ref(),
        args.bootstrap_output.as_deref(),
    ) {
        (Some(input), None, None, Some(output)) => Some((input, output, None)),
        (None, Some(input), Some(sha256), Some(output)) => Some((input, output, Some(sha256))),
        (None, None, None, None) => None,
        _ => return Err(anyhow!("release bootstrap arguments form an incomplete or mixed mode")),
    };
    if let Some((input_path, output_path, retired_sha256)) = bootstrap {
        let input_bytes =
            fs::read(input_path).with_context(|| format!("read bootstrap input {}", input_path.display()))?;
        if let Some(expected) = retired_sha256 {
            verify_retired_graph_sha256(&input_bytes, expected)?;
        }
        let input: serde_json::Value = serde_json::from_slice(&input_bytes)
            .with_context(|| format!("parse bootstrap input {}", input_path.display()))?;
        let input_channel = input
            .get("channel")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("bootstrap input is missing its channel"))?;
        validate_assets_channel_graph_manifest(&input, input_channel)?;
        let bootstrapped = if retired_sha256.is_some() {
            channel_bootstrap::bootstrap_retired_first_party_channel_source(&args.channel, &input)?
        } else {
            channel_bootstrap::bootstrap_first_party_channel_source(&args.channel, &input)?
        };
        validate_assets_channel_graph_manifest(&bootstrapped, &args.channel)?;
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let mut bytes = serde_json::to_vec_pretty(&bootstrapped).context("serialize bootstrap manifest")?;
        bytes.push(b'\n');
        fs::write(output_path, bytes).with_context(|| format!("write {}", output_path.display()))?;
        let report = serde_json::json!({
            "schema": "capsem.admin.release_bootstrap.v1",
            "ok": true,
            "channel": args.channel,
            "profile": selection.profile,
            "profile_revision": selection.profile_revision,
            "publication_identity": selection.publication_identity,
            "input_channel": input_channel,
            "donor_channel": if retired_sha256.is_none() {
                Some(input_channel)
            } else {
                None
            },
            "retired_channel": if retired_sha256.is_some() {
                Some(input_channel)
            } else {
                None
            },
            "retired": retired_sha256.is_some(),
            "package_count": bootstrapped["packages"].as_array().map_or(0, Vec::len),
            "output": output_path.display().to_string(),
        });
        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "bootstrapped {}/{} source manifest from verified {} input",
                report["channel"].as_str().unwrap_or("channel"),
                report["profile"].as_str().unwrap_or("profile"),
                input_channel
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
                serde_json::to_value(report.status)?.as_str().unwrap_or("status"),
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
            if report.dispatched { "dispatched" } else { "validated" },
            report.channel,
            report.profile,
            report.profile_revision,
            report.workflow,
            report.run_id.map(|run_id| format!(" run {run_id}")).unwrap_or_default()
        );
    }
    Ok(())
}

fn verify_retired_graph_sha256(bytes: &[u8], expected: &channel_bootstrap::RetiredGraphSha256) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected.as_str() {
        return Err(anyhow!(
            "retired graph sha256 mismatch: expected {}, got {actual}",
            expected
        ));
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
    let bytes =
        fs::read(manifest_path).with_context(|| format!("read release manifest {}", manifest_path.display()))?;
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
        publication_identity: profile_publication_identity(&args.channel, &args.profile, profile_version)?,
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
        &fs::read(manifest_path).with_context(|| format!("read release manifest {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parse release manifest {}", manifest_path.display()))?;
    let candidate: serde_json::Value = serde_json::from_slice(
        &fs::read(candidate_manifest)
            .with_context(|| format!("read candidate manifest {}", candidate_manifest.display()))?,
    )
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
                if let Some(rows) = architecture.get_mut(field).and_then(serde_json::Value::as_array_mut) {
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
    let mut bytes = serde_json::to_vec_pretty(&base).context("serialize merged profile manifest")?;
    bytes.push(b'\n');
    fs::write(manifest_path, bytes).with_context(|| format!("write release manifest {}", manifest_path.display()))?;
    Ok(ProfileReleaseReport {
        schema: "capsem.admin.profile_release.v1",
        ok: true,
        action: "release",
        channel: args.channel.clone(),
        manifest: manifest_path.display().to_string(),
        manifest_version: manifest_version.to_string(),
        profile: args.profile.clone(),
        profile_version: profile_version.to_string(),
        publication_identity: profile_publication_identity(&args.channel, &args.profile, profile_version)?,
        status,
        changed_channels: vec![args.channel.clone()],
        changed_manifests: vec![manifest_version.to_string()],
        changed_profiles: vec![args.profile.clone()],
        changed_config_refs,
        changed_image_artifacts,
        compatible_with_current_binary: compatible,
    })
}

fn graph_profile_matches_current_binary(profile: &serde_json::Value, manifest: &serde_json::Value) -> Result<bool> {
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
        .filter(|package| package.get("status").and_then(serde_json::Value::as_str) == Some("current"))
        .map(|package| {
            let version = package
                .get("version")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow!("current package has no version"))?;
            semver::Version::parse(version).with_context(|| format!("current package version is invalid: {version}"))
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

fn rewrite_profile_publication_urls(profile: &mut serde_json::Value, publication_base: &str) -> Result<()> {
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
                    .ok_or_else(|| anyhow!("candidate profile {field} row has no publication file name"))?;
                if file_name.is_empty()
                    || !file_name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
                {
                    return Err(anyhow!("candidate profile {field} file name is unsafe: {file_name}"));
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
            .filter(|row| row.get("kind").and_then(serde_json::Value::as_str) == Some("software_inventory"))
            .map(|row| {
                row.get("url")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| anyhow!("candidate profile software_inventory evidence has no publication URL"))
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
                if !row.is_object() || row.get("evidence").and_then(serde_json::Value::as_str).is_none() {
                    return Err(anyhow!("candidate profile software row has no evidence URL"));
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
    let catalog = ProfileCatalog::load_from_dir(&config_root.join("profiles")).map_err(|error| {
        anyhow!(
            "load profile directory {}: {error}",
            config_root.join("profiles").display()
        )
    })?;
    let mut profiles = Vec::new();
    for profile in catalog.profiles() {
        profiles.push(check_profile(&ProfileCheckArgs {
            path: config_root.join("profiles").join(&profile.id).join("profile.toml"),
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
    let content = fs::read_to_string(path).with_context(|| format!("read corp {}", path.display()))?;
    let file: SettingsFile = toml::from_str(&content).with_context(|| format!("parse corp {}", path.display()))?;
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
        compiled +=
            compile_rule_file("enforcement", &config_root.join(enforcement), RuleFileSourceArg::Corp)?.compiled_rules;
    }
    if let Some(sigma) = file.corp_rule_files.sigma.as_deref() {
        compiled += compile_rule_file("detection", &config_root.join(sigma), RuleFileSourceArg::Corp)?.compiled_rules;
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
        println!("generated manifest {}", args.assets_dir.join("manifest.json").display());
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
            report.corporation, report.channel, report.output_manifest, report.resolved_binary_version
        );
    }
    Ok(())
}

fn author_corporate_manifest(args: &ManifestCorporateArgs) -> Result<CorporateManifestReport> {
    validate_corporate_namespace(&args.corporation, &args.channel)?;
    validate_corporate_profile_base(&args.profile_base)?;

    let official_bytes = fs::read(&args.official_manifest)
        .with_context(|| format!("read official Capsem manifest {}", args.official_manifest.display()))?;
    let official: serde_json::Value = serde_json::from_slice(&official_bytes)
        .with_context(|| format!("parse official Capsem manifest {}", args.official_manifest.display()))?;

    let profile_bytes = fs::read(&args.profile_manifest)
        .with_context(|| format!("read corporate profile manifest {}", args.profile_manifest.display()))?;
    let mut profile_source: serde_json::Value = serde_json::from_slice(&profile_bytes)
        .with_context(|| format!("parse corporate profile manifest {}", args.profile_manifest.display()))?;
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
        return Err(anyhow!("corporate profile manifest must contain at least one profile"));
    }
    for (profile_id, profile) in profiles.iter_mut() {
        profile["source_commit"] = serde_json::to_value(&args.source_commit)?;
        validate_corporate_profile_document(profile_id, profile, &args.profile_base, &resolved_version)?;
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
    let official_canonical = fs::canonicalize(&args.official_manifest)
        .with_context(|| format!("resolve official Capsem manifest {}", args.official_manifest.display()))?;
    let profile_canonical = fs::canonicalize(&args.profile_manifest)
        .with_context(|| format!("resolve corporate profile manifest {}", args.profile_manifest.display()))?;
    if output_path == official_canonical || output_path == profile_canonical {
        return Err(anyhow!("corporate output must not overwrite an authoring input"));
    }

    let mut encoded = serde_json::to_vec_pretty(&manifest)?;
    encoded.push(b'\n');
    let temporary = output_dir.join(format!(".manifest.json.tmp-{}", std::process::id()));
    fs::write(&temporary, &encoded)
        .with_context(|| format!("write corporate manifest staging file {}", temporary.display()))?;
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
    validate_channel_name(corporation).with_context(|| format!("invalid corporation namespace {corporation:?}"))?;
    validate_channel_name(channel).with_context(|| format!("invalid corporate channel {channel:?}"))?;
    if corporation == "capsem" || matches!(channel, "stable" | "nightly") {
        return Err(anyhow!("corporate authoring cannot target a first-party namespace"));
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
        return Err(anyhow!("corporate profile {profile_id} must list architectures"));
    }
    if let Some(minimum) = profile.get("min_capsem_version").and_then(serde_json::Value::as_str) {
        let minimum = semver::Version::parse(minimum)
            .with_context(|| format!("corporate profile {profile_id} minimum Capsem version is invalid: {minimum}"))?;
        if selected_version < &minimum {
            return Err(anyhow!(
                "corporate profile {profile_id} requires Capsem {minimum} or newer, selected {selected_version}"
            ));
        }
    }
    if let Some(maximum) = profile.get("max_capsem_version").and_then(serde_json::Value::as_str) {
        let maximum = semver::Version::parse(maximum)
            .with_context(|| format!("corporate profile {profile_id} maximum Capsem version is invalid: {maximum}"))?;
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
            return Err(anyhow!("official manifest does not publish Capsem {policy}"));
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
        return Err(anyhow!("official manifest does not publish Capsem {resolved}"));
    }
    Ok((resolved, packages))
}

fn corporate_manifest_output_dir(args: &ManifestCorporateArgs) -> Result<PathBuf> {
    fs::create_dir_all(&args.output_root)
        .with_context(|| format!("create corporate manifest output root {}", args.output_root.display()))?;
    let output_root = fs::canonicalize(&args.output_root)
        .with_context(|| format!("resolve corporate manifest output root {}", args.output_root.display()))?;
    let output_dir = output_root.join(&args.corporation).join(&args.channel);
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("create corporate manifest destination {}", output_dir.display()))?;
    let output_dir = fs::canonicalize(&output_dir)
        .with_context(|| format!("resolve corporate manifest destination {}", output_dir.display()))?;
    if !output_dir.starts_with(&output_root) {
        return Err(anyhow!("corporate manifest destination escapes its owned output root"));
    }
    Ok(output_dir)
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

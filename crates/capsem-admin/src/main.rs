use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{anyhow, Context, Result};
use capsem_assets::asset_manager::{BinaryExecutable, BinaryFile, ManifestV2};
use capsem_core::net::policy_config::{
    validate_corp_toml_contract, CompiledSecurityRule, ProfileCatalog, ProfileConfigFile, ProfileObomConfig,
    ProfileObomDescriptor, SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource, SettingsFile,
};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

mod assets_channel_build;
mod assets_channel_render;
mod assets_channel_validation;
mod channel_bootstrap;
mod image_build;
mod manifest_generation;
mod package_inspection;
mod profile_images;
mod release_github;
#[allow(dead_code)]
mod release_graph;
mod source_commit;

use assets_channel_build::*;
use assets_channel_render::*;
use assets_channel_validation::*;
use image_build::*;
use manifest_generation::*;
use profile_images::*;

use package_inspection::binary_files_from_artifacts;
use release_github::{ensure_publication_identity_is_free, GhReleaseWorkflowRunner, ReleaseWorkflowRunner};
use source_commit::SourceCommit;

#[derive(Debug, Parser)]
#[command(name = "capsem-admin")]
#[command(version)]
#[command(about = "Capsem runtime, asset, and profile administration")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate a channel runtime release selection without publishing it.
    Validate(ReleaseValidateArgs),
    /// Publish one channel's runtime through the serialized release workflow.
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
    /// Author a corporation-owned manifest from official packages and an owned runtime.
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
    /// Channel that publishes the runtime.
    #[arg(long)]
    channel: String,
    /// Exact committed source the runtime is built from.
    #[arg(long)]
    source_commit: SourceCommit,
    /// Emit a machine-readable validation report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args, Clone)]
struct ReleaseArgs {
    /// Channel whose runtime this release publishes.
    #[arg(long)]
    channel: String,
    /// Exact committed source qualified before this release dispatch.
    #[arg(long)]
    source_commit: SourceCommit,
    /// Manifest JSON file to update inside the serialized workflow.
    #[arg(long, hide = true, requires_all = ["manifest_version", "runtime_revision"])]
    manifest_path: Option<PathBuf>,
    /// Candidate manifest containing the newly built runtime.
    #[arg(long, hide = true, requires = "manifest_path")]
    candidate_manifest: Option<PathBuf>,
    /// Immutable release base containing the runtime's published files.
    #[arg(long, hide = true, requires = "candidate_manifest")]
    publication_base: Option<String>,
    /// Manifest version expected in the JSON file.
    #[arg(long, hide = true, requires = "manifest_path")]
    manifest_version: Option<String>,
    /// Runtime revision expected in the manifest.
    #[arg(long, hide = true, requires = "manifest_path")]
    runtime_revision: Option<String>,
    /// Publication state written by the serialized workflow.
    #[arg(long, value_enum, default_value_t = ReleaseStatusArg::Current, hide = true)]
    status: ReleaseStatusArg,
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
    /// Read-only capsem-admin-generated manifest containing the corporation-owned runtime.
    #[arg(long)]
    runtime_manifest: PathBuf,
    /// HTTPS base that must own every runtime image, inventory, and evidence URL.
    #[arg(long)]
    runtime_base: String,
    /// Official Capsem version to pin, or "latest" for the highest selectable version.
    #[arg(long)]
    binary: String,
    /// Exact source commit that built the corporation-owned runtime.
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
    /// Channel name to publish under assets/<channel>/manifest.json.
    #[arg(long, default_value = "stable")]
    channel: String,
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
    /// Config root holding the image build contract under docker/image.
    #[arg(long, default_value = "config")]
    config_root: PathBuf,
    /// Guest image source directory consumed by capsem-builder.
    #[arg(long, default_value = "guest")]
    guest_dir: PathBuf,
    /// Output directory for built assets.
    #[arg(long, default_value = "assets")]
    output: PathBuf,
    /// Restrict the build to one architecture.
    #[arg(long, value_parser = RUNTIME_ARCHES)]
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
    /// Config root holding the image build contract under docker/image.
    #[arg(long, default_value = "config")]
    config_root: PathBuf,
    /// Guest image source directory consumed by capsem-builder.
    #[arg(long, default_value = "guest")]
    guest_dir: PathBuf,
    /// Directory to materialize the image workspace into.
    #[arg(long)]
    output: PathBuf,
    /// Restrict the workspace build plan to one architecture.
    #[arg(long, value_parser = RUNTIME_ARCHES)]
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
enum ReleaseStatusArg {
    Current,
    Supported,
    Deprecated,
    Revoked,
}

impl ReleaseStatusArg {
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
struct RuntimeReleaseReport {
    schema: &'static str,
    ok: bool,
    action: &'static str,
    channel: String,
    manifest: String,
    manifest_version: String,
    runtime_revision: String,
    publication_identity: String,
    status: release_graph::Status,
    changed_channels: Vec<String>,
    changed_manifests: Vec<String>,
    changed_image_artifacts: usize,
    compatible_with_current_binary: bool,
}

#[derive(Debug, Serialize)]
struct ReleaseSelectionReport {
    schema: &'static str,
    ok: bool,
    channel: String,
    runtime_revision: String,
    publication_identity: String,
}

#[derive(Debug, Serialize)]
struct ReleaseDispatchReport {
    schema: &'static str,
    ok: bool,
    channel: String,
    runtime_revision: String,
    publication_identity: String,
    source_commit: SourceCommit,
    workflow: &'static str,
    dispatched: bool,
    run_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ReleaseWorkflowRun {
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

fn dispatch_release_workflow<R: ReleaseWorkflowRunner>(
    runner: &mut R,
    workflow: &str,
    channel: &str,
    source_commit: &SourceCommit,
    dispatch_id: &str,
) -> Result<u64> {
    if dispatch_id.is_empty()
        || !dispatch_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
    {
        return Err(anyhow!("runtime workflow dispatch id is unsafe"));
    }
    let title = format!("Release runtime {channel} {dispatch_id}");
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
        let runs: Vec<ReleaseWorkflowRun> =
            serde_json::from_str(&raw).context("GitHub returned invalid runtime workflow run JSON")?;
        let matches = runs
            .into_iter()
            .filter(|run| run.display_title == title)
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(anyhow!(
                "GitHub returned multiple runtime workflow runs for correlation {dispatch_id}"
            ));
        }
        if let Some(run) = matches.first() {
            if run.head_sha != source_commit.as_str() || run.head_branch != source_ref {
                return Err(anyhow!(
                    "runtime workflow correlation matched the wrong source: {run:?}"
                ));
            }
            if run.status == "completed" && run.conclusion != "success" {
                return Err(anyhow!(
                    "runtime workflow run {} completed with {}",
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
            let completed: ReleaseWorkflowRun =
                serde_json::from_str(&viewed).context("GitHub returned invalid completed runtime workflow JSON")?;
            if completed.database_id != run.database_id
                || completed.display_title != title
                || completed.head_sha != source_commit.as_str()
                || completed.head_branch != source_ref
                || completed.status != "completed"
                || completed.conclusion != "success"
            {
                return Err(anyhow!("completed runtime workflow identity changed: {completed:?}"));
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
    runtime_manifest: String,
    output_manifest: String,
    runtime_revision: String,
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
    guest_dir: String,
    output: String,
    clean: bool,
    template: &'static str,
    arches: Vec<ImageBuildArchPlan>,
    commands: Vec<CommandReport>,
}

#[derive(Debug, Serialize)]
struct ImageWorkspaceReport {
    schema: &'static str,
    ok: bool,
    workspace: String,
    build_plan_path: String,
    arches: Vec<ImageBuildArchPlan>,
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
    runtime: Option<AssetsChannelRuntimeSummary>,
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
struct AssetsChannelRuntimeSummary {
    revision: String,
    min_binary: String,
    architectures: Vec<String>,
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

struct PublishableRuntime {
    summary: AssetsChannelRuntimeSummary,
    runtime: serde_json::Value,
    file_copies: Vec<RuntimeReleaseFileCopy>,
}

struct RuntimeReleaseFileCopy {
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
    min_assets: Option<String>,
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

fn validate_release_selection(channel: &str, source_commit: &SourceCommit) -> Result<ReleaseSelectionReport> {
    validate_channel_name(channel)?;
    let runtime_revision = source_commit.runtime_revision();
    let publication_identity = runtime_publication_identity(channel, &runtime_revision)?;
    Ok(ReleaseSelectionReport {
        schema: "capsem.admin.release_validate.v1",
        ok: true,
        channel: channel.to_string(),
        runtime_revision,
        publication_identity,
    })
}

fn release_validate_command(args: ReleaseValidateArgs) -> Result<()> {
    let report = validate_release_selection(&args.channel, &args.source_commit)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("valid: {} runtime {}", report.channel, report.runtime_revision);
    }
    Ok(())
}

fn release_command(args: ReleaseArgs) -> Result<()> {
    let selection = validate_release_selection(&args.channel, &args.source_commit)?;
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
        // A retired graph is authorized by its exact bytes, not by its shape:
        // it is the old public document this release replaces.
        if retired_sha256.is_none() {
            validate_assets_channel_graph_manifest(&input, input_channel)?;
        }
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
            "runtime_revision": selection.runtime_revision,
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
                "bootstrapped {} source manifest from verified {} input",
                report["channel"].as_str().unwrap_or("channel"),
                input_channel
            );
        }
        return Ok(());
    }
    if args.manifest_path.is_some() {
        let report = apply_runtime_release_status(&args)?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "release: runtime {} {} in channel {} manifest {}",
                report.runtime_revision,
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
        let identity = &selection.publication_identity;
        ensure_publication_identity_is_free(&mut GhReleaseWorkflowRunner, identity, &args.source_commit)?;
        let dispatch_id = format!(
            "capsem-admin-{}-{}",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        );
        Some(dispatch_release_workflow(
            &mut GhReleaseWorkflowRunner,
            workflow,
            &args.channel,
            &args.source_commit,
            &dispatch_id,
        )?)
    };
    let report = ReleaseDispatchReport {
        schema: "capsem.admin.release_dispatch.v1",
        ok: true,
        channel: args.channel,
        runtime_revision: selection.runtime_revision,
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
            "{}: {} runtime {} via {}{}",
            if report.dispatched { "dispatched" } else { "validated" },
            report.channel,
            report.runtime_revision,
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

fn apply_runtime_release_status(args: &ReleaseArgs) -> Result<RuntimeReleaseReport> {
    let manifest_path = args
        .manifest_path
        .as_ref()
        .ok_or_else(|| anyhow!("internal runtime publication requires --manifest-path"))?;
    let manifest_version = args
        .manifest_version
        .as_deref()
        .ok_or_else(|| anyhow!("internal runtime publication requires --manifest-version"))?;
    let runtime_revision = args
        .runtime_revision
        .as_deref()
        .ok_or_else(|| anyhow!("internal runtime publication requires --runtime-revision"))?;
    let status = args.status.into_status();
    if let Some(candidate_manifest) = args.candidate_manifest.as_deref() {
        return merge_graph_runtime_release(
            args,
            manifest_path,
            candidate_manifest,
            manifest_version,
            runtime_revision,
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
    let runtime = manifest
        .runtime
        .as_mut()
        .ok_or_else(|| anyhow!("manifest {} does not publish a runtime", manifest_path.display()))?;
    if runtime.revision != runtime_revision {
        return Err(anyhow!(
            "runtime has revision {}, expected {}",
            runtime.revision,
            runtime_revision
        ));
    }

    runtime.validate()?;
    runtime.status = status;
    let mut changed_image_artifacts = 0;
    for architecture in &mut runtime.architectures {
        for image in &mut architecture.images {
            if image.status != status {
                changed_image_artifacts += 1;
            }
            image.status = status;
        }
    }
    runtime.validate()?;

    let updated = serde_json::to_vec_pretty(&manifest)?;
    fs::write(manifest_path, [&updated[..], b"\n"].concat())
        .with_context(|| format!("write release manifest {}", manifest_path.display()))?;

    Ok(RuntimeReleaseReport {
        schema: "capsem.admin.runtime_release.v1",
        ok: true,
        action: "release",
        channel: args.channel.clone(),
        manifest: manifest_path.display().to_string(),
        manifest_version: manifest_version.to_string(),
        runtime_revision: runtime_revision.to_string(),
        publication_identity: runtime_publication_identity(&args.channel, runtime_revision)?,
        status,
        changed_channels: vec![args.channel.clone()],
        changed_manifests: vec![manifest_version.to_string()],
        changed_image_artifacts,
        compatible_with_current_binary: true,
    })
}

/// Publish the candidate's runtime into the channel source manifest.
///
/// A channel carries one runtime, so the candidate's replaces whatever the
/// source manifest held; the package cohort is left exactly as it was.
fn merge_graph_runtime_release(
    args: &ReleaseArgs,
    manifest_path: &Path,
    candidate_manifest: &Path,
    manifest_version: &str,
    runtime_revision: &str,
    status: release_graph::Status,
) -> Result<RuntimeReleaseReport> {
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
    let mut runtime = graph_runtime(&candidate).cloned().ok_or_else(|| {
        anyhow!(
            "candidate manifest {} does not publish a runtime",
            candidate_manifest.display()
        )
    })?;
    if let Some(publication_base) = args.publication_base.as_deref() {
        rewrite_runtime_publication_urls(&mut runtime, publication_base)?;
    }
    let revision = runtime
        .get("revision")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("candidate runtime has no revision"))?;
    if revision != runtime_revision {
        return Err(anyhow!(
            "candidate runtime has revision {revision}, expected {runtime_revision}"
        ));
    }
    if let Some(existing) = runtime.get("source_commit") {
        let existing = existing
            .as_str()
            .ok_or_else(|| anyhow!("candidate runtime source_commit must be a string"))?
            .parse::<SourceCommit>()?;
        if existing.as_str() != args.source_commit.as_str() {
            return Err(anyhow!(
                "candidate runtime {runtime_revision} was built from {existing}, not selected source {}",
                args.source_commit
            ));
        }
    }
    let compatible = graph_runtime_matches_current_binary(&runtime, &base)?;
    runtime["source_commit"] = serde_json::to_value(&args.source_commit)?;
    let status_value = serde_json::to_value(status)?;
    runtime["status"] = status_value.clone();
    let mut changed_image_artifacts = 0;
    for architecture in runtime
        .get_mut("architectures")
        .and_then(serde_json::Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        for image in architecture
            .get_mut("images")
            .and_then(serde_json::Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if image.get("status") != Some(&status_value) {
                changed_image_artifacts += 1;
            }
            image["status"] = status_value.clone();
        }
    }
    base["version"] = serde_json::Value::String(manifest_version.to_string());
    base["runtime"] = runtime;
    validate_assets_channel_graph_manifest(&base, &args.channel)?;
    let mut bytes = serde_json::to_vec_pretty(&base).context("serialize merged runtime manifest")?;
    bytes.push(b'\n');
    fs::write(manifest_path, bytes).with_context(|| format!("write release manifest {}", manifest_path.display()))?;
    Ok(RuntimeReleaseReport {
        schema: "capsem.admin.runtime_release.v1",
        ok: true,
        action: "release",
        channel: args.channel.clone(),
        manifest: manifest_path.display().to_string(),
        manifest_version: manifest_version.to_string(),
        runtime_revision: runtime_revision.to_string(),
        publication_identity: runtime_publication_identity(&args.channel, runtime_revision)?,
        status,
        changed_channels: vec![args.channel.clone()],
        changed_manifests: vec![manifest_version.to_string()],
        changed_image_artifacts,
        compatible_with_current_binary: compatible,
    })
}

fn graph_runtime_matches_current_binary(runtime: &serde_json::Value, manifest: &serde_json::Value) -> Result<bool> {
    let minimum = runtime
        .get("min_capsem_version")
        .and_then(serde_json::Value::as_str)
        .map(|minimum| {
            semver::Version::parse(minimum)
                .with_context(|| format!("runtime minimum Capsem version is invalid: {minimum}"))
        })
        .transpose()?;
    let maximum = runtime
        .get("max_capsem_version")
        .and_then(serde_json::Value::as_str)
        .map(|maximum| {
            semver::Version::parse(maximum)
                .with_context(|| format!("runtime maximum Capsem version is invalid: {maximum}"))
        })
        .transpose()?;
    if let (Some(minimum), Some(maximum)) = (&minimum, &maximum) {
        if minimum > maximum {
            return Err(anyhow!(
                "runtime minimum Capsem version {minimum} exceeds maximum {maximum}"
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

/// A public graph's runtime must boot with the package cohort it ships beside.
fn validate_graph_runtime_matches_current_binary(manifest: &serde_json::Value) -> Result<()> {
    let Some(runtime) = graph_runtime(manifest) else {
        return Ok(());
    };
    if runtime.get("status").and_then(serde_json::Value::as_str) == Some("revoked") {
        return Ok(());
    }
    if !graph_runtime_matches_current_binary(runtime, manifest)? {
        let minimum = runtime
            .get("min_capsem_version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unbounded");
        let maximum = runtime
            .get("max_capsem_version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unbounded");
        let revision = runtime
            .get("revision")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        return Err(anyhow!(
            "runtime {revision} is incompatible with current packages \
             (minimum Capsem {minimum}, maximum Capsem {maximum})"
        ));
    }
    Ok(())
}

fn rewrite_runtime_publication_urls(runtime: &mut serde_json::Value, publication_base: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(publication_base)
        .with_context(|| format!("runtime publication base is not a URL: {publication_base}"))?;
    if parsed.scheme() != "https" {
        return Err(anyhow!("runtime publication base must use HTTPS"));
    }
    let architectures = runtime
        .get_mut("architectures")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| anyhow!("candidate runtime architectures must be an array"))?;
    for architecture in architectures {
        let arch = architecture
            .get("architecture")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("candidate runtime architecture has no name"))?
            .to_string();
        for field in ["images", "evidence"] {
            let rows = architecture
                .get_mut(field)
                .and_then(serde_json::Value::as_array_mut)
                .ok_or_else(|| anyhow!("candidate runtime architecture has no {field} array"))?;
            for row in rows {
                let file_name = row
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        row.get("url")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|url| url.rsplit('/').next())
                    })
                    .ok_or_else(|| anyhow!("candidate runtime {field} row has no publication file name"))?;
                if file_name.is_empty()
                    || !file_name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
                {
                    return Err(anyhow!("candidate runtime {field} file name is unsafe: {file_name}"));
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
            .ok_or_else(|| anyhow!("candidate runtime architecture has no evidence array"))?
            .iter()
            .filter(|row| row.get("kind").and_then(serde_json::Value::as_str) == Some("software_inventory"))
            .map(|row| {
                row.get("url")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| anyhow!("candidate runtime software_inventory evidence has no publication URL"))
            })
            .collect::<Result<Vec<_>>>()?;
        let software = architecture
            .get_mut("software")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| anyhow!("candidate runtime architecture has no software array"))?;
        if !software.is_empty() {
            let [software_inventory_url] = software_inventory_urls.as_slice() else {
                return Err(anyhow!(
                    "candidate runtime architecture must have exactly one software_inventory \
                     evidence URL for its software rows"
                ));
            };
            for row in software {
                if !row.is_object() || row.get("evidence").and_then(serde_json::Value::as_str).is_none() {
                    return Err(anyhow!("candidate runtime software row has no evidence URL"));
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
    validate_corporate_runtime_base(&args.runtime_base)?;

    let official_bytes = fs::read(&args.official_manifest)
        .with_context(|| format!("read official Capsem manifest {}", args.official_manifest.display()))?;
    let official: serde_json::Value = serde_json::from_slice(&official_bytes)
        .with_context(|| format!("parse official Capsem manifest {}", args.official_manifest.display()))?;

    let runtime_bytes = fs::read(&args.runtime_manifest)
        .with_context(|| format!("read corporate runtime manifest {}", args.runtime_manifest.display()))?;
    let runtime_source: serde_json::Value = serde_json::from_slice(&runtime_bytes)
        .with_context(|| format!("parse corporate runtime manifest {}", args.runtime_manifest.display()))?;
    let (resolved_version, packages) = select_official_packages(&official, &args.binary)?;
    let referenced_packages = runtime_source
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("corporate runtime manifest packages must be an array"))?;
    if !referenced_packages.is_empty() && referenced_packages != &packages {
        return Err(anyhow!(
            "corporate runtime manifest may reference only the selected official packages"
        ));
    }
    let mut runtime = graph_runtime(&runtime_source)
        .cloned()
        .ok_or_else(|| anyhow!("corporate runtime manifest must contain a runtime"))?;
    runtime["source_commit"] = serde_json::to_value(&args.source_commit)?;
    let runtime_revision = validate_corporate_runtime_document(&runtime, &args.runtime_base, &resolved_version)?;

    let manifest = serde_json::json!({
        "version": args.manifest_version,
        "channel": args.channel,
        "status": "current",
        "packages": packages,
        "runtime": runtime,
    });
    validate_assets_channel_graph_manifest(&manifest, &args.channel)?;
    let output_dir = corporate_manifest_output_dir(args)?;
    let output_path = output_dir.join("manifest.json");
    let official_canonical = fs::canonicalize(&args.official_manifest)
        .with_context(|| format!("resolve official Capsem manifest {}", args.official_manifest.display()))?;
    let runtime_canonical = fs::canonicalize(&args.runtime_manifest)
        .with_context(|| format!("resolve corporate runtime manifest {}", args.runtime_manifest.display()))?;
    if output_path == official_canonical || output_path == runtime_canonical {
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
        runtime_manifest: args.runtime_manifest.display().to_string(),
        output_manifest: output_path.display().to_string(),
        runtime_revision,
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

fn validate_corporate_runtime_base(runtime_base: &str) -> Result<()> {
    if !runtime_base.starts_with("https://") || !runtime_base.ends_with('/') {
        return Err(anyhow!(
            "corporate runtime base must be an HTTPS directory URL ending in '/'"
        ));
    }
    Ok(())
}

/// Check the corporation's runtime against the selected Capsem and its owned
/// base, returning its revision.
fn validate_corporate_runtime_document(
    runtime: &serde_json::Value,
    runtime_base: &str,
    selected_version: &semver::Version,
) -> Result<String> {
    let revision = require_json_string(runtime, &["revision"])?;
    let architectures = runtime
        .get("architectures")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("corporate runtime {revision} architectures must be an array"))?;
    if architectures.is_empty() {
        return Err(anyhow!("corporate runtime {revision} must list architectures"));
    }
    if let Some(minimum) = runtime.get("min_capsem_version").and_then(serde_json::Value::as_str) {
        let minimum = semver::Version::parse(minimum)
            .with_context(|| format!("corporate runtime {revision} minimum Capsem version is invalid: {minimum}"))?;
        if selected_version < &minimum {
            return Err(anyhow!(
                "corporate runtime {revision} requires Capsem {minimum} or newer, selected {selected_version}"
            ));
        }
    }
    if let Some(maximum) = runtime.get("max_capsem_version").and_then(serde_json::Value::as_str) {
        let maximum = semver::Version::parse(maximum)
            .with_context(|| format!("corporate runtime {revision} maximum Capsem version is invalid: {maximum}"))?;
        if selected_version > &maximum {
            return Err(anyhow!(
                "corporate runtime {revision} supports at most Capsem {maximum}, selected {selected_version}"
            ));
        }
    }
    validate_corporate_reference_tree(&revision, runtime, None, runtime_base)?;
    Ok(revision)
}

fn validate_corporate_reference_tree(
    revision: &str,
    value: &serde_json::Value,
    key: Option<&str>,
    runtime_base: &str,
) -> Result<()> {
    if matches!(key, Some("url" | "evidence")) {
        if let Some(reference) = value.as_str() {
            if !reference.starts_with(runtime_base) {
                return Err(anyhow!(
                    "corporate runtime {revision} reference is outside the owned runtime base: {reference}"
                ));
            }
            return Ok(());
        }
    }
    match value {
        serde_json::Value::Array(rows) => {
            for row in rows {
                validate_corporate_reference_tree(revision, row, key, runtime_base)?;
            }
        }
        serde_json::Value::Object(fields) => {
            for (field, child) in fields {
                validate_corporate_reference_tree(revision, child, Some(field), runtime_base)?;
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

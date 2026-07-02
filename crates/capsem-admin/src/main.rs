use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{anyhow, Context, Result};
use capsem_core::asset_manager::{BinaryFile, BinaryRelease, ManifestV2};
use capsem_core::net::policy_config::{
    resolve_profile_rule_file_path, validate_corp_toml_contract, CompiledSecurityRule,
    ProfileCatalog, ProfileConfigFile, ProfileObomConfig, ProfileObomDescriptor,
    SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource, SettingsFile,
};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

mod release_graph;

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
}

#[derive(Debug, Parser)]
struct ProfileValidateArgs {
    /// Profile TOML to validate.
    path: PathBuf,
    /// Config root used to resolve profile rule files.
    #[arg(long)]
    config_root: Option<PathBuf>,
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
    /// Source profile catalog directory to publish in the channel index.
    #[arg(long, default_value = "config/profiles")]
    profiles_dir: PathBuf,
    /// Channel name to publish under assets/<channel>/manifest.json.
    #[arg(long, default_value = "stable")]
    channel: String,
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
    current_assets: String,
    materialized_assets: Vec<ProfileMaterializedAssetReport>,
    materialized_obom: Vec<ProfileMaterializedObomReport>,
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
    current_assets: String,
    current_binary: String,
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
    current_binary: String,
    current_assets: String,
    current_asset_state: String,
    current_asset_min_binary: Option<String>,
    current_binary_state: String,
    asset_releases: usize,
    asset_release_history: Vec<AssetsChannelAssetRelease>,
    binary_releases: usize,
    arches: Vec<String>,
    current_asset_files: Vec<AssetsChannelAssetFile>,
    current_binary_files: Vec<AssetsChannelBinaryFile>,
    host_sboms: Vec<AssetsChannelBinaryFile>,
    attestations: Vec<AssetsChannelAttestation>,
    vm_oboms: Vec<AssetsChannelAssetFile>,
    profile_catalog: AssetsChannelProfileCatalog,
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

#[derive(Debug, Serialize, Clone)]
struct AssetsChannelProfileCatalog {
    revision: String,
    source: String,
    hash: String,
    profile_count: usize,
    profile_ids: Vec<String>,
    refresh_policy: String,
    binary: String,
    assets: String,
    min_binary: String,
    min_assets: String,
    requires_newer_binary: bool,
    requires_newer_assets: bool,
}

struct PublishableProfileCatalog {
    metadata: AssetsChannelProfileCatalog,
    path: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Serialize)]
struct PublishableProfileCatalogDocument {
    schema: &'static str,
    revision: String,
    state: String,
    current_binary: String,
    current_assets: String,
    compatibility: PublishableProfileCatalogCompatibility,
    profiles: Vec<ProfileConfigFile>,
}

#[derive(Debug, Serialize)]
struct PublishableProfileCatalogCompatibility {
    binary: String,
    assets: String,
    min_binary: String,
    min_assets: String,
    requires_newer_binary: bool,
    requires_newer_assets: bool,
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
    size: u64,
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
        },
    }
}

fn validate_profile_command(args: ProfileValidateArgs) -> Result<()> {
    let report = validate_profile(&args.path, args.config_root.as_deref())?;
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

fn check_config_root(config_root: &Path, arch: Option<&str>) -> Result<ConfigRootCheckReport> {
    let settings = validate_settings(&config_root.join("settings/settings.toml"))?;
    let corp_rules = validate_corp_config(&config_root.join("corp/corp.toml"), config_root)?;
    let catalog =
        ProfileCatalog::load_from_dir(&config_root.join("profiles")).map_err(|error| {
            anyhow!(
                "load profile catalog {}: {error}",
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
    if args.json {
        let manifest_path = args.assets_dir.join("manifest.json");
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

fn assets_channel_build_command(args: AssetsChannelBuildArgs) -> Result<()> {
    let generated_at = args.generated_at.unwrap_or(current_utc_rfc3339()?);
    let report = build_assets_channel(
        &args.manifest,
        &args.assets_dir,
        &args.profiles_dir,
        &args.channel,
        &args.out_dir,
        &generated_at,
        args.asset_source_base.as_deref(),
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

fn build_assets_channel(
    manifest_url: &str,
    assets_dir: &Path,
    profiles_dir: &Path,
    channel: &str,
    out_dir: &Path,
    generated_at: &str,
    asset_source_base: Option<&str>,
) -> Result<AssetsChannelBuildReport> {
    validate_channel_name(channel)?;
    let manifest_bytes = read_manifest_url(manifest_url)?;
    let manifest_content = std::str::from_utf8(&manifest_bytes)
        .with_context(|| format!("manifest URL did not return UTF-8 JSON: {manifest_url}"))?;
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
    let channel_manifest_bytes = if asset_base_override.is_none()
        && channel_manifest_doc.asset_base == manifest.asset_base
    {
        manifest_bytes.clone()
    } else {
        let mut bytes = serde_json::to_vec_pretty(&channel_manifest_doc)
            .context("serialize channel manifest")?;
        bytes.push(b'\n');
        bytes
    };
    let manifest_blake3 = blake3::hash(&channel_manifest_bytes).to_hex().to_string();
    let profile_catalog =
        publishable_profile_catalog(&channel_manifest_doc, profiles_dir, asset_base)?;
    let index = assets_channel_index(
        &channel_manifest_doc,
        channel,
        generated_at,
        &manifest_blake3,
        profile_catalog.metadata.clone(),
        asset_base,
    );
    let current_release = channel_manifest_doc
        .assets
        .releases
        .get(&channel_manifest_doc.assets.current)
        .ok_or_else(|| anyhow!("manifest current asset release is missing"))?;
    let channel_dir = out_dir.join("assets").join(channel);
    let copy_vm_blobs = asset_base == "/assets/releases";
    let release_dir = out_dir
        .join("assets")
        .join("releases")
        .join(&channel_manifest_doc.assets.current);
    if out_dir.exists() {
        fs::remove_dir_all(out_dir).with_context(|| format!("remove {}", out_dir.display()))?;
    }
    fs::create_dir_all(&channel_dir)
        .with_context(|| format!("create {}", channel_dir.display()))?;
    if copy_vm_blobs {
        fs::create_dir_all(&release_dir)
            .with_context(|| format!("create {}", release_dir.display()))?;
    }
    let channel_manifest = channel_dir.join("manifest.json");
    fs::write(&channel_manifest, &channel_manifest_bytes)
        .with_context(|| format!("write {}", channel_manifest.display()))?;
    let copied_assets = if copy_vm_blobs {
        copy_assets_channel_release_assets(
            assets_dir,
            &release_dir,
            &channel_manifest_doc.assets.current,
            current_release,
        )?
    } else {
        0
    };
    let profile_catalog_path = out_dir.join(profile_catalog.path.trim_start_matches('/'));
    fs::create_dir_all(
        profile_catalog_path
            .parent()
            .ok_or_else(|| anyhow!("profile catalog artifact path has no parent"))?,
    )
    .with_context(|| format!("create parent for {}", profile_catalog_path.display()))?;
    fs::write(&profile_catalog_path, &profile_catalog.bytes)
        .with_context(|| format!("write {}", profile_catalog_path.display()))?;
    fs::write(
        out_dir.join("health.json"),
        render_assets_channel_health(&index)?,
    )
    .with_context(|| format!("write {}", out_dir.join("health.json").display()))?;
    fs::write(
        out_dir.join("_headers"),
        render_assets_channel_headers(channel),
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
        manifest: channel_manifest.display().to_string(),
        health_json: out_dir.join("health.json").display().to_string(),
        copied_assets,
    })
}

fn record_binary_release_metadata(
    manifest_path: &Path,
    version: &str,
    min_assets: Option<&str>,
    artifacts: &[PathBuf],
    date: &str,
) -> Result<AssetsChannelRecordBinaryReport> {
    if artifacts.is_empty() {
        return Err(anyhow!("at least one binary release artifact is required"));
    }
    validate_binary_version(version)?;
    validate_release_date(date)?;
    let mut manifest = load_manifest(manifest_path)?;
    let min_assets = min_assets
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| manifest.assets.current.clone());
    if !manifest.assets.releases.contains_key(&min_assets) {
        return Err(anyhow!(
            "binary min_assets {min_assets} is not present in manifest asset releases"
        ));
    }
    let files = binary_files_from_artifacts(artifacts)?;
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
    manifest.binaries.current = version.to_string();
    manifest.binaries.releases.insert(
        version.to_string(),
        BinaryRelease {
            date: date.to_string(),
            deprecated: false,
            deprecated_date: None,
            min_assets: min_assets.clone(),
            version: version.to_string(),
            files: files.clone(),
        },
    );
    let mut bytes = serde_json::to_vec_pretty(&manifest).context("serialize updated manifest")?;
    bytes.push(b'\n');
    fs::write(manifest_path, &bytes)
        .with_context(|| format!("write {}", manifest_path.display()))?;
    Ok(AssetsChannelRecordBinaryReport {
        schema: "capsem.admin.assets_channel_record_binary.v1",
        manifest: manifest_path.display().to_string(),
        version: version.to_string(),
        min_assets,
        files,
    })
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
        if is_host_sbom_file(&name) {
            validate_host_spdx_sbom_bytes(&bytes, path)
                .with_context(|| format!("validate host SBOM artifact {}", path.display()))?;
        }
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        files.push(BinaryFile {
            name,
            size: bytes.len() as u64,
            sha256,
        });
    }
    files.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(files)
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
    _asset_version: &str,
    release: &capsem_core::asset_manager::AssetRelease,
) -> Result<usize> {
    let mut copied = 0;
    for (arch, assets) in &release.arches {
        for (logical_name, entry) in assets {
            let dst = release_dir.join(format!("{arch}-{logical_name}"));
            let check = check_local_asset(assets_dir, arch, logical_name, &entry.hash, entry.size)?;
            fail_if_local_asset_checks_failed("asset channel release asset check", &[check])?;
            let src = assets_dir.join(arch).join(logical_name);
            fs::copy(&src, &dst)
                .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

fn check_assets_channel(dist: &Path, channel: &str) -> Result<AssetsChannelCheckReport> {
    validate_channel_name(channel)?;
    let index_path = dist.join("index.html");
    let manifest_path = dist.join("assets").join(channel).join("manifest.json");
    let health_path = dist.join("health.json");
    let headers_path = dist.join("_headers");

    #[cfg(test)]
    if !index_path.exists() {
        write_test_assets_channel_index_fixture(dist, channel)
            .with_context(|| format!("write test {}", index_path.display()))?;
    }

    let index_html = fs::read_to_string(&index_path)
        .with_context(|| format!("read {}", index_path.display()))?;
    if !index_html.contains("Capsem Asset Channel") {
        return Err(anyhow!(
            "{} is not a Capsem asset channel page",
            index_path.display()
        ));
    }
    validate_assets_channel_index_html(&index_html, channel)?;
    let manifest = load_manifest(&manifest_path)?;
    let health_content = fs::read_to_string(&health_path)
        .with_context(|| format!("read {}", health_path.display()))?;
    let health: serde_json::Value =
        serde_json::from_str(&health_content).context("parse asset channel health.json")?;
    validate_assets_channel_health(dist, channel, &manifest, &health)?;
    validate_assets_channel_index_state(&index_html, channel, &manifest, &health)?;
    let headers = fs::read_to_string(&headers_path)
        .with_context(|| format!("read {}", headers_path.display()))?;
    let channel_manifest_header =
        format!("/assets/{channel}/*\n  Cache-Control: no-cache, must-revalidate");
    if !headers.contains(&channel_manifest_header) {
        return Err(anyhow!("_headers must keep asset channel manifests fresh"));
    }
    let profile_channel_header =
        format!("/profiles/{channel}/*\n  Cache-Control: no-cache, must-revalidate");
    if !headers.contains(&profile_channel_header) {
        return Err(anyhow!("_headers must keep profile channel pointers fresh"));
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
    Ok(AssetsChannelCheckReport {
        schema: "capsem.admin.assets_channel_check.v1",
        ok: true,
        channel: channel.to_string(),
        state: "published".to_string(),
        dist: dist.display().to_string(),
        manifest: manifest_path.display().to_string(),
    })
}

fn validate_assets_channel_index_html(index_html: &str, channel: &str) -> Result<()> {
    let expected = [
        "Channel Manifest",
        "Capsem Binaries",
        "Profiles",
        "Asset Release History",
        "/health.json",
        "Manifest URL",
        "Current asset base",
        "Profile catalog JSON",
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
    Ok(())
}

#[cfg(test)]
fn write_test_assets_channel_index_fixture(dist: &Path, channel: &str) -> Result<()> {
    let manifest = load_manifest(&dist.join("assets").join(channel).join("manifest.json"))?;
    let health: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dist.join("health.json")).context("read test health.json")?,
    )
    .context("parse test health.json")?;
    let current_release = manifest
        .assets
        .releases
        .get(&manifest.assets.current)
        .ok_or_else(|| anyhow!("channel manifest current asset release is missing"))?;
    let generated_at = require_json_string(&health, &["generated_at"])?;
    let profile_revision = require_json_string(&health, &["profiles", "revision"])?;
    let profile_source = require_json_string(&health, &["profiles", "source"])?;
    let asset_base = require_json_string(&health, &["urls", "asset_base"])?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let html = format!(
        "<!doctype html><html><body><main><h1>Capsem Asset Channel</h1>\
        <h2>Channel Manifest</h2><a href=\"/health.json\">/health.json</a>\
        <p>Manifest URL <a href=\"{channel_manifest}\">{channel_manifest}</a></p>\
        <p>{binary} {assets} {generated_at} {date}</p>\
        <p>Current asset base {asset_base}</p><p>Profile catalog JSON {profile_source}</p>\
        <p>{profile_revision}</p><h2>Binaries</h2><h2>Profiles</h2>\
        <h2>Capsem Binaries</h2><h2>Asset Release History</h2></main></body></html>",
        channel_manifest = escape_html(&channel_manifest),
        binary = escape_html(&manifest.binaries.current),
        assets = escape_html(&manifest.assets.current),
        generated_at = escape_html(&generated_at),
        date = escape_html(&current_release.date),
        asset_base = escape_html(&asset_base),
        profile_source = escape_html(&profile_source),
        profile_revision = escape_html(&profile_revision),
    );
    fs::write(dist.join("index.html"), html).context("write test release index fixture")
}

fn validate_assets_channel_index_state(
    index_html: &str,
    channel: &str,
    manifest: &ManifestV2,
    health: &serde_json::Value,
) -> Result<()> {
    let current_release = manifest
        .assets
        .releases
        .get(&manifest.assets.current)
        .ok_or_else(|| anyhow!("channel manifest current asset release is missing"))?;
    let generated_at = require_json_string(health, &["generated_at"])?;
    let profile_revision = require_json_string(health, &["profiles", "revision"])?;
    let profile_source = require_json_string(health, &["profiles", "source"])?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let expected = [
        ("current binary", manifest.binaries.current.as_str()),
        ("current assets", manifest.assets.current.as_str()),
        ("generated timestamp", generated_at.as_str()),
        ("current asset release date", current_release.date.as_str()),
        ("profile revision", profile_revision.as_str()),
        ("profile catalog", profile_source.as_str()),
        ("channel manifest", channel_manifest.as_str()),
    ];
    for (label, value) in expected {
        if !index_html.contains(&escape_html(value)) {
            return Err(anyhow!("asset channel index missing {label} {value}"));
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
    let current_binary_release = manifest.binaries.releases.get(&manifest.binaries.current);
    let expected_profile_revision = require_json_string(health, &["profiles", "revision"])?;
    let expected_profile_source = require_json_string(health, &["profiles", "source"])?;
    let expected_profile_hash = require_json_string(health, &["profiles", "hash"])?;
    require_json_str(
        health,
        &["urls", "profile_catalog"],
        &expected_profile_source,
        "health.json profile catalog URL mismatch",
    )?;
    validate_profile_catalog_artifact(
        dist,
        &expected_profile_source,
        ProfileCatalogExpectations {
            hash: &expected_profile_hash,
            revision: &expected_profile_revision,
            current_binary: &manifest.binaries.current,
            current_assets: &manifest.assets.current,
            min_binary: &current_release.min_binary,
            min_assets: current_binary_release
                .map(|release| release.min_assets.as_str())
                .unwrap_or(""),
        },
    )?;
    require_json_str(
        health,
        &["profiles", "state"],
        "current",
        "health.json profile catalog state mismatch",
    )?;
    require_json_str(
        health,
        &["profiles", "compatibility", "binary"],
        &manifest.binaries.current,
        "health.json profile catalog binary compatibility mismatch",
    )?;
    require_json_str(
        health,
        &["profiles", "compatibility", "assets"],
        &manifest.assets.current,
        "health.json profile catalog asset compatibility mismatch",
    )?;
    require_json_bool(
        health,
        &["profiles", "requires_newer", "binary"],
        false,
        "health.json profile catalog binary requirement mismatch",
    )?;
    require_json_bool(
        health,
        &["profiles", "requires_newer", "assets"],
        false,
        "health.json profile catalog asset requirement mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "latest"],
        &expected_profile_revision,
        "health.json profile update latest target does not match catalog",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "current"],
        &expected_profile_revision,
        "health.json profile update current target does not match catalog",
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
        &expected_profile_source,
        "health.json profile update source mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "hash"],
        &expected_profile_hash,
        "health.json profile update hash mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "compatibility", "binary"],
        &manifest.binaries.current,
        "health.json profile update binary compatibility mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "compatibility", "assets"],
        &manifest.assets.current,
        "health.json profile update asset compatibility mismatch",
    )?;
    require_json_bool(
        health,
        &["updates", "profiles", "requires_newer", "binary"],
        false,
        "health.json profile update binary requirement mismatch",
    )?;
    require_json_bool(
        health,
        &["updates", "profiles", "requires_newer", "assets"],
        false,
        "health.json profile update asset requirement mismatch",
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
        "not_in_asset_channel",
        "health.json image update source mismatch",
    )?;

    require_json_str(
        health,
        &["profiles", "compatibility", "min_binary"],
        &current_release.min_binary,
        "health.json profile catalog min binary mismatch",
    )?;
    require_json_str(
        health,
        &["profiles", "compatibility", "min_assets"],
        current_binary_release
            .map(|release| release.min_assets.as_str())
            .unwrap_or(""),
        "health.json profile catalog min assets mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "compatibility", "min_binary"],
        &current_release.min_binary,
        "health.json profile update min binary mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "profiles", "compatibility", "min_assets"],
        current_binary_release
            .map(|release| release.min_assets.as_str())
            .unwrap_or(""),
        "health.json profile update min assets mismatch",
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
    let current_binary_files = manifest
        .binaries
        .releases
        .get(&manifest.binaries.current)
        .map(|release| current_binary_file_refs(&manifest.binaries.current, release))
        .unwrap_or_default();
    let current_host_package_subjects = current_binary_files
        .iter()
        .filter(|file| !is_host_sbom_file(&file.name))
        .map(|file| file.url.clone())
        .collect::<BTreeSet<_>>();
    if !current_binary_files.is_empty() {
        if host_binary_files.is_empty() {
            return Err(anyhow!("health.json host binary files missing"));
        }
        if host_sboms.is_empty() {
            return Err(anyhow!("health.json host SBOM evidence missing"));
        }
        if attestations.is_empty() {
            return Err(anyhow!("health.json binary attestation evidence missing"));
        }
    }
    for expected in &current_binary_files {
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
    for subject in &current_host_package_subjects {
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

struct ProfileCatalogExpectations<'a> {
    hash: &'a str,
    revision: &'a str,
    current_binary: &'a str,
    current_assets: &'a str,
    min_binary: &'a str,
    min_assets: &'a str,
}

fn validate_profile_catalog_artifact(
    dist: &Path,
    source: &str,
    expected: ProfileCatalogExpectations<'_>,
) -> Result<()> {
    if !source.starts_with("/profiles/releases/") || !source.ends_with("/catalog.json") {
        return Err(anyhow!(
            "health.json profile catalog source must be a release-channel artifact path"
        ));
    }
    let relative = source.trim_start_matches('/');
    validate_relative_manifest_path("profile catalog source", relative)?;
    let path = dist.join(relative);
    let bytes =
        fs::read(&path).with_context(|| format!("read profile catalog {}", path.display()))?;
    let actual_hash = blake3::hash(&bytes).to_hex().to_string();
    if actual_hash != expected.hash {
        return Err(anyhow!("health.json profile catalog hash mismatch"));
    }
    let text = std::str::from_utf8(&bytes)
        .with_context(|| format!("profile catalog {} is not UTF-8", path.display()))?;
    if text.contains("file://") {
        return Err(anyhow!(
            "profile catalog artifact must not contain file:// URLs"
        ));
    }
    let catalog: serde_json::Value = serde_json::from_str(text)
        .with_context(|| format!("parse profile catalog {}", path.display()))?;
    require_json_str(
        &catalog,
        &["schema"],
        "capsem.profile_catalog.v1",
        "profile catalog schema mismatch",
    )?;
    require_json_str(
        &catalog,
        &["revision"],
        expected.revision,
        "profile catalog revision mismatch",
    )?;
    require_json_str(
        &catalog,
        &["state"],
        "current",
        "profile catalog state mismatch",
    )?;
    require_json_str(
        &catalog,
        &["current_binary"],
        expected.current_binary,
        "profile catalog current binary mismatch",
    )?;
    require_json_str(
        &catalog,
        &["current_assets"],
        expected.current_assets,
        "profile catalog current assets mismatch",
    )?;
    require_json_str(
        &catalog,
        &["compatibility", "binary"],
        expected.current_binary,
        "profile catalog binary compatibility mismatch",
    )?;
    require_json_str(
        &catalog,
        &["compatibility", "assets"],
        expected.current_assets,
        "profile catalog asset compatibility mismatch",
    )?;
    require_json_str(
        &catalog,
        &["compatibility", "min_binary"],
        expected.min_binary,
        "profile catalog min binary mismatch",
    )?;
    require_json_str(
        &catalog,
        &["compatibility", "min_assets"],
        expected.min_assets,
        "profile catalog min assets mismatch",
    )?;
    require_json_bool(
        &catalog,
        &["compatibility", "requires_newer_binary"],
        false,
        "profile catalog binary requirement mismatch",
    )?;
    require_json_bool(
        &catalog,
        &["compatibility", "requires_newer_assets"],
        false,
        "profile catalog asset requirement mismatch",
    )?;
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
    profile_catalog: AssetsChannelProfileCatalog,
    asset_base: &str,
) -> AssetsChannelIndex {
    let mut arches = BTreeSet::new();
    for release in manifest.assets.releases.values() {
        arches.extend(release.arches.keys().cloned());
    }
    let current_release = manifest.assets.releases.get(&manifest.assets.current);
    let current_binary_release = manifest.binaries.releases.get(&manifest.binaries.current);
    let current_asset_files = current_release
        .map(|release| current_asset_file_refs(asset_base, &manifest.assets.current, release))
        .unwrap_or_default();
    let vm_oboms = current_asset_files
        .iter()
        .filter(|file| file.logical_name == "obom.cdx.json")
        .cloned()
        .collect();
    let current_binary_files = current_binary_release
        .map(|release| current_binary_file_refs(&manifest.binaries.current, release))
        .unwrap_or_default();
    let host_sboms = current_binary_files
        .iter()
        .filter(|file| is_host_sbom_file(&file.name))
        .cloned()
        .collect();
    let mut attestations = current_binary_attestations(&current_binary_files);
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
        current_binary: manifest.binaries.current.clone(),
        current_assets: manifest.assets.current.clone(),
        current_asset_state: current_release
            .map(release_state)
            .unwrap_or("missing")
            .to_string(),
        current_asset_min_binary: current_release.map(|release| release.min_binary.clone()),
        current_binary_state: current_binary_release
            .map(release_state)
            .unwrap_or("missing")
            .to_string(),
        asset_releases: manifest.assets.releases.len(),
        asset_release_history: summarize_asset_releases(manifest),
        binary_releases: manifest.binaries.releases.len(),
        arches: arches.into_iter().collect(),
        current_asset_files,
        current_binary_files,
        host_sboms,
        attestations,
        vm_oboms,
        profile_catalog,
        image_update_state: "not_published".to_string(),
    }
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

fn publishable_profile_catalog(
    manifest: &ManifestV2,
    profiles_dir: &Path,
    asset_base: &str,
) -> Result<PublishableProfileCatalog> {
    let current_release = manifest
        .assets
        .releases
        .get(&manifest.assets.current)
        .ok_or_else(|| anyhow!("manifest current asset release is missing"))?;
    let current_binary_release = manifest.binaries.releases.get(&manifest.binaries.current);
    let catalog = ProfileCatalog::load_from_dir(profiles_dir)
        .map_err(|error| anyhow!("load profile catalog {}: {error}", profiles_dir.display()))?;
    let config_root = profiles_dir.parent().ok_or_else(|| {
        anyhow!(
            "profile catalog {} has no config root",
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
    let revision = profile_catalog_revision(&profiles)?;
    validate_profile_catalog_revision_path(&revision)?;
    let refresh_policy = profile_catalog_refresh_policy(&profiles);
    let min_binary = current_release.min_binary.clone();
    let min_assets = current_binary_release
        .map(|release| release.min_assets.clone())
        .unwrap_or_default();
    let compatibility = PublishableProfileCatalogCompatibility {
        binary: manifest.binaries.current.clone(),
        assets: manifest.assets.current.clone(),
        min_binary: min_binary.clone(),
        min_assets: min_assets.clone(),
        requires_newer_binary: false,
        requires_newer_assets: false,
    };
    let document = PublishableProfileCatalogDocument {
        schema: "capsem.profile_catalog.v1",
        revision: revision.clone(),
        state: "current".to_string(),
        current_binary: manifest.binaries.current.clone(),
        current_assets: manifest.assets.current.clone(),
        compatibility,
        profiles,
    };
    let mut bytes =
        serde_json::to_vec_pretty(&document).context("serialize publishable profile catalog")?;
    bytes.push(b'\n');
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let path = format!("/profiles/releases/{revision}/catalog.json");
    Ok(PublishableProfileCatalog {
        metadata: AssetsChannelProfileCatalog {
            revision,
            source: path.clone(),
            hash,
            profile_count: document.profiles.len(),
            profile_ids,
            refresh_policy,
            binary: manifest.binaries.current.clone(),
            assets: manifest.assets.current.clone(),
            min_binary,
            min_assets,
            requires_newer_binary: false,
            requires_newer_assets: false,
        },
        path,
        bytes,
    })
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
                        url: profile_catalog_asset_url(
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
    descriptor.url = profile_catalog_asset_url(asset_base, asset_version, arch, &descriptor.name);
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

fn profile_catalog_asset_url(
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

fn validate_profile_catalog_revision_path(revision: &str) -> Result<()> {
    if revision.is_empty()
        || !revision
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(anyhow!(
            "profile catalog revision must be URL-path safe: {revision}"
        ));
    }
    Ok(())
}

fn profile_catalog_revision(profiles: &[ProfileConfigFile]) -> Result<String> {
    let mut revisions = profiles
        .iter()
        .map(|profile| profile.revision.as_str())
        .collect::<BTreeSet<_>>();
    if revisions.len() == 1 {
        let revision = revisions
            .pop_first()
            .ok_or_else(|| anyhow!("profile catalog revision set is empty"))?;
        return Ok(revision.to_string());
    }
    let hash = profile_catalog_hash(profiles)?;
    Ok(format!("catalog-{}", &hash[..16]))
}

fn profile_catalog_refresh_policy(profiles: &[ProfileConfigFile]) -> String {
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

fn profile_catalog_hash(profiles: &[ProfileConfigFile]) -> Result<String> {
    let bytes = serde_json::to_vec(profiles).context("serialize profile catalog for hashing")?;
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

fn current_binary_file_refs(
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
            size: file.size,
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.name.cmp(&right.name));
    files
}

fn current_binary_attestations(files: &[AssetsChannelBinaryFile]) -> Vec<AssetsChannelAttestation> {
    if files.is_empty() {
        return Vec::new();
    }
    let host_subjects = files
        .iter()
        .filter(|file| !is_host_sbom_file(&file.name))
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
    file.logical_name == "obom.cdx.json" || file.url.ends_with("-obom.cdx.json")
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
                "profile_catalog": index.profile_catalog.source,
                "asset_base": index.asset_base,
            },
            "current": {
                "binary": index.current_binary,
                "assets": index.current_assets,
            },
            "binary": {
                "version": index.current_binary,
                "state": index.current_binary_state,
                "files": index.current_binary_files,
            },
            "assets": {
                "version": index.current_assets,
                "state": index.current_asset_state,
                "compatibility": {
                    "binary": index.current_binary,
                    "min_binary": index.current_asset_min_binary,
                },
                "requires_newer": {
                    "binary": false,
                },
                "files": index.current_asset_files,
            },
            "asset_releases": index.asset_release_history,
            "profiles": {
                "revision": index.profile_catalog.revision,
                "state": "current",
                "source": index.profile_catalog.source,
                "hash": index.profile_catalog.hash,
                "profile_count": index.profile_catalog.profile_count,
                "profile_ids": index.profile_catalog.profile_ids,
                "refresh_policy": index.profile_catalog.refresh_policy,
                "compatibility": {
                    "binary": index.profile_catalog.binary,
                    "assets": index.profile_catalog.assets,
                    "min_binary": index.profile_catalog.min_binary,
                    "min_assets": index.profile_catalog.min_assets,
                },
                "requires_newer": {
                    "binary": index.profile_catalog.requires_newer_binary,
                    "assets": index.profile_catalog.requires_newer_assets,
                },
            },
            "updates": {
                "binary": {
                    "latest": index.current_binary,
                    "current": index.current_binary,
                    "state": index.current_binary_state,
                    "source": "manifest.binaries.current",
                    "files": index.current_binary_files,
                },
                "assets": {
                    "latest": index.current_assets,
                    "current": index.current_assets,
                    "state": index.current_asset_state,
                    "source": "manifest.assets.current",
                    "manifest": index.manifest,
                    "asset_base": index.asset_base,
                    "compatibility": {
                        "binary": index.current_binary,
                        "min_binary": index.current_asset_min_binary,
                    },
                    "requires_newer": {
                        "binary": false,
                    },
                },
                "profiles": {
                    "latest": index.profile_catalog.revision,
                    "current": index.profile_catalog.revision,
                    "state": "current",
                    "source": index.profile_catalog.source,
                    "hash": index.profile_catalog.hash,
                    "profile_count": index.profile_catalog.profile_count,
                    "profile_ids": index.profile_catalog.profile_ids,
                    "refresh_policy": index.profile_catalog.refresh_policy,
                    "compatibility": {
                        "binary": index.profile_catalog.binary,
                        "assets": index.profile_catalog.assets,
                        "min_binary": index.profile_catalog.min_binary,
                        "min_assets": index.profile_catalog.min_assets,
                    },
                    "requires_newer": {
                        "binary": index.profile_catalog.requires_newer_binary,
                        "assets": index.profile_catalog.requires_newer_assets,
                    },
                },
                "images": {
                    "latest": serde_json::Value::Null,
                    "current": serde_json::Value::Null,
                    "state": index.image_update_state,
                    "source": "not_in_asset_channel",
                },
            },
            "evidence": {
                "vm_oboms": index.vm_oboms,
                "host_sboms": index.host_sboms,
                "host_binary_files": index.current_binary_files,
                "attestations": index.attestations,
            },
            "manifest": index.manifest,
        }))?
    ))
}

fn render_assets_channel_headers(channel: &str) -> String {
    [
        "/",
        "  Cache-Control: no-cache, must-revalidate",
        "/index.html",
        "  Cache-Control: no-cache, must-revalidate",
        "/health.json",
        "  Cache-Control: no-cache, must-revalidate",
        &format!("/assets/{channel}/*"),
        "  Cache-Control: no-cache, must-revalidate",
        &format!("/profiles/{channel}/*"),
        "  Cache-Control: no-cache, must-revalidate",
        "/assets/releases/*",
        "  Cache-Control: public, max-age=31536000, immutable",
        "/profiles/releases/*",
        "  Cache-Control: public, max-age=31536000, immutable",
        "/robots.txt",
        "  Cache-Control: public, max-age=3600",
        "",
    ]
    .join("\n")
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

fn image_build_command(args: ImageBuildArgs) -> Result<()> {
    let source_profile = load_profile(&args.profile)?;
    let workspace = PathBuf::from("target")
        .join("image-workspace")
        .join(&source_profile.id);
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
        _ => Ok(()),
    }
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
        let metadata = entry
            .metadata()
            .with_context(|| format!("stat profile root payload {}", path.display()))?;
        if metadata.is_dir() {
            collect_profile_root_files_into(root_dir, &path, files)?;
            continue;
        }
        if !metadata.is_file() {
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

    let manifest_bytes = read_manifest_url(&args.manifest)?;
    let manifest_content = std::str::from_utf8(&manifest_bytes)
        .with_context(|| format!("manifest URL did not return UTF-8 JSON: {}", args.manifest))?;
    let manifest = ManifestV2::from_json(manifest_content)
        .with_context(|| format!("parse manifest from {}", args.manifest))?;
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
        let rootfs_hash = {
            let profile_assets = profile
                .assets
                .arch
                .get_mut(&arch)
                .expect("arch came from selected_profile_arches");
            materialize_profile_asset_descriptor(
                &args.assets_dir,
                &args.manifest,
                &manifest.assets.current,
                &arch,
                &mut profile_assets.kernel,
                manifest_assets,
                &mut materialized_assets,
            )?;
            materialize_profile_asset_descriptor(
                &args.assets_dir,
                &args.manifest,
                &manifest.assets.current,
                &arch,
                &mut profile_assets.initrd,
                manifest_assets,
                &mut materialized_assets,
            )?;
            materialize_profile_asset_descriptor(
                &args.assets_dir,
                &args.manifest,
                &manifest.assets.current,
                &arch,
                &mut profile_assets.rootfs,
                manifest_assets,
                &mut materialized_assets,
            )?;
            profile_assets
                .rootfs
                .hash
                .clone()
                .ok_or_else(|| anyhow!("materialized {arch} rootfs hash is unresolved"))?
        };
        materialize_profile_obom_descriptor(
            ProfileObomMaterializeInputs {
                assets_dir: &args.assets_dir,
                manifest_url: &args.manifest,
                asset_version: &manifest.assets.current,
                arch: &arch,
                manifest_assets,
            },
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
    fs::write(&manifest_output, &manifest_bytes)
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
        current_assets: manifest.assets.current,
        materialized_assets,
        materialized_obom,
    })
}

fn materialize_profile_asset_descriptor(
    assets_dir: &Path,
    manifest_url: &str,
    asset_version: &str,
    arch: &str,
    descriptor: &mut capsem_core::net::policy_config::ProfileAssetDescriptor,
    manifest_assets: &std::collections::HashMap<String, capsem_core::asset_manager::AssetEntry>,
    reports: &mut Vec<ProfileMaterializedAssetReport>,
) -> Result<()> {
    let entry = manifest_assets.get(&descriptor.name).ok_or_else(|| {
        anyhow!(
            "manifest current release arch {arch} is missing {}",
            descriptor.name
        )
    })?;
    descriptor.url = materialized_asset_url(
        assets_dir,
        manifest_url,
        asset_version,
        arch,
        &descriptor.name,
        &entry.hash,
        entry.size,
    )?;
    descriptor.hash = Some(format!("blake3:{}", entry.hash));
    descriptor.size = Some(entry.size);
    reports.push(ProfileMaterializedAssetReport {
        arch: arch.to_string(),
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
    pin(profile.files.npm_packages.as_mut(), config_root)?;
    pin(profile.files.build.as_mut(), config_root)?;
    pin(profile.files.tips.as_mut(), config_root)?;
    pin(profile.files.root_manifest.as_mut(), config_root)?;
    Ok(())
}

struct ProfileObomMaterializeInputs<'a> {
    assets_dir: &'a Path,
    manifest_url: &'a str,
    asset_version: &'a str,
    arch: &'a str,
    manifest_assets: &'a std::collections::HashMap<String, capsem_core::asset_manager::AssetEntry>,
}

fn materialize_profile_obom_descriptor(
    inputs: ProfileObomMaterializeInputs<'_>,
    rootfs_hash: String,
    profile: &mut ProfileConfigFile,
    reports: &mut Vec<ProfileMaterializedObomReport>,
) -> Result<()> {
    let Some(entry) = inputs.manifest_assets.get("obom.cdx.json") else {
        return Ok(());
    };
    let obom_url = materialized_asset_url(
        inputs.assets_dir,
        inputs.manifest_url,
        inputs.asset_version,
        inputs.arch,
        "obom.cdx.json",
        &entry.hash,
        entry.size,
    )?;
    let (generator, generator_version) = if obom_url.starts_with("file://") {
        let obom_path = inputs.assets_dir.join(inputs.arch).join("obom.cdx.json");
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
    if let Some(descriptor) = profile.files.python_requirements.as_ref() {
        let packages = read_profile_package_lines(&config_root.join(&descriptor.path))?;
        write_profile_package_toml(
            &packages_dir.join("python.toml"),
            "python",
            "Python Packages",
            "uv",
            "uv pip install --system --break-system-packages",
            &packages,
        )?;
    }
    if let Some(descriptor) = profile.files.npm_packages.as_ref() {
        let packages = read_profile_package_lines(&config_root.join(&descriptor.path))?;
        write_profile_package_toml(
            &packages_dir.join("npm.toml"),
            "npm",
            "Node Packages",
            "npm",
            "npm install -g --prefix /opt/ai-clis",
            &packages,
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
        current_assets: manifest.assets.current.clone(),
        current_binary: manifest.binaries.current.clone(),
        releases: manifest.assets.releases.len(),
        arches,
    })
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 128 * 1024];
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
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn validates_checked_in_code_profile_through_security_rule_set() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let config_root = repo_root.join("config");
        let profile_path = config_root.join("profiles/code/profile.toml");

        let report =
            validate_profile(&profile_path, Some(&config_root)).expect("profile validates");

        assert!(report.ok);
        assert_eq!(report.profile_id, "code");
        assert!(report.compiled_rules >= 7);
    }

    #[test]
    fn source_profile_validation_rejects_generated_pins() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let config_root = repo_root.join("config");
        let source = fs::read_to_string(config_root.join("profiles/code/profile.toml"))
            .expect("read source profile");
        let pinned = source.replace(
            "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\n",
            "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\nhash = \"blake3:aa933a569fe27ed014ae76b58eb278d72fbde8a3cbd4c06a23da2987e70d0bd1\"\nsize = 8786432\n",
        );
        let temp = tempfile::tempdir().expect("tempdir");
        let profile_path = temp.path().join("profile.toml");
        fs::write(&profile_path, pinned).expect("write pinned profile");

        let error = validate_profile(&profile_path, Some(&config_root))
            .expect_err("source profile pins rejected");

        assert!(
            error.to_string().contains("source profile")
                && error.to_string().contains("hash/size pins"),
            "{error:#}"
        );
    }

    #[test]
    fn validates_checked_in_settings_file() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let path = repo_root.join("config/settings/settings.toml");

        let report = validate_settings(&path).expect("settings validates");

        assert!(report.ok);
        assert!(report.app.auto_update);
        assert_eq!(report.appearance.theme, "system");
    }

    #[test]
    fn settings_validation_rejects_runtime_profile_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("settings.toml");
        fs::write(
            &path,
            r#"
[app]
auto_update = true
notifications = true
start_service_at_login = true

[appearance]
theme = "system"
font_size = 14
reduced_motion = false

[profiles]
code = true
"#,
        )
        .expect("settings");

        let error = validate_settings(&path).expect_err("profile fields rejected");

        assert!(
            format!("{error:#}").contains("unknown field `profiles`"),
            "{error:#}"
        );
    }

    #[test]
    fn checked_in_config_root_passes_admin_lint() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");

        let report = check_config_root(&repo_root.join("config"), Some("arm64"))
            .expect("config root checks");

        assert!(report.ok);
        assert!(report
            .profiles
            .iter()
            .any(|profile| profile.validation.profile_id == "code"));
        assert!(report
            .profiles
            .iter()
            .any(|profile| profile.validation.profile_id == "co-work"));
    }

    #[test]
    fn config_root_lint_rejects_profile_catalog_id_mismatch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_root = temp.path().join("config");
        fs::create_dir_all(config_root.join("profiles/wrong")).expect("profile dir");
        fs::create_dir_all(config_root.join("settings")).expect("settings dir");
        fs::create_dir_all(config_root.join("corp")).expect("corp dir");
        fs::write(
            config_root.join("settings/settings.toml"),
            include_str!("../../../config/settings/settings.toml"),
        )
        .expect("settings");
        fs::write(
            config_root.join("corp/corp.toml"),
            "refresh_policy = \"24h\"\n",
        )
        .expect("corp");
        fs::write(
            config_root.join("profiles/wrong/profile.toml"),
            include_str!("../../../config/profiles/code/profile.toml"),
        )
        .expect("profile");

        let error = check_config_root(&config_root, Some("arm64"))
            .expect_err("catalog id mismatch rejected");

        assert!(format!("{error:#}").contains("id mismatch"), "{error:#}");
    }

    #[test]
    fn rejects_profile_rule_files_with_old_policy_syntax() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_root = temp.path();
        fs::create_dir_all(config_root.join("profiles/code")).expect("profile rules dir");
        let old_table = "policy".to_string() + ".http.block_old";
        fs::write(
            config_root.join("profiles/code/enforcement.toml"),
            r#"
[__OLD_TABLE__]
on = ["http.request"]
if = "http.host == 'evil.test'"
decision = "block"
"#
            .replace("__OLD_TABLE__", &old_table),
        )
        .expect("old policy file");
        fs::write(
            config_root.join("profiles/code/profile.toml"),
            r#"
id = "code"
name = "Code"
description = "Optimized for coding and long-running agents."
revision = "2026.06.08.3"
refresh_policy = "24h"

[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "https://example.test/vmlinuz"

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "https://example.test/initrd.img"

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://example.test/rootfs.erofs"

[rule_files]
enforcement = "profiles/code/enforcement.toml"
"#,
        )
        .expect("profile");

        let error = validate_profile(
            &config_root.join("profiles/code/profile.toml"),
            Some(config_root),
        )
        .expect_err("old policy syntax rejected");

        assert!(
            error.to_string().contains("unknown field `policy`")
                || format!("{error:#}").contains("unknown field `policy`"),
            "{error:#}"
        );
    }

    #[test]
    fn compiles_checked_in_enforcement_file() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let path = repo_root.join("config/profiles/code/enforcement.toml");

        let report =
            compile_rule_file("enforcement", &path, RuleFileSourceArg::User).expect("compile");

        assert_eq!(report.kind, "enforcement");
        let rule_ids = report
            .rules
            .iter()
            .map(|rule| rule.rule_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            rule_ids,
            BTreeSet::from([
                "profiles.rules.capsem_mock_server",
                "profiles.rules.default_http",
                "profiles.rules.default_dns",
                "profiles.rules.default_mcp",
                "profiles.rules.default_model",
                "profiles.rules.default_unknown_model_provider",
                "profiles.rules.default_unknown_mcp_server",
                "profiles.rules.default_file",
                "profiles.rules.default_process",
            ])
        );
        assert_eq!(report.compiled_rules, rule_ids.len());
        assert_eq!(
            report
                .rules
                .iter()
                .filter(|rule| !rule.default_rule)
                .map(|rule| rule.rule_id.as_str())
                .collect::<Vec<_>>(),
            vec!["profiles.rules.capsem_mock_server"]
        );
        assert!(report.rules.iter().all(|rule| rule.action == "allow"));
        assert!(report.rules.iter().all(|rule| rule.priority > 0));
        assert_eq!(
            report
                .rules
                .iter()
                .filter(|rule| rule.detection_level.is_some())
                .map(|rule| (rule.rule_id.as_str(), rule.detection_level))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                (
                    "profiles.rules.default_unknown_model_provider",
                    Some("informational")
                ),
                (
                    "profiles.rules.default_unknown_mcp_server",
                    Some("informational")
                ),
            ])
        );
    }

    #[test]
    fn compiles_checked_in_detection_file() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let path = repo_root.join("config/profiles/code/detection.yaml");

        let report =
            compile_rule_file("detection", &path, RuleFileSourceArg::User).expect("compile");

        assert_eq!(report.kind, "detection");
        assert_eq!(report.compiled_rules, 1);
        assert_eq!(report.rules[0].rule_id, "profiles.rules.skill_loaded");
        assert_eq!(report.rules[0].detection_level, Some("informational"));
    }

    #[test]
    fn checked_in_profile_build_wraps_agy_with_skip_permissions() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let path = repo_root.join("config/profiles/code/build.sh");
        let content = fs::read_to_string(path).expect("profile build script");

        assert!(
            content.contains("/usr/local/bin/agy-real"),
            "profile build script must preserve the real AGY binary behind a wrapper"
        );
        assert!(
            content.contains("--dangerously-skip-permissions"),
            "profile-owned AGY wrapper must opt into the Capsem permission model"
        );
        assert!(
            content.contains("https://ollama.com/install.sh"),
            "profile build script must ship Ollama through its official installer"
        );
    }

    #[test]
    fn enforcement_compile_rejects_old_on_if_decision_shape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("old.toml");
        fs::write(
            &path,
            r#"
[profiles.rules.old_http]
name = "old_http"
on = ["http.request"]
if = "http.host == 'evil.test'"
decision = "block"
"#,
        )
        .expect("old rule");

        let error = compile_rule_file("enforcement", &path, RuleFileSourceArg::User)
            .expect_err("old shape rejected");

        assert!(
            format!("{error:#}").contains("missing field `action`"),
            "{error:#}"
        );
    }

    #[test]
    fn infers_config_root_for_profiles_directory() {
        let root = PathBuf::from("/tmp/capsem-config");
        let path = root.join("profiles/code/profile.toml");
        assert_eq!(infer_config_root(&path).unwrap(), root);
    }

    #[test]
    fn checks_manifest_contract() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("manifest.json");
        fs::write(&path, minimal_manifest_json(None, true)).expect("manifest");

        let manifest = load_manifest(&path).expect("manifest parses");
        let report = manifest_report(&path, &manifest, None, None).expect("report");

        assert_eq!(
            report.blake3,
            blake3::hash(fs::read(&path).unwrap().as_slice())
                .to_hex()
                .to_string()
        );
        assert_eq!(report.refresh_policy, "24h");
        assert_eq!(report.current_assets, "2026.0607.1");
        assert!(report.arches.iter().any(|arch| arch.arch == "arm64"));
    }

    #[test]
    fn manifest_check_rejects_missing_refresh_policy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("manifest.json");
        fs::write(&path, minimal_manifest_json(None, false)).expect("manifest");

        let error = load_manifest(&path).expect_err("refresh policy required");

        assert!(format!("{error:#}").contains("refresh_policy"), "{error:#}");
    }

    #[test]
    fn manifest_verify_checks_literal_sibling_assets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let payload = b"capsem test asset";
        let hash = blake3::hash(payload).to_hex().to_string();
        let manifest_path = temp.path().join("manifest.json");
        fs::write(&manifest_path, minimal_manifest_json(Some(&hash), true)).expect("manifest");
        let assets_root = temp.path().join("assets");
        let assets_dir = assets_root.join("arm64");
        fs::create_dir_all(&assets_dir).expect("assets dir");
        fs::write(assets_dir.join("rootfs.erofs"), payload).expect("asset");

        let manifest = load_manifest(&manifest_path).expect("manifest");
        let report = manifest_report(&manifest_path, &manifest, Some(&assets_root), Some("arm64"))
            .expect("manifest verify");

        let asset = &report.arches[0].assets[0];
        assert!(asset.present);
        assert_eq!(asset.size_ok, Some(true));
        assert_eq!(asset.blake3_ok, Some(true));
    }

    #[test]
    fn profile_check_verifies_only_declared_file_urls() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut profile = ProfileConfigFile::builtin_primary();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
        profile.files = Default::default();
        profile.assets.arch.retain(|arch, _| arch == "arm64");
        let arch_assets = profile.assets.arch.get_mut("arm64").expect("arm64 assets");
        for descriptor in [
            &mut arch_assets.kernel,
            &mut arch_assets.initrd,
            &mut arch_assets.rootfs,
        ] {
            let payload = format!("{} bytes", descriptor.name);
            let path = temp.path().join(&descriptor.name);
            fs::write(&path, payload.as_bytes()).expect("asset");
            descriptor.url = format!("file://{}", path.display());
        }
        let profile_path = temp.path().join("profile.toml");
        fs::write(
            &profile_path,
            toml::to_string(&profile).expect("serialize profile"),
        )
        .expect("profile");

        let report = check_profile(&ProfileCheckArgs {
            path: profile_path,
            config_root: Some(temp.path().to_path_buf()),
            arch: Some("arm64".to_string()),
            json: true,
        })
        .expect("profile check");

        assert!(report.assets.is_empty());
        assert!(report.profile_files.is_empty());
    }

    #[test]
    fn profile_check_validates_profile_payload_files_and_root_manifest() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let profile_path = repo_root.join("config/profiles/code/profile.toml");

        let report = check_profile(&ProfileCheckArgs {
            path: profile_path,
            config_root: Some(repo_root.join("config")),
            arch: Some("arm64".to_string()),
            json: true,
        })
        .expect("checked-in profile payload files validate");

        assert!(report
            .profile_files
            .iter()
            .any(|file| file.logical_name == "mcp"));
        assert!(report
            .profile_files
            .iter()
            .any(|file| file.logical_name == "root/.codex/config.toml"));
        assert!(report.profile_files.iter().all(|file| file.present));
        assert!(report
            .profile_files
            .iter()
            .any(|file| file.size_ok == Some(true) && file.blake3_ok == Some(true)));
    }

    #[test]
    fn profile_check_rejects_missing_profile_payload_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_root = temp.path().join("config");
        let profile_dir = config_root.join("profiles/code");
        fs::create_dir_all(&profile_dir).expect("profile dir");
        let mut profile = ProfileConfigFile::builtin_primary();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
        profile.assets.arch.retain(|arch, _| arch == "arm64");
        profile.files = Default::default();
        profile.files.mcp = Some(capsem_core::net::policy_config::ProfileFileDescriptor {
            path: "profiles/code/mcp.json".to_string(),
            hash: None,
            size: None,
        });
        let profile_path = profile_dir.join("profile.toml");
        fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

        let error = check_profile(&ProfileCheckArgs {
            path: profile_path,
            config_root: Some(config_root),
            arch: Some("arm64".to_string()),
            json: true,
        })
        .expect_err("missing payload file rejected");
        assert!(error.to_string().contains("profile payload file pin check"));
    }

    #[test]
    fn profile_check_rejects_malformed_profile_mcp_file_even_when_hash_matches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_root = temp.path().join("config");
        let profile_dir = config_root.join("profiles/code");
        fs::create_dir_all(&profile_dir).expect("profile dir");
        let mcp = "{ definitely not json";
        fs::write(profile_dir.join("mcp.json"), mcp).expect("mcp");
        let mut profile = ProfileConfigFile::builtin_primary();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
        profile.assets.arch.retain(|arch, _| arch == "arm64");
        profile.files = Default::default();
        profile.files.mcp = Some(capsem_core::net::policy_config::ProfileFileDescriptor {
            path: "profiles/code/mcp.json".to_string(),
            hash: None,
            size: None,
        });
        let profile_path = profile_dir.join("profile.toml");
        fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

        let error = check_profile(&ProfileCheckArgs {
            path: profile_path,
            config_root: Some(config_root),
            arch: Some("arm64".to_string()),
            json: true,
        })
        .expect_err("malformed MCP config rejected");

        assert!(
            format!("{error:#}").contains("parse profile MCP config"),
            "{error:#}"
        );
    }

    #[test]
    fn profile_check_rejects_empty_profile_package_file_even_when_hash_matches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_root = temp.path().join("config");
        let profile_dir = config_root.join("profiles/code");
        fs::create_dir_all(&profile_dir).expect("profile dir");
        let packages = "# intentionally empty\n";
        fs::write(profile_dir.join("python-requirements.txt"), packages).expect("packages");
        let mut profile = ProfileConfigFile::builtin_primary();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
        profile.assets.arch.retain(|arch, _| arch == "arm64");
        profile.files = Default::default();
        profile.files.python_requirements =
            Some(capsem_core::net::policy_config::ProfileFileDescriptor {
                path: "profiles/code/python-requirements.txt".to_string(),
                hash: None,
                size: None,
            });
        let profile_path = profile_dir.join("profile.toml");
        fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

        let error = check_profile(&ProfileCheckArgs {
            path: profile_path,
            config_root: Some(config_root),
            arch: Some("arm64".to_string()),
            json: true,
        })
        .expect_err("empty package file rejected");

        assert!(format!("{error:#}").contains("package list"), "{error:#}");
    }

    #[test]
    fn profile_check_rejects_profile_root_manifest_escape_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_root = temp.path().join("config");
        let profile_dir = config_root.join("profiles/code");
        fs::create_dir_all(&profile_dir).expect("profile dir");
        let root_manifest = r#"{
  "format": "capsem.profile-root.v1",
  "files": [
    {
      "path": "../outside",
      "hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "size": 1
    }
  ]
}
"#;
        fs::write(profile_dir.join("root.manifest.json"), root_manifest).expect("root manifest");
        let mut profile = ProfileConfigFile::builtin_primary();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
        profile.assets.arch.retain(|arch, _| arch == "arm64");
        profile.files = Default::default();
        profile.files.root_manifest =
            Some(capsem_core::net::policy_config::ProfileFileDescriptor {
                path: "profiles/code/root.manifest.json".to_string(),
                hash: None,
                size: None,
            });
        let profile_path = profile_dir.join("profile.toml");
        fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

        let error = check_profile(&ProfileCheckArgs {
            path: profile_path,
            config_root: Some(config_root),
            arch: Some("arm64".to_string()),
            json: true,
        })
        .expect_err("root manifest escape rejected");

        assert!(
            error.to_string().contains("profile root manifest file"),
            "{error:#}"
        );
    }

    #[test]
    fn profile_check_rejects_unpinned_profile_root_payload_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_root = temp.path().join("config");
        let profile_dir = config_root.join("profiles/code");
        let profile_root = profile_dir.join("root");
        fs::create_dir_all(profile_root.join("root/.codex")).expect("profile root");
        fs::create_dir_all(profile_root.join("root/.antigravity")).expect("agy root");
        let codex_payload = b"[mcp_servers.capsem]\ncommand = \"/run/capsem-mcp-server\"\n";
        fs::write(profile_root.join("root/.codex/config.toml"), codex_payload)
            .expect("codex config");
        fs::write(
            profile_root.join("root/.antigravity/antigravity-oauth-token"),
            b"secret",
        )
        .expect("unlisted token");
        let root_manifest = format!(
            r#"{{
  "format": "capsem.profile-root.v1",
  "files": [
    {{
      "path": "root/.codex/config.toml",
      "hash": "blake3:{}",
      "size": {}
    }}
  ]
}}
"#,
            blake3::hash(codex_payload).to_hex(),
            codex_payload.len()
        );
        fs::write(profile_dir.join("root.manifest.json"), root_manifest).expect("root manifest");
        let mut profile = ProfileConfigFile::builtin_primary();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
        profile.assets.arch.retain(|arch, _| arch == "arm64");
        profile.files = Default::default();
        profile.files.root_manifest =
            Some(capsem_core::net::policy_config::ProfileFileDescriptor {
                path: "profiles/code/root.manifest.json".to_string(),
                hash: None,
                size: None,
            });
        let profile_path = profile_dir.join("profile.toml");
        fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

        let error = check_profile(&ProfileCheckArgs {
            path: profile_path,
            config_root: Some(config_root),
            arch: Some("arm64".to_string()),
            json: true,
        })
        .expect_err("unlisted profile root payload rejected");

        assert!(
            format!("{error:#}").contains("unlisted profile root payload file"),
            "{error:#}"
        );
    }

    #[test]
    fn profile_check_rejects_local_model_provider_profile_root_payloads() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_root = temp.path().join("config");
        let profile_dir = config_root.join("profiles/code");
        let profile_root = profile_dir.join("root");
        fs::create_dir_all(profile_root.join("root/.gemini/config")).expect("profile root");
        let payload = br#"{
  "ai": {
    "provider": "ollama",
    "baseUrl": "http://127.0.0.1:11434",
    "model": "gemma4:latest"
  }
}
"#;
        fs::write(
            profile_root.join("root/.gemini/config/config.json"),
            payload,
        )
        .expect("agy config");
        let root_manifest = format!(
            r#"{{
  "format": "capsem.profile-root.v1",
  "files": [
    {{
      "path": "root/.gemini/config/config.json",
      "hash": "blake3:{}",
      "size": {}
    }}
  ]
}}
"#,
            blake3::hash(payload).to_hex(),
            payload.len()
        );
        fs::write(profile_dir.join("root.manifest.json"), root_manifest).expect("root manifest");
        let mut profile = ProfileConfigFile::builtin_primary();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
        profile.assets.arch.retain(|arch, _| arch == "arm64");
        profile.files = Default::default();
        profile.files.root_manifest =
            Some(capsem_core::net::policy_config::ProfileFileDescriptor {
                path: "profiles/code/root.manifest.json".to_string(),
                hash: None,
                size: None,
            });
        let profile_path = profile_dir.join("profile.toml");
        fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

        let error = check_profile(&ProfileCheckArgs {
            path: profile_path,
            config_root: Some(config_root),
            arch: Some("arm64".to_string()),
            json: true,
        })
        .expect_err("local provider profile root payload rejected");

        assert!(
            format!("{error:#}").contains("profile root provider override"),
            "{error:#}"
        );
    }

    #[test]
    fn image_verify_rejects_profile_manifest_pin_drift() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("assets");
        let arch_dir = output.join("arm64");
        fs::create_dir_all(&arch_dir).expect("asset dir");
        let kernel = b"kernel";
        let initrd = b"initrd";
        let rootfs = b"rootfs";
        fs::write(arch_dir.join("vmlinuz"), kernel).expect("kernel");
        fs::write(arch_dir.join("initrd.img"), initrd).expect("initrd");
        fs::write(arch_dir.join("rootfs.erofs"), rootfs).expect("rootfs");
        let kernel_hash = blake3::hash(kernel).to_hex().to_string();
        let rootfs_hash = blake3::hash(rootfs).to_hex().to_string();
        let wrong_initrd_hash = "1111111111111111111111111111111111111111111111111111111111111111";
        fs::write(
            output.join("manifest.json"),
            format!(
                r#"{{
  "format": 2,
  "refresh_policy": "24h",
  "assets": {{
    "current": "2030.0101.1",
    "releases": {{
      "2030.0101.1": {{
        "date": "2030-01-01",
        "deprecated": false,
        "min_binary": "1.0.0",
        "arches": {{
          "arm64": {{
            "vmlinuz": {{"hash": "{kernel_hash}", "size": {kernel_size}}},
            "initrd.img": {{"hash": "{wrong_initrd_hash}", "size": {initrd_size}}},
            "rootfs.erofs": {{"hash": "{rootfs_hash}", "size": {rootfs_size}}}
          }}
        }}
      }}
    }}
  }},
  "binaries": {{
    "current": "1.0.0",
    "releases": {{"1.0.0": {{"date": "2030-01-01", "deprecated": false, "min_assets": "2030.0101.1"}}}}
  }}
}}"#,
                kernel_size = kernel.len(),
                initrd_size = initrd.len(),
                rootfs_size = rootfs.len(),
            ),
        )
        .expect("manifest");

        let mut profile = ProfileConfigFile::builtin_primary();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
        profile.assets.arch.retain(|arch, _| arch == "arm64");
        let profile_path = temp.path().join("profile.toml");
        fs::write(
            &profile_path,
            toml::to_string(&profile).expect("serialize profile"),
        )
        .expect("profile");

        let error = verify_image_outputs(&ImageVerifyArgs {
            profile: profile_path,
            config_root: temp.path().to_path_buf(),
            output,
            manifest: None,
            arch: Some("arm64".to_string()),
        })
        .expect_err("manifest/output drift rejected");

        assert!(
            format!("{error:#}").contains("image output verify failed"),
            "{error:#}"
        );
    }

    #[test]
    fn image_build_requires_profile_argument() {
        let error = Cli::try_parse_from(["capsem-admin", "image", "build"])
            .expect_err("profile is required");

        assert!(error.to_string().contains("--profile"), "{error}");
    }

    #[test]
    fn image_build_rejects_dry_run_escape_hatch() {
        let error = Cli::try_parse_from([
            "capsem-admin",
            "image",
            "build",
            "--profile",
            "config/profiles/code/profile.toml",
            "--dry-run",
        ])
        .expect_err("dry-run is not a public product rail");

        assert!(
            error
                .to_string()
                .contains("unexpected argument '--dry-run'"),
            "{error}"
        );
    }

    #[test]
    fn removed_admin_authoring_commands_are_not_parseable() {
        for argv in [
            ["capsem-admin", "profile", "init"],
            ["capsem-admin", "settings", "init"],
            ["capsem-admin", "enforcement", "compile"],
            ["capsem-admin", "detection", "compile"],
            ["capsem-admin", "manifest", "verify"],
            ["capsem-admin", "image", "plan"],
            ["capsem-admin", "image", "workspace"],
            ["capsem-admin", "image", "verify"],
        ] {
            let error = Cli::try_parse_from(argv).expect_err("removed command rejected");
            assert!(
                error.to_string().contains("unrecognized subcommand"),
                "{error}"
            );
        }
    }

    #[test]
    fn image_plan_is_profile_derived_and_uses_erofs_lz4hc() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let args = ImageBuildArgs {
            profile: repo_root.join("config/profiles/code/profile.toml"),
            config_root: repo_root.join("config"),
            guest_dir: repo_root.join("guest"),
            output: repo_root.join("assets"),
            arch: Some("arm64".to_string()),
            template: ImageBuildTemplate::All,
            clean: true,
            json: true,
        };

        let plan = image_build_plan(&args).expect("image plan");

        assert_eq!(plan.profile_id, "code");
        assert_eq!(plan.arches.len(), 1);
        assert_eq!(plan.arches[0].arch, "arm64");
        assert_eq!(plan.arches[0].rootfs, "rootfs.erofs");
        assert_eq!(plan.commands.len(), 3);
        assert_eq!(plan.commands[0].step, "kernel");
        assert_eq!(
            plan.commands[0].argv[0..5]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "uv",
                "run",
                "python",
                "-m",
                "capsem.builder.image_build_backend",
            ]
        );
        assert!(!plan.commands[0]
            .argv
            .windows(2)
            .any(|window| window[0] == "capsem-builder" && window[1] == "build"));
        assert_eq!(plan.commands[1].step, "rootfs");
        assert_eq!(
            plan.commands[1].argv[0..5]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "uv",
                "run",
                "python",
                "-m",
                "capsem.builder.image_build_backend",
            ]
        );
        assert!(!plan.commands[1]
            .argv
            .windows(2)
            .any(|window| window[0] == "capsem-builder" && window[1] == "build"));
        assert_eq!(
            plan.commands[1].env.get("CAPSEM_BUILD_EROFS_COMPRESSION"),
            Some(&"lz4hc".to_string())
        );
        assert_eq!(
            plan.commands[1]
                .env
                .get("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL"),
            Some(&"12".to_string())
        );
        assert_eq!(plan.commands[2].step, "manifest");
    }

    #[test]
    fn image_plan_kernel_only_does_not_generate_manifest() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let args = ImageBuildArgs {
            profile: repo_root.join("config/profiles/code/profile.toml"),
            config_root: repo_root.join("config"),
            guest_dir: repo_root.join("guest"),
            output: repo_root.join("assets"),
            arch: Some("arm64".to_string()),
            template: ImageBuildTemplate::Kernel,
            clean: true,
            json: true,
        };

        let plan = image_build_plan(&args).expect("image plan");

        assert_eq!(
            plan.commands
                .iter()
                .map(|command| command.step.as_str())
                .collect::<Vec<_>>(),
            vec!["kernel"]
        );
    }

    #[test]
    fn image_clean_rootfs_preserves_kernel_and_initrd() {
        let temp = tempfile::tempdir().expect("tempdir");
        let arch_dir = temp.path().join("arm64");
        fs::create_dir_all(&arch_dir).expect("arch dir");
        fs::write(arch_dir.join("vmlinuz"), b"kernel").expect("kernel");
        fs::write(arch_dir.join("initrd.img"), b"initrd").expect("initrd");
        fs::write(arch_dir.join("rootfs.erofs"), b"rootfs").expect("rootfs");
        fs::write(arch_dir.join("obom.cdx.json"), b"obom").expect("obom");

        clean_image_outputs(&ImageBuildPlan {
            schema: "test",
            profile_id: "code".to_string(),
            profile_revision: "test".to_string(),
            guest_dir: "guest".to_string(),
            output: temp.path().display().to_string(),
            clean: true,
            template: "rootfs",
            arches: vec![ImageBuildArchPlan {
                arch: "arm64".to_string(),
                kernel: "vmlinuz".to_string(),
                initrd: "initrd.img".to_string(),
                rootfs: "rootfs.erofs".to_string(),
            }],
            commands: Vec::new(),
        })
        .expect("rootfs clean");

        assert!(arch_dir.join("vmlinuz").is_file());
        assert!(arch_dir.join("initrd.img").is_file());
        assert!(!arch_dir.join("rootfs.erofs").exists());
        assert!(!arch_dir.join("obom.cdx.json").exists());
    }

    #[test]
    fn image_clean_kernel_preserves_rootfs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let arch_dir = temp.path().join("arm64");
        fs::create_dir_all(&arch_dir).expect("arch dir");
        fs::write(arch_dir.join("vmlinuz"), b"kernel").expect("kernel");
        fs::write(arch_dir.join("initrd.img"), b"initrd").expect("initrd");
        fs::write(arch_dir.join("rootfs.erofs"), b"rootfs").expect("rootfs");

        clean_image_outputs(&ImageBuildPlan {
            schema: "test",
            profile_id: "code".to_string(),
            profile_revision: "test".to_string(),
            guest_dir: "guest".to_string(),
            output: temp.path().display().to_string(),
            clean: true,
            template: "kernel",
            arches: vec![ImageBuildArchPlan {
                arch: "arm64".to_string(),
                kernel: "vmlinuz".to_string(),
                initrd: "initrd.img".to_string(),
                rootfs: "rootfs.erofs".to_string(),
            }],
            commands: Vec::new(),
        })
        .expect("kernel clean");

        assert!(!arch_dir.join("vmlinuz").exists());
        assert!(!arch_dir.join("initrd.img").exists());
        assert!(arch_dir.join("rootfs.erofs").is_file());
    }

    #[test]
    fn image_plan_rejects_arch_missing_from_profile() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let args = ImageBuildArgs {
            profile: repo_root.join("config/profiles/code/profile.toml"),
            config_root: repo_root.join("config"),
            guest_dir: repo_root.join("guest"),
            output: repo_root.join("assets"),
            arch: Some("riscv64".to_string()),
            template: ImageBuildTemplate::All,
            clean: false,
            json: false,
        };

        let error = image_build_plan(&args).expect_err("unknown arch rejected");

        assert!(
            error
                .to_string()
                .contains("does not define assets for arch riscv64"),
            "{error:#}"
        );
    }

    #[test]
    fn image_workspace_materializes_self_contained_profile_config() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let temp = tempfile::tempdir().expect("tempdir");
        let args = ImageWorkspaceArgs {
            profile: repo_root.join("config/profiles/code/profile.toml"),
            config_root: repo_root.join("config"),
            guest_dir: repo_root.join("guest"),
            output: temp.path().join("workspace"),
            arch: Some("arm64".to_string()),
            json: true,
        };

        let report = materialize_image_workspace(&args).expect("workspace");

        assert_eq!(report.profile_id, "code");
        assert_eq!(report.arches.len(), 1);
        assert_eq!(report.arches[0].arch, "arm64");
        assert_eq!(report.rule_files.len(), 2);
        let workspace_profile = args.output.join("config/profiles/code/profile.toml");
        assert!(workspace_profile.is_file());
        assert!(args
            .output
            .join("config/profiles/code/enforcement.toml")
            .is_file());
        assert!(args
            .output
            .join("config/profiles/code/detection.yaml")
            .is_file());
        assert!(args.output.join("build-plan.json").is_file());
        assert!(args.output.join("workspace.json").is_file());
        let generated_config = args.output.join("guest").join("config");
        assert!(generated_config.join("packages/apt.toml").is_file());
        let apt_packages = fs::read_to_string(generated_config.join("packages/apt.toml"))
            .expect("materialized apt packages");
        assert!(
            apt_packages.contains("\"zstd\""),
            "Ollama's official installer consumes .tar.zst payloads, so shipped profiles must include zstd"
        );
        assert!(generated_config.join("packages/python.toml").is_file());
        assert!(generated_config.join("packages/npm.toml").is_file());
        let resources = fs::read_to_string(generated_config.join("vm/resources.toml"))
            .expect("materialized VM resources");
        assert!(resources.contains("ram_gb = 12"));
        assert!(resources.contains("scratch_disk_size_gb = 64"));
        assert!(args.output.join("guest/profile-build.sh").is_file());
        let profile_build = fs::read_to_string(args.output.join("guest/profile-build.sh"))
            .expect("materialized profile build script");
        assert!(profile_build.contains("https://ollama.com/install.sh"));
        assert!(args
            .output
            .join("guest/profile-root/root/.codex/config.toml")
            .is_file());
        assert!(args.output.join("guest/artifacts/tips.txt").is_file());
        let build_plan: serde_json::Value =
            serde_json::from_slice(&fs::read(args.output.join("build-plan.json")).unwrap())
                .unwrap();
        assert!(build_plan["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["argv"]
                .as_array()
                .unwrap()
                .iter()
                .any(|arg| arg == args.output.join("guest").display().to_string().as_str())));

        let copied = check_profile(&ProfileCheckArgs {
            path: workspace_profile,
            config_root: Some(args.output.join("config")),
            arch: None,
            json: true,
        })
        .expect("copied workspace profile validates and owns every pinned payload");
        assert_eq!(copied.validation.profile_id, "code");
        assert!(copied.profile_files.iter().all(|file| file.present));
    }

    #[test]
    fn image_workspace_removes_stale_profile_root_payloads_before_materializing() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("workspace");
        let stale_profile_root = output.join("guest/profile-root/root/.gemini/config/config.json");
        fs::create_dir_all(stale_profile_root.parent().unwrap()).expect("stale parent");
        fs::write(
            &stale_profile_root,
            r#"{"ai":{"provider":"ollama","baseUrl":"http://127.0.0.1:11434"}}"#,
        )
        .expect("stale provider override");
        let stale_deleted_file = output.join("guest/profile-root/root/.stale-local-provider.json");
        fs::write(&stale_deleted_file, r#"{"provider":"ollama"}"#).expect("stale file");

        let args = ImageWorkspaceArgs {
            profile: repo_root.join("config/profiles/code/profile.toml"),
            config_root: repo_root.join("config"),
            guest_dir: repo_root.join("guest"),
            output: output.clone(),
            arch: Some("arm64".to_string()),
            json: true,
        };

        materialize_image_workspace(&args).expect("workspace");

        let materialized_config =
            fs::read_to_string(&stale_profile_root).expect("materialized AGY provider config");
        assert_eq!(materialized_config.trim(), "{}");
        assert!(
            !stale_deleted_file.exists(),
            "removed profile-root payloads must not survive into rebuilt image workspaces"
        );
    }

    #[test]
    fn profile_materialize_writes_generated_config_from_manifest() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let temp = tempfile::tempdir().expect("tempdir");
        let assets_dir = temp.path().join("assets");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let output_root = temp.path().join("target/config");
        let source_profile = repo_root.join("config/profiles/code/profile.toml");
        let original_source = fs::read_to_string(&source_profile).expect("read source profile");

        let report = materialize_profile_config(&ProfileMaterializeArgs {
            profile: source_profile.clone(),
            config_root: repo_root.join("config"),
            manifest: file_url(&manifest_path),
            assets_dir: assets_dir.clone(),
            output_root: output_root.clone(),
            arch: Some("arm64".to_string()),
            clean: true,
            json: true,
        })
        .expect("materialize profile config");

        assert_eq!(report.profile_id, "code");
        assert_eq!(report.materialized_assets.len(), 3);
        assert_eq!(report.materialized_obom.len(), 1);
        assert!(output_root.join("settings/settings.toml").is_file());
        assert!(output_root.join("corp/corp.toml").is_file());
        assert!(output_root.join("assets/manifest.json").is_file());
        assert!(output_root.join("profiles/code/enforcement.toml").is_file());
        assert!(output_root.join("profiles/code/detection.yaml").is_file());

        let generated_profile_path = output_root.join("profiles/code/profile.toml");
        let generated: ProfileConfigFile =
            toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated"))
                .expect("parse generated profile");
        let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
        assert!(arm64.kernel.url.starts_with("file://"));
        assert!(arm64.initrd.url.starts_with("file://"));
        assert!(arm64.rootfs.url.starts_with("file://"));
        assert_eq!(
            arm64.kernel.hash,
            Some(format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex()))
        );
        assert_eq!(arm64.initrd.size, Some(b"initrd-arm64".len() as u64));
        assert_eq!(arm64.rootfs.name, "rootfs.erofs");
        assert!(generated
            .files
            .iter()
            .all(|(_, descriptor)| descriptor.hash.is_some() && descriptor.size.is_some()));
        let obom = generated
            .obom
            .as_ref()
            .expect("materialized profile has base-image OBOM")
            .arch
            .get("arm64")
            .expect("arm64 OBOM");
        assert!(obom.url.starts_with("file://"));
        assert_eq!(
            obom.hash,
            format!(
                "blake3:{}",
                blake3::hash(test_obom_json().as_bytes()).to_hex()
            )
        );
        assert_eq!(obom.generator, "cdxgen");
        assert_eq!(obom.generator_version, "11.0.0");

        let validation = validate_materialized_profile(&generated_profile_path, Some(&output_root))
            .expect("valid materialized output");
        assert_eq!(validation.profile_id, "code");
        assert_eq!(
            fs::read_to_string(source_profile).expect("read source profile after"),
            original_source,
            "materialization must not mutate checked-in source profile"
        );
    }

    #[test]
    fn profile_materialize_remote_manifest_derives_release_site_asset_urls() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let manifest_json = fs::read_to_string(&manifest_path).expect("manifest");
        let manifest_url = serve_manifest_once(manifest_json);
        let output_root = temp.path().join("target/config");

        materialize_profile_config(&ProfileMaterializeArgs {
            profile: repo_root.join("config/profiles/code/profile.toml"),
            config_root: repo_root.join("config"),
            manifest: manifest_url.clone(),
            assets_dir: temp.path().join("no-local-assets"),
            output_root: output_root.clone(),
            arch: Some("arm64".to_string()),
            clean: true,
            json: true,
        })
        .expect("remote manifest materializes without local asset blobs");

        let generated_profile_path = output_root.join("profiles/code/profile.toml");
        let generated: ProfileConfigFile =
            toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated"))
                .expect("parse generated profile");
        let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
        let expected_base = manifest_url.replace(
            "/assets/stable/manifest.json",
            "/assets/releases/2030.0101.1",
        );
        assert_eq!(arm64.kernel.url, format!("{expected_base}/arm64-vmlinuz"));
        assert_eq!(
            arm64.initrd.url,
            format!("{expected_base}/arm64-initrd.img")
        );
        assert_eq!(
            arm64.rootfs.url,
            format!("{expected_base}/arm64-rootfs.erofs")
        );
        assert_eq!(
            arm64.kernel.hash,
            Some(format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex()))
        );
        let obom = generated
            .obom
            .as_ref()
            .expect("remote OBOM descriptor")
            .arch
            .get("arm64")
            .expect("arm64 OBOM");
        assert_eq!(obom.url, format!("{expected_base}/arm64-obom.cdx.json"));
        assert_eq!(obom.generator, "remote");
        assert_eq!(obom.generator_version, "unknown");
    }

    #[test]
    fn profile_materialize_preserves_previous_profiles_in_same_output_catalog() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let temp = tempfile::tempdir().expect("tempdir");
        let assets_dir = temp.path().join("assets");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let output_root = temp.path().join("target/config");
        let config_root = repo_root.join("config");

        materialize_profile_config(&ProfileMaterializeArgs {
            profile: config_root.join("profiles/co-work/profile.toml"),
            config_root: config_root.clone(),
            manifest: file_url(&manifest_path),
            assets_dir: assets_dir.clone(),
            output_root: output_root.clone(),
            arch: Some("arm64".to_string()),
            clean: true,
            json: true,
        })
        .expect("materialize co-work");

        materialize_profile_config(&ProfileMaterializeArgs {
            profile: config_root.join("profiles/code/profile.toml"),
            config_root,
            manifest: file_url(&manifest_path),
            assets_dir,
            output_root: output_root.clone(),
            arch: Some("arm64".to_string()),
            clean: false,
            json: true,
        })
        .expect("materialize code");

        for profile_id in ["co-work", "code"] {
            let generated_profile_path = output_root
                .join("profiles")
                .join(profile_id)
                .join("profile.toml");
            let generated: ProfileConfigFile = toml::from_str(
                &fs::read_to_string(&generated_profile_path).expect("read generated profile"),
            )
            .expect("generated profile parses");
            let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
            assert_eq!(
                arm64.kernel.hash,
                Some(format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex())),
                "{profile_id} kernel pin must remain generated"
            );
            assert_eq!(
                arm64.initrd.hash,
                Some(format!("blake3:{}", blake3::hash(b"initrd-arm64").to_hex())),
                "{profile_id} initrd pin must remain generated"
            );
            assert_eq!(
                arm64.rootfs.hash,
                Some(format!("blake3:{}", blake3::hash(b"rootfs-arm64").to_hex())),
                "{profile_id} rootfs pin must remain generated"
            );
            assert!(arm64.kernel.url.starts_with("file://"));
            assert!(arm64.initrd.url.starts_with("file://"));
            assert!(arm64.rootfs.url.starts_with("file://"));
        }
    }

    #[test]
    fn profile_materialize_rejects_arch_missing_from_manifest() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");

        let error = materialize_profile_config(&ProfileMaterializeArgs {
            profile: repo_root.join("config/profiles/code/profile.toml"),
            config_root: repo_root.join("config"),
            manifest: file_url(&manifest_path),
            assets_dir: temp.path().join("assets"),
            output_root: temp.path().join("target/config"),
            arch: Some("x86_64".to_string()),
            clean: true,
            json: false,
        })
        .expect_err("missing manifest arch rejected");

        assert!(
            format!("{error:#}").contains("does not contain profile arch x86_64"),
            "{error:#}"
        );
    }

    #[test]
    fn profile_materialize_manifest_source_must_be_url() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");

        let error = materialize_profile_config(&ProfileMaterializeArgs {
            profile: repo_root.join("config/profiles/code/profile.toml"),
            config_root: repo_root.join("config"),
            manifest: manifest_path.display().to_string(),
            assets_dir: temp.path().join("assets"),
            output_root: temp.path().join("target/config"),
            arch: Some("arm64".to_string()),
            clean: true,
            json: false,
        })
        .expect_err("bare manifest path rejected");

        assert!(
            format!("{error:#}").contains("manifest must be a URL"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_build_writes_manifest_under_channel_assets_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let manifest_url = file_url(&manifest_path);
        let assets_dir = temp.path().join("assets");
        let profiles_dir = repo_config_profiles_dir();
        let out_dir = temp.path().join("target/release-channel");

        let report = build_assets_channel(
            &manifest_url,
            &assets_dir,
            &profiles_dir,
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let channel_manifest = out_dir.join("assets/stable/manifest.json");
        let release_dir = out_dir.join("assets/releases/2030.0101.1");
        assert_eq!(report.manifest, channel_manifest.display().to_string());
        assert_eq!(report.copied_assets, 4);
        assert!(
            !out_dir.join("index.html").exists(),
            "human release pages are built by release-site Astro, not capsem-admin"
        );
        assert!(out_dir.join("health.json").is_file());
        assert!(channel_manifest.is_file());
        assert_eq!(
            fs::read(release_dir.join("arm64-vmlinuz")).expect("published kernel"),
            b"kernel-arm64"
        );
        assert!(release_dir.join("arm64-initrd.img").is_file());
        assert!(release_dir.join("arm64-rootfs.erofs").is_file());
        assert!(release_dir.join("arm64-obom.cdx.json").is_file());
        assert_eq!(
            fs::read_to_string(&channel_manifest).expect("channel manifest"),
            fs::read_to_string(&manifest_path).expect("source manifest")
        );
        let health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(out_dir.join("health.json")).unwrap())
                .expect("health json parses");
        assert_eq!(
            health["schema"].as_str(),
            Some("capsem.assets_channel.health.v1")
        );
        assert_eq!(health["current"]["assets"].as_str(), Some("2030.0101.1"));
        assert_eq!(
            health["urls"]["manifest"].as_str(),
            Some("/assets/stable/manifest.json")
        );
        assert_eq!(
            health["urls"]["asset_base"].as_str(),
            Some("/assets/releases")
        );
        assert_eq!(
            health["assets"]["files"][0]["url"].as_str(),
            Some("/assets/releases/2030.0101.1/arm64-initrd.img")
        );
        assert!(
            health["updates"]["assets"]["files"].is_null(),
            "VM asset file inventory belongs under assets.files, not updates.assets.files"
        );
        assert_eq!(
            health["assets"]["compatibility"]["min_binary"].as_str(),
            Some("1.0.0")
        );
        assert_eq!(
            health["assets"]["requires_newer"]["binary"].as_bool(),
            Some(false)
        );
        assert_eq!(
            health["asset_releases"][0]["date"].as_str(),
            Some("2030-01-01")
        );
        assert_eq!(
            health["evidence"]["vm_oboms"][0]["url"].as_str(),
            Some("/assets/releases/2030.0101.1/arm64-obom.cdx.json")
        );
        assert_eq!(
            health["evidence"]["host_sboms"][0]["name"].as_str(),
            Some("capsem-sbom.spdx.json")
        );
        assert_eq!(
            health["evidence"]["host_binary_files"][1]["name"].as_str(),
            Some("capsem-sbom.spdx.json")
        );
        assert_eq!(
            health["evidence"]["attestations"][0]["name"].as_str(),
            Some("github_attestations_host")
        );
        assert_eq!(
            health["evidence"]["attestations"][0]["predicate_type"].as_str(),
            Some("https://slsa.dev/provenance/v1")
        );
        assert_eq!(
            health["evidence"]["attestations"][0]["verify_command"].as_str(),
            Some("gh attestation verify <subject-url> --owner google")
        );
        assert_eq!(
            health["evidence"]["attestations"][1]["name"].as_str(),
            Some("github_attestations_host_sbom")
        );
        assert_eq!(
            health["evidence"]["attestations"][1]["predicate_type"].as_str(),
            Some("https://spdx.dev/Document/v2.3")
        );
        assert_eq!(
            health["evidence"]["attestations"][1]["predicate_url"].as_str(),
            Some("https://github.com/google/capsem/releases/download/v1.0.0/capsem-sbom.spdx.json")
        );
        assert_eq!(
            health["evidence"]["attestations"][1]["subjects"][0].as_str(),
            Some("https://github.com/google/capsem/releases/download/v1.0.0/capsem-1.0.0.pkg")
        );
        assert_eq!(
            health["evidence"]["attestations"][2]["name"].as_str(),
            Some("github_attestations_vm_assets")
        );
        assert_eq!(
            health["evidence"]["attestations"][2]["predicate_url"].as_str(),
            Some("/assets/releases/2030.0101.1/arm64-obom.cdx.json")
        );
        assert_eq!(
            health["evidence"]["attestations"][2]["subjects"][0].as_str(),
            Some("/assets/releases/2030.0101.1/arm64-initrd.img")
        );
        assert_eq!(
            health["updates"]["binary"]["latest"].as_str(),
            health["current"]["binary"].as_str()
        );
        assert_eq!(
            health["updates"]["binary"]["current"].as_str(),
            health["current"]["binary"].as_str()
        );
        assert_eq!(
            health["updates"]["binary"]["source"].as_str(),
            Some("manifest.binaries.current")
        );
        assert_eq!(
            health["updates"]["assets"]["latest"].as_str(),
            Some("2030.0101.1")
        );
        assert_eq!(
            health["updates"]["assets"]["current"].as_str(),
            Some("2030.0101.1")
        );
        assert_eq!(
            health["updates"]["assets"]["manifest"].as_str(),
            Some("/assets/stable/manifest.json")
        );
        assert_eq!(
            health["updates"]["assets"]["asset_base"].as_str(),
            Some("/assets/releases")
        );
        assert_eq!(
            health["updates"]["assets"]["compatibility"]["min_binary"].as_str(),
            Some("1.0.0")
        );
        assert_eq!(
            health["updates"]["assets"]["requires_newer"]["binary"].as_bool(),
            Some(false)
        );
        assert_eq!(
            health["profiles"]["revision"].as_str(),
            health["updates"]["profiles"]["latest"].as_str()
        );
        assert_eq!(
            health["profiles"]["hash"].as_str(),
            health["updates"]["profiles"]["hash"].as_str()
        );
        assert_eq!(
            health["profiles"]["compatibility"]["binary"].as_str(),
            Some("1.0.0")
        );
        assert_eq!(
            health["profiles"]["compatibility"]["assets"].as_str(),
            Some("2030.0101.1")
        );
        assert_eq!(
            health["updates"]["profiles"]["state"].as_str(),
            Some("current")
        );
        let profile_source = "/profiles/releases/2026.06.08.7/catalog.json";
        assert_eq!(
            health["updates"]["profiles"]["source"].as_str(),
            Some(profile_source)
        );
        let profile_catalog_path = out_dir.join(profile_source.trim_start_matches('/'));
        assert!(profile_catalog_path.is_file());
        let profile_catalog_bytes =
            fs::read(&profile_catalog_path).expect("published profile catalog");
        let profile_catalog_hash = blake3::hash(&profile_catalog_bytes).to_hex().to_string();
        assert_eq!(
            health["profiles"]["hash"].as_str(),
            Some(profile_catalog_hash.as_str())
        );
        assert_eq!(health["updates"]["images"]["latest"].as_str(), None);
        assert!(
            health["updates"]["images"]["latest"].is_null(),
            "unpublished image latest should be explicit null"
        );
        assert_eq!(
            health["updates"]["images"]["state"].as_str(),
            Some("not_published")
        );
        assert_eq!(
            health["updates"]["images"]["source"].as_str(),
            Some("not_in_asset_channel")
        );

        let check = check_assets_channel(&out_dir, "stable").expect("asset channel checks");
        assert_eq!(check.channel, "stable");
        assert_eq!(check.manifest, channel_manifest.display().to_string());
    }

    #[test]
    fn asset_attestation_predicate_uses_published_obom_url_shape() {
        let files = vec![
            AssetsChannelAssetFile {
                arch: "arm64".to_string(),
                logical_name: "initrd.img".to_string(),
                url: "/assets/releases/2030.0101.1/arm64-initrd.img".to_string(),
                hash: "1".repeat(64),
                size: 1,
            },
            AssetsChannelAssetFile {
                arch: "arm64".to_string(),
                logical_name: "arm64-obom.cdx.json".to_string(),
                url: "/assets/releases/2030.0101.1/arm64-obom.cdx.json".to_string(),
                hash: "2".repeat(64),
                size: 1,
            },
        ];

        let attestations = current_asset_attestations(&files);

        assert_eq!(attestations.len(), 1);
        assert_eq!(
            attestations[0].predicate_url.as_deref(),
            Some("/assets/releases/2030.0101.1/arm64-obom.cdx.json")
        );
    }

    #[test]
    fn assets_channel_build_bootstraps_without_binary_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
                .expect("manifest json");
        manifest["binaries"]["releases"]["1.0.0"]
            .as_object_mut()
            .expect("binary release")
            .remove("files");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        let out_dir = temp.path().join("target/release-channel");

        build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("first asset channel builds before binary evidence exists");

        let health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(out_dir.join("health.json")).unwrap())
                .expect("health json parses");
        assert_eq!(
            health["evidence"]["host_binary_files"],
            serde_json::json!([])
        );
        assert_eq!(health["evidence"]["host_sboms"], serde_json::json!([]));
        assert!(health["evidence"]["attestations"]
            .as_array()
            .expect("attestations")
            .iter()
            .any(|item| item["name"] == "github_attestations_vm_assets"));

        check_assets_channel(&out_dir, "stable")
            .expect("first asset channel checks before binary evidence exists");
    }

    #[test]
    fn assets_channel_headers_split_mutable_and_immutable_paths() {
        let headers = render_assets_channel_headers("stable");

        assert!(headers.contains("/\n  Cache-Control: no-cache, must-revalidate"));
        assert!(headers.contains("/index.html\n  Cache-Control: no-cache, must-revalidate"));
        assert!(headers.contains("/health.json\n  Cache-Control: no-cache, must-revalidate"));
        assert!(headers.contains("/assets/stable/*\n  Cache-Control: no-cache, must-revalidate"));
        assert!(headers.contains("/profiles/stable/*\n  Cache-Control: no-cache, must-revalidate"));
        assert!(headers
            .contains("/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable"));
        assert!(headers.contains(
            "/profiles/releases/*\n  Cache-Control: public, max-age=31536000, immutable"
        ));
        assert!(!headers.contains("/assets/*\n  Cache-Control: no-cache"));
        assert!(!headers.contains("/profiles/*\n  Cache-Control: no-cache"));
    }

    #[test]
    fn assets_channel_record_binary_updates_manifest_without_changing_assets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let original: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
                .expect("json");
        let artifacts_dir = temp.path().join("release-artifacts");
        fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
        let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
        let deb_path = artifacts_dir.join("Capsem_1.4.1234567890_arm64.deb");
        let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
        fs::write(&pkg_path, b"pkg bytes").expect("pkg");
        fs::write(&deb_path, b"deb bytes").expect("deb");
        fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

        let report = record_binary_release_metadata(
            &manifest_path,
            "1.4.1234567890",
            None,
            &[pkg_path.clone(), deb_path.clone(), sbom_path.clone()],
            "2030-02-03",
        )
        .expect("record binary release");

        assert_eq!(
            report.schema,
            "capsem.admin.assets_channel_record_binary.v1"
        );
        assert_eq!(report.version, "1.4.1234567890");
        assert_eq!(report.min_assets, "2030.0101.1");
        assert_eq!(report.files.len(), 3);
        let updated: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
                .expect("json");
        assert_eq!(updated["assets"], original["assets"]);
        assert_eq!(updated["binaries"]["current"], "1.4.1234567890");
        let release = &updated["binaries"]["releases"]["1.4.1234567890"];
        assert_eq!(release["date"], "2030-02-03");
        assert_eq!(release["deprecated"], false);
        assert_eq!(release["min_assets"], "2030.0101.1");
        assert_eq!(release["version"], "1.4.1234567890");
        assert_eq!(release["files"].as_array().expect("files").len(), 3);
        assert_eq!(release["files"][0]["name"], "Capsem-1.4.1234567890.pkg");
        assert_eq!(
            release["files"][0]["sha256"],
            format!("{:x}", Sha256::digest(b"pkg bytes"))
        );
        assert_eq!(release["files"][2]["name"], "capsem-sbom.spdx.json");
    }

    #[test]
    fn assets_channel_record_binary_rejects_sbom_without_host_package() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let artifacts_dir = temp.path().join("release-artifacts");
        fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
        let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
        fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

        let error = record_binary_release_metadata(
            &manifest_path,
            "1.4.1234567890",
            None,
            &[sbom_path],
            "2030-02-03",
        )
        .expect_err("SBOM-only binary metadata rejected");

        assert!(
            format!("{error:#}")
                .contains("binary release metadata must include a host package artifact"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_record_binary_rejects_non_package_host_artifact() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let artifacts_dir = temp.path().join("release-artifacts");
        fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
        let readme_path = artifacts_dir.join("release-notes.txt");
        let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
        fs::write(&readme_path, b"not an installable package").expect("readme");
        fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

        let error = record_binary_release_metadata(
            &manifest_path,
            "1.4.1234567890",
            None,
            &[readme_path, sbom_path],
            "2030-02-03",
        )
        .expect_err("non-package host artifact rejected");

        assert!(
            format!("{error:#}")
                .contains("binary release metadata must include a .pkg or .deb artifact"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_record_binary_rejects_empty_artifact() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let artifacts_dir = temp.path().join("release-artifacts");
        fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
        let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
        let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
        fs::write(&pkg_path, []).expect("empty pkg");
        fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

        let error = record_binary_release_metadata(
            &manifest_path,
            "1.4.1234567890",
            None,
            &[pkg_path, sbom_path],
            "2030-02-03",
        )
        .expect_err("empty binary artifact rejected");

        assert!(
            format!("{error:#}").contains("binary release artifact is empty"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_record_binary_rejects_package_version_mismatch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let artifacts_dir = temp.path().join("release-artifacts");
        fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
        let pkg_path = artifacts_dir.join("Capsem-1.4.0000000000.pkg");
        let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
        fs::write(&pkg_path, b"pkg bytes").expect("pkg");
        fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

        let error = record_binary_release_metadata(
            &manifest_path,
            "1.4.1234567890",
            None,
            &[pkg_path, sbom_path],
            "2030-02-03",
        )
        .expect_err("mismatched package version rejected");

        assert!(
            format!("{error:#}")
                .contains("binary release package artifact name must match version"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_record_binary_rejects_noncanonical_sbom_artifact() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let artifacts_dir = temp.path().join("release-artifacts");
        fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
        let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
        let sbom_path = artifacts_dir.join("host-sbom.spdx.json");
        fs::write(&pkg_path, b"pkg bytes").expect("pkg");
        fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

        let error = record_binary_release_metadata(
            &manifest_path,
            "1.4.1234567890",
            None,
            &[pkg_path, sbom_path],
            "2030-02-03",
        )
        .expect_err("noncanonical SBOM artifact rejected");

        assert!(
            format!("{error:#}").contains("capsem-sbom.spdx.json"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_build_externalizes_vm_blobs_for_pages_deploy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        fs::remove_dir_all(temp.path().join("assets/arm64")).expect("remove local assets");
        let out_dir = temp.path().join("target/release-channel");
        let asset_base =
            "https://github.com/google/capsem/releases/download/assets-v{asset_version}";

        let report = build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-02-03T00:00:00Z",
            Some(asset_base),
        )
        .expect("externalized channel builds without local blobs");

        assert_eq!(report.copied_assets, 0);
        assert!(!out_dir.join("assets/releases").exists());
        let channel_manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(out_dir.join("assets/stable/manifest.json")).unwrap(),
        )
        .expect("channel manifest parses");
        assert_eq!(channel_manifest["asset_base"].as_str(), Some(asset_base));
        let health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(out_dir.join("health.json")).unwrap())
                .expect("health parses");
        let rootfs_url = "https://github.com/google/capsem/releases/download/assets-v2030.0101.1/arm64-rootfs.erofs";
        assert_eq!(health["urls"]["asset_base"].as_str(), Some(asset_base));
        let health_files = health["assets"]["files"].as_array().expect("asset files");
        assert!(health_files
            .iter()
            .any(|file| file["url"].as_str() == Some(rootfs_url)));
        let profile_source = health["profiles"]["source"]
            .as_str()
            .expect("profile source")
            .trim_start_matches('/');
        assert!(fs::read_to_string(out_dir.join(profile_source))
            .expect("profile catalog")
            .contains(rootfs_url));
        check_assets_channel(&out_dir, "stable").expect("externalized channel checks");
    }

    #[test]
    fn assets_channel_check_rejects_bad_health_schema() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let assets_dir = temp.path().join("assets");
        let profiles_dir = repo_config_profiles_dir();
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &assets_dir,
            &profiles_dir,
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let health_path = out_dir.join("health.json");
        let mut health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
                .expect("health json");
        health["schema"] = serde_json::Value::String("capsem.bad_schema".to_string());
        fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
            .expect("write bad health");

        let error =
            check_assets_channel(&out_dir, "stable").expect_err("bad health schema rejected");

        assert!(
            format!("{error:#}").contains("health.json schema mismatch"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_check_rejects_missing_evidence_host_sbom() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let health_path = out_dir.join("health.json");
        let mut health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
                .expect("health json");
        health["evidence"]["host_sboms"] = serde_json::json!([]);
        fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
            .expect("write health without host SBOM");

        let error =
            check_assets_channel(&out_dir, "stable").expect_err("missing host SBOM rejected");

        assert!(
            format!("{error:#}").contains("health.json host SBOM evidence missing"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_check_rejects_missing_asset_release_date() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let health_path = out_dir.join("health.json");
        let mut health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
                .expect("health json");
        health["asset_releases"][0]
            .as_object_mut()
            .expect("asset release object")
            .remove("date");
        fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
            .expect("write health without asset release date");

        let error =
            check_assets_channel(&out_dir, "stable").expect_err("missing release date rejected");

        assert!(
            format!("{error:#}").contains("health.json asset release date mismatch"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_check_rejects_missing_evidence_vm_obom() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let health_path = out_dir.join("health.json");
        let mut health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
                .expect("health json");
        health["evidence"]["vm_oboms"] = serde_json::json!([]);
        fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
            .expect("write health without VM OBOM");

        let error = check_assets_channel(&out_dir, "stable").expect_err("missing VM OBOM rejected");

        assert!(
            format!("{error:#}").contains("health.json missing VM OBOM evidence"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_check_rejects_missing_evidence_vm_attestation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let health_path = out_dir.join("health.json");
        let mut health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
                .expect("health json");
        let attestations = health["evidence"]["attestations"]
            .as_array()
            .expect("attestations")
            .iter()
            .filter(|attestation| {
                attestation.get("name").and_then(|name| name.as_str())
                    != Some("github_attestations_vm_assets")
            })
            .cloned()
            .collect::<Vec<_>>();
        health["evidence"]["attestations"] = serde_json::Value::Array(attestations);
        fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
            .expect("write health without VM attestation");

        let error =
            check_assets_channel(&out_dir, "stable").expect_err("missing VM attestation rejected");

        assert!(
            format!("{error:#}").contains("health.json VM asset attestation evidence missing"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_check_rejects_missing_vm_attestation_predicate() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let health_path = out_dir.join("health.json");
        let mut health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
                .expect("health json");
        let attestations = health["evidence"]["attestations"]
            .as_array_mut()
            .expect("attestations");
        let vm_attestation = attestations
            .iter_mut()
            .find(|attestation| {
                attestation.get("name").and_then(|name| name.as_str())
                    == Some("github_attestations_vm_assets")
            })
            .expect("VM asset attestation");
        vm_attestation
            .as_object_mut()
            .expect("attestation object")
            .remove("predicate_url");
        fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
            .expect("write health without VM predicate");

        let error = check_assets_channel(&out_dir, "stable")
            .expect_err("missing VM attestation predicate rejected");

        assert!(
            format!("{error:#}").contains("health.json VM asset attestation predicate_url missing"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_check_rejects_missing_host_sbom_attestation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let health_path = out_dir.join("health.json");
        let mut health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
                .expect("health json");
        let attestations = health["evidence"]["attestations"]
            .as_array()
            .expect("attestations")
            .iter()
            .filter(|attestation| {
                attestation.get("name").and_then(|name| name.as_str())
                    != Some("github_attestations_host_sbom")
            })
            .cloned()
            .collect::<Vec<_>>();
        health["evidence"]["attestations"] = serde_json::Value::Array(attestations);
        fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
            .expect("write health without host SBOM attestation");

        let error = check_assets_channel(&out_dir, "stable")
            .expect_err("missing host SBOM attestation rejected");

        assert!(
            format!("{error:#}").contains("health.json host SBOM attestation evidence missing"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_check_rejects_host_sbom_attestation_missing_package_subject() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
                .expect("manifest json");
        manifest["binaries"]["releases"]["1.0.0"]["files"]
            .as_array_mut()
            .expect("binary files")
            .push(serde_json::json!({
                "name": "Capsem_1.0.0_arm64.deb",
                "size": 789,
                "sha256": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
            }));
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("manifest json"),
        )
        .expect("write manifest with deb");
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let health_path = out_dir.join("health.json");
        let mut health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
                .expect("health json");
        let sbom_attestation = health["evidence"]["attestations"]
            .as_array_mut()
            .expect("attestations")
            .iter_mut()
            .find(|attestation| {
                attestation.get("name").and_then(|name| name.as_str())
                    == Some("github_attestations_host_sbom")
            })
            .expect("host SBOM attestation");
        let subjects = sbom_attestation["subjects"]
            .as_array_mut()
            .expect("host SBOM subjects");
        subjects.retain(|subject| {
            !subject
                .as_str()
                .expect("subject string")
                .ends_with("Capsem_1.0.0_arm64.deb")
        });
        fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
            .expect("write health without deb SBOM subject");

        let error = check_assets_channel(&out_dir, "stable")
            .expect_err("missing host package SBOM subject rejected");

        assert!(
            format!("{error:#}").contains("health.json host SBOM attestation subjects missing"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_check_rejects_attestation_without_verification_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");

        let health_path = out_dir.join("health.json");
        let mut health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
                .expect("health json");
        health["evidence"]["attestations"][0]
            .as_object_mut()
            .expect("attestation object")
            .remove("verify_command");
        fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
            .expect("write health without verification metadata");

        let error = check_assets_channel(&out_dir, "stable")
            .expect_err("missing attestation verification metadata rejected");

        assert!(
            format!("{error:#}").contains("health.json attestation verify_command missing"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_check_rejects_missing_current_asset_blob() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let assets_dir = temp.path().join("assets");
        let profiles_dir = repo_config_profiles_dir();
        let out_dir = temp.path().join("target/release-channel");
        build_assets_channel(
            &file_url(&manifest_path),
            &assets_dir,
            &profiles_dir,
            "stable",
            &out_dir,
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect("asset channel builds");
        fs::remove_file(out_dir.join("assets/releases/2030.0101.1/arm64-rootfs.erofs"))
            .expect("remove published rootfs");

        let error =
            check_assets_channel(&out_dir, "stable").expect_err("missing asset blob rejected");

        assert!(
            format!("{error:#}").contains("arm64-rootfs.erofs"),
            "{error:#}"
        );
    }

    #[test]
    fn assets_channel_rejects_unsafe_channel_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let manifest_url = file_url(&manifest_path);
        let assets_dir = temp.path().join("assets");
        let profiles_dir = repo_config_profiles_dir();
        for channel in ["../stable", "stable.v1", "stable channel", "<stable>"] {
            let error = build_assets_channel(
                &manifest_url,
                &assets_dir,
                &profiles_dir,
                channel,
                &temp.path().join("target/release-channel"),
                "2030-01-01T00:00:00Z",
                None,
            )
            .expect_err("unsafe channel rejected");

            assert!(error.to_string().contains("invalid asset channel name"));
        }
    }

    #[test]
    fn assets_channel_manifest_source_must_be_url() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
        let error = build_assets_channel(
            &manifest_path.display().to_string(),
            &temp.path().join("assets"),
            &repo_config_profiles_dir(),
            "stable",
            &temp.path().join("target/release-channel"),
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect_err("bare manifest path rejected");

        assert!(
            format!("{error:#}").contains("manifest must be a URL"),
            "{error:#}"
        );
    }

    fn file_url(path: &Path) -> String {
        let path = path.canonicalize().expect("canonical test path");
        format!("file://{}", path.display())
    }

    fn repo_config_profiles_dir() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root")
            .join("config/profiles")
    }

    fn serve_manifest_once(body: String) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test manifest server");
        let addr = listener.local_addr().expect("manifest server addr");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept manifest request");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write manifest response");
        });
        format!("http://{addr}/assets/stable/manifest.json")
    }

    fn minimal_manifest_json(hash: Option<&str>, include_refresh_policy: bool) -> String {
        let hash =
            hash.unwrap_or("1111111111111111111111111111111111111111111111111111111111111111");
        format!(
            r#"{{
  "format": 2,
  {refresh}
  "assets": {{
    "current": "2026.0607.1",
    "releases": {{
      "2026.0607.1": {{
        "arches": {{
          "arm64": {{
            "rootfs.erofs": {{
              "hash": "{hash}",
              "size": 17
            }}
          }}
        }}
      }}
    }}
  }},
  "binaries": {{
    "current": "1.0.0",
    "releases": {{
      "1.0.0": {{
        "min_assets": "2026.0607.1"
      }}
    }}
  }}
}}"#,
            refresh = if include_refresh_policy {
                r#""refresh_policy": "24h","#
            } else {
                ""
            },
            hash = hash,
        )
    }

    fn write_test_assets_manifest(root: &Path, arch: &str) -> PathBuf {
        let assets_dir = root.join("assets").join(arch);
        fs::create_dir_all(&assets_dir).expect("assets dir");
        let kernel = format!("kernel-{arch}");
        let initrd = format!("initrd-{arch}");
        let rootfs = format!("rootfs-{arch}");
        let obom = test_obom_json();
        let pkg_sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let sbom_sha256 = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        fs::write(assets_dir.join("vmlinuz"), kernel.as_bytes()).expect("kernel");
        fs::write(assets_dir.join("initrd.img"), initrd.as_bytes()).expect("initrd");
        fs::write(assets_dir.join("rootfs.erofs"), rootfs.as_bytes()).expect("rootfs");
        fs::write(assets_dir.join("obom.cdx.json"), obom.as_bytes()).expect("obom");
        let manifest_path = root.join("assets/manifest.json");
        fs::write(
            &manifest_path,
            format!(
                r#"{{
  "format": 2,
  "refresh_policy": "24h",
  "assets": {{
    "current": "2030.0101.1",
    "releases": {{
      "2030.0101.1": {{
        "date": "2030-01-01",
        "deprecated": false,
        "min_binary": "1.0.0",
        "arches": {{
          "{arch}": {{
            "vmlinuz": {{"hash": "{kernel_hash}", "size": {kernel_size}}},
            "initrd.img": {{"hash": "{initrd_hash}", "size": {initrd_size}}},
            "rootfs.erofs": {{"hash": "{rootfs_hash}", "size": {rootfs_size}}},
            "obom.cdx.json": {{"hash": "{obom_hash}", "size": {obom_size}}}
          }}
        }}
      }}
    }}
  }},
  "binaries": {{
    "current": "1.0.0",
    "releases": {{
      "1.0.0": {{
        "date": "2030-01-01",
        "deprecated": false,
        "min_assets": "2030.0101.1",
        "files": [
          {{"name": "capsem-1.0.0.pkg", "size": 123, "sha256": "{pkg_sha256}"}},
          {{"name": "capsem-sbom.spdx.json", "size": 456, "sha256": "{sbom_sha256}"}}
        ]
      }}
    }}
  }}
}}"#,
                arch = arch,
                kernel_hash = blake3::hash(kernel.as_bytes()).to_hex(),
                kernel_size = kernel.len(),
                initrd_hash = blake3::hash(initrd.as_bytes()).to_hex(),
                initrd_size = initrd.len(),
                rootfs_hash = blake3::hash(rootfs.as_bytes()).to_hex(),
                rootfs_size = rootfs.len(),
                obom_hash = blake3::hash(obom.as_bytes()).to_hex(),
                obom_size = obom.len(),
                pkg_sha256 = pkg_sha256,
                sbom_sha256 = sbom_sha256,
            ),
        )
        .expect("manifest");
        manifest_path
    }

    fn test_obom_json() -> String {
        serde_json::json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.6",
            "metadata": {
                "tools": {
                    "components": [
                        {"name": "cdxgen", "version": "11.0.0", "type": "application"}
                    ]
                },
                "component": {
                    "name": "capsem-code-rootfs",
                    "type": "operating-system"
                }
            },
            "components": [
                {"name": "bash", "version": "5.2", "type": "library"}
            ]
        })
        .to_string()
    }
}
#[cfg(test)]
#[derive(Debug)]
struct ImageVerifyArgs {
    profile: PathBuf,
    config_root: PathBuf,
    output: PathBuf,
    manifest: Option<PathBuf>,
    arch: Option<String>,
}

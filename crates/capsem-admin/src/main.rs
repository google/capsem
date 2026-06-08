use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{anyhow, Context, Result};
use capsem_core::asset_manager::ManifestV2;
use capsem_core::net::policy_config::{
    resolve_profile_rule_file_path, CompiledSecurityRule, ProfileConfigFile, SecurityRuleProfile,
    SecurityRuleSet, SecurityRuleSource,
};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

const CODE_PROFILE_TEMPLATE: &str = include_str!("../../../config/profiles/code.toml");
const SETTINGS_TEMPLATE: &str = include_str!("../../../config/settings.toml");

#[derive(Debug, Parser)]
#[command(name = "capsem-admin")]
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
    Image(ImageCommand),
}

#[derive(Debug, Parser)]
struct ProfileCommand {
    #[command(subcommand)]
    command: ProfileSubcommand,
}

#[derive(Debug, Subcommand)]
enum ProfileSubcommand {
    Init(InitArgs),
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
    Init(InitArgs),
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
    Compile(RuleFileArgs),
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
    Verify(ManifestVerifyArgs),
}

#[derive(Debug, Parser)]
struct ImageCommand {
    #[command(subcommand)]
    command: ImageSubcommand,
}

#[derive(Debug, Subcommand)]
enum ImageSubcommand {
    Plan(ImageBuildArgs),
    Build(ImageBuildArgs),
    Workspace(ImageWorkspaceArgs),
    Verify(ImageVerifyArgs),
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
    /// Generated asset manifest to use for current build hashes.
    #[arg(long, default_value = "assets/manifest.json")]
    manifest: PathBuf,
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
struct InitArgs {
    /// Destination file to create.
    #[arg(long)]
    output: PathBuf,
    /// Replace an existing destination file.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Parser)]
struct RuleFileArgs {
    /// Enforcement TOML or Sigma YAML file to validate.
    path: PathBuf,
    /// Treat the rules as this source when resolving priority.
    #[arg(long, value_enum, default_value_t = RuleFileSourceArg::User)]
    source: RuleFileSourceArg,
    /// Emit a machine-readable validation or compile report.
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
struct ManifestVerifyArgs {
    /// Manifest JSON file to validate against sibling built assets.
    path: PathBuf,
    /// Restrict verification to one manifest arch.
    #[arg(long)]
    arch: Option<String>,
    /// Emit a machine-readable manifest report.
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
    /// Print the plan without executing Docker/capsem-builder.
    #[arg(long)]
    dry_run: bool,
    /// Emit a machine-readable build plan/report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ImageVerifyArgs {
    /// Profile TOML that owns the image build.
    #[arg(long)]
    profile: PathBuf,
    /// Config root used to validate profile rule files.
    #[arg(long, default_value = "config")]
    config_root: PathBuf,
    /// Output directory containing built assets.
    #[arg(long, default_value = "assets")]
    output: PathBuf,
    /// Manifest JSON generated for the built assets.
    #[arg(long)]
    manifest: Option<PathBuf>,
    /// Restrict verification to one profile architecture.
    #[arg(long)]
    arch: Option<String>,
    /// Emit a machine-readable verification report.
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Profile(command) => match command.command {
            ProfileSubcommand::Init(args) => init_file_command(args, CODE_PROFILE_TEMPLATE),
            ProfileSubcommand::Validate(args) => validate_profile_command(args),
            ProfileSubcommand::Check(args) => profile_check_command(args),
            ProfileSubcommand::Materialize(args) => profile_materialize_command(args),
        },
        Commands::Settings(command) => match command.command {
            SettingsSubcommand::Init(args) => init_file_command(args, SETTINGS_TEMPLATE),
            SettingsSubcommand::Validate(args) => validate_settings_command(args),
        },
        Commands::Enforcement(command) => match command.command {
            RuleFileSubcommand::Validate(args) => validate_rule_file_command("enforcement", args),
            RuleFileSubcommand::Compile(args) => compile_rule_file_command("enforcement", args),
        },
        Commands::Detection(command) => match command.command {
            RuleFileSubcommand::Validate(args) => validate_rule_file_command("detection", args),
            RuleFileSubcommand::Compile(args) => compile_rule_file_command("detection", args),
        },
        Commands::Manifest(command) => match command.command {
            ManifestSubcommand::Check(args) => manifest_check_command(args),
            ManifestSubcommand::Generate(args) => manifest_generate_command(args),
            ManifestSubcommand::Verify(args) => manifest_verify_command(args),
        },
        Commands::Image(command) => match command.command {
            ImageSubcommand::Plan(args) => image_plan_command(args),
            ImageSubcommand::Build(args) => image_build_command(args),
            ImageSubcommand::Workspace(args) => image_workspace_command(args),
            ImageSubcommand::Verify(args) => image_verify_command(args),
        },
    }
}

fn init_file_command(args: InitArgs, template: &str) -> Result<()> {
    if args.output.exists() && !args.force {
        return Err(anyhow!(
            "refusing to overwrite existing file {}; pass --force to replace it",
            args.output.display()
        ));
    }
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent directory {}", parent.display()))?;
    }
    fs::write(&args.output, template)
        .with_context(|| format!("write {}", args.output.display()))?;
    println!("wrote {}", args.output.display());
    Ok(())
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

fn compile_rule_file_command(kind: &'static str, args: RuleFileArgs) -> Result<()> {
    let report = compile_rule_file(kind, &args.path, args.source)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
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

fn manifest_verify_command(args: ManifestVerifyArgs) -> Result<()> {
    let manifest = load_manifest(&args.path)?;
    let assets_dir = args.path.parent().ok_or_else(|| {
        anyhow!(
            "manifest {} has no parent asset directory",
            args.path.display()
        )
    })?;
    let report = manifest_report(
        &args.path,
        &manifest,
        Some(assets_dir),
        args.arch.as_deref(),
    )?;
    let failed = report
        .arches
        .iter()
        .flat_map(|arch| arch.assets.iter())
        .any(|asset| {
            !asset.present
                || asset.size_ok.is_some_and(|ok| !ok)
                || asset.blake3_ok.is_some_and(|ok| !ok)
        });
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if !failed {
        println!("valid: manifest assets {}", args.path.display());
    }
    if failed {
        return Err(anyhow!(
            "manifest asset verify failed for {}",
            args.path.display()
        ));
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

fn image_plan_command(args: ImageBuildArgs) -> Result<()> {
    let plan = image_build_plan(&args)?;
    print_image_build_plan(&plan, args.json)?;
    Ok(())
}

fn image_build_command(args: ImageBuildArgs) -> Result<()> {
    let plan = image_build_plan(&args)?;
    if args.dry_run {
        print_image_build_plan(&plan, args.json)?;
        return Ok(());
    }
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
    let report = materialize_image_workspace(&args)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "materialized: image workspace for profile {} at {}",
            report.profile_id, report.workspace
        );
    }
    Ok(())
}

fn image_verify_command(args: ImageVerifyArgs) -> Result<()> {
    let report = verify_image_outputs(&args)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        let count = report
            .arches
            .iter()
            .map(|arch| arch.assets.len())
            .sum::<usize>();
        println!(
            "valid: image outputs for profile {} ({} assets)",
            report.profile_id, count
        );
    }
    Ok(())
}

fn validate_profile(path: &Path, config_root: Option<&Path>) -> Result<ProfileValidationReport> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read profile {}", path.display()))?;
    let profile: ProfileConfigFile =
        toml::from_str(&content).with_context(|| format!("parse profile {}", path.display()))?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", path.display()))?;

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

fn check_profile(args: &ProfileCheckArgs) -> Result<ProfileCheckReport> {
    let validation = validate_profile(&args.path, args.config_root.as_deref())?;
    let profile = load_profile(&args.path)?;
    let mut assets = Vec::new();
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
            if let Some(path) = descriptor.url.strip_prefix("file://") {
                assets.push(check_exact_local_asset(
                    Path::new(path),
                    &arch,
                    &descriptor.name,
                    normalized_blake3(&descriptor.hash)?,
                    descriptor.size,
                )?);
            }
        }
    }
    fail_if_local_asset_checks_failed("profile file:// asset pin check", &assets)?;
    Ok(ProfileCheckReport {
        schema: "capsem.admin.profile_check.v1",
        ok: true,
        validation,
        assets,
    })
}

fn materialize_profile_config(args: &ProfileMaterializeArgs) -> Result<ProfileMaterializeReport> {
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
    copy_dir_recursive(&args.config_root, &args.output_root)?;

    let manifest = load_manifest(&args.manifest)?;
    let current_release = manifest
        .assets
        .releases
        .get(&manifest.assets.current)
        .ok_or_else(|| {
            anyhow!(
                "manifest {} current asset release {} is missing",
                args.manifest.display(),
                manifest.assets.current
            )
        })?;

    let mut profile = load_profile(&args.profile)?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", args.profile.display()))?;

    let selected_arches = selected_profile_arches(&profile, args.arch.as_deref())?;
    let mut materialized_assets = Vec::new();
    for arch in selected_arches {
        let manifest_assets = current_release.arches.get(&arch).ok_or_else(|| {
            anyhow!(
                "manifest {} current release {} does not contain profile arch {arch}",
                args.manifest.display(),
                manifest.assets.current
            )
        })?;
        let profile_assets = profile
            .assets
            .arch
            .get_mut(&arch)
            .expect("arch came from selected_profile_arches");
        materialize_profile_asset_descriptor(
            &args.assets_dir,
            &arch,
            &mut profile_assets.kernel,
            manifest_assets,
            &mut materialized_assets,
        )?;
        materialize_profile_asset_descriptor(
            &args.assets_dir,
            &arch,
            &mut profile_assets.initrd,
            manifest_assets,
            &mut materialized_assets,
        )?;
        materialize_profile_asset_descriptor(
            &args.assets_dir,
            &arch,
            &mut profile_assets.rootfs,
            manifest_assets,
            &mut materialized_assets,
        )?;
    }

    let output_profile_path = args
        .output_root
        .join("profiles")
        .join(format!("{}.toml", profile.id));
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
    fs::copy(&args.manifest, &manifest_output).with_context(|| {
        format!(
            "copy manifest {} to {}",
            args.manifest.display(),
            manifest_output.display()
        )
    })?;

    let copied_validation = validate_profile(&output_profile_path, Some(&args.output_root))?;
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
    })
}

fn materialize_profile_asset_descriptor(
    assets_dir: &Path,
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
    let check = check_local_asset(assets_dir, arch, &descriptor.name, &entry.hash, entry.size)?;
    fail_if_local_asset_checks_failed("profile materialize asset check", &[check])?;
    let asset_path = assets_dir.join(arch).join(&descriptor.name);
    let asset_path = asset_path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", asset_path.display()))?;
    descriptor.url = format!("file://{}", asset_path.display());
    descriptor.hash = format!("blake3:{}", entry.hash);
    descriptor.size = entry.size;
    reports.push(ProfileMaterializedAssetReport {
        arch: arch.to_string(),
        logical_name: descriptor.name.clone(),
        url: descriptor.url.clone(),
        hash: descriptor.hash.clone(),
        size: descriptor.size,
    });
    Ok(())
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
                    "capsem-builder".to_string(),
                    "build".to_string(),
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
                    "capsem-builder".to_string(),
                    "build".to_string(),
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
    commands.push(manifest_generate_command_report(&ManifestGenerateArgs {
        assets_dir: args.output.clone(),
        version: None,
        json: false,
    }));

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
            let profile_hash = normalized_blake3(&descriptor.hash)?;
            if profile_hash != entry.hash || descriptor.size != entry.size {
                return Err(anyhow!(
                    "profile asset pin drift for {arch}/{}: profile has blake3:{} size {}, \
                     manifest current {} has blake3:{} size {}",
                    descriptor.name,
                    profile_hash,
                    descriptor.size,
                    manifest.assets.current,
                    entry.hash,
                    entry.size
                ));
            }
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
    let workspace_config_root = workspace.join("config");
    let workspace_profile_path = workspace_config_root
        .join("profiles")
        .join(format!("{}.toml", profile.id));
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

    let copied_validation =
        validate_profile(&workspace_profile_path, Some(&workspace_config_root))?;
    if copied_validation.profile_id != profile.id {
        return Err(anyhow!(
            "workspace profile id drifted: expected {}, got {}",
            profile.id,
            copied_validation.profile_id
        ));
    }

    let plan = image_build_plan(&ImageBuildArgs {
        profile: workspace_profile_path.clone(),
        config_root: workspace_config_root.clone(),
        guest_dir: args.guest_dir.clone(),
        output: workspace.join("assets"),
        arch: args.arch.clone(),
        template: ImageBuildTemplate::All,
        clean: false,
        dry_run: true,
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
    let failed = assets.iter().any(|asset| {
        !asset.present
            || asset.size_ok.is_some_and(|ok| !ok)
            || asset.blake3_ok.is_some_and(|ok| !ok)
    });
    if failed {
        return Err(anyhow!("{context} failed"));
    }
    Ok(())
}

fn normalized_blake3(value: &str) -> Result<&str> {
    value
        .strip_prefix("blake3:")
        .ok_or_else(|| anyhow!("expected blake3:<hash>, got {value}"))
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
        if path.exists() {
            fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
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
        let profile_path = config_root.join("profiles/code.toml");

        let report =
            validate_profile(&profile_path, Some(&config_root)).expect("profile validates");

        assert!(report.ok);
        assert_eq!(report.profile_id, "code");
        assert!(report.compiled_rules >= 7);
    }

    #[test]
    fn validates_checked_in_settings_file() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let path = repo_root.join("config/settings.toml");

        let report = validate_settings(&path).expect("settings validates");

        assert!(report.ok);
        assert_eq!(report.app.auto_update, true);
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
    fn init_writes_templates_and_refuses_overwrite_without_force() {
        let temp = tempfile::tempdir().expect("tempdir");
        let profile_path = temp.path().join("profiles/code.toml");
        init_file_command(
            InitArgs {
                output: profile_path.clone(),
                force: false,
            },
            CODE_PROFILE_TEMPLATE,
        )
        .expect("profile init");
        let profile: ProfileConfigFile =
            toml::from_str(&fs::read_to_string(&profile_path).expect("read profile"))
                .expect("profile template parses");
        assert_eq!(profile.id, "code");

        let error = init_file_command(
            InitArgs {
                output: profile_path,
                force: false,
            },
            CODE_PROFILE_TEMPLATE,
        )
        .expect_err("overwrite rejected");
        assert!(
            error.to_string().contains("refusing to overwrite"),
            "{error:#}"
        );
    }

    #[test]
    fn profile_init_template_carries_release_ready_defaults() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let temp = tempfile::tempdir().expect("tempdir");
        let profile_path = temp.path().join("code.toml");
        init_file_command(
            InitArgs {
                output: profile_path.clone(),
                force: false,
            },
            CODE_PROFILE_TEMPLATE,
        )
        .expect("profile init");

        let profile: ProfileConfigFile =
            toml::from_str(&fs::read_to_string(&profile_path).expect("read profile"))
                .expect("profile template parses");
        assert_eq!(profile.id, "code");
        assert_eq!(profile.refresh_policy, "24h");
        assert!(profile.availability.web);
        assert!(profile.availability.shell);
        assert!(profile.availability.mobile);
        assert_eq!(profile.vm.cpu_count, 4);
        assert_eq!(profile.vm.ram_gb, 12);
        assert_eq!(profile.vm.scratch_disk_size_gb, 64);
        for arch in ["arm64", "x86_64"] {
            let assets = profile.assets.arch.get(arch).expect("arch assets");
            assert_eq!(assets.kernel.name, "vmlinuz");
            assert_eq!(assets.initrd.name, "initrd.img");
            assert_eq!(assets.rootfs.name, "rootfs.erofs");
            assert!(assets.rootfs.hash.starts_with("blake3:"));
        }
        let broker = profile
            .plugins
            .get("credential_broker")
            .expect("credential broker plugin");
        assert_eq!(broker.mode.as_str(), "rewrite");
        assert_eq!(broker.detection_level.as_str(), "informational");
        assert!(profile.mcp.is_some());

        let rules = profile
            .compile_security_rule_set_from_files(
                &repo_root.join("config"),
                SecurityRuleSource::User,
            )
            .expect("profile rules compile");
        assert!(
            rules
                .rules()
                .iter()
                .any(|rule| rule.rule_id == "profiles.rules.default_http"
                    && rule.action.as_str() == "allow"),
            "profile default HTTP allow rule must compile"
        );
    }

    #[test]
    fn rejects_profile_rule_files_with_old_policy_syntax() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_root = temp.path();
        fs::create_dir_all(config_root.join("profiles/code")).expect("profile rules dir");
        fs::write(
            config_root.join("profiles/code/enforcement.toml"),
            r#"
[policy.http.block_old]
on = ["http.request"]
if = "http.host == 'evil.test'"
decision = "block"
"#,
        )
        .expect("old policy file");
        fs::write(
            config_root.join("code.toml"),
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
hash = "blake3:1111111111111111111111111111111111111111111111111111111111111111"
size = 1

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "https://example.test/initrd.img"
hash = "blake3:2222222222222222222222222222222222222222222222222222222222222222"
size = 1

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://example.test/rootfs.erofs"
hash = "blake3:3333333333333333333333333333333333333333333333333333333333333333"
size = 1

[rule_files]
enforcement = "profiles/code/enforcement.toml"
"#,
        )
        .expect("profile");

        let error = validate_profile(&config_root.join("code.toml"), Some(config_root))
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
        assert_eq!(report.compiled_rules, 6);
        assert!(report.rules.iter().all(|rule| rule.default_rule));
        assert!(report.rules.iter().all(|rule| rule.action == "allow"));
        assert!(report.rules.iter().all(|rule| rule.priority > 0));
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
        let path = root.join("profiles/code.toml");
        assert_eq!(infer_config_root(&path).unwrap(), root);
    }

    #[test]
    fn checks_manifest_contract() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("manifest.json");
        fs::write(&path, minimal_manifest_json(None, true)).expect("manifest");

        let manifest = load_manifest(&path).expect("manifest parses");
        let report = manifest_report(&path, &manifest, None, None).expect("report");

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
        let mut profile = ProfileConfigFile::builtin_code();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
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
            descriptor.hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
            descriptor.size = payload.len() as u64;
        }
        let profile_path = temp.path().join("code.toml");
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

        assert_eq!(report.assets.len(), 3);
        assert!(report.assets.iter().all(|asset| asset.present));
        assert!(report
            .assets
            .iter()
            .all(|asset| asset.size_ok == Some(true)));
        assert!(report
            .assets
            .iter()
            .all(|asset| asset.blake3_ok == Some(true)));
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
        let initrd_hash = blake3::hash(initrd).to_hex().to_string();
        let rootfs_hash = blake3::hash(rootfs).to_hex().to_string();
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
            "initrd.img": {{"hash": "{initrd_hash}", "size": {initrd_size}}},
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

        let mut profile = ProfileConfigFile::builtin_code();
        profile.rule_files.enforcement = None;
        profile.rule_files.sigma = None;
        profile.assets.arch.retain(|arch, _| arch == "arm64");
        let assets = profile.assets.arch.get_mut("arm64").expect("arm64 assets");
        assets.kernel.hash = format!("blake3:{kernel_hash}");
        assets.kernel.size = kernel.len() as u64;
        assets.initrd.hash =
            "blake3:1111111111111111111111111111111111111111111111111111111111111111".into();
        assets.initrd.size = initrd.len() as u64;
        assets.rootfs.hash = format!("blake3:{rootfs_hash}");
        assets.rootfs.size = rootfs.len() as u64;
        let profile_path = temp.path().join("code.toml");
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
            json: true,
        })
        .expect_err("profile/manifest drift rejected");

        assert!(
            format!("{error:#}").contains("profile asset pin drift for arm64/initrd.img"),
            "{error:#}"
        );
    }

    #[test]
    fn image_build_requires_profile_argument() {
        let error = Cli::try_parse_from(["capsem-admin", "image", "build", "--dry-run"])
            .expect_err("profile is required");

        assert!(error.to_string().contains("--profile"), "{error}");
    }

    #[test]
    fn image_plan_is_profile_derived_and_uses_erofs_lz4hc() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let args = ImageBuildArgs {
            profile: repo_root.join("config/profiles/code.toml"),
            config_root: repo_root.join("config"),
            guest_dir: repo_root.join("guest"),
            output: repo_root.join("assets"),
            arch: Some("arm64".to_string()),
            template: ImageBuildTemplate::All,
            clean: true,
            dry_run: true,
            json: true,
        };

        let plan = image_build_plan(&args).expect("image plan");

        assert_eq!(plan.profile_id, "code");
        assert_eq!(plan.arches.len(), 1);
        assert_eq!(plan.arches[0].arch, "arm64");
        assert_eq!(plan.arches[0].rootfs, "rootfs.erofs");
        assert_eq!(plan.commands.len(), 3);
        assert_eq!(plan.commands[0].step, "kernel");
        assert_eq!(plan.commands[1].step, "rootfs");
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
    fn image_plan_rejects_arch_missing_from_profile() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let args = ImageBuildArgs {
            profile: repo_root.join("config/profiles/code.toml"),
            config_root: repo_root.join("config"),
            guest_dir: repo_root.join("guest"),
            output: repo_root.join("assets"),
            arch: Some("riscv64".to_string()),
            template: ImageBuildTemplate::All,
            clean: false,
            dry_run: true,
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
            profile: repo_root.join("config/profiles/code.toml"),
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
        let workspace_profile = args.output.join("config/profiles/code.toml");
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

        let copied = validate_profile(&workspace_profile, Some(&args.output.join("config")))
            .expect("copied workspace profile validates");
        assert_eq!(copied.profile_id, "code");
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
        let source_profile = repo_root.join("config/profiles/code.toml");
        let original_source = fs::read_to_string(&source_profile).expect("read source profile");

        let report = materialize_profile_config(&ProfileMaterializeArgs {
            profile: source_profile.clone(),
            config_root: repo_root.join("config"),
            manifest: manifest_path,
            assets_dir: assets_dir.clone(),
            output_root: output_root.clone(),
            arch: Some("arm64".to_string()),
            clean: true,
            json: true,
        })
        .expect("materialize profile config");

        assert_eq!(report.profile_id, "code");
        assert_eq!(report.materialized_assets.len(), 3);
        assert!(output_root.join("settings.toml").is_file());
        assert!(output_root.join("corp.toml").is_file());
        assert!(output_root.join("assets/manifest.json").is_file());
        assert!(output_root.join("profiles/code/enforcement.toml").is_file());
        assert!(output_root.join("profiles/code/detection.yaml").is_file());

        let generated_profile_path = output_root.join("profiles/code.toml");
        let generated: ProfileConfigFile =
            toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated"))
                .expect("parse generated profile");
        let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
        assert!(arm64.kernel.url.starts_with("file://"));
        assert!(arm64.initrd.url.starts_with("file://"));
        assert!(arm64.rootfs.url.starts_with("file://"));
        assert_eq!(
            arm64.kernel.hash,
            format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex())
        );
        assert_eq!(arm64.initrd.size, b"initrd-arm64".len() as u64);
        assert_eq!(arm64.rootfs.name, "rootfs.erofs");

        let validation =
            validate_profile(&generated_profile_path, Some(&output_root)).expect("valid output");
        assert_eq!(validation.profile_id, "code");
        assert_eq!(
            fs::read_to_string(source_profile).expect("read source profile after"),
            original_source,
            "materialization must not mutate checked-in source profile"
        );
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
            profile: repo_root.join("config/profiles/code.toml"),
            config_root: repo_root.join("config"),
            manifest: manifest_path,
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
        fs::write(assets_dir.join("vmlinuz"), kernel.as_bytes()).expect("kernel");
        fs::write(assets_dir.join("initrd.img"), initrd.as_bytes()).expect("initrd");
        fs::write(assets_dir.join("rootfs.erofs"), rootfs.as_bytes()).expect("rootfs");
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
                arch = arch,
                kernel_hash = blake3::hash(kernel.as_bytes()).to_hex(),
                kernel_size = kernel.len(),
                initrd_hash = blake3::hash(initrd.as_bytes()).to_hex(),
                initrd_size = initrd.len(),
                rootfs_hash = blake3::hash(rootfs.as_bytes()).to_hex(),
                rootfs_size = rootfs.len(),
            ),
        )
        .expect("manifest");
        manifest_path
    }
}

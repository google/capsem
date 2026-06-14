use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{anyhow, Context, Result};
use capsem_core::asset_manager::ManifestV2;
use capsem_core::net::policy_config::{
    resolve_profile_rule_file_path, validate_corp_toml_contract, CompiledSecurityRule,
    ProfileCatalog, ProfileConfigFile, ProfileObomConfig, ProfileObomDescriptor,
    SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource, SettingsFile,
};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

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
                args.manifest.display(),
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
            profile_assets
                .rootfs
                .hash
                .clone()
                .ok_or_else(|| anyhow!("materialized {arch} rootfs hash is unresolved"))?
        };
        materialize_profile_obom_descriptor(
            &args.assets_dir,
            &arch,
            manifest_assets,
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
    fs::copy(&args.manifest, &manifest_output).with_context(|| {
        format!(
            "copy manifest {} to {}",
            args.manifest.display(),
            manifest_output.display()
        )
    })?;

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

fn materialize_profile_obom_descriptor(
    assets_dir: &Path,
    arch: &str,
    manifest_assets: &std::collections::HashMap<String, capsem_core::asset_manager::AssetEntry>,
    rootfs_hash: String,
    profile: &mut ProfileConfigFile,
    reports: &mut Vec<ProfileMaterializedObomReport>,
) -> Result<()> {
    let Some(entry) = manifest_assets.get("obom.cdx.json") else {
        return Ok(());
    };
    let check = check_local_asset(assets_dir, arch, "obom.cdx.json", &entry.hash, entry.size)?;
    fail_if_local_asset_checks_failed("profile materialize OBOM check", &[check])?;
    let obom_path = assets_dir.join(arch).join("obom.cdx.json");
    let obom_path = obom_path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", obom_path.display()))?;
    let (generator, generator_version) = read_obom_generator(&obom_path)?;
    let descriptor = ProfileObomDescriptor {
        name: "obom.cdx.json".to_string(),
        url: format!("file://{}", obom_path.display()),
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
        .insert(arch.to_string(), descriptor.clone());
    reports.push(ProfileMaterializedObomReport {
        arch: arch.to_string(),
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
    let source_config = source_guest_dir.join("config");
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
        assert!(args.output.join("guest/config/packages/apt.toml").is_file());
        let apt_packages = fs::read_to_string(args.output.join("guest/config/packages/apt.toml"))
            .expect("materialized apt packages");
        assert!(
            apt_packages.contains("\"zstd\""),
            "Ollama's official installer consumes .tar.zst payloads, so shipped profiles must include zstd"
        );
        assert!(args
            .output
            .join("guest/config/packages/python.toml")
            .is_file());
        assert!(args.output.join("guest/config/packages/npm.toml").is_file());
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
            manifest: manifest_path.clone(),
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
            manifest: manifest_path,
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
        let obom = test_obom_json();
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
                obom_hash = blake3::hash(obom.as_bytes()).to_hex(),
                obom_size = obom.len(),
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

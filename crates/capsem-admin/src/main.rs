use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use capsem_core::asset_manager::{hash_filename, ManifestV2};
use capsem_core::net::policy_config::{
    CompiledSecurityRule, ProfileConfigFile, SecurityRuleProfile, SecurityRuleSet,
    SecurityRuleSource,
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
    DownloadCheck(ManifestDownloadCheckArgs),
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
struct ManifestDownloadCheckArgs {
    /// Manifest JSON file to validate against downloaded assets.
    path: PathBuf,
    /// Asset directory containing hash-prefixed downloaded files.
    #[arg(long)]
    assets_dir: PathBuf,
    /// Restrict verification to one manifest arch.
    #[arg(long)]
    arch: Option<String>,
    /// Emit a machine-readable manifest report.
    #[arg(long)]
    json: bool,
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
    downloaded_name: String,
    present: bool,
    size_ok: Option<bool>,
    blake3_ok: Option<bool>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Profile(command) => match command.command {
            ProfileSubcommand::Init(args) => init_file_command(args, CODE_PROFILE_TEMPLATE),
            ProfileSubcommand::Validate(args) => validate_profile_command(args),
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
            ManifestSubcommand::DownloadCheck(args) => manifest_download_check_command(args),
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

fn manifest_download_check_command(args: ManifestDownloadCheckArgs) -> Result<()> {
    let manifest = load_manifest(&args.path)?;
    let report = manifest_report(
        &args.path,
        &manifest,
        Some(&args.assets_dir),
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
    } else if failed {
        return Err(anyhow!(
            "download check failed for manifest {} in {}",
            args.path.display(),
            args.assets_dir.display()
        ));
    } else {
        println!(
            "valid: downloaded assets for manifest {} in {}",
            args.path.display(),
            args.assets_dir.display()
        );
    }
    if failed {
        return Err(anyhow!(
            "download check failed for manifest {} in {}",
            args.path.display(),
            args.assets_dir.display()
        ));
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
                let downloaded_name = hash_filename(name, &entry.hash);
                let (present, size_ok, blake3_ok) = match assets_dir {
                    Some(dir) => {
                        let file_path = dir.join(arch).join(&downloaded_name);
                        let fallback_path = dir.join(&downloaded_name);
                        let file_path = if file_path.exists() {
                            file_path
                        } else {
                            fallback_path
                        };
                        if !file_path.is_file() {
                            (false, None, None)
                        } else {
                            let metadata = fs::metadata(&file_path).with_context(|| {
                                format!("stat downloaded asset {}", file_path.display())
                            })?;
                            let digest = hash_file(&file_path)?;
                            (
                                true,
                                Some(metadata.len() == entry.size),
                                Some(digest == entry.hash),
                            )
                        }
                    }
                    None => (false, None, None),
                };
                asset_reports.push(ManifestAssetReport {
                    logical_name: name.clone(),
                    hash: entry.hash.clone(),
                    size: entry.size,
                    downloaded_name,
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
revision = "2026.06.07.1"
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
    fn download_check_verifies_hash_prefixed_assets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let payload = b"capsem test asset";
        let hash = blake3::hash(payload).to_hex().to_string();
        let manifest_path = temp.path().join("manifest.json");
        fs::write(&manifest_path, minimal_manifest_json(Some(&hash), true)).expect("manifest");
        let assets_dir = temp.path().join("assets/arm64");
        fs::create_dir_all(&assets_dir).expect("assets dir");
        let downloaded = hash_filename("rootfs.erofs", &hash);
        fs::write(assets_dir.join(downloaded), payload).expect("asset");

        let manifest = load_manifest(&manifest_path).expect("manifest");
        let report = manifest_report(
            &manifest_path,
            &manifest,
            Some(&temp.path().join("assets")),
            Some("arm64"),
        )
        .expect("download check");

        let asset = &report.arches[0].assets[0];
        assert!(asset.present);
        assert_eq!(asset.size_ok, Some(true));
        assert_eq!(asset.blake3_ok, Some(true));
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
}

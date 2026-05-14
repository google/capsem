//! Setup wizard orchestrator.
//!
//! `capsem setup` walks the user through first-time configuration:
//! corp config provisioning, security preset, AI provider keys,
//! repository access, service installation, and VM boot verification.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use capsem_core::net::policy_config;
use capsem_core::net::policy_config::corp_provision;
use capsem_core::setup_state::SetupState;

use crate::client::{self, UdsClient};

/// Options passed from CLI flags.
pub struct SetupOptions {
    pub non_interactive: bool,
    pub preset: Option<String>,
    pub force: bool,
    pub accept_detected: bool,
    pub corp_config: Option<String>,
    /// Reset only the GUI wizard flags (onboarding_completed, onboarding_version)
    /// without wiping CLI install state. No other setup steps run.
    pub force_onboarding: bool,
}

fn capsem_dir() -> Result<PathBuf> {
    crate::paths::capsem_home()
}

fn state_path_in(capsem_dir: &Path) -> PathBuf {
    capsem_dir.join("setup-state.json")
}

fn load_state_from(capsem_dir: &Path) -> SetupState {
    capsem_core::setup_state::load_state(&state_path_in(capsem_dir))
}

fn save_state_to(capsem_dir: &Path, state: &SetupState) -> Result<()> {
    capsem_core::setup_state::save_state(&state_path_in(capsem_dir), state)
}

fn load_setup_manifest_for_assets(
    assets_dir: &Path,
) -> Result<Option<capsem_core::asset_manager::ManifestV2>> {
    capsem_core::asset_manager::load_verified_manifest_for_assets(assets_dir, true)
}

fn setup_requires_manifest_for_layout(layout: &crate::platform::InstallLayout) -> bool {
    !matches!(layout, crate::platform::InstallLayout::Development)
}

const SETUP_SERVICE_TRUTH_TIMEOUT: Duration = Duration::from_secs(8);
const SETUP_SERVICE_TRUTH_POLL: Duration = Duration::from_millis(250);

enum SetupAssetProbe {
    Available(client::AssetHealth),
    Unavailable(String),
}

fn evaluate_setup_asset_health(asset_health: &client::AssetHealth) -> Result<bool> {
    match asset_health.state.as_str() {
        "ready" => {
            if !asset_health.ready {
                anyhow::bail!("service asset state is inconsistent: state=ready but ready=false");
            }
            if !asset_health.missing.is_empty() {
                anyhow::bail!(
                    "service asset state is inconsistent: state=ready but missing={}",
                    asset_health.missing.join(", ")
                );
            }
            if !asset_health.saved_vm_dependencies.is_empty() {
                return Ok(false);
            }
            Ok(true)
        }
        "checking" | "updating" => {
            if asset_health.ready {
                anyhow::bail!(
                    "service asset state is inconsistent: state={} but ready=true",
                    asset_health.state
                );
            }
            Ok(false)
        }
        "error" => Ok(false),
        "unknown" => anyhow::bail!("service asset state is unknown"),
        other => anyhow::bail!("service asset state is unsupported: {}", other),
    }
}

async fn fetch_setup_asset_health(capsem_dir: &Path) -> SetupAssetProbe {
    let sock = capsem_dir.join("run/service.sock");
    let isolation_mode = crate::service_install::test_isolation_env_active();
    let client = UdsClient::new(sock, isolation_mode);
    let deadline = Instant::now() + SETUP_SERVICE_TRUTH_TIMEOUT;

    loop {
        let observation = if isolation_mode {
            match client
                .get::<client::ApiResponse<client::ListResponse>>("/list")
                .await
            {
                Ok(resp) => match resp.into_result() {
                    Ok(list) => {
                        if let Some(asset_health) = list.asset_health {
                            return SetupAssetProbe::Available(asset_health);
                        }
                        "service /list response missing asset_health".to_string()
                    }
                    Err(e) => format!("service /list returned error: {e:#}"),
                },
                Err(e) => format!("service /list query failed: {e:#}"),
            }
        } else {
            match crate::service_install::service_status().await {
                Ok(status) if status.running => match client
                    .get::<client::ApiResponse<client::ListResponse>>("/list")
                    .await
                {
                    Ok(resp) => match resp.into_result() {
                        Ok(list) => {
                            if let Some(asset_health) = list.asset_health {
                                return SetupAssetProbe::Available(asset_health);
                            }
                            "service /list response missing asset_health".to_string()
                        }
                        Err(e) => format!("service /list returned error: {e:#}"),
                    },
                    Err(e) => format!("service /list query failed: {e:#}"),
                },
                Ok(_) => "service is not running".to_string(),
                Err(e) => format!("failed to read service status: {e:#}"),
            }
        };

        if Instant::now() >= deadline {
            return SetupAssetProbe::Unavailable(observation);
        }
        tokio::time::sleep(SETUP_SERVICE_TRUTH_POLL).await;
    }
}

/// Run the setup wizard.
pub async fn run_setup(opts: SetupOptions) -> Result<()> {
    let cd = capsem_dir()?;
    std::fs::create_dir_all(&cd)?;

    // Fast path: --force-onboarding resets only the GUI wizard flags.
    // Everything else about install state (security preset, detected
    // providers, corp config, completed steps) is preserved.
    if opts.force_onboarding && !opts.force {
        let mut state = load_state_from(&cd);
        state.reset_onboarding();
        save_state_to(&cd, &state)?;
        println!("Onboarding reset. The welcome wizard will show on next app launch.");
        return Ok(());
    }

    let mut state = if opts.force {
        SetupState::default()
    } else {
        load_state_from(&cd)
    };
    state.schema_version = 2;

    // Step 0: Corp config provisioning
    if let Some(ref source) = opts.corp_config {
        if opts.force || !state.is_step_done("corp_config") {
            step_corp_config(&cd, source, &mut state).await?;
        }
    }

    // Load merged settings for corp-awareness
    let (_user_settings, corp_settings) = policy_config::load_settings_files();

    // Step 1: Welcome + asset-manifest readiness checks.
    if opts.force || !state.is_step_done("welcome") {
        step_welcome(&cd, &mut state).await?;
    }

    // Step 3: Security preset
    if opts.force || !state.is_step_done("security_preset") {
        step_security_preset(&cd, &mut state, &opts, &corp_settings)?;
    }

    // Step 4: AI Providers
    if opts.force || !state.is_step_done("providers") {
        step_providers(&cd, &mut state, &opts, &corp_settings)?;
    }

    // Step 5: Repositories
    if opts.force || !state.is_step_done("repositories") {
        step_repositories(&cd, &mut state, &opts, &corp_settings)?;
    }

    // Step 6: Summary (guarded like other steps to avoid re-killing the service)
    if opts.force || !state.is_step_done("summary") {
        step_summary(&cd, &mut state, &opts).await?;
    }

    // All mandatory steps finished -- the CLI side of install is done.
    // Separate from onboarding_completed, which only the GUI wizard can flip.
    state.install_completed = state.is_step_done("summary");

    save_state_to(&cd, &state)?;
    println!("\nSetup complete.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Step implementations
// ---------------------------------------------------------------------------

async fn step_corp_config(capsem_dir: &Path, source: &str, state: &mut SetupState) -> Result<()> {
    println!("[1/6] Corp config provisioning...");

    let _content = if source.starts_with("http://") || source.starts_with("https://") {
        let client = reqwest::Client::new();
        let (body, etag) = corp_provision::fetch_corp_config(&client, source).await?;
        let content_hash = blake3::hash(body.as_bytes()).to_hex().to_string();
        let cs = corp_provision::CorpSource {
            url: Some(source.to_string()),
            file_path: None,
            fetched_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            etag,
            content_hash,
            refresh_interval_hours: corp_provision::parse_refresh_interval(&body),
        };
        corp_provision::install_corp_config(capsem_dir, &body, &cs)?;
        body
    } else {
        // Local file path
        let body = std::fs::read_to_string(source)
            .with_context(|| format!("cannot read corp config from {}", source))?;
        corp_provision::validate_corp_toml(&body)?;
        let content_hash = blake3::hash(body.as_bytes()).to_hex().to_string();
        let cs = corp_provision::CorpSource {
            url: None,
            file_path: Some(source.to_string()),
            fetched_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            etag: None,
            content_hash,
            refresh_interval_hours: corp_provision::parse_refresh_interval(&body),
        };
        corp_provision::install_corp_config(capsem_dir, &body, &cs)?;
        body
    };

    println!("  Corp config installed.");
    state.corp_config_source = Some(source.to_string());
    state.mark_done("corp_config");
    save_state_to(capsem_dir, state)?;
    Ok(())
}

async fn step_welcome(capsem_dir: &Path, state: &mut SetupState) -> Result<()> {
    println!("[2/6] Welcome to Capsem!");
    println!("  The fastest way to ship with AI securely.");

    let assets_dir = capsem_dir.join("assets");
    let manifest = match load_setup_manifest_for_assets(&assets_dir)? {
        Some(m) => m,
        None if setup_requires_manifest_for_layout(&crate::platform::detect_install_layout()) => {
            anyhow::bail!(
                "signed asset manifest missing at {}",
                assets_dir.join("manifest.json").display()
            );
        }
        None => {
            println!(
                "  Skipping asset check: no manifest at {}.",
                assets_dir.join("manifest.json").display()
            );
            state.mark_done("welcome");
            save_state_to(capsem_dir, state)?;
            return Ok(());
        }
    };
    let current = manifest.assets.current.clone();
    println!(
        "  Asset manifest verified (release {}). Service will verify and update assets in the background.",
        current
    );

    state.mark_done("welcome");
    save_state_to(capsem_dir, state)?;
    Ok(())
}

fn step_security_preset(
    capsem_dir: &Path,
    state: &mut SetupState,
    opts: &SetupOptions,
    corp: &policy_config::SettingsFile,
) -> Result<()> {
    println!("[3/6] Security preset...");

    let preset_locked = policy_config::is_setting_corp_locked("security.preset", corp);

    if preset_locked {
        if let Some(entry) = corp.settings.get("security.preset") {
            println!(
                "  Security preset configured by your organization: {:?}",
                entry.value
            );
        }
        state.security_preset = Some("corp-locked".to_string());
    } else if let Some(ref preset) = opts.preset {
        println!("  Applying preset: {}", preset);
        policy_config::apply_preset(preset).map_err(|e| anyhow::anyhow!(e))?;
        state.security_preset = Some(preset.clone());
    } else if opts.non_interactive {
        println!("  Using default preset: medium");
        policy_config::apply_preset("medium").map_err(|e| anyhow::anyhow!(e))?;
        state.security_preset = Some("medium".to_string());
    } else {
        // Interactive: prompt with inquire
        let choices = vec!["medium", "high"];
        let preset = inquire::Select::new("Select security preset:", choices)
            .prompt()
            .context("security preset selection cancelled")?;
        policy_config::apply_preset(preset).map_err(|e| anyhow::anyhow!(e))?;
        state.security_preset = Some(preset.to_string());
    }

    state.mark_done("security_preset");
    save_state_to(capsem_dir, state)?;
    Ok(())
}

fn step_providers(
    capsem_dir: &Path,
    state: &mut SetupState,
    opts: &SetupOptions,
    _corp: &policy_config::SettingsFile,
) -> Result<()> {
    println!("[4/6] AI providers...");

    // Detect and write to settings in one shot
    let summary = capsem_core::host_config::detect_and_write_to_settings();

    if opts.non_interactive || opts.accept_detected {
        let mut found = vec![];
        if summary.anthropic_api_key_present {
            found.push("Anthropic");
        }
        if summary.google_api_key_present || summary.google_adc_present {
            found.push("Google");
        }
        if summary.openai_api_key_present {
            found.push("OpenAI");
        }
        if found.is_empty() {
            println!("  No API keys detected. Configure later with `capsem setup --force`.");
        } else {
            println!("  Detected: {}", found.join(", "));
        }
    } else {
        println!("  Detecting credentials...");
        if summary.anthropic_api_key_present {
            println!("  Anthropic API key detected.");
        }
        if summary.openai_api_key_present {
            println!("  OpenAI API key detected.");
        }
        if summary.github_token_present {
            println!("  GitHub token detected.");
        }
    }

    if !summary.settings_written.is_empty() {
        println!(
            "  Wrote {} setting(s) to user.toml.",
            summary.settings_written.len()
        );
    }

    state.providers_done = true;
    state.mark_done("providers");
    save_state_to(capsem_dir, state)?;
    Ok(())
}

fn step_repositories(
    capsem_dir: &Path,
    state: &mut SetupState,
    _opts: &SetupOptions,
    _corp: &policy_config::SettingsFile,
) -> Result<()> {
    println!("[5/6] Repository access...");

    // Detection + settings write already happened in step_providers.
    // Just report what's available.
    let detected = capsem_core::host_config::detect();
    if detected.git_name.is_some() {
        println!("  Git configuration detected.");
    }
    if detected.ssh_public_key.is_some() {
        println!("  SSH keys detected.");
    }
    if detected.github_token.is_some() {
        println!("  GitHub access available.");
    }

    state.repositories_done = true;
    state.mark_done("repositories");
    save_state_to(capsem_dir, state)?;
    Ok(())
}

async fn step_summary(
    capsem_dir: &Path,
    state: &mut SetupState,
    _opts: &SetupOptions,
) -> Result<()> {
    println!("[6/6] Summary...");

    // PATH check (Linux/macOS)
    let bin_dir = capsem_dir.join("bin");
    if let Ok(path_var) = std::env::var("PATH") {
        if !path_var.split(':').any(|p| Path::new(p) == bin_dir) {
            println!();
            println!("  WARNING: {} is not in your PATH", bin_dir.display());
            println!("  Add to your shell profile: export PATH=\"$HOME/.capsem/bin:$PATH\"");
        }
    }

    if crate::service_install::test_isolation_env_active() {
        println!("  Test-isolation mode: skipping persistent service unit install.");
        state.service_installed = false;
    } else {
        crate::service_install::install_service()
            .await
            .context("service installation failed during setup")?;
        println!("  Service installed.");
        state.service_installed = true;
    }

    match fetch_setup_asset_health(capsem_dir).await {
        SetupAssetProbe::Available(asset_health) => {
            state.vm_verified = evaluate_setup_asset_health(&asset_health)?;
            if state.vm_verified {
                println!("  VM assets ready.");
            } else if asset_health.state == "error" {
                let detail = asset_health
                    .error
                    .as_deref()
                    .unwrap_or("service reported an unspecified asset error");
                println!(
                    "  VM assets are in error: {}. Setup completed config, but VM readiness is not verified.",
                    detail
                );
            } else {
                println!(
                    "  VM assets are still {}. Setup completed config; VM readiness will follow service progress.",
                    asset_health.state
                );
            }
        }
        SetupAssetProbe::Unavailable(observation) => {
            state.vm_verified = false;
            println!(
                "  Service asset status unavailable: {}. Setup completed config, but VM readiness is not verified.",
                observation
            );
        }
    }

    state.mark_done("summary");
    save_state_to(capsem_dir, state)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const UNSIGNED_MANIFEST: &str = r#"{
        "format": 2,
        "assets": {
            "current": "2026.0415.1",
            "releases": {
                "2026.0415.1": {
                    "date": "2026-04-15",
                    "deprecated": false,
                    "min_binary": "1.0.0",
                    "arches": {
                        "arm64": {
                            "vmlinuz": { "hash": "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c", "size": 7797248 },
                            "initrd.img": { "hash": "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456", "size": 2270154 },
                            "rootfs.squashfs": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }
                        }
                    }
                }
            }
        },
        "binaries": {
            "current": "1.0.1776269479",
            "releases": {
                "1.0.1776269479": {
                    "date": "2026-04-15",
                    "deprecated": false,
                    "min_assets": "2026.0415.1"
                }
            }
        }
    }"#;

    fn tmp_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    // ---- state_path_in ------------------------------------------------

    #[test]
    fn state_path_is_under_capsem_dir() {
        let d = tmp_dir();
        let p = state_path_in(d.path());
        assert_eq!(p, d.path().join("setup-state.json"));
    }

    // ---- load_state_from / save_state_to -------------------------------

    #[test]
    fn load_state_from_missing_dir_returns_default() {
        // Directory that's never had setup-state.json written.
        let d = tmp_dir();
        let s = load_state_from(d.path());
        assert_eq!(s.schema_version, 0);
        assert!(s.completed_steps.is_empty());
        assert!(s.security_preset.is_none());
        assert!(!s.providers_done);
        assert!(!s.onboarding_completed);
    }

    #[test]
    fn load_state_from_nonexistent_dir_also_returns_default() {
        // Not just empty dir -- nonexistent parent.
        let s = load_state_from(Path::new("/tmp/definitely-does-not-exist-capsem-test"));
        assert_eq!(s.schema_version, 0);
    }

    #[test]
    fn save_state_to_creates_parent_dirs() {
        let d = tmp_dir();
        // Write to a subdir that doesn't exist yet -- save_state should mkdir -p.
        let sub = d.path().join("deep").join("nested");
        let mut s = SetupState {
            schema_version: 2,
            ..SetupState::default()
        };
        s.mark_done("corp_config");
        s.security_preset = Some("high".into());
        save_state_to(&sub, &s).unwrap();
        assert!(
            sub.join("setup-state.json").exists(),
            "file was not written"
        );
    }

    #[test]
    fn save_then_load_roundtrips_fields() {
        let d = tmp_dir();
        let mut s = SetupState {
            schema_version: 2,
            providers_done: true,
            security_preset: Some("medium".into()),
            corp_config_source: Some("/tmp/corp.toml".into()),
            ..SetupState::default()
        };
        s.mark_done("welcome");
        s.mark_done("providers");
        save_state_to(d.path(), &s).unwrap();

        let loaded = load_state_from(d.path());
        assert_eq!(loaded.schema_version, 2);
        assert!(loaded.is_step_done("welcome"));
        assert!(loaded.is_step_done("providers"));
        assert_eq!(loaded.security_preset.as_deref(), Some("medium"));
        assert!(loaded.providers_done);
        assert_eq!(loaded.corp_config_source.as_deref(), Some("/tmp/corp.toml"));
    }

    #[test]
    fn save_state_is_atomic_overwrite() {
        let d = tmp_dir();
        // First write
        let mut s = SetupState {
            security_preset: Some("medium".into()),
            ..SetupState::default()
        };
        save_state_to(d.path(), &s).unwrap();
        // Overwrite with different state
        s.security_preset = Some("high".into());
        s.mark_done("summary");
        save_state_to(d.path(), &s).unwrap();
        // No temp file left behind.
        assert!(!d.path().join("setup-state.json.tmp").exists());
        let loaded = load_state_from(d.path());
        assert_eq!(loaded.security_preset.as_deref(), Some("high"));
        assert!(loaded.is_step_done("summary"));
    }

    #[test]
    fn load_state_from_corrupt_file_returns_default() {
        let d = tmp_dir();
        std::fs::write(state_path_in(d.path()), b"not valid json at all").unwrap();
        // load should silently return default -- no panic, no error propagation.
        let s = load_state_from(d.path());
        assert_eq!(s.schema_version, 0);
    }

    #[test]
    fn setup_manifest_loader_rejects_unsigned_manifest() {
        let d = tmp_dir();
        std::fs::write(d.path().join("manifest.json"), UNSIGNED_MANIFEST).unwrap();

        let err = load_setup_manifest_for_assets(d.path()).unwrap_err();
        assert!(
            format!("{err:#}").contains("signature missing"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn setup_manifest_loader_rejects_invalid_signature() {
        let d = tmp_dir();
        std::fs::write(d.path().join("manifest.json"), UNSIGNED_MANIFEST).unwrap();
        std::fs::write(d.path().join("manifest.json.minisig"), "not a signature").unwrap();

        let err = load_setup_manifest_for_assets(d.path()).unwrap_err();
        assert!(
            format!("{err:#}").contains("verify"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn setup_requires_manifest_for_installed_layouts_only() {
        use crate::platform::InstallLayout;

        assert!(!setup_requires_manifest_for_layout(
            &InstallLayout::Development
        ));
        assert!(setup_requires_manifest_for_layout(&InstallLayout::UserDir));
        assert!(setup_requires_manifest_for_layout(&InstallLayout::MacosPkg));
        assert!(setup_requires_manifest_for_layout(&InstallLayout::LinuxDeb));
    }

    fn asset_health(state: &str, ready: bool) -> crate::client::AssetHealth {
        crate::client::AssetHealth {
            ready,
            state: state.to_string(),
            version: Some("2026.0415.1".to_string()),
            arch: Some("arm64".to_string()),
            missing: Vec::new(),
            progress: None,
            error: None,
            retry_count: 0,
            retryable: false,
            saved_vm_dependencies: Vec::new(),
        }
    }

    #[test]
    fn setup_asset_health_ready_verifies_vm() {
        let health = asset_health("ready", true);
        assert!(evaluate_setup_asset_health(&health).unwrap());
    }

    #[test]
    fn setup_asset_health_ready_must_match_ready_flag() {
        let health = asset_health("ready", false);
        let err = evaluate_setup_asset_health(&health).unwrap_err();
        assert!(
            err.to_string().contains("state=ready but ready=false"),
            "unexpected error: {err:#}",
        );
    }

    #[test]
    fn setup_asset_health_checking_or_updating_is_pending() {
        let checking = asset_health("checking", false);
        let updating = asset_health("updating", false);
        assert!(!evaluate_setup_asset_health(&checking).unwrap());
        assert!(!evaluate_setup_asset_health(&updating).unwrap());
    }

    #[test]
    fn setup_asset_health_error_is_pending_and_unknown_fails() {
        let mut errored = asset_health("error", false);
        errored.error = Some("release source unavailable".to_string());
        assert!(!evaluate_setup_asset_health(&errored).unwrap());

        let unknown = asset_health("unknown", false);
        let unknown_error = evaluate_setup_asset_health(&unknown).unwrap_err();
        assert!(
            unknown_error.to_string().contains("state is unknown"),
            "unexpected error: {unknown_error:#}",
        );
    }

    // ---- step_corp_config (happy path + validation error) -------------

    #[tokio::test]
    async fn corp_config_from_local_file_marks_step_done() {
        let d = tmp_dir();
        let corp_toml = r#"
[metadata]
version = 1
org = "Test Co"

[policy.http.allow_example_docs]
on = "http.request"
if = 'request.host == "example.com"'
decision = "allow"
"#;
        let corp_path = d.path().join("corp.toml");
        std::fs::write(&corp_path, corp_toml).unwrap();

        let mut state = SetupState::default();
        step_corp_config(d.path(), corp_path.to_str().unwrap(), &mut state)
            .await
            .expect("corp config should install cleanly");

        assert!(state.is_step_done("corp_config"));
        assert_eq!(state.corp_config_source.as_deref(), corp_path.to_str());

        // save_state_to wrote it through; load should see the same thing.
        let loaded = load_state_from(d.path());
        assert!(loaded.is_step_done("corp_config"));
        assert_eq!(
            loaded.corp_config_source.as_deref(),
            corp_path.to_str(),
            "persisted state must reflect the corp source",
        );
    }

    #[tokio::test]
    async fn corp_config_rejects_invalid_toml() {
        let d = tmp_dir();
        let corp_path = d.path().join("bad.toml");
        std::fs::write(&corp_path, b"this is not = [valid toml").unwrap();

        let mut state = SetupState::default();
        let err = step_corp_config(d.path(), corp_path.to_str().unwrap(), &mut state)
            .await
            .expect_err("invalid TOML should produce error");
        assert!(!err.to_string().is_empty());
        // Step must NOT be marked done on failure.
        assert!(!state.is_step_done("corp_config"));
    }

    #[tokio::test]
    async fn corp_config_missing_file_errors_with_context() {
        let d = tmp_dir();
        let missing = d.path().join("does-not-exist.toml");
        let mut state = SetupState::default();
        let err = step_corp_config(d.path(), missing.to_str().unwrap(), &mut state)
            .await
            .expect_err("missing corp-config file should error");
        assert!(
            err.to_string().contains("cannot read corp config"),
            "error lost path context: {err}",
        );
        assert!(!state.is_step_done("corp_config"));
    }

    // ---- SetupOptions sanity ------------------------------------------

    #[test]
    fn setup_options_defaults_are_non_interactive_safe() {
        // This struct doesn't derive Default; spot-check that construction
        // works with the fields we depend on in tests.
        let o = SetupOptions {
            non_interactive: true,
            preset: None,
            force: false,
            accept_detected: false,
            corp_config: None,
            force_onboarding: false,
        };
        assert!(o.non_interactive);
        assert!(!o.force);
    }

    // ---- --force-onboarding fast path ---------------------------------
    //
    // The fast path in `run_setup` does: load -> reset_onboarding -> save.
    // All three primitives are already unit-tested individually:
    //   * load_state_from / save_state_to -- setup.rs tests above
    //   * SetupState::reset_onboarding     -- setup_state.rs tests
    // So the glue is exercised by walking the same primitives here and
    // confirming install-side fields survive the reset (i.e. that we didn't
    // accidentally call `SetupState::default()` on the force_onboarding path).
    #[test]
    fn force_onboarding_glue_preserves_install_state() {
        let d = tmp_dir();
        let mut state = SetupState {
            schema_version: 2,
            install_completed: true,
            onboarding_completed: true,
            onboarding_version: capsem_core::setup_state::CURRENT_ONBOARDING_VERSION,
            security_preset: Some("medium".into()),
            providers_done: true,
            ..SetupState::default()
        };
        state.mark_done("summary");
        save_state_to(d.path(), &state).unwrap();

        // Mirror run_setup's force_onboarding fast path.
        let mut loaded = load_state_from(d.path());
        loaded.reset_onboarding();
        save_state_to(d.path(), &loaded).unwrap();

        let after = load_state_from(d.path());
        assert!(!after.onboarding_completed);
        assert_eq!(after.onboarding_version, 0);
        assert!(after.install_completed);
        assert!(after.providers_done);
        assert_eq!(after.security_preset.as_deref(), Some("medium"));
        assert!(after.is_step_done("summary"));
    }
}

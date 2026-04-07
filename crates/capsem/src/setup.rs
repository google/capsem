//! Setup wizard orchestrator.
//!
//! `capsem setup` walks the user through first-time configuration:
//! corp config provisioning, security preset, AI provider keys,
//! repository access, service installation, and VM boot verification.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use tracing::{info, warn};

use capsem_core::asset_manager;
use capsem_core::net::policy_config;
use capsem_core::net::policy_config::corp_provision;

/// Options passed from CLI flags.
pub struct SetupOptions {
    pub non_interactive: bool,
    pub preset: Option<String>,
    pub force: bool,
    pub accept_detected: bool,
    pub corp_config: Option<String>,
}

/// Persistent state written to ~/.capsem/setup-state.json.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SetupState {
    pub schema_version: u32,
    #[serde(default)]
    pub completed_steps: Vec<String>,
    pub security_preset: Option<String>,
    #[serde(default)]
    pub providers_done: bool,
    #[serde(default)]
    pub repositories_done: bool,
    #[serde(default)]
    pub service_installed: bool,
    #[serde(default)]
    pub vm_verified: bool,
    pub corp_config_source: Option<String>,
}

impl SetupState {
    fn is_step_done(&self, step: &str) -> bool {
        self.completed_steps.iter().any(|s| s == step)
    }

    fn mark_done(&mut self, step: &str) {
        if !self.is_step_done(step) {
            self.completed_steps.push(step.to_string());
        }
    }
}

fn capsem_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".capsem"))
}

fn state_path() -> Result<PathBuf> {
    Ok(capsem_dir()?.join("setup-state.json"))
}

fn load_state() -> SetupState {
    state_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

fn save_state(state: &SetupState) -> Result<()> {
    let dir = capsem_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("setup-state.json");
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Run the setup wizard.
pub async fn run_setup(opts: SetupOptions) -> Result<()> {
    let mut state = if opts.force {
        SetupState::default()
    } else {
        load_state()
    };
    state.schema_version = 1;

    let cd = capsem_dir()?;
    std::fs::create_dir_all(&cd)?;

    // Step 0: Corp config provisioning
    if let Some(ref source) = opts.corp_config {
        if opts.force || !state.is_step_done("corp_config") {
            step_corp_config(&cd, source, &mut state).await?;
        }
    }

    // Load merged settings for corp-awareness
    let (_user_settings, corp_settings) = policy_config::load_settings_files();

    // Step 1: Welcome + start background asset download
    let bg_download = if opts.force || !state.is_step_done("welcome") {
        step_welcome(&mut state).await?
    } else {
        None
    };

    // Step 3: Security preset
    if opts.force || !state.is_step_done("security_preset") {
        step_security_preset(&mut state, &opts, &corp_settings)?;
    }

    // Step 4: AI Providers
    if opts.force || !state.is_step_done("providers") {
        step_providers(&mut state, &opts, &corp_settings)?;
    }

    // Step 5: Repositories
    if opts.force || !state.is_step_done("repositories") {
        step_repositories(&mut state, &opts, &corp_settings)?;
    }

    // Wait for background download to finish before summary
    if let Some(handle) = bg_download {
        match handle.await {
            Ok(Ok(result)) => {
                if result.downloaded > 0 {
                    println!("  Downloaded {} asset(s).", result.downloaded);
                }
                if result.failed > 0 {
                    println!("  Warning: {} asset(s) failed to download.", result.failed);
                }
            }
            Ok(Err(e)) => {
                warn!(error = %e, "background asset download failed");
                println!("  Asset download failed: {}. Run `capsem update` later.", e);
            }
            Err(e) => {
                warn!(error = %e, "background download task panicked");
            }
        }
    }

    // Step 6: Summary
    step_summary(&cd, &mut state, &opts).await?;

    save_state(&state)?;
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
    save_state(state)?;
    Ok(())
}

/// Type alias for the background download join handle.
type BgDownloadHandle = tokio::task::JoinHandle<anyhow::Result<asset_manager::BackgroundDownloadResult>>;

async fn step_welcome(state: &mut SetupState) -> Result<Option<BgDownloadHandle>> {
    println!("[2/6] Welcome to Capsem!");
    println!("  Capsem sandboxes AI agents in air-gapped Linux VMs.");

    // Start background asset download while the wizard continues
    let handle = match start_asset_download().await {
        Ok((handle, _rx)) => {
            println!("  Downloading VM assets in the background...");
            Some(handle)
        }
        Err(e) => {
            info!(error = %e, "skipping background download");
            println!("  Asset download skipped (run `capsem update` later).");
            None
        }
    };

    state.mark_done("welcome");
    save_state(state)?;
    Ok(handle)
}

async fn start_asset_download() -> Result<(
    BgDownloadHandle,
    tokio::sync::mpsc::Receiver<asset_manager::BackgroundProgress>,
)> {
    let client = reqwest::Client::new();
    let (version, manifest) = asset_manager::fetch_latest_manifest(&client).await?;
    let assets_dir = asset_manager::default_assets_dir()
        .ok_or_else(|| anyhow::anyhow!("HOME not set"))?;
    let arch = if cfg!(target_arch = "aarch64") {
        Some("arm64".to_string())
    } else {
        Some("x86_64".to_string())
    };
    let (handle, rx) = asset_manager::start_background_download(
        manifest, version, assets_dir, arch,
    );
    Ok((handle, rx))
}

fn step_security_preset(
    state: &mut SetupState,
    opts: &SetupOptions,
    corp: &policy_config::SettingsFile,
) -> Result<()> {
    println!("[3/6] Security preset...");

    let preset_locked = policy_config::is_setting_corp_locked("security.preset", corp);

    if preset_locked {
        if let Some(entry) = corp.settings.get("security.preset") {
            println!("  Security preset configured by your organization: {:?}", entry.value);
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
    save_state(state)?;
    Ok(())
}

fn step_providers(
    state: &mut SetupState,
    opts: &SetupOptions,
    _corp: &policy_config::SettingsFile,
) -> Result<()> {
    println!("[4/6] AI providers...");

    if opts.non_interactive || opts.accept_detected {
        // Auto-detect credentials
        let detected = capsem_core::host_config::detect();
        let mut found = vec![];
        if detected.anthropic_api_key.is_some() {
            found.push("Anthropic");
        }
        if detected.google_api_key.is_some() || detected.google_adc.is_some() {
            found.push("Google");
        }
        if detected.openai_api_key.is_some() {
            found.push("OpenAI");
        }
        if found.is_empty() {
            println!("  No API keys detected. Configure later with `capsem setup --force`.");
        } else {
            println!("  Detected: {}", found.join(", "));
        }
    } else {
        // Interactive mode would prompt for each provider
        println!("  Detecting credentials...");
        let detected = capsem_core::host_config::detect();
        if detected.anthropic_api_key.is_some() {
            println!("  Anthropic API key detected.");
        }
        if detected.openai_api_key.is_some() {
            println!("  OpenAI API key detected.");
        }
        if detected.github_token.is_some() {
            println!("  GitHub token detected.");
        }
    }

    state.providers_done = true;
    state.mark_done("providers");
    save_state(state)?;
    Ok(())
}

fn step_repositories(
    state: &mut SetupState,
    _opts: &SetupOptions,
    _corp: &policy_config::SettingsFile,
) -> Result<()> {
    println!("[5/6] Repository access...");

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
    save_state(state)?;
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

    // Install service
    match crate::service_install::install_service().await {
        Ok(()) => {
            println!("  Service installed.");
            state.service_installed = true;
        }
        Err(e) => {
            println!("  Service installation skipped: {}", e);
        }
    }

    state.mark_done("summary");
    Ok(())
}

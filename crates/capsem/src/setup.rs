//! Setup wizard orchestrator.
//!
//! `capsem setup` walks the user through first-time configuration:
//! corp config provisioning, security preset, AI provider keys,
//! repository access, service installation, and VM boot verification.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use tracing::warn;

use capsem_core::net::policy_config;
use capsem_core::net::policy_config::corp_provision;
use capsem_core::setup_state::SetupState;

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

    // Step 1: Welcome + start background asset download
    let bg_download = if opts.force || !state.is_step_done("welcome") {
        step_welcome(&cd, &mut state).await?
    } else {
        None
    };

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

    // Wait for background download to finish before summary
    if let Some(handle) = bg_download {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!(error = %e, "background asset download failed");
                println!("  Asset download failed: {}. Run `capsem update` later.", e);
            }
            Err(e) => {
                warn!(error = %e, "background download task panicked");
            }
        }
    }

    // Step 6: Summary (guarded like other steps to avoid re-killing the service)
    if opts.force || !state.is_step_done("summary") {
        step_summary(&cd, &mut state, &opts).await?;
    }

    // All mandatory steps finished -- the CLI side of install is done.
    // Separate from onboarding_completed, which only the GUI wizard can flip.
    state.install_completed = true;

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

/// Type alias for the background download join handle.
type BgDownloadHandle = tokio::task::JoinHandle<anyhow::Result<()>>;

async fn step_welcome(capsem_dir: &Path, state: &mut SetupState) -> Result<Option<BgDownloadHandle>> {
    println!("[2/6] Welcome to Capsem!");
    println!("  Capsem sandboxes AI agents in air-gapped Linux VMs.");

    // TODO: Asset download will be implemented with the orthogonal CI sprint.
    // For now, assets are built locally with `just build-assets`.
    println!("  Assets: use `just build-assets` to build VM images.");

    state.mark_done("welcome");
    save_state_to(capsem_dir, state)?;
    Ok(None)
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
        println!("  Wrote {} setting(s) to user.toml.", summary.settings_written.len());
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
    save_state_to(capsem_dir, state)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
        assert!(sub.join("setup-state.json").exists(), "file was not written");
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

    // ---- step_corp_config (happy path + validation error) -------------

    #[tokio::test]
    async fn corp_config_from_local_file_marks_step_done() {
        let d = tmp_dir();
        let corp_toml = r#"
[metadata]
version = 1
org = "Test Co"

[policy]
default_action = "allow"
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

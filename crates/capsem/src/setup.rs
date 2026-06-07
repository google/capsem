//! Setup wizard orchestrator.
//!
//! `capsem setup` walks the user through first-time configuration:
//! corp config provisioning, security preset, AI provider keys,
//! repository access, service installation, and VM boot verification.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::json;

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

const SETUP_SERVICE_TRUTH_TIMEOUT: Duration = Duration::from_secs(8);
const SETUP_SERVICE_TRUTH_POLL: Duration = Duration::from_millis(250);

enum SetupAssetProbe {
    Available(Box<client::AssetHealth>),
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
                            return SetupAssetProbe::Available(Box::new(asset_health));
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
                                return SetupAssetProbe::Available(Box::new(asset_health));
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

    // Step 1: Welcome + asset-manifest readiness checks.
    if opts.force || !state.is_step_done("welcome") {
        step_welcome(&cd, &mut state).await?;
    }

    // Step 3: Security preset
    if opts.force || !state.is_step_done("security_preset") {
        step_security_preset(&cd, &mut state, &opts)?;
    }

    // Step 4: AI Providers
    if opts.force || !state.is_step_done("providers") {
        step_providers(&cd, &mut state, &opts)?;
    }
    if let Some(profile_id) = state.security_preset.as_deref() {
        if let Some(asset_root) = local_profile_asset_root(&cd) {
            install_local_profile_revision_from_asset_root(
                &cd,
                profile_id,
                &asset_root,
                host_profile_asset_arch(),
            )
            .context("install local profile revision from assets")?;
        }
    }

    // Step 5: Repositories
    if opts.force || !state.is_step_done("repositories") {
        step_repositories(&cd, &mut state, &opts)?;
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
    println!("[1/6] Corp profile provisioning...");

    let body = if source.starts_with("http://") || source.starts_with("https://") {
        let client = reqwest::Client::new();
        let response = client
            .get(source)
            .header("User-Agent", "capsem")
            .send()
            .await
            .with_context(|| format!("failed to fetch corp profile from {source}"))?;
        if !response.status().is_success() {
            anyhow::bail!(
                "corp profile fetch failed: HTTP {} for {source}",
                response.status()
            );
        }
        response
            .text()
            .await
            .context("failed to read corp profile body")?
    } else {
        std::fs::read_to_string(source)
            .with_context(|| format!("cannot read corp profile from {}", source))?
    };
    capsem_core::settings_profiles::install_corp_profile_toml(capsem_dir, &body)
        .map_err(|e| anyhow::anyhow!(e))?;

    println!("  Corp profile installed.");
    state.corp_config_source = Some(source.to_string());
    state.mark_done("corp_config");
    save_state_to(capsem_dir, state)?;
    Ok(())
}

async fn step_welcome(capsem_dir: &Path, state: &mut SetupState) -> Result<()> {
    println!("[2/6] Welcome to Capsem!");
    println!("  The fastest way to ship with AI securely.");
    println!("  VM assets are selected and verified from the active profile.");

    state.mark_done("welcome");
    save_state_to(capsem_dir, state)?;
    Ok(())
}

fn step_security_preset(
    capsem_dir: &Path,
    state: &mut SetupState,
    opts: &SetupOptions,
) -> Result<()> {
    println!("[3/6] Default profile...");

    let selected_profile = if let Some(ref preset) = opts.preset {
        normalize_setup_profile_id(preset)
    } else if opts.non_interactive {
        capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID.to_string()
    } else {
        let choices = vec![capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID];
        inquire::Select::new("Select default profile:", choices)
            .prompt()
            .context("default profile selection cancelled")?
            .to_string()
    };
    let service_path = capsem_dir.join("service.toml");
    let mut service_settings =
        capsem_core::settings_profiles::load_service_settings_or_default(&service_path)
            .map_err(|e| anyhow::anyhow!(e))?;
    cleanup_package_profile_runtime_duplicates(&service_settings.profiles)
        .context("clean installed package profile duplicates")?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&service_settings.profiles)
        .map_err(|e| anyhow::anyhow!(e))?;
    if catalog.get(&selected_profile).is_none() {
        anyhow::bail!("unknown profile preset '{selected_profile}'");
    }
    service_settings.profiles.default_profile = selected_profile.clone();
    capsem_core::settings_profiles::write_service_settings(&service_path, &service_settings)
        .map_err(|e| anyhow::anyhow!(e))?;
    if let Some(asset_root) = local_profile_asset_root(capsem_dir) {
        install_local_profile_revision_from_asset_root(
            capsem_dir,
            &selected_profile,
            &asset_root,
            host_profile_asset_arch(),
        )
        .context("install local profile revision from assets")?;
    }
    println!("  Using default profile: {selected_profile}");
    state.security_preset = Some(selected_profile);

    state.mark_done("security_preset");
    save_state_to(capsem_dir, state)?;
    Ok(())
}

fn normalize_setup_profile_id(value: &str) -> String {
    match value {
        "medium" | "high" => capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID.to_string(),
        other => other.to_string(),
    }
}

fn local_profile_asset_root(capsem_dir: &Path) -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("CAPSEM_ASSETS_DIR").map(PathBuf::from) {
        return Some(root);
    }
    let root = capsem_dir.join("assets");
    if root.join("manifest.json").is_file() {
        Some(root)
    } else {
        None
    }
}

fn install_local_profile_revision_from_asset_root(
    capsem_dir: &Path,
    profile_id: &str,
    assets_root: &Path,
    arch: &str,
) -> Result<()> {
    const LOCAL_PROFILE_REVISION: &str = "2026.0520.1";

    let (profile_type, ui, profile_name) = if profile_id == "coding" {
        ("coding", "coding", "Coding")
    } else {
        ("everyday-work", "everyday", "Everyday Work")
    };

    let service_path = capsem_dir.join("service.toml");
    let mut service_settings =
        capsem_core::settings_profiles::load_service_settings_or_default(&service_path)
            .map_err(|e| anyhow::anyhow!(e))?;
    if service_settings.profiles.corp_dirs.is_empty() {
        service_settings
            .profiles
            .corp_dirs
            .push(capsem_dir.join("profiles").join("corp"));
    }
    service_settings.profiles.default_profile = profile_id.to_string();
    capsem_core::settings_profiles::write_service_settings(&service_path, &service_settings)
        .map_err(|e| anyhow::anyhow!(e))?;

    if install_packaged_profile_sidecar(&service_settings.profiles, profile_id)? {
        return Ok(());
    }

    let kernel = local_asset_path(assets_root, arch, "vmlinuz")?;
    let initrd = local_asset_path(assets_root, arch, "initrd.img")?;
    let rootfs = local_asset_path(assets_root, arch, "rootfs.squashfs")?;

    let payload = json!({
        "schema": "capsem.profile.v2",
        "version": 2,
        "id": profile_id,
        "revision": LOCAL_PROFILE_REVISION,
        "name": profile_name,
        "description": "Local development profile derived from the active VM assets.",
        "best_for": "Local development and smoke diagnostics.",
        "profile_type": profile_type,
        "ui": ui,
        "compatibility": {
            "min_binary": env!("CARGO_PKG_VERSION"),
            "guest_abi": "capsem-guest-v2"
        },
        "vm": {
            "memory_mib": 8192,
            "cpus": 4,
            "disk_mib": 32768,
            "network": "proxied",
            "track_rootfs_dependencies": true,
            "assets": {
                arch: {
                    "kernel": local_asset_json(&kernel, "application/octet-stream")?,
                    "initrd": local_asset_json(&initrd, "application/octet-stream")?,
                    "rootfs": local_asset_json(&rootfs, "application/vnd.squashfs")?
                }
            }
        },
        "packages": {
            "runtimes": {
                "python": "3.12",
                "node": "22",
                "uv": "0.4"
            },
            "python_modules": {},
            "node_packages": {},
            "system": {
                "distro": "debian",
                "release": "bookworm",
                "apt": {}
            }
        },
        "tools": {
            "capsem_doctor": {
                "version": "dev",
                "required": true,
                "source": "guest"
            }
        },
        "security": {
            "capabilities": {
                "credential_brokerage": "ask",
                "pii_detection": "ask",
                "mcp_rag": "allow",
                "mcp_tools": "allow",
                "network_egress": "ask",
                "file_boundaries": "ask",
                "audit": "audit"
            },
            "rules": {
                "dns": {
                    "allow_elie_net": {
                        "on": "dns.request",
                        "if": "dns.request.qname == 'elie.net'",
                        "decision": "allow",
                        "priority": 1,
                        "reason": "Local development read allowlist."
                    },
                    "allow_wildcard_elie_net": {
                        "on": "dns.request",
                        "if": "dns.request.qname == '*.elie.net'",
                        "decision": "allow",
                        "priority": 1,
                        "reason": "Local development read allowlist."
                    },
                    "allow_en_wikipedia_org": {
                        "on": "dns.request",
                        "if": "dns.request.qname == 'en.wikipedia.org'",
                        "decision": "allow",
                        "priority": 1,
                        "reason": "Local development read allowlist."
                    },
                    "allow_wildcard_wikipedia_org": {
                        "on": "dns.request",
                        "if": "dns.request.qname == '*.wikipedia.org'",
                        "decision": "allow",
                        "priority": 1,
                        "reason": "Local development read allowlist."
                    }
                },
                "http": {
                    "block_example_post": {
                        "on": "http.request",
                        "if": "http.request.host == 'example.com' && http.request.method == 'POST'",
                        "decision": "block",
                        "priority": 0,
                        "reason": "Doctor write-deny fixture."
                    },
                    "allow_elie_net": {
                        "on": "http.request",
                        "if": "http.request.host == 'elie.net'",
                        "decision": "allow",
                        "priority": 1,
                        "reason": "Local development read allowlist."
                    },
                    "allow_wildcard_elie_net": {
                        "on": "http.request",
                        "if": "http.request.host == '*.elie.net'",
                        "decision": "allow",
                        "priority": 1,
                        "reason": "Local development read allowlist."
                    },
                    "allow_en_wikipedia_org": {
                        "on": "http.request",
                        "if": "http.request.host == 'en.wikipedia.org'",
                        "decision": "allow",
                        "priority": 1,
                        "reason": "Local development read allowlist."
                    },
                    "allow_wildcard_wikipedia_org": {
                        "on": "http.request",
                        "if": "http.request.host == '*.wikipedia.org'",
                        "decision": "allow",
                        "priority": 1,
                        "reason": "Local development read allowlist."
                    }
                }
            }
        }
    });
    let payload_json =
        serde_json::to_string_pretty(&payload).context("serialize local profile payload")?;
    let manifest = capsem_core::profile_manifest::ProfileManifest::from_json(&format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "{profile_id}": {{
              "current_revision": "{LOCAL_PROFILE_REVISION}",
              "revisions": {{
                "{LOCAL_PROFILE_REVISION}": {{
                  "status": "active",
                  "min_binary": "{}",
                  "profile_url": "file://local-dev-profile.json",
                  "profile_hash": "blake3:{}",
                  "profile_signature_url": "file://local-dev-profile.json.minisig"
                }}
              }}
            }}
          }}
        }}"#,
        env!("CARGO_PKG_VERSION"),
        blake3::hash(payload_json.as_bytes()).to_hex()
    ))
    .context("build local profile manifest")?;
    let revision = manifest
        .revision(profile_id, LOCAL_PROFILE_REVISION)
        .context("resolve local profile manifest revision")?;
    let verified =
        capsem_core::profile_manifest::verify_installable_profile_payload(revision, &payload_json)
            .context("verify local profile payload")?;
    capsem_core::settings_profiles::install_verified_profile_payload(
        &service_settings.profiles,
        &verified,
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

fn install_packaged_profile_sidecar(
    roots: &capsem_core::settings_profiles::ProfileRootSettings,
    profile_id: &str,
) -> Result<bool> {
    let Some(profile_path) = find_packaged_profile_path(roots, profile_id) else {
        return Ok(false);
    };
    let input = std::fs::read_to_string(&profile_path)
        .with_context(|| format!("read package profile {}", profile_path.display()))?;
    let profile = capsem_core::settings_profiles::Profile::from_toml_str(&input)
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("parse package profile {}", profile_path.display()))?;
    let payload_json =
        serde_json::to_string_pretty(&profile).context("serialize package profile payload")?;
    let revision = profile
        .revision
        .clone()
        .filter(|revision| !revision.trim().is_empty())
        .context("package profile revision is required for install sidecar")?;
    let manifest = capsem_core::profile_manifest::ProfileManifest::from_json(&format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "{profile_id}": {{
              "current_revision": "{revision}",
              "revisions": {{
                "{revision}": {{
                  "status": "active",
                  "min_binary": "{}",
                  "profile_url": "file://packaged-profile.json",
                  "profile_hash": "blake3:{}",
                  "profile_signature_url": "file://packaged-profile.json.minisig"
                }}
              }}
            }}
          }}
        }}"#,
        env!("CARGO_PKG_VERSION"),
        blake3::hash(payload_json.as_bytes()).to_hex()
    ))
    .context("build package profile manifest")?;
    let revision_record = manifest
        .revision(profile_id, &revision)
        .context("resolve package profile manifest revision")?;
    let verified = capsem_core::profile_manifest::verify_installable_profile_payload(
        revision_record,
        &payload_json,
    )
    .context("verify package profile payload")?;
    capsem_core::settings_profiles::install_verified_profile_payload_sidecar(roots, &verified)
        .map_err(|e| anyhow::anyhow!(e))?;
    cleanup_package_profile_runtime_duplicates(roots)
        .context("clean package profile runtime duplicates")?;
    Ok(true)
}

fn find_packaged_profile_path(
    roots: &capsem_core::settings_profiles::ProfileRootSettings,
    profile_id: &str,
) -> Option<PathBuf> {
    let profile_filename = format!("{profile_id}.profile.toml");
    let legacy_filename = format!("{profile_id}.toml");
    roots.base_dirs.iter().find_map(|dir| {
        [dir.join(&profile_filename), dir.join(&legacy_filename)]
            .into_iter()
            .find(|path| path.is_file())
    })
}

fn cleanup_package_profile_runtime_duplicates(
    roots: &capsem_core::settings_profiles::ProfileRootSettings,
) -> Result<()> {
    for corp_dir in &roots.corp_dirs {
        if !corp_dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(corp_dir)
            .with_context(|| format!("read corp profile dir {}", corp_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }
            let Some(profile_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if find_packaged_profile_path(roots, profile_id).is_none() {
                continue;
            }
            let current = corp_dir
                .join(".catalog")
                .join("profiles")
                .join(profile_id)
                .join("current.json");
            if current.is_file() {
                std::fs::remove_file(&path).with_context(|| {
                    format!("remove duplicate package profile {}", path.display())
                })?;
            }
        }
    }
    Ok(())
}

fn local_asset_path(assets_root: &Path, arch: &str, logical_name: &str) -> Result<PathBuf> {
    let arch_path = assets_root.join(arch).join(logical_name);
    if arch_path.is_file() {
        return arch_path
            .canonicalize()
            .with_context(|| format!("canonicalize {}", arch_path.display()));
    }
    let flat_path = assets_root.join(logical_name);
    if flat_path.is_file() {
        return flat_path
            .canonicalize()
            .with_context(|| format!("canonicalize {}", flat_path.display()));
    }
    anyhow::bail!(
        "missing local profile asset {logical_name}; checked {} and {}",
        arch_path.display(),
        flat_path.display()
    );
}

fn local_asset_json(path: &Path, content_type: &str) -> Result<serde_json::Value> {
    let hash = capsem_core::asset_manager::hash_file(path)
        .with_context(|| format!("hash local profile asset {}", path.display()))?;
    let size = std::fs::metadata(path)
        .with_context(|| format!("stat local profile asset {}", path.display()))?
        .len();
    let url = reqwest::Url::from_file_path(path).map_err(|_| {
        anyhow::anyhow!(
            "asset path cannot be converted to file URL: {}",
            path.display()
        )
    })?;
    let signature_path = PathBuf::from(format!("{}.minisig", path.display()));
    let signature_url = reqwest::Url::from_file_path(&signature_path).map_err(|_| {
        anyhow::anyhow!(
            "asset signature path cannot be converted to file URL: {}",
            path.display()
        )
    })?;
    Ok(json!({
        "url": url.as_str(),
        "hash": format!("blake3:{hash}"),
        "signature_url": signature_url.as_str(),
        "size": size,
        "content_type": content_type
    }))
}

fn host_profile_asset_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        _ => std::env::consts::ARCH,
    }
}

fn step_providers(capsem_dir: &Path, state: &mut SetupState, opts: &SetupOptions) -> Result<()> {
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
            "  Wrote {} credential(s) to service.toml.",
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
            corp_config_source: Some("/tmp/corp-profile.toml".into()),
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
        assert_eq!(
            loaded.corp_config_source.as_deref(),
            Some("/tmp/corp-profile.toml")
        );
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

    fn asset_health(state: &str, ready: bool) -> crate::client::AssetHealth {
        crate::client::AssetHealth {
            ready,
            state: state.to_string(),
            profile_id: None,
            profile_revision: None,
            profile_payload_hash: None,
            profile_assets: Vec::new(),
            version: Some("2026.0415.1".to_string()),
            arch: Some("arm64".to_string()),
            missing: Vec::new(),
            progress: None,
            error: None,
            retry_count: 0,
            retryable: false,
            saved_vm_dependencies: Vec::new(),
            checked_at_unix_secs: None,
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
        let corp_profile_toml = r#"
version = 1
id = "test-corp"
name = "Test Corp"
best_for = "Managed test sessions."
profile_type = "coding"

[security.rules.http.allow_example_docs]
on = "http.request"
if = 'http.request.host == "example.com"'
decision = "allow"
"#;
        let corp_path = d.path().join("corp-profile.toml");
        std::fs::write(&corp_path, corp_profile_toml).unwrap();

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
            err.to_string().contains("cannot read corp profile"),
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

    #[test]
    fn local_profile_revision_installs_signed_catalog_shape_from_assets() {
        let d = tmp_dir();
        let assets = d.path().join("assets").join("arm64");
        let base_dir = d.path().join("profiles/base");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(assets.join("vmlinuz"), b"kernel").unwrap();
        std::fs::write(assets.join("initrd.img"), b"initrd").unwrap();
        std::fs::write(assets.join("rootfs.squashfs"), b"rootfs").unwrap();

        let mut settings = capsem_core::settings_profiles::ServiceSettings::default();
        settings.profiles.base_dirs = vec![base_dir];
        capsem_core::settings_profiles::write_service_settings(
            d.path().join("service.toml"),
            &settings,
        )
        .unwrap();

        install_local_profile_revision_from_asset_root(
            d.path(),
            capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID,
            &d.path().join("assets"),
            "arm64",
        )
        .unwrap();

        let settings = capsem_core::settings_profiles::load_service_settings_or_default(
            d.path().join("service.toml"),
        )
        .unwrap();
        let installed = capsem_core::settings_profiles::load_complete_installed_profile_revision(
            &settings.profiles,
            capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID,
        )
        .unwrap()
        .expect("local setup should install a complete profile revision");
        assert_eq!(installed.revision, "2026.0520.1");

        let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
            .expect("installed runtime profile should parse");
        let profile = &catalog
            .get(capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID)
            .expect("installed profile should be discoverable")
            .profile;
        let arm64 = &profile.vm.assets["arm64"];
        assert_eq!(
            arm64.kernel.hash,
            format!(
                "blake3:{}",
                capsem_core::asset_manager::hash_file(&assets.join("vmlinuz")).unwrap()
            )
        );
        assert_eq!(
            arm64.initrd.hash,
            format!(
                "blake3:{}",
                capsem_core::asset_manager::hash_file(&assets.join("initrd.img")).unwrap()
            )
        );
        assert_eq!(
            arm64.rootfs.hash,
            format!(
                "blake3:{}",
                capsem_core::asset_manager::hash_file(&assets.join("rootfs.squashfs")).unwrap()
            )
        );
        assert!(profile.security.rules.http.contains_key("allow_elie_net"));
        assert!(profile.security.rules.dns.contains_key("allow_elie_net"));
        assert_eq!(
            profile.security.rules.http["block_example_post"].condition,
            "http.request.host == 'example.com' && http.request.method == 'POST'"
        );
    }

    #[test]
    fn package_profile_revision_installs_sidecar_without_duplicate_profile() {
        let d = tmp_dir();
        let assets = d.path().join("assets").join("arm64");
        let base_dir = d.path().join("profiles/base");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(assets.join("vmlinuz"), b"kernel").unwrap();
        std::fs::write(assets.join("initrd.img"), b"initrd").unwrap();
        std::fs::write(assets.join("rootfs.squashfs"), b"rootfs").unwrap();

        std::fs::write(
            base_dir.join("everyday-work.profile.toml"),
            include_str!("../../../config/profiles/base/everyday-work.profile.toml"),
        )
        .unwrap();
        let mut settings = capsem_core::settings_profiles::ServiceSettings::default();
        settings.profiles.base_dirs = vec![base_dir.clone()];
        capsem_core::settings_profiles::write_service_settings(
            d.path().join("service.toml"),
            &settings,
        )
        .unwrap();

        install_local_profile_revision_from_asset_root(
            d.path(),
            capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID,
            &d.path().join("assets"),
            "arm64",
        )
        .unwrap();

        let settings = capsem_core::settings_profiles::load_service_settings_or_default(
            d.path().join("service.toml"),
        )
        .unwrap();
        let corp_dir = settings.profiles.corp_dirs[0].clone();
        assert!(
            !corp_dir.join("everyday-work.toml").exists(),
            "package sidecar install must not create a duplicate corp profile"
        );
        let installed = capsem_core::settings_profiles::load_complete_installed_profile_revision(
            &settings.profiles,
            capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID,
        )
        .unwrap()
        .expect("package profile sidecar should be complete");
        assert_eq!(
            installed.runtime_profile_path,
            base_dir.join("everyday-work.profile.toml")
        );
        capsem_core::settings_profiles::discover_profiles(&settings.profiles)
            .expect("package sidecar must not create duplicate profile ids");
    }

    #[test]
    fn package_profile_revision_installs_sidecar_without_local_heavy_assets() {
        let d = tmp_dir();
        let assets_root = d.path().join("assets");
        let base_dir = d.path().join("profiles/base");
        std::fs::create_dir_all(&assets_root).unwrap();
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(assets_root.join("manifest.json"), r#"{"format":2}"#).unwrap();
        std::fs::write(
            base_dir.join("everyday-work.profile.toml"),
            include_str!("../../../config/profiles/base/everyday-work.profile.toml"),
        )
        .unwrap();
        let mut settings = capsem_core::settings_profiles::ServiceSettings::default();
        settings.profiles.base_dirs = vec![base_dir.clone()];
        capsem_core::settings_profiles::write_service_settings(
            d.path().join("service.toml"),
            &settings,
        )
        .unwrap();

        install_local_profile_revision_from_asset_root(
            d.path(),
            capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID,
            &assets_root,
            "arm64",
        )
        .unwrap();

        let settings = capsem_core::settings_profiles::load_service_settings_or_default(
            d.path().join("service.toml"),
        )
        .unwrap();
        let installed = capsem_core::settings_profiles::load_complete_installed_profile_revision(
            &settings.profiles,
            capsem_core::settings_profiles::EVERYDAY_WORK_PROFILE_ID,
        )
        .unwrap()
        .expect("package profile sidecar should install without bundled heavy assets");
        assert_eq!(
            installed.runtime_profile_path,
            base_dir.join("everyday-work.profile.toml")
        );
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

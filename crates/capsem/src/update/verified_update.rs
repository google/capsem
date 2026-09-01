use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdatePlanStep {
    Binary,
    Profiles,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VerifiedUpdatePlan {
    pub(super) installed_binary: String,
    pub(super) selected_binary: String,
    pub(super) steps: Vec<UpdatePlanStep>,
}

#[derive(Debug)]
pub(super) struct StagedUpdate {
    pub(super) manifest_path: PathBuf,
    pub(super) installer_path: Option<PathBuf>,
    pub(super) assets_dir: Option<PathBuf>,
    pub(super) profiles_dir: Option<PathBuf>,
}

pub(super) fn plan_verified_update(
    check: &UpdateCheck,
    manifest_bytes: &[u8],
    installed_binary: &str,
) -> Result<VerifiedUpdatePlan> {
    if check.validation_status.as_deref() != Some("valid") || check.validation_error.is_some() {
        anyhow::bail!("cannot plan an update from a manifest that did not validate");
    }
    let source = check
        .source
        .as_deref()
        .filter(|source| !source.trim().is_empty())
        .context("verified update is missing its manifest source")?;
    let expected_hash = check
        .channel_hash
        .as_deref()
        .context("verified update is missing its manifest SHA-256")?;
    validate_hex_digest(expected_hash, 64, "verified manifest SHA-256")?;
    let actual_hash = channel_payload_hash(manifest_bytes);
    if !actual_hash.eq_ignore_ascii_case(expected_hash) {
        anyhow::bail!(
            "verified manifest snapshot SHA-256 mismatch for {source}: expected {expected_hash}, got {actual_hash}"
        );
    }

    let selected_binary = check.latest_version.as_deref().unwrap_or(installed_binary).to_string();
    semver::Version::parse(installed_binary)
        .with_context(|| format!("parse installed Capsem version {installed_binary}"))?;
    semver::Version::parse(&selected_binary)
        .with_context(|| format!("parse selected Capsem version {selected_binary}"))?;

    let graph = serde_json::from_slice::<ReleaseGraphManifest>(manifest_bytes).ok();
    if let Some(graph) = graph
        .as_ref()
        .filter(|graph| !graph.packages.is_empty() || !graph.profiles.is_empty())
    {
        validate_release_graph_update_pairing(graph, &selected_binary)?;
    } else {
        let manifest_text = std::str::from_utf8(manifest_bytes).context("release manifest is not valid UTF-8")?;
        let manifest = capsem_assets::asset_manager::ManifestV2::from_json(manifest_text)
            .context("parse verified release manifest")?;
        validate_v2_update_pairing(&manifest, &selected_binary)?;
    }

    let mut steps = Vec::new();
    if check.update_available {
        let installer = check.binary_installer.as_ref().ok_or_else(|| {
            anyhow::anyhow!("selected binary {selected_binary} has no verified installer for this installation")
        })?;
        validate_binary_installer_metadata(installer)?;
        steps.push(UpdatePlanStep::Binary);
    }

    let profiles_changed =
        check.profiles_update_available || check.assets_update_available || check.images_update_available;
    if profiles_changed {
        steps.push(UpdatePlanStep::Profiles);
    }

    let blocked = [
        check.profiles_blocked_reason.as_deref(),
        check.assets_blocked_reason.as_deref(),
        check.images_blocked_reason.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if !blocked.is_empty() && !check.update_available {
        anyhow::bail!(
            "selected profile update is incompatible with installed binary {installed_binary}: {}",
            blocked.join("; ")
        );
    }

    Ok(VerifiedUpdatePlan {
        installed_binary: installed_binary.to_string(),
        selected_binary,
        steps,
    })
}

pub(super) fn validate_release_graph_update_pairing(graph: &ReleaseGraphManifest, selected_binary: &str) -> Result<()> {
    let selected = semver::Version::parse(selected_binary)
        .with_context(|| format!("parse selected Capsem version {selected_binary}"))?;
    if let Some(graph_binary) = graph_current_binary_version(&graph.packages)? {
        if graph_binary != selected_binary {
            anyhow::bail!(
                "verified manifest selects binary {graph_binary}, but update check selected {selected_binary}"
            );
        }
    }

    for (profile_id, profile) in &graph.profiles {
        if release_channel_status_is_revoked(&profile.status) {
            continue;
        }
        if let Some(minimum) = profile
            .extra
            .get("min_capsem_version")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            let minimum_version = semver::Version::parse(minimum)
                .with_context(|| format!("profile {profile_id} has invalid minimum Capsem version {minimum}"))?;
            if selected < minimum_version {
                anyhow::bail!("profile {profile_id} requires Capsem {minimum} or newer, selected {selected_binary}");
            }
        }
        if let Some(maximum) = profile
            .extra
            .get("max_capsem_version")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            let maximum_version = semver::Version::parse(maximum)
                .with_context(|| format!("profile {profile_id} has invalid maximum Capsem version {maximum}"))?;
            if selected > maximum_version {
                anyhow::bail!("profile {profile_id} supports Capsem through {maximum}, selected {selected_binary}");
            }
        }
    }
    Ok(())
}

pub(super) fn validate_v2_update_pairing(
    manifest: &capsem_assets::asset_manager::ManifestV2,
    selected_binary: &str,
) -> Result<()> {
    if manifest.binaries.current != selected_binary {
        anyhow::bail!(
            "verified manifest selects binary {}, but update check selected {selected_binary}",
            manifest.binaries.current
        );
    }
    let resolved = manifest
        .resolve(
            selected_binary,
            capsem_assets::asset_manager::host_manifest_arch(),
            Path::new("."),
        )
        .with_context(|| {
            format!("selected binary {selected_binary} has no compatible profile assets in the verified manifest")
        })?;
    if resolved.asset_version != manifest.assets.current {
        anyhow::bail!(
            "verified manifest selects incompatible binary/profile state: binary {selected_binary} resolves assets {}, not selected {}",
            resolved.asset_version,
            manifest.assets.current
        );
    }
    Ok(())
}

pub(super) async fn stage_verified_update_at(
    capsem_home: &Path,
    plan: &VerifiedUpdatePlan,
    check: &UpdateCheck,
    manifest_bytes: &[u8],
) -> Result<StagedUpdate> {
    let derived = plan_verified_update(check, manifest_bytes, &plan.installed_binary)?;
    if &derived != plan {
        anyhow::bail!("verified update plan changed before artifact staging");
    }
    let source = check
        .source
        .as_deref()
        .context("verified update stage is missing its manifest source")?;
    let identity = check
        .channel_hash
        .as_deref()
        .context("verified update stage is missing its manifest SHA-256")?;
    validate_hex_digest(identity, 64, "verified update candidate identity")?;

    let candidates_dir = capsem_home.join("updates").join("candidates");
    std::fs::create_dir_all(&candidates_dir).with_context(|| format!("create {}", candidates_dir.display()))?;
    let final_root = candidates_dir.join(identity);
    let stage_root = candidates_dir.join(format!(".{identity}.{}.tmp", std::process::id()));
    if stage_root.exists() {
        std::fs::remove_dir_all(&stage_root).with_context(|| format!("remove stale {}", stage_root.display()))?;
    }
    std::fs::create_dir(&stage_root).with_context(|| format!("create {}", stage_root.display()))?;

    let stage_result: Result<Option<PathBuf>> = async {
        atomic_write(&stage_root.join("manifest.json"), manifest_bytes)?;
        let installer_path = if plan.steps.contains(&UpdatePlanStep::Binary) {
            let installer = check
                .binary_installer
                .as_ref()
                .context("binary update stage is missing its installer")?;
            Some(download_binary_installer_at(capsem_home, installer).await?)
        } else {
            None
        };
        if plan.steps.contains(&UpdatePlanStep::Profiles) {
            stage_profile_candidate(&stage_root, source, manifest_bytes, &plan.selected_binary, check).await?;
        }
        Ok(installer_path)
    }
    .await;

    let installer_path = match stage_result {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&stage_root);
            return Err(error).context("stage verified update candidate");
        }
    };
    if final_root.exists() {
        std::fs::remove_dir_all(&final_root)
            .with_context(|| format!("replace staged candidate {}", final_root.display()))?;
    }
    std::fs::rename(&stage_root, &final_root).with_context(|| {
        format!(
            "commit staged candidate {} to {}",
            stage_root.display(),
            final_root.display()
        )
    })?;

    let assets_dir = final_root.join("assets");
    let profiles_dir = final_root.join("profiles");
    Ok(StagedUpdate {
        manifest_path: final_root.join("manifest.json"),
        installer_path,
        assets_dir: assets_dir.is_dir().then_some(assets_dir),
        profiles_dir: profiles_dir.is_dir().then_some(profiles_dir),
    })
}

pub(super) async fn stage_profile_candidate(
    stage_root: &Path,
    source: &str,
    manifest_bytes: &[u8],
    selected_binary: &str,
    check: &UpdateCheck,
) -> Result<()> {
    let body = std::str::from_utf8(manifest_bytes)
        .with_context(|| format!("manifest URL did not return UTF-8 JSON: {source}"))?;
    let document: serde_json::Value =
        serde_json::from_str(body).with_context(|| format!("parse manifest JSON from {source}"))?;
    if document.get("format").is_none() && document.get("profiles").is_some() {
        let arch = capsem_assets::asset_manager::host_manifest_arch();
        let graph = manifest_from_release_channel_profile_graph(body, arch)?;
        capsem_assets::asset_manager::ManifestV2::from_json(body)
            .context("validate release graph through the runtime manifest parser")?;
        hydrate_release_channel_profile_assets(&stage_root.join("assets"), source, &graph.asset_downloads).await?;
        stage_release_channel_profile_configs(
            source,
            &graph.config_downloads,
            &graph.runtime_pins,
            &stage_root.join("profiles"),
        )
        .await?;
        return Ok(());
    }

    capsem_assets::asset_manager::ManifestV2::from_json(body)
        .with_context(|| format!("parse format 2 manifest from {source}"))?;
    let assets_dir = stage_root.join("assets");
    std::fs::create_dir_all(&assets_dir).with_context(|| format!("create {}", assets_dir.display()))?;
    atomic_write(&assets_dir.join("manifest.json"), manifest_bytes)?;
    let metadata = serde_json::json!({
        "schema": "capsem.manifest_metadata.v1",
        "origin": "update_candidate",
        "manifest_url": source,
    });
    atomic_write(
        &assets_dir.join("manifest-metadata.json"),
        &serde_json::to_vec_pretty(&metadata)?,
    )?;
    hydrate_assets_for_binary(&assets_dir, selected_binary).await?;

    match (
        check.profile_catalog_source.as_deref(),
        check.profile_catalog_hash.as_deref(),
    ) {
        (Some(_), Some(_)) => stage_published_profile_catalog(check, &stage_root.join("profiles")).await,
        (None, None) => Ok(()),
        _ => anyhow::bail!("profile catalog update must provide both its immutable source and BLAKE3 digest"),
    }
}

pub(super) fn activate_staged_update_at(
    capsem_home: &Path,
    installed_assets: &Path,
    staged: &StagedUpdate,
    check: &UpdateCheck,
    transition: &ChannelTransition,
) -> Result<()> {
    let source = check
        .source
        .as_deref()
        .context("staged update activation is missing its manifest source")?;
    let expected_hash = check
        .channel_hash
        .as_deref()
        .context("staged update activation is missing its manifest SHA-256")?;
    let manifest_bytes =
        std::fs::read(&staged.manifest_path).with_context(|| format!("read {}", staged.manifest_path.display()))?;
    let actual_hash = channel_payload_hash(&manifest_bytes);
    if !actual_hash.eq_ignore_ascii_case(expected_hash) {
        anyhow::bail!(
            "staged manifest SHA-256 mismatch before activation: expected {expected_hash}, got {actual_hash}"
        );
    }

    std::fs::create_dir_all(installed_assets).with_context(|| format!("create {}", installed_assets.display()))?;
    let previous_manifest = InstalledManifestSnapshot::capture(installed_assets)?;
    let profiles_dir = capsem_home.join("profiles");
    let profile_stage = capsem_home.join(format!("profiles.activating.{}", std::process::id()));
    let profile_backup = capsem_home.join(format!("profiles.previous.{}", std::process::id()));
    remove_directory_if_present(&profile_stage)?;
    remove_directory_if_present(&profile_backup)?;

    let mut created_assets = Vec::new();
    let mut profiles_swapped = false;
    let activation_result: Result<()> = (|| {
        if let Some(candidate_assets) = staged.assets_dir.as_deref() {
            created_assets = copy_staged_asset_files(candidate_assets, installed_assets)
                .with_context(|| format!("activate staged profile assets from {}", candidate_assets.display()))?;
        }

        if let Some(candidate_profiles) = staged.profiles_dir.as_deref() {
            copy_directory_tree(candidate_profiles, &profile_stage)?;
            if profiles_dir.exists() {
                if !profiles_dir.is_dir() {
                    anyhow::bail!(
                        "installed profile catalog is not a directory: {}",
                        profiles_dir.display()
                    );
                }
                std::fs::rename(&profiles_dir, &profile_backup).with_context(|| {
                    format!(
                        "move installed profile catalog {} to {}",
                        profiles_dir.display(),
                        profile_backup.display()
                    )
                })?;
            }
            if let Err(error) = std::fs::rename(&profile_stage, &profiles_dir) {
                if profile_backup.exists() {
                    let _ = std::fs::rename(&profile_backup, &profiles_dir);
                }
                return Err(anyhow::Error::new(error)
                    .context(format!("activate staged profile catalog at {}", profiles_dir.display())));
            }
            profiles_swapped = true;
        }

        atomic_write(&installed_assets.join("manifest.json"), &manifest_bytes)?;
        write_installed_manifest_metadata(installed_assets, source, &manifest_bytes)?;
        persist_channel_transition(installed_assets, transition)?;
        Ok(())
    })();

    if let Err(error) = activation_result {
        let mut rollback_errors = Vec::new();
        if profiles_swapped {
            if let Err(rollback_error) = remove_directory_if_present(&profiles_dir) {
                rollback_errors.push(format!("{rollback_error:#}"));
            }
            if profile_backup.exists() {
                if let Err(rollback_error) = std::fs::rename(&profile_backup, &profiles_dir) {
                    rollback_errors.push(format!(
                        "restore profile catalog {}: {rollback_error}",
                        profiles_dir.display()
                    ));
                }
            }
        }
        if let Err(rollback_error) = previous_manifest.restore(installed_assets) {
            rollback_errors.push(format!("{rollback_error:#}"));
        }
        for path in created_assets.iter().rev() {
            if let Err(rollback_error) = std::fs::remove_file(path) {
                if rollback_error.kind() != std::io::ErrorKind::NotFound {
                    rollback_errors.push(format!("remove {}: {rollback_error}", path.display()));
                }
            }
        }
        let _ = remove_directory_if_present(&profile_stage);
        if rollback_errors.is_empty() {
            return Err(error).context("activate staged update; restored previous installed state");
        }
        anyhow::bail!(
            "activate staged update failed: {error:#}; rollback also failed: {}",
            rollback_errors.join("; ")
        );
    }

    remove_directory_if_present(&profile_backup)?;
    Ok(())
}

pub(super) fn activate_staged_update_with_asset_audit(
    capsem_home: &Path,
    installed_assets: &Path,
    staged: &StagedUpdate,
    check: &UpdateCheck,
    transition: &ChannelTransition,
) -> Result<()> {
    let source = check
        .source
        .as_deref()
        .context("staged update audit is missing its manifest source")?;
    let candidate_manifest_sha256 = check
        .channel_hash
        .as_deref()
        .context("staged update audit is missing its manifest SHA-256")?;
    let previous_state = installed_asset_audit_state(installed_assets);
    append_update_audit(serde_json::json!({
        "event": "asset_update_start",
        "action": "asset_update",
        "outcome": "started",
        "source": source,
        "channel": channel_from_source(source),
        "candidate_manifest_sha256": candidate_manifest_sha256,
        "previous": previous_state
    }));

    if let Err(error) = activate_staged_update_at(capsem_home, installed_assets, staged, check, transition) {
        append_update_audit(serde_json::json!({
            "event": "asset_update_failed",
            "action": "asset_update",
            "outcome": "failure",
            "source": source,
            "channel": channel_from_source(source),
            "candidate_manifest_sha256": candidate_manifest_sha256,
            "previous": previous_state,
            "current": installed_asset_audit_state(installed_assets),
            "error": format!("{error:#}")
        }));
        return Err(error);
    }

    let current_state = installed_asset_audit_state(installed_assets);
    append_update_audit(serde_json::json!({
        "event": "asset_update_complete",
        "action": "asset_update",
        "outcome": "success",
        "source": source,
        "channel": channel_from_source(source),
        "candidate_manifest_sha256": candidate_manifest_sha256,
        "previous": previous_state,
        "current": current_state,
        "changed_fields": changed_asset_audit_fields(&previous_state, &current_state)
    }));
    Ok(())
}

pub(super) fn copy_staged_asset_files(source_root: &Path, target_root: &Path) -> Result<Vec<PathBuf>> {
    let mut created = Vec::new();
    for source in regular_files_below(source_root)? {
        let relative = source
            .strip_prefix(source_root)
            .context("staged asset escaped its candidate root")?;
        if relative.components().count() == 1
            && matches!(
                relative.file_name().and_then(|name| name.to_str()),
                Some("manifest.json" | "manifest-metadata.json")
            )
        {
            continue;
        }
        let target = target_root.join(relative);
        if target.exists() {
            let source_bytes = std::fs::read(&source).with_context(|| format!("read {}", source.display()))?;
            let target_bytes = std::fs::read(&target).with_context(|| format!("read {}", target.display()))?;
            if source_bytes != target_bytes {
                anyhow::bail!(
                    "installed content-addressed asset conflicts with staged bytes: {}",
                    target.display()
                );
            }
            continue;
        }
        let parent = target.parent().context("staged asset target has no parent directory")?;
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        let file_name = target
            .file_name()
            .and_then(|name| name.to_str())
            .context("staged asset target filename is not UTF-8")?;
        let tmp = parent.join(format!(".{file_name}.activate.tmp"));
        let _ = std::fs::remove_file(&tmp);
        std::fs::copy(&source, &tmp).with_context(|| format!("copy {} to {}", source.display(), tmp.display()))?;
        std::fs::rename(&tmp, &target).with_context(|| format!("activate {}", target.display()))?;
        created.push(target);
    }
    Ok(created)
}

pub(super) fn copy_directory_tree(source: &Path, target: &Path) -> Result<()> {
    if !source.is_dir() {
        anyhow::bail!("staged directory is missing: {}", source.display());
    }
    remove_directory_if_present(target)?;
    std::fs::create_dir_all(target).with_context(|| format!("create {}", target.display()))?;
    for entry in std::fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry.with_context(|| format!("read entry under {}", source.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("inspect {}", entry.path().display()))?;
        let destination = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory_tree(&entry.path(), &destination)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &destination).with_context(|| {
                format!(
                    "copy staged profile {} to {}",
                    entry.path().display(),
                    destination.display()
                )
            })?;
        } else {
            anyhow::bail!(
                "staged profile catalog contains unsupported filesystem entry {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

pub(super) fn regular_files_below(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).with_context(|| format!("read {}", directory.display()))? {
            let entry = entry.with_context(|| format!("read entry under {}", directory.display()))?;
            let file_type = entry
                .file_type()
                .with_context(|| format!("inspect {}", entry.path().display()))?;
            if file_type.is_dir() {
                directories.push(entry.path());
            } else if file_type.is_file() {
                files.push(entry.path());
            } else {
                anyhow::bail!(
                    "staged update contains unsupported filesystem entry {}",
                    entry.path().display()
                );
            }
        }
    }
    files.sort();
    Ok(files)
}

pub(super) fn remove_directory_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

impl ReleaseChannelUpdateTarget {
    #[allow(dead_code)]
    pub(super) fn latest_version(&self) -> Option<String> {
        self.latest.clone().or_else(|| self.current.clone())
    }
}

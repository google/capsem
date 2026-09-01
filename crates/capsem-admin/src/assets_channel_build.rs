use super::*;

pub(super) fn assets_channel_build_command(args: AssetsChannelBuildArgs) -> Result<()> {
    let generated_at = args.generated_at.unwrap_or(current_utc_rfc3339()?);
    let report = build_assets_channel_with_policy(
        &args.manifest,
        &args.assets_dir,
        &args.profiles_dir,
        &args.channel,
        &args.manifest_version,
        &args.out_dir,
        &generated_at,
        args.asset_source_base.as_deref(),
        args.profile_revision_policy,
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("generated assets channel {} at {}", report.channel, report.out_dir);
    }
    Ok(())
}

pub(super) fn assets_channel_check_command(args: AssetsChannelCheckArgs) -> Result<()> {
    let report = check_assets_channel(&args.dist, &args.channel)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("valid: assets channel {} ({})", report.channel, args.dist.display());
    }
    Ok(())
}

pub(super) fn assets_channel_record_binary_command(args: AssetsChannelRecordBinaryArgs) -> Result<()> {
    let date = args.date.unwrap_or(current_utc_date()?);
    let report = record_binary_release_metadata(
        &args.manifest_path,
        &args.version,
        &args.source_commit,
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

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) fn build_assets_channel(
    manifest_url: &str,
    assets_dir: &Path,
    profiles_dir: &Path,
    channel: &str,
    manifest_version: &str,
    out_dir: &Path,
    generated_at: &str,
    asset_source_base: Option<&str>,
) -> Result<AssetsChannelBuildReport> {
    build_assets_channel_with_policy(
        manifest_url,
        assets_dir,
        profiles_dir,
        channel,
        manifest_version,
        out_dir,
        generated_at,
        asset_source_base,
        ProfileRevisionPolicyArg::Strict,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_assets_channel_with_policy(
    manifest_url: &str,
    assets_dir: &Path,
    profiles_dir: &Path,
    channel: &str,
    manifest_version: &str,
    out_dir: &Path,
    generated_at: &str,
    asset_source_base: Option<&str>,
    profile_revision_policy: ProfileRevisionPolicyArg,
) -> Result<AssetsChannelBuildReport> {
    validate_channel_name(channel)?;
    let manifest_bytes = read_manifest_url(manifest_url)?;
    let manifest_content = std::str::from_utf8(&manifest_bytes)
        .with_context(|| format!("manifest URL did not return UTF-8 JSON: {manifest_url}"))?;
    let manifest_value: serde_json::Value =
        serde_json::from_str(manifest_content).with_context(|| format!("parse manifest from {manifest_url}"))?;
    if is_release_graph_manifest_value(&manifest_value) {
        return build_assets_channel_from_graph(manifest_value, channel, manifest_version, out_dir, generated_at);
    }
    let manifest =
        ManifestV2::from_json(manifest_content).with_context(|| format!("parse manifest from {manifest_url}"))?;
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
    let channel_dir = out_dir.join("assets").join(channel);
    let copy_vm_blobs = asset_base == "/assets/releases";
    let current_asset_version = channel_manifest_doc.assets.current.clone();
    let release_dir = out_dir.join("assets").join("releases").join(&current_asset_version);
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    if channel_dir.exists() {
        fs::remove_dir_all(&channel_dir).with_context(|| format!("remove {}", channel_dir.display()))?;
    }
    let graph_channel_dir = out_dir.join("manifests").join(channel);
    if graph_channel_dir.exists() {
        fs::remove_dir_all(&graph_channel_dir).with_context(|| format!("remove {}", graph_channel_dir.display()))?;
    }
    if copy_vm_blobs && release_dir.exists() {
        fs::remove_dir_all(&release_dir).with_context(|| format!("remove {}", release_dir.display()))?;
    }
    fs::create_dir_all(&channel_dir).with_context(|| format!("create {}", channel_dir.display()))?;
    if copy_vm_blobs {
        fs::create_dir_all(&release_dir).with_context(|| format!("create {}", release_dir.display()))?;
    }
    let mut asset_digest_cache = AssetDigestCache::new();
    let copied_assets = if copy_vm_blobs {
        let current_release = channel_manifest_doc
            .assets
            .releases
            .get_mut(&current_asset_version)
            .ok_or_else(|| anyhow!("manifest current asset release is missing"))?;
        copy_assets_channel_release_assets(assets_dir, &release_dir, current_release, &mut asset_digest_cache)?
    } else {
        hydrate_current_asset_entry_sha256(&mut channel_manifest_doc, assets_dir, &mut asset_digest_cache)?;
        0
    };
    let publishable_profiles = publishable_profiles(
        &channel_manifest_doc,
        profiles_dir,
        channel,
        asset_base,
        assets_dir,
        &mut asset_digest_cache,
        profile_revision_policy,
    )?;
    copy_profile_release_files(out_dir, &publishable_profiles.file_copies)?;
    validate_graph_manifest_version(manifest_version)?;
    let graph_manifest_version = manifest_version.to_string();
    let graph_manifest_url = format!("/assets/{channel}/manifest.json");
    let graph_manifest = render_graph_release_manifest(
        &channel_manifest_doc,
        channel,
        &publishable_profiles.profiles,
        asset_base,
        &graph_manifest_version,
    )?;
    let channel_manifest = channel_dir.join("manifest.json");
    fs::write(&channel_manifest, &graph_manifest).with_context(|| format!("write {}", channel_manifest.display()))?;
    let graph_manifest_sha256 = format!("{:x}", Sha256::digest(graph_manifest.as_bytes()));
    let graph_manifest_blake3 = blake3::hash(graph_manifest.as_bytes()).to_hex().to_string();
    let index = assets_channel_index(
        &channel_manifest_doc,
        channel,
        generated_at,
        &graph_manifest_blake3,
        publishable_profiles.summary,
        asset_base,
    );
    fs::write(
        out_dir.join("channels.json"),
        render_assets_channels_catalog(
            &out_dir.join("channels.json"),
            &index,
            &graph_manifest_version,
            &graph_manifest_url,
            &graph_manifest_sha256,
            &graph_manifest_blake3,
        )?,
    )
    .with_context(|| format!("write {}", out_dir.join("channels.json").display()))?;
    let health_json = render_assets_channel_health(&index)?;
    fs::write(out_dir.join("health.json"), &health_json)
        .with_context(|| format!("write {}", out_dir.join("health.json").display()))?;
    fs::write(channel_dir.join("health.json"), &health_json)
        .with_context(|| format!("write {}", channel_dir.join("health.json").display()))?;
    fs::write(
        out_dir.join("_headers"),
        render_assets_channel_headers_for_dist(out_dir, channel)?,
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
        channels_json: out_dir.join("channels.json").display().to_string(),
        manifest: channel_manifest.display().to_string(),
        health_json: out_dir.join("health.json").display().to_string(),
        copied_assets,
    })
}

pub(super) fn record_binary_release_metadata(
    manifest_path: &Path,
    version: &str,
    source_commit: &SourceCommit,
    min_assets: Option<&str>,
    artifacts: &[PathBuf],
    date: &str,
) -> Result<AssetsChannelRecordBinaryReport> {
    if artifacts.is_empty() {
        return Err(anyhow!("at least one binary release artifact is required"));
    }
    validate_binary_version(version)?;
    validate_release_date(date)?;
    // Validate the candidate bytes independently of the manifest shape. A
    // legacy graph is no longer writable because it cannot carry package
    // provenance, but that must not turn a malformed package into a plausible
    // provenance-only failure.
    let files = binary_files_from_artifacts(artifacts)?;
    validate_binary_release_files(version, &files)?;
    let manifest_content =
        fs::read_to_string(manifest_path).with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest_value: serde_json::Value = serde_json::from_str(&manifest_content)
        .with_context(|| format!("parse manifest {}", manifest_path.display()))?;
    if is_release_graph_manifest_value(&manifest_value) {
        return record_graph_binary_release_metadata(
            manifest_path,
            manifest_value,
            version,
            source_commit,
            min_assets,
            &files,
        );
    }
    Err(anyhow!(
        "record-binary requires a release graph manifest so every package row records source_commit"
    ))
}

pub(super) fn build_assets_channel_from_graph(
    mut graph_manifest: serde_json::Value,
    channel: &str,
    manifest_version: &str,
    out_dir: &Path,
    generated_at: &str,
) -> Result<AssetsChannelBuildReport> {
    validate_assets_channel_graph_manifest(&graph_manifest, channel)?;
    validate_graph_profiles_match_current_binary(&graph_manifest)?;
    graph_manifest["version"] = serde_json::Value::String(manifest_version.to_string());
    graph_manifest["channel"] = serde_json::Value::String(channel.to_string());
    graph_manifest["status"] = serde_json::Value::String("current".to_string());
    let channel_dir = out_dir.join("assets").join(channel);
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    if channel_dir.exists() {
        fs::remove_dir_all(&channel_dir).with_context(|| format!("remove {}", channel_dir.display()))?;
    }
    fs::create_dir_all(&channel_dir).with_context(|| format!("create {}", channel_dir.display()))?;
    let graph_manifest = format!(
        "{}\n",
        serde_json::to_string_pretty(&graph_manifest).context("serialize graph manifest")?
    );
    let channel_manifest = channel_dir.join("manifest.json");
    fs::write(&channel_manifest, &graph_manifest).with_context(|| format!("write {}", channel_manifest.display()))?;
    let graph_manifest_sha256 = format!("{:x}", Sha256::digest(graph_manifest.as_bytes()));
    let graph_manifest_blake3 = blake3::hash(graph_manifest.as_bytes()).to_hex().to_string();
    let graph_value: serde_json::Value =
        serde_json::from_str(&graph_manifest).context("parse rendered graph manifest")?;
    let index = assets_channel_index_from_graph(&graph_value, channel, generated_at, &graph_manifest_blake3)?;
    let graph_manifest_url = format!("/assets/{channel}/manifest.json");
    fs::write(
        out_dir.join("channels.json"),
        render_assets_channels_catalog(
            &out_dir.join("channels.json"),
            &index,
            manifest_version,
            &graph_manifest_url,
            &graph_manifest_sha256,
            &graph_manifest_blake3,
        )?,
    )
    .with_context(|| format!("write {}", out_dir.join("channels.json").display()))?;
    let health_json = render_assets_channel_health(&index)?;
    fs::write(out_dir.join("health.json"), &health_json)
        .with_context(|| format!("write {}", out_dir.join("health.json").display()))?;
    fs::write(channel_dir.join("health.json"), &health_json)
        .with_context(|| format!("write {}", channel_dir.join("health.json").display()))?;
    fs::write(
        out_dir.join("_headers"),
        render_assets_channel_headers_for_dist(out_dir, channel)?,
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
        channels_json: out_dir.join("channels.json").display().to_string(),
        manifest: channel_manifest.display().to_string(),
        health_json: out_dir.join("health.json").display().to_string(),
        copied_assets: 0,
    })
}

pub(super) fn record_graph_binary_release_metadata(
    manifest_path: &Path,
    mut manifest: serde_json::Value,
    version: &str,
    source_commit: &SourceCommit,
    min_assets: Option<&str>,
    files: &[BinaryFile],
) -> Result<AssetsChannelRecordBinaryReport> {
    let profiles = manifest
        .get("profiles")
        .and_then(|value| value.as_object())
        .ok_or_else(|| anyhow!("graph manifest profiles must be an object"))?;
    if profiles.is_empty() {
        return Err(anyhow!("graph manifest profiles must not be empty"));
    }
    let min_assets = min_assets
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| graph_profile_revision_summary(profiles));
    let packages = graph_packages_from_binary_files(version, source_commit, files)?;
    manifest["packages"] = serde_json::Value::Array(packages);
    validate_graph_profiles_match_current_binary(&manifest)?;
    let mut bytes = serde_json::to_vec_pretty(&manifest).context("serialize updated manifest")?;
    bytes.push(b'\n');
    fs::write(manifest_path, &bytes).with_context(|| format!("write {}", manifest_path.display()))?;
    Ok(AssetsChannelRecordBinaryReport {
        schema: "capsem.admin.assets_channel_record_binary.v1",
        manifest: manifest_path.display().to_string(),
        version: version.to_string(),
        min_assets,
        files: files.to_vec(),
    })
}

pub(super) fn validate_binary_release_files(version: &str, files: &[BinaryFile]) -> Result<()> {
    if !files.iter().any(|file| is_host_sbom_file(&file.name)) {
        return Err(anyhow!("binary release metadata must include capsem-sbom.spdx.json"));
    }
    if !files.iter().any(|file| !is_host_sbom_file(&file.name)) {
        return Err(anyhow!("binary release metadata must include a host package artifact"));
    }
    if !files.iter().any(|file| is_host_package_file(&file.name)) {
        return Err(anyhow!("binary release metadata must include a .pkg or .deb artifact"));
    }
    if let Some(file) = files
        .iter()
        .find(|file| is_host_package_file(&file.name) && !host_package_name_matches_version(&file.name, version))
    {
        return Err(anyhow!(
            "binary release package artifact name must match version {version}: {}",
            file.name
        ));
    }
    Ok(())
}

pub(super) fn graph_packages_from_binary_files(
    version: &str,
    source_commit: &SourceCommit,
    files: &[BinaryFile],
) -> Result<Vec<serde_json::Value>> {
    let host_sbom = files
        .iter()
        .find(|file| is_host_sbom_file(&file.name))
        .ok_or_else(|| anyhow!("binary release metadata must include capsem-sbom.spdx.json"))?;
    let mut packages = files
        .iter()
        .filter(|file| is_host_package_file(&file.name))
        .map(|file| graph_package_from_binary_file(version, source_commit, file, host_sbom))
        .collect::<Result<Vec<_>>>()?;
    packages.sort_by(|left, right| {
        let left_name = left.get("name").and_then(|value| value.as_str()).unwrap_or("");
        let right_name = right.get("name").and_then(|value| value.as_str()).unwrap_or("");
        left_name.cmp(right_name)
    });
    Ok(packages)
}

pub(super) fn graph_package_from_binary_file(
    version: &str,
    source_commit: &SourceCommit,
    file: &BinaryFile,
    host_sbom: &BinaryFile,
) -> Result<serde_json::Value> {
    let package_kind = package_kind_for_name(&file.name);
    let platform = package_platform_for_kind(package_kind);
    let architecture = release_graph::PackageArchitecture::from_package_name(&file.name)?;
    let package_id = release_graph_id(&file.name);
    let package_url = capsem_assets::asset_manager::release_url(version);
    let package_url = format!("{}/{}", package_url.trim_end_matches('/'), file.name);
    let host_sbom_url = capsem_assets::asset_manager::release_url(version);
    let host_sbom_url = format!("{}/{}", host_sbom_url.trim_end_matches('/'), host_sbom.name);
    let binaries = file
        .binaries
        .iter()
        .map(|binary| {
            serde_json::json!({
                "name": binary.name,
                "description": binary.description,
                "version": version,
                "installed_path": binary.installed_path,
                "platform": platform,
                "architecture": architecture,
                "bytes": binary.size,
                "digest": {
                    "sha256": binary.sha256,
                    "blake3": binary.blake3,
                },
                "status": "current",
                "sbom_component_ref": binary.sbom_component_ref,
            })
        })
        .collect::<Vec<_>>();
    if binaries.is_empty() {
        return Err(anyhow!(
            "binary release package artifact must contain executable inventory: {}",
            file.name
        ));
    }
    Ok(serde_json::json!({
        "id": package_id,
        "kind": package_kind,
        "name": file.name,
        "version": version,
        "source_commit": source_commit,
        "platform": platform,
        "architecture": architecture,
        "url": package_url,
        "bytes": file.size,
        "digest": {
            "sha256": file.sha256,
            "blake3": file.blake3,
        },
        "binaries": binaries,
        "evidence": [
            {
                "kind": "sbom",
                "name": host_sbom.name,
                "url": host_sbom_url,
                "bytes": host_sbom.size,
                "digest": {
                    "sha256": host_sbom.sha256,
                    "blake3": host_sbom.blake3,
                },
                "status": "current",
            }
        ],
        "status": "current",
    }))
}

pub(super) fn graph_profile_revision_summary(profiles: &serde_json::Map<String, serde_json::Value>) -> String {
    let revisions = profiles
        .values()
        .filter_map(|profile| profile.get("revision").and_then(|value| value.as_str()))
        .collect::<BTreeSet<_>>();
    if revisions.len() == 1 {
        revisions.into_iter().next().unwrap_or("unknown").to_string()
    } else {
        "mixed".to_string()
    }
}

pub(super) fn validate_binary_version(version: &str) -> Result<()> {
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

pub(super) fn validate_release_date(date: &str) -> Result<()> {
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

pub(super) fn copy_assets_channel_release_assets(
    assets_dir: &Path,
    release_dir: &Path,
    release: &mut capsem_assets::asset_manager::AssetRelease,
    cache: &mut AssetDigestCache,
) -> Result<usize> {
    let mut copied = 0;
    for (arch, assets) in &mut release.arches {
        for (logical_name, entry) in assets {
            let dst = release_dir.join(format!("{arch}-{logical_name}"));
            let src = assets_dir.join(arch).join(logical_name);
            let (bytes, digest) = copy_file_with_digest(&src, &dst)?;
            validate_asset_digest(arch, logical_name, entry, bytes, &digest)?;
            if entry.sha256.is_empty() {
                entry.sha256 = digest["sha256"].as_str().unwrap_or_default().to_string();
            }
            cache.insert((arch.clone(), logical_name.clone()), (bytes, digest));
            copied += 1;
        }
    }
    Ok(copied)
}

pub(super) fn hydrate_current_asset_entry_sha256(
    manifest: &mut ManifestV2,
    assets_dir: &Path,
    cache: &mut AssetDigestCache,
) -> Result<()> {
    let asset_version = manifest.assets.current.clone();
    let release = manifest
        .assets
        .releases
        .get_mut(&asset_version)
        .ok_or_else(|| anyhow!("manifest current asset release is missing"))?;
    for (arch, assets) in &mut release.arches {
        for (logical_name, entry) in assets {
            if !entry.sha256.is_empty() {
                continue;
            }
            let source = assets_dir.join(arch).join(logical_name);
            let (bytes, digest) = file_digest(&source).with_context(|| {
                format!(
                    "hydrate current asset {asset_version} {arch}/{logical_name} from {}",
                    source.display()
                )
            })?;
            validate_asset_digest(arch, logical_name, entry, bytes, &digest)?;
            entry.sha256 = digest["sha256"].as_str().unwrap_or_default().to_string();
            cache.insert((arch.clone(), logical_name.clone()), (bytes, digest));
        }
    }
    Ok(())
}

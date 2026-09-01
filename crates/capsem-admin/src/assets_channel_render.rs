use super::*;

pub(super) fn assets_channel_index(
    manifest: &ManifestV2,
    channel: &str,
    generated_at: &str,
    manifest_blake3: &str,
    profiles: AssetsChannelProfilesSummary,
    asset_base: &str,
) -> AssetsChannelIndex {
    let mut arches = BTreeSet::new();
    for release in manifest.assets.releases.values() {
        arches.extend(release.arches.keys().cloned());
    }
    let current_release = manifest.assets.releases.get(&manifest.assets.current);
    let binary_release = manifest.binaries.releases.get(&manifest.binaries.current);
    let current_asset_files = current_release
        .map(|release| current_asset_file_refs(asset_base, &manifest.assets.current, release))
        .unwrap_or_default();
    let vm_oboms = current_asset_files
        .iter()
        .filter(|file| file.logical_name == "obom.cdx.json")
        .cloned()
        .collect();
    let binary_files = binary_release
        .map(|release| binary_package_file_refs(&manifest.binaries.current, release))
        .unwrap_or_default();
    let host_sboms = binary_files
        .iter()
        .filter(|file| is_host_sbom_file(&file.name))
        .cloned()
        .collect();
    let mut attestations = binary_package_attestations(&binary_files);
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
        binary_version: manifest.binaries.current.clone(),
        asset_version: manifest.assets.current.clone(),
        asset_state: current_release.map(release_state).unwrap_or("missing").to_string(),
        asset_min_binary: current_release.map(|release| release.min_binary.clone()),
        binary_state: binary_release.map(release_state).unwrap_or("missing").to_string(),
        asset_releases: manifest.assets.releases.len(),
        asset_release_history: summarize_asset_releases(manifest),
        binary_releases: manifest.binaries.releases.len(),
        arches: arches.into_iter().collect(),
        current_asset_files,
        binary_files,
        host_sboms,
        attestations,
        vm_oboms,
        profiles,
        image_update_state: "not_published".to_string(),
    }
}

pub(super) fn assets_channel_index_from_graph(
    manifest: &serde_json::Value,
    channel: &str,
    generated_at: &str,
    manifest_blake3: &str,
) -> Result<AssetsChannelIndex> {
    let packages = manifest
        .get("packages")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("graph manifest packages must be an array"))?;
    let profiles = manifest
        .get("profiles")
        .and_then(|value| value.as_object())
        .ok_or_else(|| anyhow!("graph manifest profiles must be an object"))?;
    let binary_version = graph_binary_version(packages);
    let profiles_summary = graph_profiles_summary(profiles)?;
    let current_asset_files = graph_asset_files(profiles)?;
    let vm_oboms = current_asset_files
        .iter()
        .filter(|file| is_vm_obom_asset_file(file))
        .cloned()
        .collect::<Vec<_>>();
    let binary_files = graph_binary_files(packages)?;
    let host_sboms = binary_files
        .iter()
        .filter(|file| is_host_sbom_file(&file.name))
        .cloned()
        .collect();
    let mut attestations = binary_package_attestations(&binary_files);
    attestations.extend(current_asset_attestations(&current_asset_files));
    let arches = current_asset_files
        .iter()
        .map(|file| file.arch.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Ok(AssetsChannelIndex {
        schema_version: 1,
        channel: channel.to_string(),
        state: "published".to_string(),
        generated_at: generated_at.to_string(),
        release_site: "https://release.capsem.org/".to_string(),
        summary: "Capsem asset channel generated from release graph manifest.".to_string(),
        manifest: format!("/assets/{channel}/manifest.json"),
        asset_base: "/profiles/releases".to_string(),
        manifest_blake3: manifest_blake3.to_string(),
        binary_version,
        asset_version: profiles_summary.revision.clone(),
        asset_state: "current".to_string(),
        asset_min_binary: Some(profiles_summary.min_binary.clone()),
        binary_state: if packages.is_empty() { "missing" } else { "current" }.to_string(),
        asset_releases: 1,
        asset_release_history: vec![AssetsChannelAssetRelease {
            version: profiles_summary.revision.clone(),
            date: generated_at.get(..10).unwrap_or(generated_at).to_string(),
            state: "current".to_string(),
            deprecated: false,
            deprecated_date: None,
            min_binary: profiles_summary.min_binary.clone(),
            arches,
        }],
        binary_releases: if packages.is_empty() { 0 } else { 1 },
        arches: current_asset_files
            .iter()
            .map(|file| file.arch.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        current_asset_files,
        binary_files,
        host_sboms,
        attestations,
        vm_oboms,
        profiles: profiles_summary,
        image_update_state: "not_published".to_string(),
    })
}

pub(super) fn graph_binary_version(packages: &[serde_json::Value]) -> String {
    packages
        .iter()
        .filter_map(|package| package.get("version").and_then(|value| value.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .next()
        .unwrap_or("not_published")
        .to_string()
}

pub(super) fn graph_profiles_summary(
    profiles: &serde_json::Map<String, serde_json::Value>,
) -> Result<AssetsChannelProfilesSummary> {
    let profile_ids = profiles.keys().cloned().collect::<Vec<_>>();
    let revision = graph_profile_revision_summary(profiles);
    let min_binary = profiles
        .values()
        .filter_map(|profile| profile.get("min_capsem_version").and_then(|value| value.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .next()
        .unwrap_or("")
        .to_string();
    Ok(AssetsChannelProfilesSummary {
        revision,
        profile_count: profiles.len(),
        profile_ids,
        refresh_policy: "graph".to_string(),
        min_binary,
        requires_newer_binary: false,
    })
}

pub(super) fn graph_asset_files(
    profiles: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<AssetsChannelAssetFile>> {
    let mut files = Vec::new();
    for profile in profiles.values() {
        let architectures = profile
            .get("architectures")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("graph profile architectures must be an array"))?;
        for arch_doc in architectures {
            let arch = require_json_string(arch_doc, &["architecture"])?;
            for field in ["images", "evidence"] {
                for item in arch_doc
                    .get(field)
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                {
                    let url = require_json_string(item, &["url"])?;
                    let digest = require_json_string(item, &["digest", "blake3"])?;
                    let size = item
                        .get("bytes")
                        .and_then(|value| value.as_u64())
                        .ok_or_else(|| anyhow!("graph asset file bytes missing"))?;
                    let logical_name = item
                        .get("name")
                        .and_then(|value| value.as_str())
                        .or_else(|| item.get("kind").and_then(|value| value.as_str()))
                        .unwrap_or("asset")
                        .to_string();
                    files.push(AssetsChannelAssetFile {
                        arch: arch.clone(),
                        logical_name,
                        url,
                        hash: digest,
                        size,
                    });
                }
            }
        }
    }
    files.sort_by(|left, right| left.url.cmp(&right.url));
    files.dedup_by(|left, right| left.url == right.url);
    Ok(files)
}

pub(super) fn graph_binary_files(packages: &[serde_json::Value]) -> Result<Vec<AssetsChannelBinaryFile>> {
    let mut files = Vec::new();
    for package in packages {
        files.push(graph_binary_file(package)?);
        for evidence in package
            .get("evidence")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
        {
            files.push(graph_binary_file(evidence)?);
        }
    }
    files.sort_by(|left, right| left.url.cmp(&right.url));
    files.dedup_by(|left, right| left.url == right.url);
    Ok(files)
}

pub(super) fn graph_binary_file(value: &serde_json::Value) -> Result<AssetsChannelBinaryFile> {
    let name = require_json_string(value, &["name"])?;
    let url = require_json_string(value, &["url"])?;
    let sha256 = require_json_string(value, &["digest", "sha256"])?;
    let blake3 = require_json_string(value, &["digest", "blake3"])?;
    let size = value
        .get("bytes")
        .and_then(|item| item.as_u64())
        .ok_or_else(|| anyhow!("graph binary file bytes missing"))?;
    let binaries = value
        .get("binaries")
        .and_then(|item| item.as_array())
        .map(|items| items.iter().map(graph_binary_executable).collect::<Result<Vec<_>>>())
        .transpose()?
        .unwrap_or_default();
    Ok(AssetsChannelBinaryFile {
        name,
        url,
        sha256,
        blake3,
        size,
        binaries,
    })
}

pub(super) fn graph_binary_executable(value: &serde_json::Value) -> Result<BinaryExecutable> {
    Ok(BinaryExecutable {
        name: require_json_string(value, &["name"])?,
        description: value
            .get("description")
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .to_string(),
        installed_path: require_json_string(value, &["installed_path"])?,
        size: value
            .get("bytes")
            .and_then(|item| item.as_u64())
            .ok_or_else(|| anyhow!("graph binary bytes missing"))?,
        sha256: require_json_string(value, &["digest", "sha256"])?,
        blake3: require_json_string(value, &["digest", "blake3"])?,
        sbom_component_ref: require_json_string(value, &["sbom_component_ref"])?,
    })
}

pub(super) fn summarize_asset_releases(manifest: &ManifestV2) -> Vec<AssetsChannelAssetRelease> {
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

pub(super) fn publishable_profiles(
    manifest: &ManifestV2,
    profiles_dir: &Path,
    channel: &str,
    asset_base: &str,
    assets_dir: &Path,
    asset_digest_cache: &mut AssetDigestCache,
    profile_revision_policy: ProfileRevisionPolicyArg,
) -> Result<PublishableProfiles> {
    let current_release = manifest
        .assets
        .releases
        .get(&manifest.assets.current)
        .ok_or_else(|| anyhow!("manifest current asset release is missing"))?;
    let catalog = ProfileCatalog::load_from_dir(profiles_dir)
        .map_err(|error| anyhow!("load profile directory {}: {error}", profiles_dir.display()))?;
    let config_root = profiles_dir
        .parent()
        .ok_or_else(|| anyhow!("profile directory {} has no config root", profiles_dir.display()))?;
    let mut profiles = catalog
        .profiles()
        .cloned()
        .map(|profile| publishable_profile_config(profile, config_root, manifest, current_release, asset_base))
        .collect::<Result<Vec<_>>>()?;
    profiles.sort_by(|left, right| left.id.cmp(&right.id));
    let profile_ids = profiles.iter().map(|profile| profile.id.clone()).collect::<Vec<_>>();
    let revision = profile_release_revision(&profiles, profile_revision_policy)?;
    validate_profile_revision_path(&revision)?;
    let refresh_policy = profile_refresh_policy(&profiles);
    let min_binary = current_release.min_binary.clone();
    let mut file_copies = Vec::new();
    let mut graph_profiles = Vec::new();
    let graph_context = ProfileGraphContext {
        channel,
        manifest,
        current_release,
        asset_base,
        assets_dir,
    };
    for profile in &profiles {
        graph_profiles.push(graph_profile_document(
            profile,
            config_root,
            &graph_context,
            &mut file_copies,
            asset_digest_cache,
        )?);
    }
    Ok(PublishableProfiles {
        summary: AssetsChannelProfilesSummary {
            revision,
            profile_count: graph_profiles.len(),
            profile_ids,
            refresh_policy,
            min_binary,
            requires_newer_binary: false,
        },
        profiles: graph_profiles,
        file_copies,
    })
}

pub(super) fn validate_graph_manifest_version(version: &str) -> Result<()> {
    if version.trim().is_empty() {
        return Err(anyhow!("manifest version must not be empty"));
    }
    if version.contains("+assets.") {
        return Err(anyhow!(
            "manifest version must be independent from asset and binary versions"
        ));
    }
    Ok(())
}

pub(super) fn render_graph_release_manifest(
    manifest: &ManifestV2,
    channel: &str,
    profiles: &[serde_json::Value],
    _asset_base: &str,
    version: &str,
) -> Result<String> {
    let packages = graph_package_rows(manifest)?;
    let profile_map = profiles
        .iter()
        .map(|profile| {
            let id = profile
                .get("id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("graph profile missing id"))?;
            Ok((id.to_string(), profile.clone()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&serde_json::json!({
            "version": version,
            "channel": channel,
            "status": "current",
            "packages": packages,
            "profiles": profile_map,
        }))
        .context("serialize graph release manifest")?
    ))
}

pub(super) struct ProfileGraphContext<'a> {
    channel: &'a str,
    manifest: &'a ManifestV2,
    current_release: &'a capsem_assets::asset_manager::AssetRelease,
    asset_base: &'a str,
    assets_dir: &'a Path,
}

pub(super) fn graph_package_rows(manifest: &ManifestV2) -> Result<Vec<serde_json::Value>> {
    let Some(release) = manifest.binaries.releases.get(&manifest.binaries.current) else {
        return Ok(Vec::new());
    };
    let binary_files = binary_package_file_refs(&manifest.binaries.current, release);
    let rows = binary_files
        .iter()
        .filter(|file| !is_host_sbom_file(&file.name) && !is_package_sbom_file(&file.name))
        .map(|file| -> Result<serde_json::Value> {
            let package_kind = package_kind_for_name(&file.name);
            let platform = package_platform_for_kind(package_kind);
            let architecture = release_graph::PackageArchitecture::from_package_name(&file.name)?;
            let package_id = release_graph_id(&file.name);
            let package_sboms = package_sbom_refs(&package_id, &binary_files, release);
            let binaries = file
                .binaries
                .iter()
                .map(|binary| {
                    serde_json::json!({
                        "name": binary.name,
                        "description": binary.description,
                        "version": manifest.binaries.current,
                        "installed_path": binary.installed_path,
                        "platform": platform,
                        "architecture": architecture,
                        "bytes": binary.size,
                        "digest": {
                            "sha256": binary.sha256,
                            "blake3": binary.blake3,
                        },
                        "status": release_state(release),
                        "sbom_component_ref": binary.sbom_component_ref,
                    })
                })
                .collect::<Vec<_>>();
            Ok(serde_json::json!({
                "id": package_id,
                "kind": package_kind,
                "name": file.name,
                "version": manifest.binaries.current,
                "platform": platform,
                "architecture": architecture,
                "url": file.url,
                "bytes": file.size,
                "digest": {
                    "sha256": file.sha256,
                    "blake3": file.blake3,
                },
                "binaries": binaries,
                "evidence": package_sboms,
                "status": release_state(release),
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(rows)
}

pub(super) fn package_sbom_refs(
    package_id: &str,
    binary_files: &[AssetsChannelBinaryFile],
    release: &capsem_assets::asset_manager::BinaryRelease,
) -> Vec<serde_json::Value> {
    let expected = package_sbom_file_name(package_id);
    binary_files
        .iter()
        .filter(|file| file.name == expected)
        .map(|file| {
            serde_json::json!({
                "kind": "sbom",
                "name": file.name,
                "url": file.url,
                "bytes": file.size,
                "digest": {
                    "sha256": file.sha256,
                    "blake3": file.blake3,
                },
                "status": release_state(release),
            })
        })
        .collect()
}

pub(super) fn graph_profile_document(
    profile: &ProfileConfigFile,
    config_root: &Path,
    context: &ProfileGraphContext<'_>,
    file_copies: &mut Vec<ProfileReleaseFileCopy>,
    asset_digest_cache: &mut AssetDigestCache,
) -> Result<serde_json::Value> {
    let revision = profile.revision.clone();
    let images = graph_profile_images(profile, &revision, context, file_copies, asset_digest_cache)?;
    let software = graph_profile_software(profile, &revision, context, asset_digest_cache)?;
    let image_records = images
        .as_array()
        .ok_or_else(|| anyhow!("profile {} image graph is not an array", profile.id))?;
    let mut architectures = Vec::new();
    for image in image_records {
        let arch = image
            .get("architecture")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("profile {} image record missing architecture", profile.id))?;
        let config = graph_profile_config_refs(profile, config_root, context.channel, &revision, arch, file_copies)?;
        let arch_software = software.get(arch).cloned().unwrap_or_default();
        let image_artifacts = image
            .get("artifacts")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let evidence = image
            .get("evidence")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        architectures.push(serde_json::json!({
            "architecture": arch,
            "package_inventory_revision": context.manifest.assets.current,
            "image_revision": context.manifest.assets.current,
            "software": arch_software,
            "config": config,
            "images": image_artifacts,
            "evidence": evidence,
        }));
    }
    Ok(serde_json::json!({
        "id": profile.id,
        "name": profile.name,
        "description": profile.description,
        "version": profile.revision,
        "revision": profile.revision,
        "status": "current",
        "min_capsem_version": context.current_release.min_binary,
        "architectures": architectures,
    }))
}

pub(super) fn graph_profile_config_refs(
    profile: &ProfileConfigFile,
    config_root: &Path,
    channel: &str,
    revision: &str,
    arch: &str,
    file_copies: &mut Vec<ProfileReleaseFileCopy>,
) -> Result<Vec<serde_json::Value>> {
    let mut files = Vec::new();
    let profile_toml = format!("profiles/{}/profile.toml", profile.id);
    files.push(("profile".to_string(), profile_toml, None));
    for (kind, descriptor) in profile_file_descriptors(profile) {
        files.push((kind.to_string(), descriptor.path.clone(), None));
    }
    if let Some(root_manifest_descriptor) = profile.files.root_manifest.as_ref() {
        let manifest_path = config_root.join(&root_manifest_descriptor.path);
        check_profile_root_manifest(&manifest_path)?;
        let manifest: ProfileRootManifest = serde_json::from_slice(
            &fs::read(&manifest_path)
                .with_context(|| format!("read profile root manifest {}", manifest_path.display()))?,
        )
        .with_context(|| format!("parse profile root manifest {}", manifest_path.display()))?;
        let manifest_parent = Path::new(&root_manifest_descriptor.path)
            .parent()
            .ok_or_else(|| anyhow!("profile {} root manifest has no parent path", profile.id))?;
        for entry in manifest.files {
            let relative_path = manifest_parent.join("root").join(&entry.path);
            let relative = relative_path
                .to_str()
                .ok_or_else(|| {
                    anyhow!(
                        "profile {} root payload path is not UTF-8: {}",
                        profile.id,
                        relative_path.display()
                    )
                })?
                .replace(std::path::MAIN_SEPARATOR, "/");
            validate_relative_manifest_path("profile root publication path", &relative)?;
            files.push(("root_payload".to_string(), relative, Some("root-payload".to_string())));
        }
    }
    files.sort_by(|left, right| left.1.cmp(&right.1));
    files.dedup_by(|left, right| left.1 == right.1);

    let mut rows = Vec::new();
    let mut urls = BTreeMap::new();
    let mut digest_urls = BTreeMap::new();
    for (kind, relative, publication_name) in files {
        let source = config_root.join(&relative);
        let (bytes, digest) = file_digest(&source)?;
        let file_name = match publication_name {
            Some(prefix) => format!(
                "{prefix}-{}",
                digest
                    .get("blake3")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| { anyhow!("profile {} config path lacks BLAKE3 digest: {relative}", profile.id) })?
            ),
            None => Path::new(&relative)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| anyhow!("profile {} config path has no file name: {relative}", profile.id))?
                .to_string(),
        };
        let identity = (
            bytes,
            digest
                .get("sha256")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            digest
                .get("blake3")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        );
        let proposed_url = profile_release_url(channel, &profile.id, revision, arch, &file_name)?;
        let url = digest_urls.entry(identity.clone()).or_insert(proposed_url).clone();
        if let Some(previous) = urls.insert(url.clone(), identity.clone()) {
            if previous != identity {
                return Err(anyhow!(
                    "profile {}/{} config publication URL collides: {}",
                    profile.id,
                    arch,
                    url
                ));
            }
        }
        file_copies.push(ProfileReleaseFileCopy {
            source,
            url: url.clone(),
        });
        rows.push(serde_json::json!({
            "kind": kind,
            "path": relative,
            "url": url,
            "bytes": bytes,
            "digest": digest,
            "status": "current",
        }));
    }
    Ok(rows)
}

pub(super) fn profile_release_url(
    channel: &str,
    profile: &str,
    revision: &str,
    architecture: &str,
    file_name: &str,
) -> Result<String> {
    profile_publication_identity(channel, profile, revision)?;
    for (label, value) in [
        ("profile architecture", architecture),
        ("profile publication file", file_name),
    ] {
        let valid = !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid {
            return Err(anyhow!("{label} cannot form an immutable profile path: {value}"));
        }
    }
    Ok(format!(
        "/profiles/releases/{channel}/{profile}/{revision}/{architecture}/{file_name}"
    ))
}

pub(super) fn graph_profile_images(
    profile: &ProfileConfigFile,
    revision: &str,
    context: &ProfileGraphContext<'_>,
    file_copies: &mut Vec<ProfileReleaseFileCopy>,
    asset_digest_cache: &mut AssetDigestCache,
) -> Result<serde_json::Value> {
    let mut images = Vec::new();
    for (arch, arch_assets) in &profile.assets.arch {
        let manifest_assets = context.current_release.arches.get(arch).ok_or_else(|| {
            anyhow!(
                "manifest current release {} does not contain profile arch {arch}",
                context.manifest.assets.current
            )
        })?;
        let artifacts = [
            ("kernel", &arch_assets.kernel),
            ("initrd", &arch_assets.initrd),
            ("rootfs", &arch_assets.rootfs),
        ]
        .into_iter()
        .map(|(kind, descriptor)| {
            let entry = manifest_assets
                .get(&descriptor.name)
                .ok_or_else(|| anyhow!("manifest current release arch {arch} is missing {}", descriptor.name))?;
            let (bytes, digest) =
                asset_entry_digest(context.assets_dir, arch, &descriptor.name, entry, asset_digest_cache)?;
            let url = if context.asset_base == "/assets/releases" {
                let url = profile_release_url(context.channel, &profile.id, revision, arch, &descriptor.name)?;
                file_copies.push(ProfileReleaseFileCopy {
                    source: context.assets_dir.join(arch).join(&descriptor.name),
                    url: url.clone(),
                });
                url
            } else {
                channel_asset_url(
                    context.asset_base,
                    &context.manifest.assets.current,
                    arch,
                    &descriptor.name,
                )
            };
            Ok(serde_json::json!({
                "kind": kind,
                "name": descriptor.name,
                "url": url,
                "bytes": bytes,
                "digest": digest,
                "status": "current",
            }))
        })
        .collect::<Result<Vec<_>>>()?;

        let mut evidence = Vec::new();
        for (kind, logical_name) in [
            ("abom", "abom.cdx.json"),
            ("obom", "obom.cdx.json"),
            ("software_inventory", "software-inventory.json"),
        ] {
            if let Some(entry) = manifest_assets.get(logical_name) {
                let (bytes, digest) =
                    asset_entry_digest(context.assets_dir, arch, logical_name, entry, asset_digest_cache)?;
                let url = if context.asset_base == "/assets/releases" {
                    let url = profile_release_url(context.channel, &profile.id, revision, arch, logical_name)?;
                    file_copies.push(ProfileReleaseFileCopy {
                        source: context.assets_dir.join(arch).join(logical_name),
                        url: url.clone(),
                    });
                    url
                } else {
                    channel_asset_url(context.asset_base, &context.manifest.assets.current, arch, logical_name)
                };
                evidence.push(serde_json::json!({
                    "kind": kind,
                    "url": url,
                    "bytes": bytes,
                    "digest": digest,
                    "status": "current",
                }));
            }
        }
        images.push(serde_json::json!({
            "architecture": arch,
            "artifacts": artifacts,
            "evidence": evidence,
        }));
    }
    images.sort_by(|left, right| {
        left.get("architecture")
            .and_then(|value| value.as_str())
            .cmp(&right.get("architecture").and_then(|value| value.as_str()))
    });
    Ok(serde_json::Value::Array(images))
}

pub(super) type AssetDigestCache = BTreeMap<(String, String), (u64, serde_json::Value)>;

pub(super) fn asset_entry_digest(
    _assets_dir: &Path,
    arch: &str,
    logical_name: &str,
    entry: &capsem_assets::asset_manager::AssetEntry,
    cache: &mut AssetDigestCache,
) -> Result<(u64, serde_json::Value)> {
    let cache_key = (arch.to_string(), logical_name.to_string());
    if let Some((bytes, digest)) = cache.get(&cache_key) {
        return Ok((*bytes, digest.clone()));
    }
    if entry.sha256.is_empty() {
        return Err(anyhow!(
            "asset {arch}/{logical_name} manifest entry does not carry sha256"
        ));
    }
    let result = (
        entry.size,
        serde_json::json!({
            "sha256": entry.sha256.clone(),
            "blake3": entry.hash.clone(),
        }),
    );
    cache.insert(cache_key, result.clone());
    Ok(result)
}

pub(super) fn graph_profile_software(
    profile: &ProfileConfigFile,
    revision: &str,
    context: &ProfileGraphContext<'_>,
    asset_digest_cache: &mut AssetDigestCache,
) -> Result<BTreeMap<String, Vec<serde_json::Value>>> {
    let mut rows: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for arch in profile.assets.arch.keys() {
        let manifest_assets = context.current_release.arches.get(arch).ok_or_else(|| {
            anyhow!(
                "manifest current release {} does not contain profile arch {arch}",
                context.manifest.assets.current
            )
        })?;
        let logical_name = "software-inventory.json";
        let entry = manifest_assets.get(logical_name).ok_or_else(|| {
            anyhow!(
                "manifest current release {} arch {arch} missing software-inventory.json",
                context.manifest.assets.current
            )
        })?;
        asset_entry_digest(context.assets_dir, arch, logical_name, entry, asset_digest_cache)?;
        let inventory_path = context.assets_dir.join(arch).join(logical_name);
        let inventory_bytes =
            fs::read(&inventory_path).with_context(|| format!("read {}", inventory_path.display()))?;
        let inventory: serde_json::Value =
            serde_json::from_slice(&inventory_bytes).with_context(|| format!("parse {}", inventory_path.display()))?;
        if inventory.get("schema").and_then(|value| value.as_str()) != Some("capsem.profile_software_inventory.v1") {
            return Err(anyhow!(
                "{} schema must be capsem.profile_software_inventory.v1",
                inventory_path.display()
            ));
        }
        let packages = inventory
            .get("packages")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("{} missing packages array", inventory_path.display()))?;
        let evidence = if context.asset_base == "/assets/releases" {
            profile_release_url(context.channel, &profile.id, revision, arch, logical_name)?
        } else {
            channel_asset_url(context.asset_base, &context.manifest.assets.current, arch, logical_name)
        };
        for package in packages {
            let name = require_json_string_value(package, "name")
                .with_context(|| format!("{} package missing name", inventory_path.display()))?;
            let version = require_json_string_value(package, "version")
                .with_context(|| format!("{name} missing version in {}", inventory_path.display()))?;
            if version == "unversioned" {
                return Err(anyhow!(
                    "{name} in {} has unversioned version",
                    inventory_path.display()
                ));
            }
            let source = require_json_string_value(package, "source")
                .with_context(|| format!("{name} missing source in {}", inventory_path.display()))?;
            let row_core = serde_json::json!({
                "name": name,
                "version": version,
                "source": source,
                "architecture": arch,
                "evidence": evidence,
            });
            let digest = json_digest(&row_core)?;
            rows.entry(arch.clone()).or_default().push(serde_json::json!({
                "name": name,
                "version": version,
                "source": source,
                "architecture": arch,
                "digest": digest,
                "evidence": evidence,
            }));
        }
    }
    for arch_rows in rows.values_mut() {
        arch_rows.sort_by(|left, right| {
            left.get("name")
                .and_then(|value| value.as_str())
                .cmp(&right.get("name").and_then(|value| value.as_str()))
        });
    }
    Ok(rows)
}

pub(super) fn require_json_string_value<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(|child| child.as_str())
        .filter(|child| !child.is_empty())
        .ok_or_else(|| anyhow!("missing string field {key}"))
}

pub(super) fn json_digest(value: &serde_json::Value) -> Result<serde_json::Value> {
    let bytes = serde_json::to_vec(value).context("serialize json digest payload")?;
    Ok(serde_json::json!({
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
        "blake3": blake3::hash(&bytes).to_hex().to_string(),
    }))
}

pub(super) fn profile_file_descriptors(
    profile: &ProfileConfigFile,
) -> Vec<(&'static str, &capsem_core::net::policy_config::ProfileFileDescriptor)> {
    let mut descriptors = Vec::new();
    if let Some(value) = profile.files.enforcement.as_ref() {
        descriptors.push(("enforcement", value));
    }
    if let Some(value) = profile.files.detection.as_ref() {
        descriptors.push(("detection", value));
    }
    if let Some(value) = profile.files.mcp.as_ref() {
        descriptors.push(("mcp", value));
    }
    if let Some(value) = profile.files.apt_packages.as_ref() {
        descriptors.push(("apt_packages", value));
    }
    if let Some(value) = profile.files.python_requirements.as_ref() {
        descriptors.push(("python_requirements", value));
    }
    if let Some(value) = profile.files.python_requirements_lock.as_ref() {
        descriptors.push(("python_requirements_lock", value));
    }
    if let Some(value) = profile.files.npm_packages.as_ref() {
        descriptors.push(("npm_packages", value));
    }
    if let Some(value) = profile.files.npm_package_lock.as_ref() {
        descriptors.push(("npm_package_lock", value));
    }
    if let Some(value) = profile.files.build.as_ref() {
        descriptors.push(("build", value));
    }
    if let Some(value) = profile.files.tips.as_ref() {
        descriptors.push(("tips", value));
    }
    if let Some(value) = profile.files.root_manifest.as_ref() {
        descriptors.push(("root_manifest", value));
    }
    descriptors
}

pub(super) fn copy_profile_release_files(out_dir: &Path, copies: &[ProfileReleaseFileCopy]) -> Result<()> {
    for copy in copies {
        let dst = out_dir.join(copy.url.trim_start_matches('/'));
        fs::create_dir_all(
            dst.parent()
                .ok_or_else(|| anyhow!("profile release file path has no parent"))?,
        )
        .with_context(|| format!("create parent for {}", dst.display()))?;
        hardlink_or_copy(&copy.source, &dst)?;
    }
    Ok(())
}

pub(super) fn file_digest(path: &Path) -> Result<(u64, serde_json::Value)> {
    let mut source = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut sha256 = Sha256::new();
    let mut blake3 = blake3::Hasher::new();
    let mut bytes = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = source
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        sha256.update(&buffer[..read]);
        blake3.update(&buffer[..read]);
    }
    Ok((
        bytes,
        serde_json::json!({
            "sha256": format!("{:x}", sha256.finalize()),
            "blake3": blake3.finalize().to_hex().to_string(),
        }),
    ))
}

pub(super) fn copy_file_with_digest(source: &Path, destination: &Path) -> Result<(u64, serde_json::Value)> {
    hardlink_or_copy(source, destination)?;
    file_digest(destination)
}

/// Stage a file into release output.
///
/// Delegates, because the decision is not "link if you can". Linking a
/// checked-in file into published output makes them one file: this put 48
/// `config/` seeds inside the release channel sharing an inode, where a chmod
/// on the artifact rewrote tracked source and no content digest noticed. See
/// `capsem_core::auditfs`.
pub(super) fn hardlink_or_copy(source: &Path, destination: &Path) -> Result<()> {
    capsem_core::auditfs::stage(source, destination, &repo_root())
}

/// The checkout this admin invocation is staging from.
pub(super) fn repo_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub(super) fn validate_asset_digest(
    arch: &str,
    logical_name: &str,
    entry: &capsem_assets::asset_manager::AssetEntry,
    bytes: u64,
    digest: &serde_json::Value,
) -> Result<()> {
    if bytes != entry.size {
        return Err(anyhow!("asset {arch}/{logical_name} byte count mismatch"));
    }
    let actual_blake3 = digest["blake3"].as_str().unwrap_or_default();
    if actual_blake3 != entry.hash {
        return Err(anyhow!("asset {arch}/{logical_name} blake3 mismatch"));
    }
    if !entry.sha256.is_empty() {
        let actual_sha256 = digest["sha256"].as_str().unwrap_or_default();
        if actual_sha256 != entry.sha256 {
            return Err(anyhow!("asset {arch}/{logical_name} sha256 mismatch"));
        }
    }
    Ok(())
}

pub(super) fn release_graph_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub(super) fn package_kind_for_name(name: &str) -> &'static str {
    if name.ends_with(".pkg") {
        "macos_pkg"
    } else if name.ends_with(".deb") {
        "debian_package"
    } else {
        "package"
    }
}

pub(super) fn package_platform_for_kind(kind: &str) -> &'static str {
    match kind {
        "macos_pkg" => "macos",
        "debian_package" => "linux",
        _ => "unknown",
    }
}

pub(super) fn binary_description_for_name(name: &str) -> &'static str {
    match name {
        "capsem-app" => "Capsem desktop application executable",
        "capsem-tray" => "Capsem tray companion executable",
        "capsem-service" => "Capsem host service executable",
        "capsem-gateway" => "Capsem local gateway executable",
        "capsem-mcp" => "Capsem MCP server executable",
        "capsem-process" => "Capsem guest process bridge executable",
        "capsem" => "Capsem command-line executable",
        _ => "Capsem packaged executable",
    }
}

pub(super) fn publishable_profile_config(
    mut profile: ProfileConfigFile,
    config_root: &Path,
    manifest: &ManifestV2,
    current_release: &capsem_assets::asset_manager::AssetRelease,
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
                        url: profile_release_asset_url(asset_base, &manifest.assets.current, arch, "obom.cdx.json"),
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

pub(super) fn rewrite_publishable_asset_descriptor(
    asset_version: &str,
    arch: &str,
    descriptor: &mut capsem_core::net::policy_config::ProfileAssetDescriptor,
    manifest_assets: &std::collections::HashMap<String, capsem_assets::asset_manager::AssetEntry>,
    asset_base: &str,
) -> Result<()> {
    let entry = manifest_assets
        .get(&descriptor.name)
        .ok_or_else(|| anyhow!("manifest current release arch {arch} is missing {}", descriptor.name))?;
    descriptor.url = profile_release_asset_url(asset_base, asset_version, arch, &descriptor.name);
    descriptor.hash = Some(format!("blake3:{}", entry.hash));
    descriptor.size = Some(entry.size);
    Ok(())
}

pub(super) fn channel_asset_url(asset_base: &str, asset_version: &str, arch: &str, logical_name: &str) -> String {
    if asset_base.starts_with('/') {
        return format!(
            "{}/{asset_version}/{arch}-{logical_name}",
            asset_base.trim_end_matches('/')
        );
    }
    capsem_assets::asset_manager::asset_download_url_with_base(asset_base, asset_version, arch, logical_name)
}

pub(super) fn profile_release_asset_url(
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

pub(super) fn validate_profile_revision_path(revision: &str) -> Result<()> {
    if revision.is_empty()
        || !revision
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(anyhow!("profile revision must be URL-path safe: {revision}"));
    }
    Ok(())
}

pub(super) fn profile_release_revision(
    profiles: &[ProfileConfigFile],
    policy: ProfileRevisionPolicyArg,
) -> Result<String> {
    for profile in profiles {
        validate_profile_revision_path(&profile.revision)
            .with_context(|| format!("profile {} declares an unsafe revision", profile.id))?;
        let strict = release_graph::parse_profile_revision(&profile.revision);
        if strict.is_err()
            && (policy == ProfileRevisionPolicyArg::Strict
                || !release_graph::is_legacy_profile_revision(&profile.revision))
        {
            strict.with_context(|| format!("profile {} declares an unusable revision", profile.id))?;
        }
    }
    let mut revisions = profiles
        .iter()
        .map(|profile| profile.revision.as_str())
        .collect::<BTreeSet<_>>();
    if revisions.len() == 1 {
        let revision = revisions
            .pop_first()
            .ok_or_else(|| anyhow!("profile revision set is empty"))?;
        return Ok(revision.to_string());
    }
    let hash = profile_config_set_hash(profiles)?;
    Ok(format!("profiles-{}", &hash[..16]))
}

pub(super) fn profile_refresh_policy(profiles: &[ProfileConfigFile]) -> String {
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

pub(super) fn profile_config_set_hash(profiles: &[ProfileConfigFile]) -> Result<String> {
    let bytes = serde_json::to_vec(profiles).context("serialize profile set for hashing")?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

pub(super) fn release_state<T: ReleaseDeprecated>(release: &T) -> &'static str {
    if release.is_deprecated() {
        "deprecated"
    } else {
        "current"
    }
}

pub(super) trait ReleaseDeprecated {
    fn is_deprecated(&self) -> bool;
}

impl ReleaseDeprecated for capsem_assets::asset_manager::AssetRelease {
    fn is_deprecated(&self) -> bool {
        self.deprecated
    }
}

impl ReleaseDeprecated for capsem_assets::asset_manager::BinaryRelease {
    fn is_deprecated(&self) -> bool {
        self.deprecated
    }
}

pub(super) fn current_asset_file_refs(
    asset_base: &str,
    asset_version: &str,
    release: &capsem_assets::asset_manager::AssetRelease,
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

pub(super) fn binary_package_file_refs(
    binary_version: &str,
    release: &capsem_assets::asset_manager::BinaryRelease,
) -> Vec<AssetsChannelBinaryFile> {
    let base = capsem_assets::asset_manager::release_url(binary_version);
    let mut files = release
        .files
        .iter()
        .map(|file| AssetsChannelBinaryFile {
            name: file.name.clone(),
            url: format!("{}/{}", base.trim_end_matches('/'), file.name),
            sha256: file.sha256.clone(),
            blake3: file.blake3.clone(),
            size: file.size,
            binaries: file.binaries.clone(),
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.name.cmp(&right.name));
    files
}

pub(super) fn binary_package_attestations(files: &[AssetsChannelBinaryFile]) -> Vec<AssetsChannelAttestation> {
    if files.is_empty() {
        return Vec::new();
    }
    let host_subjects = files
        .iter()
        .filter(|file| !is_host_sbom_file(&file.name) && !is_package_sbom_file(&file.name))
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

pub(super) fn current_asset_attestations(files: &[AssetsChannelAssetFile]) -> Vec<AssetsChannelAttestation> {
    if files.is_empty() {
        return Vec::new();
    }
    let subjects = files.iter().map(|file| file.url.clone()).collect::<Vec<_>>();
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

pub(super) fn is_vm_obom_asset_file(file: &AssetsChannelAssetFile) -> bool {
    file.logical_name == "obom"
        || file.logical_name == "obom.cdx.json"
        || file.url.ends_with("/obom.cdx.json")
        || file.url.ends_with("-obom.cdx.json")
}

pub(super) fn render_assets_channels_catalog(
    existing_catalog_path: &Path,
    index: &AssetsChannelIndex,
    manifest_version: &str,
    manifest_url: &str,
    manifest_sha256: &str,
    manifest_blake3: &str,
) -> Result<String> {
    let mut catalog = if existing_catalog_path.exists() {
        serde_json::from_str::<AssetsChannelsCatalog>(
            &fs::read_to_string(existing_catalog_path)
                .with_context(|| format!("read {}", existing_catalog_path.display()))?,
        )
        .with_context(|| format!("parse {}", existing_catalog_path.display()))?
    } else {
        AssetsChannelsCatalog {
            version: 1,
            generated_at: index.generated_at.clone(),
            release_site: index.release_site.clone(),
            channels: BTreeMap::new(),
        }
    };
    catalog.version = 1;
    catalog.generated_at = index.generated_at.clone();
    catalog.release_site = index.release_site.clone();
    catalog.channels.insert(
        index.channel.clone(),
        AssetsChannelsCatalogChannel {
            label: title_case_channel(&index.channel),
            manifests: vec![AssetsChannelsCatalogManifest {
                version: manifest_version.to_string(),
                status: "current".to_string(),
                url: manifest_url.to_string(),
                digest: AssetsChannelsCatalogDigest {
                    sha256: manifest_sha256.to_string(),
                    blake3: manifest_blake3.to_string(),
                },
            }],
        },
    );
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&catalog).context("serialize channels catalog")?
    ))
}

pub(super) fn render_assets_channel_health(index: &AssetsChannelIndex) -> Result<String> {
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
                "asset_base": index.asset_base,
            },
            "current": {
                "binary": index.binary_version,
                "assets": index.asset_version,
            },
            "binary": {
                "version": index.binary_version,
                "state": index.binary_state,
                "files": index.binary_files,
            },
            "assets": {
                "version": index.asset_version,
                "state": index.asset_state,
                "compatibility": {
                    "binary": index.binary_version,
                    "min_binary": index.asset_min_binary,
                },
                "requires_newer": {
                    "binary": false,
                },
                "files": index.current_asset_files,
            },
            "asset_releases": index.asset_release_history,
                "profiles": {
                    "revision": index.profiles.revision,
                    "state": "current",
                    "source": "manifest.profiles",
                    "profile_count": index.profiles.profile_count,
                    "profile_ids": index.profiles.profile_ids,
                    "refresh_policy": index.profiles.refresh_policy,
                    "min_binary": index.profiles.min_binary,
                    "requires_newer_binary": index.profiles.requires_newer_binary,
                },
            "updates": {
                "binary": {
                    "latest": index.binary_version,
                    "current": index.binary_version,
                    "state": index.binary_state,
                    "source": "manifest.binaries.current",
                    "files": index.binary_files,
                },
                "assets": {
                    "latest": index.asset_version,
                    "current": index.asset_version,
                    "state": index.asset_state,
                    "source": "manifest.assets.current",
                    "manifest": index.manifest,
                    "asset_base": index.asset_base,
                    "compatibility": {
                        "binary": index.binary_version,
                        "min_binary": index.asset_min_binary,
                    },
                    "requires_newer": {
                        "binary": false,
                    },
                },
                "profiles": {
                    "latest": index.profiles.revision,
                    "current": index.profiles.revision,
                    "state": "current",
                    "source": "manifest.profiles",
                    "profile_count": index.profiles.profile_count,
                    "profile_ids": index.profiles.profile_ids,
                    "refresh_policy": index.profiles.refresh_policy,
                    "min_binary": index.profiles.min_binary,
                    "requires_newer_binary": index.profiles.requires_newer_binary,
                },
                "images": {
                    "latest": serde_json::Value::Null,
                    "current": serde_json::Value::Null,
                    "state": index.image_update_state,
                    "source": "manifest.profiles.images",
                },
            },
            "evidence": {
                "vm_oboms": index.vm_oboms,
                "host_sboms": index.host_sboms,
                "host_binary_files": index.binary_files,
                "attestations": index.attestations,
            },
            "manifest": index.manifest,
        }))?
    ))
}

#[cfg(test)]
pub(super) fn render_assets_channel_headers(channel: &str) -> String {
    render_assets_channel_headers_for_channels(&[channel.to_string()])
}

pub(super) fn render_assets_channel_headers_for_dist(out_dir: &Path, fallback_channel: &str) -> Result<String> {
    let channels_path = out_dir.join("channels.json");
    let channels = if channels_path.exists() {
        let catalog: AssetsChannelsCatalog = serde_json::from_str(
            &fs::read_to_string(&channels_path).with_context(|| format!("read {}", channels_path.display()))?,
        )
        .with_context(|| format!("parse {}", channels_path.display()))?;
        catalog.channels.keys().cloned().collect::<Vec<_>>()
    } else {
        vec![fallback_channel.to_string()]
    };
    Ok(render_assets_channel_headers_for_channels(&channels))
}

pub(super) fn render_assets_channel_headers_for_channels(channels: &[String]) -> String {
    let mut lines = vec![
        "/".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
        "/index.html".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
        "/404".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
        "/404.html".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
        "/channels.json".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
        "/health.json".to_string(),
        "  Cache-Control: no-cache, must-revalidate".to_string(),
    ];
    for channel in channels {
        lines.push(format!("/assets/{channel}/*"));
        lines.push("  Cache-Control: no-cache, must-revalidate".to_string());
    }
    lines.extend([
        "/assets/releases/*".to_string(),
        "  Cache-Control: public, max-age=31536000, immutable".to_string(),
        "/profiles/releases/*".to_string(),
        "  Cache-Control: public, max-age=31536000, immutable".to_string(),
        "/robots.txt".to_string(),
        "  Cache-Control: public, max-age=3600".to_string(),
        "".to_string(),
    ]);
    lines.join("\n")
}

pub(super) fn title_case_channel(channel: &str) -> String {
    let mut chars = channel.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

pub(super) fn validate_channel_name(channel: &str) -> Result<()> {
    let valid = !channel.is_empty()
        && channel
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !valid {
        return Err(anyhow!("invalid asset channel name: {channel}"));
    }
    Ok(())
}

pub(super) fn profile_publication_identity(channel: &str, profile: &str, revision: &str) -> Result<String> {
    validate_channel_name(channel)?;
    for (label, value) in [("profile", profile), ("profile revision", revision)] {
        let valid = !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid {
            return Err(anyhow!(
                "{label} cannot form an immutable publication identity: {value}"
            ));
        }
    }
    Ok(format!("profile-{channel}-{profile}-{revision}"))
}

pub(super) fn current_utc_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .replace_microsecond(0)
        .context("truncate current timestamp")?
        .format(&Rfc3339)
        .context("format current timestamp")
}

pub(super) fn current_utc_date() -> Result<String> {
    let timestamp = current_utc_rfc3339()?;
    timestamp
        .get(..10)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("current UTC timestamp was shorter than a date"))
}

pub(super) fn is_host_sbom_file(name: &str) -> bool {
    name == "capsem-sbom.spdx.json"
}

pub(super) fn is_package_sbom_file(name: &str) -> bool {
    name.ends_with("-sbom.spdx.json") && !is_host_sbom_file(name)
}

pub(super) fn package_sbom_file_name(package_id: &str) -> String {
    format!("{package_id}-sbom.spdx.json")
}

pub(super) fn validate_host_spdx_sbom_bytes(bytes: &[u8], path: &Path) -> Result<()> {
    let document: serde_json::Value =
        serde_json::from_slice(bytes).with_context(|| format!("parse host SPDX SBOM {}", path.display()))?;
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
    if let Some(files) = document.get("files") {
        let files = files
            .as_array()
            .ok_or_else(|| anyhow!("{} SPDX files must be an array", path.display()))?;
        for file in files {
            let spdx_id = file
                .get("SPDXID")
                .and_then(|value| value.as_str())
                .unwrap_or("<unknown>");
            let checksums = file
                .get("checksums")
                .and_then(|value| value.as_array())
                .ok_or_else(|| anyhow!("{} SPDX file {spdx_id} missing checksums with SHA256", path.display()))?;
            let has_sha256 = checksums.iter().any(|checksum| {
                checksum
                    .get("algorithm")
                    .and_then(|value| value.as_str())
                    .is_some_and(|algorithm| algorithm.eq_ignore_ascii_case("SHA256"))
                    && checksum
                        .get("checksumValue")
                        .and_then(|value| value.as_str())
                        .is_some_and(|value| value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit()))
            });
            if !has_sha256 {
                return Err(anyhow!(
                    "{} SPDX file {spdx_id} missing SHA256 checksum",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_vm_cyclonedx_obom_bytes(bytes: &[u8], path: &Path) -> Result<()> {
    let document: serde_json::Value =
        serde_json::from_slice(bytes).with_context(|| format!("parse VM CycloneDX OBOM {}", path.display()))?;
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

pub(super) fn is_host_package_file(name: &str) -> bool {
    name.ends_with(".pkg") || name.ends_with(".deb")
}

pub(super) fn host_package_name_matches_version(name: &str, version: &str) -> bool {
    name == format!("Capsem-{version}.pkg")
        || (name.starts_with(&format!("Capsem_{version}_")) && name.ends_with(".deb"))
}

pub(super) fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

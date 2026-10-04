use super::*;

pub(super) fn assets_channel_index(
    manifest: &ManifestV2,
    channel: &str,
    generated_at: &str,
    manifest_blake3: &str,
    runtime: AssetsChannelRuntimeSummary,
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
        runtime: Some(runtime),
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
    let runtime = graph_runtime(manifest);
    let binary_version = graph_binary_version(packages);
    let runtime_summary = runtime.map(graph_runtime_summary).transpose()?;
    let current_asset_files = runtime.map(graph_asset_files).transpose()?.unwrap_or_default();
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
    let asset_release_history = runtime_summary
        .iter()
        .map(|summary| AssetsChannelAssetRelease {
            version: summary.revision.clone(),
            date: generated_at.get(..10).unwrap_or(generated_at).to_string(),
            state: "current".to_string(),
            deprecated: false,
            deprecated_date: None,
            min_binary: summary.min_binary.clone(),
            arches: summary.architectures.clone(),
        })
        .collect::<Vec<_>>();
    Ok(AssetsChannelIndex {
        schema_version: 1,
        channel: channel.to_string(),
        state: "published".to_string(),
        generated_at: generated_at.to_string(),
        release_site: "https://release.capsem.org/".to_string(),
        summary: "Capsem asset channel generated from release graph manifest.".to_string(),
        manifest: format!("/assets/{channel}/manifest.json"),
        asset_base: "/runtime/releases".to_string(),
        manifest_blake3: manifest_blake3.to_string(),
        binary_version,
        asset_version: runtime_summary
            .as_ref()
            .map_or_else(|| "not_published".to_string(), |summary| summary.revision.clone()),
        asset_state: if runtime_summary.is_some() {
            "current"
        } else {
            "missing"
        }
        .to_string(),
        asset_min_binary: runtime_summary.as_ref().map(|summary| summary.min_binary.clone()),
        binary_state: if packages.is_empty() { "missing" } else { "current" }.to_string(),
        asset_releases: asset_release_history.len(),
        asset_release_history,
        binary_releases: if packages.is_empty() { 0 } else { 1 },
        arches: runtime_summary
            .as_ref()
            .map(|summary| summary.architectures.clone())
            .unwrap_or_default(),
        current_asset_files,
        binary_files,
        host_sboms,
        attestations,
        vm_oboms,
        runtime: runtime_summary,
    })
}

/// The graph's runtime document, or `None` before its first runtime release.
pub(super) fn graph_runtime(manifest: &serde_json::Value) -> Option<&serde_json::Value> {
    manifest.get("runtime").filter(|runtime| !runtime.is_null())
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

pub(super) fn graph_runtime_summary(runtime: &serde_json::Value) -> Result<AssetsChannelRuntimeSummary> {
    let architectures = runtime
        .get("architectures")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("graph runtime architectures must be an array"))?
        .iter()
        .map(|architecture| require_json_string(architecture, &["architecture"]))
        .collect::<Result<BTreeSet<_>>>()?;
    Ok(AssetsChannelRuntimeSummary {
        revision: require_json_string(runtime, &["revision"])?,
        min_binary: runtime
            .get("min_capsem_version")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string(),
        architectures: architectures.into_iter().collect(),
    })
}

pub(super) fn graph_asset_files(runtime: &serde_json::Value) -> Result<Vec<AssetsChannelAssetFile>> {
    let mut files = Vec::new();
    let architectures = runtime
        .get("architectures")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("graph runtime architectures must be an array"))?;
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

/// The runtime document for the manifest's current asset release.
///
/// The runtime revision is the asset release itself (`assets.current`), and
/// every architecture that release builds publishes one image set.
pub(super) fn publishable_runtime(
    manifest: &ManifestV2,
    channel: &str,
    asset_base: &str,
    assets_dir: &Path,
    asset_digest_cache: &mut AssetDigestCache,
) -> Result<PublishableRuntime> {
    let revision = manifest.assets.current.as_str();
    release_graph::validate_runtime_revision(revision)?;
    let current_release = manifest
        .assets
        .releases
        .get(revision)
        .ok_or_else(|| anyhow!("manifest current asset release is missing"))?;
    let context = RuntimeGraphContext {
        channel,
        revision,
        asset_base,
        assets_dir,
    };
    let mut file_copies = Vec::new();
    let mut architectures = Vec::new();
    let arch_names = current_release.arches.keys().cloned().collect::<BTreeSet<_>>();
    for arch in &arch_names {
        architectures.push(runtime_architecture_document(
            arch,
            &current_release.arches[arch],
            &context,
            &mut file_copies,
            asset_digest_cache,
        )?);
    }
    if architectures.is_empty() {
        return Err(anyhow!("manifest current release {revision} builds no architecture"));
    }
    let min_binary = current_release.min_binary.clone();
    let mut runtime = serde_json::json!({
        "revision": revision,
        "status": "current",
        "architectures": architectures,
    });
    if !min_binary.is_empty() {
        runtime["min_capsem_version"] = serde_json::Value::String(min_binary.clone());
    }
    Ok(PublishableRuntime {
        summary: AssetsChannelRuntimeSummary {
            revision: revision.to_string(),
            min_binary,
            architectures: arch_names.into_iter().collect(),
        },
        runtime,
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
    runtime: &serde_json::Value,
    version: &str,
) -> Result<String> {
    let packages = graph_package_rows(manifest)?;
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&serde_json::json!({
            "version": version,
            "channel": channel,
            "status": "current",
            "packages": packages,
            "runtime": runtime,
        }))
        .context("serialize graph release manifest")?
    ))
}

pub(super) struct RuntimeGraphContext<'a> {
    channel: &'a str,
    revision: &'a str,
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

/// Where the channel copies VM blobs it publishes itself.
const LOCAL_ASSET_BASE: &str = "/assets/releases";

/// The boot images every runtime architecture carries, by graph kind.
const RUNTIME_IMAGE_FILES: [(&str, &str); 3] = [
    ("kernel", "vmlinuz"),
    ("initrd", "initrd.img"),
    ("rootfs", "rootfs.erofs"),
];

/// The evidence a runtime architecture publishes when its build produced it.
const RUNTIME_EVIDENCE_FILES: [(&str, &str); 3] = [
    ("abom", "abom.cdx.json"),
    ("obom", "obom.cdx.json"),
    ("software_inventory", "software-inventory.json"),
];

pub(super) fn runtime_architecture_document(
    arch: &str,
    manifest_assets: &std::collections::HashMap<String, capsem_assets::asset_manager::AssetEntry>,
    context: &RuntimeGraphContext<'_>,
    file_copies: &mut Vec<RuntimeReleaseFileCopy>,
    asset_digest_cache: &mut AssetDigestCache,
) -> Result<serde_json::Value> {
    let mut images = Vec::new();
    for (kind, logical_name) in RUNTIME_IMAGE_FILES {
        let entry = manifest_assets.get(logical_name).ok_or_else(|| {
            anyhow!(
                "manifest current release {} arch {arch} is missing {logical_name}",
                context.revision
            )
        })?;
        let (bytes, digest) = asset_entry_digest(arch, logical_name, entry, asset_digest_cache)?;
        images.push(serde_json::json!({
            "kind": kind,
            "name": logical_name,
            "url": publish_runtime_file(context, arch, logical_name, file_copies)?,
            "bytes": bytes,
            "digest": digest,
            "status": "current",
        }));
    }
    let mut evidence = Vec::new();
    for (kind, logical_name) in RUNTIME_EVIDENCE_FILES {
        let Some(entry) = manifest_assets.get(logical_name) else {
            continue;
        };
        let (bytes, digest) = asset_entry_digest(arch, logical_name, entry, asset_digest_cache)?;
        evidence.push(serde_json::json!({
            "kind": kind,
            "url": publish_runtime_file(context, arch, logical_name, file_copies)?,
            "bytes": bytes,
            "digest": digest,
            "status": "current",
        }));
    }
    let software = runtime_architecture_software(arch, manifest_assets, context, asset_digest_cache)?;
    Ok(serde_json::json!({
        "architecture": arch,
        "package_inventory_revision": context.revision,
        "image_revision": context.revision,
        "software": software,
        "images": images,
        "evidence": evidence,
    }))
}

/// The URL one runtime file is published at in this channel build.
pub(super) fn runtime_file_url(context: &RuntimeGraphContext<'_>, arch: &str, logical_name: &str) -> Result<String> {
    if context.asset_base == LOCAL_ASSET_BASE {
        return runtime_release_url(context.channel, context.revision, arch, logical_name);
    }
    Ok(channel_asset_url(
        context.asset_base,
        context.revision,
        arch,
        logical_name,
    ))
}

/// The URL of one runtime file, staging its bytes when the channel hosts them.
pub(super) fn publish_runtime_file(
    context: &RuntimeGraphContext<'_>,
    arch: &str,
    logical_name: &str,
    file_copies: &mut Vec<RuntimeReleaseFileCopy>,
) -> Result<String> {
    let url = runtime_file_url(context, arch, logical_name)?;
    if context.asset_base == LOCAL_ASSET_BASE {
        file_copies.push(RuntimeReleaseFileCopy {
            source: source_asset_path(context.assets_dir, arch, logical_name)?,
            url: url.clone(),
        });
    }
    Ok(url)
}

pub(super) fn runtime_release_url(
    channel: &str,
    revision: &str,
    architecture: &str,
    file_name: &str,
) -> Result<String> {
    runtime_publication_identity(channel, revision)?;
    for (label, value) in [
        ("runtime architecture", architecture),
        ("runtime publication file", file_name),
    ] {
        let valid = !value.is_empty()
            && value != ".."
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid {
            return Err(anyhow!("{label} cannot form an immutable runtime path: {value}"));
        }
    }
    Ok(format!(
        "/runtime/releases/{channel}/{revision}/{architecture}/{file_name}"
    ))
}

pub(super) type AssetDigestCache = BTreeMap<(String, String), (u64, serde_json::Value)>;

pub(super) fn asset_entry_digest(
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

pub(super) fn runtime_architecture_software(
    arch: &str,
    manifest_assets: &std::collections::HashMap<String, capsem_assets::asset_manager::AssetEntry>,
    context: &RuntimeGraphContext<'_>,
    asset_digest_cache: &mut AssetDigestCache,
) -> Result<Vec<serde_json::Value>> {
    let logical_name = "software-inventory.json";
    let entry = manifest_assets.get(logical_name).ok_or_else(|| {
        anyhow!(
            "manifest current release {} arch {arch} missing software-inventory.json",
            context.revision
        )
    })?;
    asset_entry_digest(arch, logical_name, entry, asset_digest_cache)?;
    let inventory_path = source_asset_path(context.assets_dir, arch, logical_name)?;
    let inventory_bytes = fs::read(&inventory_path).with_context(|| format!("read {}", inventory_path.display()))?;
    let inventory: serde_json::Value =
        serde_json::from_slice(&inventory_bytes).with_context(|| format!("parse {}", inventory_path.display()))?;
    if inventory.get("schema").and_then(|value| value.as_str()) != Some("capsem.runtime_software_inventory.v1") {
        return Err(anyhow!(
            "{} schema must be capsem.runtime_software_inventory.v1",
            inventory_path.display()
        ));
    }
    let packages = inventory
        .get("packages")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("{} missing packages array", inventory_path.display()))?;
    let evidence = runtime_file_url(context, arch, logical_name)?;
    let mut rows = Vec::new();
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
        rows.push(serde_json::json!({
            "name": name,
            "version": version,
            "source": source,
            "architecture": arch,
            "digest": digest,
            "evidence": evidence,
        }));
    }
    rows.sort_by(|left, right| {
        left.get("name")
            .and_then(|value| value.as_str())
            .cmp(&right.get("name").and_then(|value| value.as_str()))
    });
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

pub(super) fn copy_runtime_release_files(out_dir: &Path, copies: &[RuntimeReleaseFileCopy]) -> Result<()> {
    for copy in copies {
        let dst = out_dir.join(copy.url.trim_start_matches('/'));
        fs::create_dir_all(
            dst.parent()
                .ok_or_else(|| anyhow!("runtime release file path has no parent"))?,
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
        "capsem-process" => "Capsem guest process bridge executable",
        "capsem" => "Capsem command-line executable",
        _ => "Capsem packaged executable",
    }
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
    let runtime_revision = index.runtime.as_ref().map(|runtime| runtime.revision.as_str());
    let runtime_state = if runtime_revision.is_some() {
        "current"
    } else {
        "not_published"
    };
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
            "runtime": index.runtime.as_ref().map(|runtime| serde_json::json!({
                "revision": runtime.revision,
                "state": "current",
                "source": "manifest.runtime",
                "min_binary": runtime.min_binary,
                "architectures": runtime.architectures,
            })),
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
                "runtime": {
                    "latest": runtime_revision,
                    "current": runtime_revision,
                    "state": runtime_state,
                    "source": "manifest.runtime",
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
        "/runtime/releases/*".to_string(),
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

/// The immutable publication identity (GitHub release tag) of one runtime.
pub(super) fn runtime_publication_identity(channel: &str, revision: &str) -> Result<String> {
    validate_channel_name(channel)?;
    release_graph::validate_runtime_revision(revision)?;
    Ok(format!("runtime-{channel}-{revision}"))
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

use super::*;

pub(super) fn validate_blake3_hex(field: &str, value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("{field} must be a 64-character BLAKE3 hex digest");
    }
    Ok(())
}

pub(super) fn validate_hex_digest(value: &str, expected_len: usize, field: &str) -> Result<()> {
    if value.len() != expected_len || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("{field} must be a {expected_len}-character hex digest");
    }
    Ok(())
}

/// Pull any missing / hash-mismatched VM assets from the release URL.
pub(super) async fn refresh_assets(
    explicit_manifest: Option<&ExplicitManifestInput<'_>>,
    selected_channel: Option<&ResolvedReleaseChannelManifest>,
    selected_payload: Option<&[u8]>,
) -> Result<()> {
    let assets_dir = capsem_assets::asset_manager::default_assets_dir()
        .context("cannot resolve CAPSEM_HOME -- set $HOME or $CAPSEM_HOME")?;
    let refresh_source = if let Some(input) = explicit_manifest {
        Some(input.source.to_string())
    } else if let Some(selection) = selected_channel {
        Some(selection.url.clone())
    } else {
        remote_manifest_asset_source(&assets_dir)?
    };
    if let Some(source) = refresh_source {
        let candidate_manifest_sha256 = selected_payload
            .or_else(|| explicit_manifest.and_then(|input| input.payload.as_deref()))
            .map(sha256_hex);
        let previous = InstalledManifestSnapshot::capture(&assets_dir)?;
        let previous_state = installed_asset_audit_state(&assets_dir);
        append_update_audit(serde_json::json!({
            "event": "asset_update_start",
            "action": "asset_update",
            "outcome": "started",
            "source": source,
            "channel": channel_from_source(&source),
            "candidate_manifest_sha256": candidate_manifest_sha256.as_deref(),
            "previous": previous_state
        }));
        let refresh_result: Result<()> = async {
            let fetched_payload = if selected_payload.is_none() && explicit_manifest.is_some() {
                let input = explicit_manifest.context("explicit manifest input disappeared")?;
                match input.payload.as_deref() {
                    Some(payload) => Some(payload.to_vec()),
                    None => Some(read_manifest_source(&source).await?),
                }
            } else {
                None
            };
            let payload = selected_payload.or(fetched_payload.as_deref());
            let transition = if let Some(selection) = selected_channel {
                channel_transition_for_request(&assets_dir, Some(selection.channel.as_str()), None)?
            } else if explicit_manifest.is_some_and(|input| input.payload.is_some()) {
                channel_transition_for_preverified_install_payload(
                    &assets_dir,
                    &source,
                    payload.context("preverified install manifest payload was not read")?,
                )?
            } else if explicit_manifest.is_some() {
                channel_transition_for_explicit_manifest_payload(
                    &assets_dir,
                    &source,
                    payload.context("explicit manifest payload was not fetched")?,
                )?
            } else {
                ChannelTransition::Preserve
            };
            if let Some(bytes) = payload {
                if let Some(selection) = selected_channel {
                    verify_selected_channel_manifest(selection, bytes)?;
                }
                install_manifest_bytes(&assets_dir, &source, bytes, transition.manifest_metadata_policy()).await?;
            } else {
                install_manifest_source(&assets_dir, &source).await?;
            }
            if should_hydrate_assets(explicit_manifest) {
                hydrate_installed_assets(&assets_dir).await?;
            }
            persist_channel_transition(&assets_dir, &transition)?;
            Ok(())
        }
        .await;
        if let Err(error) = refresh_result {
            let _ = previous.restore(&assets_dir);
            append_update_audit(serde_json::json!({
                "event": "asset_update_failed",
                "action": "asset_update",
                "outcome": "failure",
                "source": source,
                "channel": channel_from_source(&source),
                "candidate_manifest_sha256": candidate_manifest_sha256.as_deref(),
                "previous": previous_state,
                "current": installed_asset_audit_state(&assets_dir),
                "error": format!("{error:#}")
            }));
            return Err(error).context("asset refresh failed; restored previous installed manifest");
        }
        let current_state = installed_asset_audit_state(&assets_dir);
        append_update_audit(serde_json::json!({
            "event": "asset_update_complete",
            "action": "asset_update",
            "outcome": "success",
            "source": source,
            "channel": channel_from_source(&source),
            "candidate_manifest_sha256": candidate_manifest_sha256.as_deref(),
            "previous": previous_state,
            "current": current_state,
            "changed_fields": changed_asset_audit_fields(&previous_state, &current_state)
        }));
        return Ok(());
    }

    hydrate_installed_assets(&assets_dir).await
}

pub(super) fn should_hydrate_assets(explicit_manifest: Option<&ExplicitManifestInput<'_>>) -> bool {
    !explicit_manifest.is_some_and(|input| input.payload.is_some())
}

pub(super) fn append_update_audit(mut event: serde_json::Value) {
    let now = now_secs();
    if let Some(object) = event.as_object_mut() {
        object.insert(
            "schema".to_string(),
            serde_json::Value::String("capsem.update_audit.v1".to_string()),
        );
        object.insert("timestamp".to_string(), serde_json::Value::from(now));
    }
    if let Err(error) = append_update_audit_inner(&event) {
        warn!(error = format!("{error:#}"), "failed to write update audit log");
    }
}

pub(super) fn append_update_audit_inner(event: &serde_json::Value) -> Result<()> {
    let home = crate::paths::capsem_home()?;
    let log_dir = home.join("logs");
    std::fs::create_dir_all(&log_dir).with_context(|| format!("create {}", log_dir.display()))?;
    let path = log_dir.join("update.log");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    serde_json::to_writer(&mut file, event).context("serialize update audit event")?;
    file.write_all(b"\n")
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub(super) fn installed_asset_audit_state(assets_dir: &Path) -> serde_json::Value {
    let manifest_path = assets_dir.join("manifest.json");
    let metadata_path = assets_dir.join("manifest-metadata.json");
    let manifest_bytes = read_optional_file(&manifest_path).ok().flatten();
    let metadata = read_optional_file(&metadata_path)
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let manifest = manifest_bytes
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok());
    let manifest_sha256 = manifest_bytes.as_deref().map(sha256_hex);
    serde_json::json!({
        "source": metadata.as_ref().and_then(|value| value.get("manifest_url")).and_then(|value| value.as_str()),
        "origin": metadata.as_ref().and_then(|value| value.get("origin")).and_then(|value| value.as_str()),
        "channel": metadata.as_ref().and_then(|value| value.get("channel")).and_then(|value| value.as_str()),
        "channel_kind": metadata.as_ref().and_then(|value| value.get("channel_kind")).and_then(|value| value.as_str()),
        "channel_locked": metadata.as_ref().and_then(|value| value.get("channel_locked")).and_then(|value| value.as_bool()),
        "package_version": metadata.as_ref().and_then(|value| value.get("package_version")).and_then(|value| value.as_str()),
        "manifest_sha256": manifest_sha256,
        "asset_version": manifest.as_ref()
            .and_then(|value| value.get("assets"))
            .and_then(|value| value.get("current"))
            .and_then(|value| value.as_str()),
        "binary_version": manifest.as_ref()
            .and_then(|value| value.get("binaries"))
            .and_then(|value| value.get("current"))
            .and_then(|value| value.as_str())
    })
}

pub(super) fn changed_asset_audit_fields(
    previous: &serde_json::Value,
    current: &serde_json::Value,
) -> Vec<&'static str> {
    [
        "source",
        "origin",
        "channel",
        "channel_kind",
        "channel_locked",
        "package_version",
        "manifest_sha256",
        "asset_version",
        "binary_version",
    ]
    .into_iter()
    .filter(|field| previous.get(field) != current.get(field))
    .collect()
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(super) fn channel_from_source(source: &str) -> Option<String> {
    let url = reqwest::Url::parse(source).ok()?;
    let segments: Vec<&str> = url.path_segments()?.filter(|segment| !segment.is_empty()).collect();
    for window in segments.windows(3) {
        if window[0] == "assets" && window[2] == "manifest.json" {
            return Some(window[1].to_string());
        }
    }
    if segments.last() == Some(&"manifest.json") {
        return segments
            .get(segments.len().saturating_sub(2))
            .map(|segment| (*segment).to_string());
    }
    None
}

pub(super) fn channel_transition_for_explicit_manifest_payload(
    assets_dir: &Path,
    source: &str,
    payload: &[u8],
) -> Result<ChannelTransition> {
    let document: serde_json::Value =
        serde_json::from_slice(payload).with_context(|| format!("parse manifest JSON from {source}"))?;
    let declared_channel = match document.get("channel") {
        Some(value) => Some(value.as_str().context("release manifest channel must be a string")?),
        None => None,
    };
    let metadata = installed_manifest_metadata(assets_dir)?;
    let preserves_packaged_public_channel = metadata.as_ref().is_some_and(|value| {
        value.get("origin").and_then(serde_json::Value::as_str) == Some("package")
            && value.get("channel_kind").and_then(serde_json::Value::as_str) == Some("public")
            && value.get("channel_locked").and_then(serde_json::Value::as_bool) == Some(false)
            && value.get("channel").and_then(serde_json::Value::as_str) == declared_channel
    });
    if preserves_packaged_public_channel {
        return Ok(ChannelTransition::Preserve);
    }
    channel_transition_for_request(assets_dir, None, Some(source))
}

pub(super) fn channel_transition_for_preverified_install_payload(
    assets_dir: &Path,
    source: &str,
    payload: &[u8],
) -> Result<ChannelTransition> {
    let metadata = installed_manifest_metadata(assets_dir)?
        .context("preverified install manifest requires packaged manifest metadata")?;
    if metadata.get("origin").and_then(serde_json::Value::as_str) != Some("package") {
        anyhow::bail!("preverified install manifest requires package-origin metadata");
    }
    let package_version = metadata
        .get("package_version")
        .and_then(serde_json::Value::as_str)
        .context("package-origin metadata has no package_version")?;
    let check = update_check_from_release_payload(
        payload,
        &platform::detect_install_layout(),
        source,
        Some(channel_payload_hash(payload)),
    )
    .with_context(|| format!("validate preverified install manifest from {source}"))?;
    let selected_version = check
        .latest_version
        .as_deref()
        .context("preverified install manifest selects no package version")?;
    if selected_version != package_version || package_version != env!("CARGO_PKG_VERSION") {
        anyhow::bail!(
            "preverified install manifest selects package {selected_version}, but installed package metadata selects {package_version} and binary is {}",
            env!("CARGO_PKG_VERSION")
        );
    }
    Ok(ChannelTransition::PreservePackageOrigin)
}

pub(super) fn installed_manifest_metadata(assets_dir: &Path) -> Result<Option<serde_json::Value>> {
    let path = assets_dir.join("manifest-metadata.json");
    let Some(bytes) = read_optional_file(&path)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", path.display()))
        .map(Some)
}

pub(super) fn channel_transition_for_request(
    assets_dir: &Path,
    public_channel: Option<&str>,
    explicit_manifest: Option<&str>,
) -> Result<ChannelTransition> {
    let metadata = installed_manifest_metadata(assets_dir)?;
    let locked = metadata
        .as_ref()
        .and_then(|value| value.get("channel_locked"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let current_source = metadata
        .as_ref()
        .and_then(|value| value.get("manifest_url"))
        .and_then(serde_json::Value::as_str);

    if locked {
        if let Some(channel) = public_channel {
            anyhow::bail!("installed corporate channel is locked; cannot switch to public channel {channel}");
        }
        if let Some(source) = explicit_manifest {
            if current_source != Some(source) {
                anyhow::bail!(
                    "installed corporate channel is locked to {}; cannot switch to {source}",
                    current_source.unwrap_or("its configured manifest")
                );
            }
        }
        return Ok(ChannelTransition::Preserve);
    }

    if let Some(channel) = public_channel {
        return Ok(ChannelTransition::Public(channel.to_string()));
    }
    if let Some(source) = explicit_manifest {
        let package_hydration = metadata.as_ref().is_some_and(|value| {
            value.get("origin").and_then(serde_json::Value::as_str) == Some("package") && current_source == Some(source)
        });
        if package_hydration {
            return Ok(ChannelTransition::Preserve);
        }
        return Ok(ChannelTransition::Corporate);
    }
    Ok(ChannelTransition::Preserve)
}

pub(super) fn persist_channel_transition(assets_dir: &Path, transition: &ChannelTransition) -> Result<()> {
    if matches!(
        transition,
        ChannelTransition::Preserve | ChannelTransition::PreservePackageOrigin
    ) {
        return Ok(());
    }
    let path = assets_dir.join("manifest-metadata.json");
    let mut metadata = installed_manifest_metadata(assets_dir)?
        .context("installed manifest metadata disappeared while persisting channel selection")?;
    let object = metadata
        .as_object_mut()
        .context("installed manifest metadata must be a JSON object")?;
    match transition {
        ChannelTransition::Public(channel) => {
            object.insert("channel".to_string(), serde_json::json!(channel));
            object.insert("channel_kind".to_string(), serde_json::json!("public"));
            object.insert("channel_locked".to_string(), serde_json::json!(false));
        }
        ChannelTransition::Corporate => {
            object.insert("channel".to_string(), serde_json::json!("corp"));
            object.insert("channel_kind".to_string(), serde_json::json!("corporate"));
            object.insert("channel_locked".to_string(), serde_json::json!(true));
        }
        ChannelTransition::Preserve | ChannelTransition::PreservePackageOrigin => unreachable!(),
    }
    let bytes = serde_json::to_vec_pretty(&metadata).context("serialize manifest metadata")?;
    atomic_write(&path, &bytes)
}

pub(super) async fn hydrate_installed_assets(assets_dir: &Path) -> Result<()> {
    hydrate_assets_for_binary(assets_dir, env!("CARGO_PKG_VERSION")).await
}

pub(super) async fn hydrate_assets_for_binary(assets_dir: &Path, binary_version: &str) -> Result<()> {
    let manifest_path = assets_dir.join("manifest.json");
    let manifest_bytes =
        std::fs::read_to_string(&manifest_path).with_context(|| format!("read {}", manifest_path.display()))?;
    if release_graph_from_payload(manifest_bytes.as_bytes())?.is_some_and(|graph| graph.runtime.is_none()) {
        println!("Release channel publishes no VM runtime yet; no assets to refresh.");
        return Ok(());
    }
    let manifest = capsem_assets::asset_manager::ManifestV2::from_json(&manifest_bytes)
        .with_context(|| format!("parse {}", manifest_path.display()))?;

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };

    println!("Refreshing VM assets into {}...", assets_dir.display());
    if let Some(local_source) = local_manifest_asset_source(assets_dir)? {
        println!("Using local asset source {}...", local_source.display());
        let copied = capsem_assets::asset_manager::copy_missing_local_assets(
            &manifest,
            binary_version,
            arch,
            &local_source,
            assets_dir,
            |p| {
                if p.done {
                    let mb = p.bytes_done as f64 / 1_048_576.0;
                    println!("  {} ({:.1} MB)", p.logical_name, mb);
                }
            },
        )
        .context("local asset hydration failed")?;

        if copied.is_empty() {
            println!("All assets already up to date.");
        } else {
            println!("Refreshed {} asset(s).", copied.len());
        }
        return Ok(());
    }

    let downloaded =
        capsem_assets::asset_manager::download_missing_assets(&manifest, binary_version, arch, assets_dir, |p| {
            if p.done {
                let mb = p.bytes_done as f64 / 1_048_576.0;
                println!("  {} ({:.1} MB)", p.logical_name, mb);
            }
        })
        .await
        .context("asset download failed")?;

    if downloaded.is_empty() {
        println!("All assets already up to date.");
    } else {
        println!("Refreshed {} asset(s).", downloaded.len());
    }
    Ok(())
}

pub(super) struct InstalledManifestSnapshot {
    manifest: Option<Vec<u8>>,
    metadata: Option<Vec<u8>>,
}

impl InstalledManifestSnapshot {
    pub(super) fn capture(assets_dir: &Path) -> Result<Self> {
        Ok(Self {
            manifest: read_optional_file(&assets_dir.join("manifest.json"))?,
            metadata: read_optional_file(&assets_dir.join("manifest-metadata.json"))?,
        })
    }

    pub(super) fn restore(&self, assets_dir: &Path) -> Result<()> {
        restore_optional_file(&assets_dir.join("manifest.json"), self.manifest.as_deref())?;
        restore_optional_file(&assets_dir.join("manifest-metadata.json"), self.metadata.as_deref())?;
        Ok(())
    }
}

pub(super) async fn provision_corp_config(source: &str) -> Result<()> {
    let capsem_dir = crate::paths::capsem_home()?;
    capsem_core::net::policy_config::corp_provision::provision_from_source(&capsem_dir, source)
        .await
        .with_context(|| format!("provision corp config from {source}"))?;
    println!("Corp config updated from {source}.");
    Ok(())
}

/// Resolve a release-graph artifact reference against the manifest it came from.
pub(super) fn resolve_release_channel_artifact_url(channel_source: &str, artifact: &str) -> Result<String> {
    let trimmed = artifact.trim();
    if trimmed.is_empty() {
        anyhow::bail!("release channel artifact URL is empty");
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") || trimmed.starts_with("file://") {
        let parsed =
            reqwest::Url::parse(trimmed).with_context(|| format!("parse release channel artifact URL {trimmed}"))?;
        return Ok(parsed.to_string());
    }

    let base =
        reqwest::Url::parse(channel_source).with_context(|| format!("parse release channel URL {channel_source}"))?;
    if trimmed.starts_with('/') {
        // A site-root-relative reference, because a generated channel is a
        // website: the manifest sits at `<root>/assets/<channel>/manifest.json`
        // and its artifacts are recorded as `/runtime/...`.
        //
        // Over http(s) the site root is the origin, so replacing the path is
        // exactly right. A `file://` channel is that same tree on disk, and its
        // root is the dist directory rather than the filesystem root -- so
        // `set_path` alone sent every local hydration to `/runtime/...` and
        // failed with ENOENT.
        if base.scheme() == "file" {
            if let Some(dist) = base.path().rfind("/assets/") {
                let mut root = base.clone();
                root.set_path(&format!("{}{trimmed}", &base.path()[..dist]));
                root.set_query(None);
                root.set_fragment(None);
                return Ok(root.to_string());
            }
        }
        let mut root = base;
        root.set_path(trimmed);
        root.set_query(None);
        root.set_fragment(None);
        return Ok(root.to_string());
    }
    base.join(trimmed)
        .with_context(|| format!("resolve release channel artifact {trimmed} against {channel_source}"))
        .map(|url| url.to_string())
}

/// The verified downloads that install a graph's runtime on `arch`.
///
/// A graph without a runtime is a binary-only channel: it installs no assets.
pub(super) fn runtime_asset_downloads(
    graph: &ReleaseGraphManifest,
    arch: &str,
) -> Result<Vec<ReleaseChannelAssetDownload>> {
    let Some(runtime) = graph.runtime.as_ref() else {
        return Ok(Vec::new());
    };
    if release_channel_status_is_revoked(&runtime.status) {
        anyhow::bail!("release channel runtime {} is revoked", runtime.revision);
    }
    let architecture = runtime
        .architectures
        .iter()
        .find(|candidate| candidate.architecture.as_str() == arch)
        .with_context(|| format!("release channel runtime {} has no {arch} image set", runtime.revision))?;
    let mut downloads = Vec::new();
    for image in &architecture.images {
        if release_channel_status_is_revoked(&image.status) {
            continue;
        }
        if image.name.trim().is_empty() {
            anyhow::bail!(
                "release channel runtime {} architecture {arch} has an unnamed {} image",
                runtime.revision,
                image.kind
            );
        }
        let Some(logical_name) = release_channel_image_logical_name(&image.kind) else {
            continue;
        };
        validate_release_channel_digest(&image.digest)?;
        downloads.push(ReleaseChannelAssetDownload {
            logical_name: logical_name.to_string(),
            url: image.url.clone(),
            size: image.size,
            sha256: image.digest.sha256.clone(),
            blake3: image.digest.blake3.clone(),
        });
    }
    for required in ["vmlinuz", "initrd.img", "rootfs.erofs"] {
        if !downloads.iter().any(|download| download.logical_name == required) {
            anyhow::bail!(
                "release channel runtime {} architecture {arch} missing {required} image",
                runtime.revision
            );
        }
    }
    Ok(dedupe_release_channel_downloads(downloads))
}

pub(super) fn release_channel_image_logical_name(kind: &str) -> Option<&'static str> {
    match kind {
        "kernel" => Some("vmlinuz"),
        "initrd" => Some("initrd.img"),
        "rootfs" => Some("rootfs.erofs"),
        _ => None,
    }
}

pub(super) fn release_channel_status_is_revoked(status: &str) -> bool {
    status.eq_ignore_ascii_case("revoked")
}

pub(super) fn validate_release_channel_digest(digest: &ReleaseChannelRuntimeDigest) -> Result<()> {
    validate_blake3_hex("runtime image blake3", &digest.blake3)?;
    if digest.sha256.len() != 64 || !digest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("runtime image sha256 must be a 64-character hex digest");
    }
    Ok(())
}

pub(super) fn dedupe_release_channel_downloads(
    downloads: Vec<ReleaseChannelAssetDownload>,
) -> Vec<ReleaseChannelAssetDownload> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();
    for download in downloads {
        let key = (download.logical_name.clone(), download.blake3.clone());
        if seen.insert(key) {
            unique.push(download);
        }
    }
    unique.sort_by(|left, right| {
        left.logical_name
            .cmp(&right.logical_name)
            .then_with(|| left.url.cmp(&right.url))
    });
    unique
}

/// Install a release graph: its runtime's images for this host, then the
/// manifest itself. A graph without a runtime installs only the manifest.
pub(super) async fn install_release_channel_graph(
    assets_dir: &Path,
    source: &str,
    body: &str,
    graph: &ReleaseGraphManifest,
    metadata_policy: ManifestMetadataPolicy,
) -> Result<()> {
    let downloads = runtime_asset_downloads(graph, capsem_assets::asset_manager::host_manifest_arch())?;
    if !downloads.is_empty() {
        capsem_assets::asset_manager::ManifestV2::from_json(body)
            .context("validate release graph through the runtime manifest parser")?;
        hydrate_release_channel_runtime_assets(assets_dir, source, &downloads).await?;
    }

    std::fs::create_dir_all(assets_dir).with_context(|| format!("cannot create {}", assets_dir.display()))?;
    atomic_write(&assets_dir.join("manifest.json"), body.as_bytes())?;
    write_installed_manifest_metadata_with_policy(assets_dir, source, body.as_bytes(), metadata_policy)?;
    println!("Installed asset manifest from {source}.");
    Ok(())
}

pub(super) async fn hydrate_release_channel_runtime_assets(
    assets_dir: &Path,
    source: &str,
    downloads: &[ReleaseChannelAssetDownload],
) -> Result<()> {
    let arch = capsem_assets::asset_manager::host_manifest_arch();
    let arch_dir = assets_dir.join(arch);
    std::fs::create_dir_all(&arch_dir).with_context(|| format!("create {}", arch_dir.display()))?;

    for download in downloads {
        download_release_channel_runtime_asset(&arch_dir, source, download).await?;
    }
    Ok(())
}

pub(super) async fn download_release_channel_runtime_asset(
    arch_dir: &Path,
    manifest_source: &str,
    download: &ReleaseChannelAssetDownload,
) -> Result<()> {
    validate_blake3_hex("runtime image blake3", &download.blake3)?;
    let target = arch_dir.join(capsem_assets::asset_manager::hash_filename(
        &download.logical_name,
        &download.blake3,
    ));
    if target.exists() {
        if verify_release_channel_asset_file(&target, download)
            .with_context(|| format!("verify existing runtime image asset {}", target.display()))?
        {
            return Ok(());
        }
        let _ = std::fs::remove_file(&target);
    }

    let url = resolve_release_channel_artifact_url(manifest_source, &download.url)?;
    let parsed = reqwest::Url::parse(&url).with_context(|| format!("parse release channel runtime image URL {url}"))?;
    match parsed.scheme() {
        "file" => download_release_channel_runtime_asset_from_file(&target, &parsed, download)
            .with_context(|| format!("copy runtime image {}", download.url))?,
        "http" | "https" => download_release_channel_runtime_asset_from_http(&target, &url, download).await?,
        scheme => anyhow::bail!("unsupported runtime image URL scheme {scheme}: use https://, http://, or file://"),
    }
    Ok(())
}

pub(super) fn verify_release_channel_asset_file(path: &Path, download: &ReleaseChannelAssetDownload) -> Result<bool> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut blake3_hasher = blake3::Hasher::new();
    let mut sha256_hasher = Sha256::new();
    let mut bytes_done = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            break;
        }
        blake3_hasher.update(&buffer[..n]);
        sha256_hasher.update(&buffer[..n]);
        bytes_done += n as u64;
    }
    let actual_blake3 = blake3_hasher.finalize().to_hex().to_string();
    let actual_sha256 = format!("{:x}", sha256_hasher.finalize());
    Ok(bytes_done == download.size
        && actual_blake3 == download.blake3
        && actual_sha256.eq_ignore_ascii_case(&download.sha256))
}

pub(super) fn download_release_channel_runtime_asset_from_file(
    target: &Path,
    url: &reqwest::Url,
    download: &ReleaseChannelAssetDownload,
) -> Result<()> {
    use std::io::{Read, Write};

    let source_path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("runtime image file URL must be absolute: {}", url.as_str()))?;
    let tmp = target.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut source = std::fs::File::open(&source_path).with_context(|| format!("open {}", source_path.display()))?;
    let mut dest = std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    let mut blake3_hasher = blake3::Hasher::new();
    let mut sha256_hasher = Sha256::new();
    let mut bytes_done = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let n = source
            .read(&mut buffer)
            .with_context(|| format!("read {}", source_path.display()))?;
        if n == 0 {
            break;
        }
        dest.write_all(&buffer[..n])
            .with_context(|| format!("write {}", tmp.display()))?;
        blake3_hasher.update(&buffer[..n]);
        sha256_hasher.update(&buffer[..n]);
        bytes_done += n as u64;
    }
    dest.flush().with_context(|| format!("flush {}", tmp.display()))?;
    drop(dest);
    finish_release_channel_asset_download(
        target,
        &tmp,
        download,
        bytes_done,
        blake3_hasher.finalize().to_hex().to_string(),
        format!("{:x}", sha256_hasher.finalize()),
    )
}

pub(super) async fn download_release_channel_runtime_asset_from_http(
    target: &Path,
    url: &str,
    download: &ReleaseChannelAssetDownload,
) -> Result<()> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("parse runtime image URL {url}"))?;
    let bytes = release_http_get_bytes(parsed, None, url)
        .await
        .with_context(|| format!("read runtime image body from {url}"))?;

    let tmp = target.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let bytes_done = bytes.len() as u64;
    let actual_blake3 = blake3::hash(&bytes).to_hex().to_string();
    let actual_sha256 = sha256_hex(&bytes);
    if let Err(error) = std::fs::write(&tmp, &bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::Error::new(error).context(format!("write {}", tmp.display())));
    }

    finish_release_channel_asset_download(target, &tmp, download, bytes_done, actual_blake3, actual_sha256)
}

pub(super) fn finish_release_channel_asset_download(
    target: &Path,
    tmp: &Path,
    download: &ReleaseChannelAssetDownload,
    bytes_done: u64,
    actual_blake3: String,
    actual_sha256: String,
) -> Result<()> {
    if bytes_done != download.size {
        let _ = std::fs::remove_file(tmp);
        anyhow::bail!(
            "{}: size mismatch (expected {}, got {})",
            download.logical_name,
            download.size,
            bytes_done
        );
    }
    if actual_blake3 != download.blake3 {
        let _ = std::fs::remove_file(tmp);
        anyhow::bail!(
            "{}: hash mismatch (expected {}, got {})",
            download.logical_name,
            download.blake3,
            actual_blake3
        );
    }
    if !actual_sha256.eq_ignore_ascii_case(&download.sha256) {
        let _ = std::fs::remove_file(tmp);
        anyhow::bail!(
            "{}: sha256 mismatch (expected {}, got {})",
            download.logical_name,
            download.sha256,
            actual_sha256
        );
    }
    std::fs::rename(tmp, target).with_context(|| format!("rename {} -> {}", tmp.display(), target.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o444));
    }
    Ok(())
}

pub(super) async fn install_manifest_source(assets_dir: &std::path::Path, source: &str) -> Result<()> {
    let bytes = read_manifest_source(source).await?;
    install_manifest_bytes(assets_dir, source, &bytes, ManifestMetadataPolicy::RecordSource).await
}

pub(super) async fn install_manifest_bytes(
    assets_dir: &std::path::Path,
    source: &str,
    bytes: &[u8],
    metadata_policy: ManifestMetadataPolicy,
) -> Result<()> {
    let body =
        std::str::from_utf8(bytes).with_context(|| format!("manifest URL did not return UTF-8 JSON: {source}"))?;
    if let Some(graph) =
        release_graph_from_payload(bytes).with_context(|| format!("parse manifest JSON from {source}"))?
    {
        install_release_channel_graph(assets_dir, source, body, &graph, metadata_policy)
            .await
            .with_context(|| format!("install release channel graph from {source}"))?;
        return Ok(());
    }
    capsem_assets::asset_manager::ManifestV2::from_json(body)
        .with_context(|| format!("parse format 2 manifest from {source}"))?;

    std::fs::create_dir_all(assets_dir).with_context(|| format!("cannot create {}", assets_dir.display()))?;
    atomic_write(&assets_dir.join("manifest.json"), bytes)?;
    write_installed_manifest_metadata_with_policy(assets_dir, source, bytes, metadata_policy)?;
    println!("Installed asset manifest from {source}.");
    Ok(())
}

pub(super) fn write_manifest_metadata(assets_dir: &Path, source: &str) -> Result<()> {
    let metadata_path = assets_dir.join("manifest-metadata.json");
    let mut metadata = read_manifest_metadata_value(&metadata_path)?.unwrap_or_else(|| {
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1"
        })
    });
    let object = metadata
        .as_object_mut()
        .context("manifest metadata must be a JSON object")?;
    object.insert("schema".to_string(), serde_json::json!("capsem.manifest_metadata.v1"));
    object.insert("origin".to_string(), serde_json::json!("update"));
    object.insert("manifest_url".to_string(), serde_json::json!(source));
    object.insert("refreshed_at".to_string(), serde_json::json!(now_secs()));
    object
        .entry("installed_at".to_string())
        .or_insert_with(|| serde_json::json!(now_secs()));
    let metadata_bytes = serde_json::to_vec_pretty(&metadata)?;
    atomic_write(&metadata_path, &metadata_bytes)?;
    Ok(())
}

pub(super) fn write_install_timestamps(assets_dir: &Path) -> Result<()> {
    let metadata_path = assets_dir.join("manifest-metadata.json");
    let mut metadata = read_manifest_metadata_value(&metadata_path)?
        .context("package manifest metadata disappeared during preactivation")?;
    let object = metadata
        .as_object_mut()
        .context("manifest metadata must be a JSON object")?;
    let now = now_secs();
    object.insert("refreshed_at".to_string(), serde_json::json!(now));
    object
        .entry("installed_at".to_string())
        .or_insert_with(|| serde_json::json!(now));
    let metadata_bytes = serde_json::to_vec_pretty(&metadata)?;
    atomic_write(&metadata_path, &metadata_bytes)
}

pub(super) fn write_installed_manifest_metadata(assets_dir: &Path, source: &str, manifest_bytes: &[u8]) -> Result<()> {
    write_installed_manifest_metadata_with_policy(
        assets_dir,
        source,
        manifest_bytes,
        ManifestMetadataPolicy::RecordSource,
    )
}

pub(super) fn write_installed_manifest_metadata_with_policy(
    assets_dir: &Path,
    source: &str,
    manifest_bytes: &[u8],
    metadata_policy: ManifestMetadataPolicy,
) -> Result<()> {
    if metadata_policy == ManifestMetadataPolicy::RecordSource {
        write_manifest_metadata(assets_dir, source)?;
    } else {
        let metadata = installed_manifest_metadata(assets_dir)?
            .context("package manifest metadata disappeared during preactivation")?;
        if metadata.get("origin").and_then(serde_json::Value::as_str) != Some("package") {
            anyhow::bail!("preactivation can preserve only package-origin manifest metadata");
        }
        write_install_timestamps(assets_dir)?;
    }
    let check = update_check_from_release_payload(
        manifest_bytes,
        &platform::detect_install_layout(),
        source,
        Some(channel_payload_hash(manifest_bytes)),
    )
    .with_context(|| format!("derive installed manifest status from {source}"))?;
    write_cache_to_path(&assets_dir.join("manifest-metadata.json"), &check).context("write installed manifest status")
}

pub(super) async fn release_http_get_bytes(
    url: reqwest::Url,
    accept: Option<&'static str>,
    display_url: &str,
) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .user_agent("capsem")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .context("build release HTTP client")?;

    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 1..=RELEASE_HTTP_ATTEMPTS {
        let mut request = client.get(url.clone());
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    match response.bytes().await {
                        Ok(bytes) => return Ok(bytes.to_vec()),
                        Err(error) => {
                            let error = anyhow::Error::new(error).context(format!("read {display_url}"));
                            if attempt == RELEASE_HTTP_ATTEMPTS {
                                return Err(error);
                            }
                            warn!(
                                attempt,
                                max_attempts = RELEASE_HTTP_ATTEMPTS,
                                url = %display_url,
                                error = %error,
                                "release HTTP body read failed; retrying"
                            );
                            last_error = Some(error);
                        }
                    }
                } else if release_http_status_is_retryable(status) {
                    let error = anyhow::anyhow!("GET {} returned {}", display_url, status);
                    if attempt == RELEASE_HTTP_ATTEMPTS {
                        return Err(error);
                    }
                    warn!(
                        attempt,
                        max_attempts = RELEASE_HTTP_ATTEMPTS,
                        url = %display_url,
                        status = %status,
                        "release HTTP status is retryable"
                    );
                    last_error = Some(error);
                } else {
                    anyhow::bail!("GET {} returned {}", display_url, status);
                }
            }
            Err(error) => {
                let error = anyhow::Error::new(error).context(format!("GET {display_url}"));
                if attempt == RELEASE_HTTP_ATTEMPTS {
                    return Err(error);
                }
                warn!(
                    attempt,
                    max_attempts = RELEASE_HTTP_ATTEMPTS,
                    url = %display_url,
                    error = %error,
                    "release HTTP request failed; retrying"
                );
                last_error = Some(error);
            }
        }

        tokio::time::sleep(release_http_retry_backoff(attempt)).await;
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("GET {display_url} failed")))
}

pub(super) fn release_http_status_is_retryable(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

pub(super) fn release_http_retry_backoff(attempt: usize) -> Duration {
    let multiplier = 1u64 << attempt.saturating_sub(1).min(4);
    Duration::from_millis(RELEASE_HTTP_INITIAL_BACKOFF_MS * multiplier)
}

pub(super) async fn read_manifest_source(source: &str) -> Result<Vec<u8>> {
    let url = reqwest::Url::parse(source).with_context(|| {
        format!("--manifest must be a URL: use https://..., http://..., or file:///absolute/path, got {source}")
    })?;
    match url.scheme() {
        "file" => {
            if !has_scheme_authority_prefix(source, "file") {
                anyhow::bail!("--manifest file URL must start with file://: {source}");
            }
            let path = url
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("--manifest file URL must be absolute: {source}"))?;
            std::fs::read(&path).with_context(|| format!("read manifest {}", path.display()))
        }
        "http" | "https" => {
            if !has_scheme_authority_prefix(source, url.scheme()) {
                anyhow::bail!("--manifest must use https://, http://, or file:// URLs, got {source}");
            }
            release_http_get_bytes(url.clone(), Some("application/json"), source)
                .await
                .with_context(|| format!("read manifest body from {source}"))
        }
        scheme => anyhow::bail!("unsupported --manifest URL scheme {scheme}: use https://, http://, or file://"),
    }
}

pub(super) fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

pub(super) fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

pub(super) fn restore_optional_file(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    if let Some(bytes) = bytes {
        if std::fs::read(path).ok().as_deref() == Some(bytes) {
            return Ok(());
        }
        atomic_write(path, bytes)
    } else if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
    } else {
        Ok(())
    }
}

pub(super) fn local_manifest_asset_source(assets_dir: &std::path::Path) -> Result<Option<PathBuf>> {
    let metadata_path = assets_dir.join("manifest-metadata.json");
    if !metadata_path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&metadata_path).with_context(|| format!("read {}", metadata_path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&content).with_context(|| format!("parse {}", metadata_path.display()))?;
    let Some(source) = value.get("manifest_url").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(None);
    }
    let parsed = reqwest::Url::parse(source).with_context(|| {
        format!(
            "asset manifest metadata source must be a URL: use https://..., http://..., or file:///absolute/path, got {source}"
        )
    })?;
    if parsed.scheme() != "file" {
        anyhow::bail!(
            "unsupported asset manifest metadata URL scheme {}: use https://, http://, or file://",
            parsed.scheme()
        );
    }
    if !has_scheme_authority_prefix(source, "file") {
        anyhow::bail!("asset manifest metadata file URL must start with file://: {source}");
    }
    let path = parsed
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("asset manifest metadata file URL must be absolute: {source}"))?;
    if !path.is_file() {
        return Ok(None);
    }
    Ok(path.parent().map(|parent| parent.to_path_buf()))
}

pub(super) fn remote_manifest_asset_source(assets_dir: &std::path::Path) -> Result<Option<String>> {
    let metadata_path = assets_dir.join("manifest-metadata.json");
    if !metadata_path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&metadata_path).with_context(|| format!("read {}", metadata_path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&content).with_context(|| format!("parse {}", metadata_path.display()))?;
    let Some(source) = value.get("manifest_url").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    if !(source.starts_with("http://") || source.starts_with("https://")) {
        return Ok(None);
    }
    let parsed = reqwest::Url::parse(source).with_context(|| {
        format!(
            "asset manifest metadata source must be a URL: use https://..., http://..., or file:///absolute/path, got {source}"
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!(
            "unsupported asset manifest metadata URL scheme {}: use https://, http://, or file://",
            parsed.scheme()
        );
    }
    if !has_scheme_authority_prefix(source, parsed.scheme()) {
        anyhow::bail!("asset manifest metadata must use https://, http://, or file:// URLs, got {source}");
    }
    Ok(Some(source.to_string()))
}

use super::*;

pub(super) fn check_assets_channel(dist: &Path, channel: &str) -> Result<AssetsChannelCheckReport> {
    validate_channel_name(channel)?;
    let index_path = dist.join("index.html");
    let channel_index_path = dist.join("channels").join(channel).join("index.html");
    let manifest_path = dist.join("assets").join(channel).join("manifest.json");
    let channel_health_path = dist.join("assets").join(channel).join("health.json");
    let root_health_path = dist.join("health.json");
    let health_path = if channel_health_path.exists()
        && root_health_path.exists()
        && root_health_belongs_to_other_channel(&root_health_path, channel)
    {
        channel_health_path
    } else {
        root_health_path
    };
    let headers_path = dist.join("_headers");

    #[cfg(test)]
    if !index_path.exists() {
        write_test_assets_channel_index_fixture(dist, channel)
            .with_context(|| format!("write test {}", index_path.display()))?;
    }

    let index_html = fs::read_to_string(&index_path).with_context(|| format!("read {}", index_path.display()))?;
    let channel_index_html =
        fs::read_to_string(&channel_index_path).with_context(|| format!("read {}", channel_index_path.display()))?;
    if !index_html.contains("Capsem Release Channels") {
        return Err(anyhow!("{} is not a Capsem release channel page", index_path.display()));
    }
    validate_assets_channel_index_html(&index_html, channel)?;
    validate_assets_channel_page_html(&channel_index_html, channel)?;
    let manifest_content =
        fs::read_to_string(&manifest_path).with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest_json: serde_json::Value =
        serde_json::from_str(&manifest_content).context("parse channel manifest JSON")?;
    let headers = fs::read_to_string(&headers_path).with_context(|| format!("read {}", headers_path.display()))?;
    validate_assets_channel_headers(&headers, channel)?;
    validate_assets_channel_catalog_manifest_digest(dist, channel, &manifest_content)?;
    if !is_release_graph_manifest_value(&manifest_json) {
        return Err(anyhow!("{} is not a release graph manifest", manifest_path.display()));
    }
    validate_assets_channel_graph_manifest(&manifest_json, channel)?;
    let health_content = fs::read_to_string(&health_path).with_context(|| format!("read {}", health_path.display()))?;
    let health: serde_json::Value = serde_json::from_str(&health_content).context("parse asset channel health.json")?;
    validate_assets_channel_graph_health(dist, channel, &manifest_json, &health)?;
    validate_assets_channel_graph_index_state(&index_html, channel, &manifest_json, &health)?;
    validate_assets_channel_graph_page_state(&channel_index_html, channel, &manifest_json, &health)?;
    Ok(AssetsChannelCheckReport {
        schema: "capsem.admin.assets_channel_check.v1",
        ok: true,
        channel: channel.to_string(),
        state: "published".to_string(),
        dist: dist.display().to_string(),
        manifest: manifest_path.display().to_string(),
    })
}

pub(super) fn validate_assets_channel_headers(headers: &str, channel: &str) -> Result<()> {
    let channel_manifest_header = format!("/assets/{channel}/*\n  Cache-Control: no-cache, must-revalidate");
    if !headers.contains(&channel_manifest_header) {
        return Err(anyhow!("_headers must keep asset channel manifests fresh"));
    }
    if !headers.contains("/channels.json\n  Cache-Control: no-cache, must-revalidate") {
        return Err(anyhow!("_headers must keep channels.json fresh"));
    }
    for path in ["/404", "/404.html"] {
        let rule = format!("{path}\n  Cache-Control: no-cache, must-revalidate");
        if !headers.contains(&rule) {
            return Err(anyhow!("_headers must keep {path} fresh"));
        }
    }
    if !headers.contains("/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable") {
        return Err(anyhow!("_headers must cache immutable asset releases"));
    }
    if !headers.contains("/runtime/releases/*\n  Cache-Control: public, max-age=31536000, immutable") {
        return Err(anyhow!("_headers must cache immutable runtime releases"));
    }
    Ok(())
}

/// A release graph carries a package cohort; a format-2 asset manifest
/// carries `assets`/`binaries` sections instead.
pub(super) fn is_release_graph_manifest_value(manifest: &serde_json::Value) -> bool {
    manifest.get("packages").is_some()
}

/// Validate one channel's release graph: its JSON shape, the typed graph model
/// (which admits no `profiles` map and no per-architecture config refs), and
/// the runtime document when the channel has published one.
pub(super) fn validate_assets_channel_graph_manifest(manifest: &serde_json::Value, channel: &str) -> Result<()> {
    if manifest.get("profiles").is_some() {
        return Err(anyhow!(
            "graph manifest must not publish profiles; it carries one runtime"
        ));
    }
    require_json_string(manifest, &["version"])?;
    require_json_str(manifest, &["channel"], channel, "graph manifest channel mismatch")?;
    require_json_str(manifest, &["status"], "current", "graph manifest status mismatch")?;
    let packages = manifest
        .get("packages")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("graph manifest packages must be an array"))?;
    for package in packages {
        require_json_string(package, &["name"])?;
        require_json_string(package, &["url"])?;
        require_json_string(package, &["digest", "sha256"])?;
        require_json_string(package, &["digest", "blake3"])?;
        let binaries = package
            .get("binaries")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("graph package must list binaries"))?;
        if binaries.is_empty() {
            return Err(anyhow!("graph package must list at least one binary"));
        }
        for binary in binaries {
            require_json_string(binary, &["name"])?;
            require_json_string(binary, &["version"])?;
            require_json_string(binary, &["installed_path"])?;
            require_json_string(binary, &["digest", "sha256"])?;
            require_json_string(binary, &["digest", "blake3"])?;
            require_json_string(binary, &["sbom_component_ref"])?;
        }
    }
    let graph: release_graph::ReleaseManifest =
        serde_json::from_value(manifest.clone()).context("graph manifest does not match the release graph model")?;
    if let Some(runtime) = &graph.runtime {
        runtime
            .validate()
            .with_context(|| format!("graph manifest runtime {} is invalid", runtime.revision))?;
    }
    Ok(())
}

pub(super) fn validate_assets_channel_graph_health(
    dist: &Path,
    channel: &str,
    manifest: &serde_json::Value,
    health: &serde_json::Value,
) -> Result<()> {
    require_json_str(
        health,
        &["schema"],
        "capsem.assets_channel.health.v1",
        "health.json schema mismatch",
    )?;
    require_json_str(health, &["channel"], channel, "health.json channel mismatch")?;
    require_json_bool(health, &["ok"], true, "health.json ok mismatch")?;
    require_json_str(health, &["state"], "published", "health.json state mismatch")?;
    require_json_str(
        health,
        &["urls", "index"],
        "/index.html",
        "health.json index URL mismatch",
    )?;
    require_json_str(
        health,
        &["urls", "health"],
        "/health.json",
        "health.json health URL mismatch",
    )?;
    require_json_str(
        health,
        &["urls", "manifest"],
        &format!("/assets/{channel}/manifest.json"),
        "health.json manifest URL does not match channel",
    )?;
    let expected_asset_base = require_json_string(health, &["urls", "asset_base"])?;
    let current_assets = require_json_string(health, &["current", "assets"])?;
    let current_binary = require_json_string(health, &["current", "binary"])?;
    require_json_str(
        health,
        &["assets", "version"],
        &current_assets,
        "health.json assets value does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["binary", "version"],
        &current_binary,
        "health.json binary value does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "latest"],
        &current_assets,
        "health.json asset update latest target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "current"],
        &current_assets,
        "health.json asset update target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "source"],
        "manifest.assets.current",
        "health.json asset update source mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "manifest"],
        &format!("/assets/{channel}/manifest.json"),
        "health.json asset update manifest mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "assets", "asset_base"],
        &expected_asset_base,
        "health.json asset update base mismatch",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "latest"],
        &current_binary,
        "health.json binary update latest target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "current"],
        &current_binary,
        "health.json binary update target does not match channel manifest",
    )?;
    require_json_str(
        health,
        &["updates", "binary", "source"],
        "manifest.binaries.current",
        "health.json binary update source mismatch",
    )?;
    for retired in ["profiles", "updates.profiles", "updates.images"] {
        require_json_absent(
            health,
            &retired.split('.').collect::<Vec<_>>(),
            &format!("health.json must not publish {retired}"),
        )?;
    }
    validate_assets_channel_graph_health_runtime(manifest, health)?;
    let asset_files = require_json_array(health, &["assets", "files"])?;
    let asset_releases = require_json_array(health, &["asset_releases"])?;
    for release in asset_releases {
        require_json_string(release, &["date"]).map_err(|_| anyhow!("health.json asset release date mismatch"))?;
    }
    let vm_oboms = require_json_array(health, &["evidence", "vm_oboms"])?;
    let host_sboms = require_json_array(health, &["evidence", "host_sboms"])?;
    let host_binary_files = require_json_array(health, &["evidence", "host_binary_files"])?;
    let attestations = require_json_array(health, &["evidence", "attestations"])?;
    let packages = manifest
        .get("packages")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("graph manifest packages must be an array"))?;

    let mut package_urls = BTreeSet::new();
    let mut expected_host_files = BTreeMap::new();
    let mut package_versions = BTreeSet::new();
    for package in packages {
        let package_url = package
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("graph package url missing"))?;
        package_urls.insert(package_url.to_string());
        let package_version = package
            .get("version")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("graph package version missing"))?;
        package_versions.insert(package_version.to_string());
        expected_host_files.insert(package_url.to_string(), package);
        let binaries = package
            .get("binaries")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("graph package must list binaries"))?;
        if binaries.is_empty() {
            return Err(anyhow!("graph package must list at least one binary"));
        }
        for evidence in package
            .get("evidence")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
        {
            let url = evidence
                .get("url")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("graph package evidence url missing"))?;
            expected_host_files.insert(url.to_string(), evidence);
        }
    }
    if package_versions.len() == 1 {
        let expected = package_versions.iter().next().expect("one package version");
        if expected != &current_binary {
            return Err(anyhow!(
                "health.json current binary value does not match graph package version"
            ));
        }
    }
    if !packages.is_empty() && host_binary_files.is_empty() {
        return Err(anyhow!("health.json host binary files missing"));
    }
    let has_host_sbom_attestation = attestations
        .iter()
        .any(|item| item.get("name").and_then(|value| value.as_str()) == Some("github_attestations_host_sbom"));
    if has_host_sbom_attestation && host_sboms.is_empty() {
        return Err(anyhow!("health.json host SBOM evidence missing"));
    }
    for host_file in host_binary_files {
        let url = host_file
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json host binary file url missing"))?;
        let Some(expected) = expected_host_files.get(url) else {
            continue;
        };
        let expected_name = expected
            .get("name")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("graph host binary name missing for {url}"))?;
        if host_file.get("name").and_then(|value| value.as_str()) != Some(expected_name) {
            return Err(anyhow!("health.json host binary name mismatch for {url}"));
        }
        let expected_sha256 = require_json_string(expected, &["digest", "sha256"])?;
        let is_digest = |value: &str| value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit());
        if !is_digest(&expected_sha256) {
            return Err(anyhow!(
                "channel manifest host binary {} has malformed sha256",
                expected_name
            ));
        }
        if !is_digest(&require_json_string(expected, &["digest", "blake3"])?) {
            return Err(anyhow!(
                "channel manifest host binary {} has malformed blake3",
                expected_name
            ));
        }
        if host_file.get("sha256").and_then(|value| value.as_str()) != Some(expected_sha256.as_str()) {
            return Err(anyhow!("health.json host binary sha256 mismatch for {url}"));
        }
        let expected_blake3 = require_json_string(expected, &["digest", "blake3"])?;
        if host_file.get("blake3").and_then(|value| value.as_str()) != Some(expected_blake3.as_str()) {
            return Err(anyhow!("health.json host binary blake3 mismatch for {url}"));
        }
        let expected_bytes = expected
            .get("bytes")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| anyhow!("graph host binary bytes missing for {url}"))?;
        if host_file.get("size").and_then(|value| value.as_u64()) != Some(expected_bytes) {
            return Err(anyhow!("health.json host binary size mismatch for {url}"));
        }
    }
    for sbom in host_sboms {
        let sbom_url = sbom
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json host SBOM evidence missing url"))?;
        if sbom.get("name").and_then(|value| value.as_str()) != Some("capsem-sbom.spdx.json") {
            return Err(anyhow!("health.json host SBOM evidence name mismatch for {sbom_url}"));
        }
        let Some(host_binary) = host_binary_files
            .iter()
            .find(|item| item.get("url").and_then(|value| value.as_str()) == Some(sbom_url))
        else {
            return Err(anyhow!(
                "health.json host SBOM evidence {sbom_url} missing from host binary files"
            ));
        };
        if host_binary.get("name").and_then(|value| value.as_str()) != Some("capsem-sbom.spdx.json") {
            return Err(anyhow!(
                "health.json host SBOM evidence binary name mismatch for {sbom_url}"
            ));
        }
    }

    let mut current_asset_subjects = BTreeSet::new();
    for file in asset_files {
        let url = file
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json asset file url missing"))?;
        current_asset_subjects.insert(url.to_string());
        let size = file
            .get("size")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| anyhow!("health.json asset file size missing for {url}"))?;
        let hash = file
            .get("hash")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json asset file hash missing for {url}"))?;
        if url.starts_with('/') {
            let local_path = dist.join(url.trim_start_matches('/'));
            let bytes =
                fs::read(&local_path).with_context(|| format!("read asset channel blob {}", local_path.display()))?;
            if bytes.len() as u64 != size {
                return Err(anyhow!("asset channel blob {} size mismatch", local_path.display()));
            }
            if blake3::hash(&bytes).to_hex().as_str() != hash {
                return Err(anyhow!("asset channel blob {} hash mismatch", local_path.display()));
            }
            if file.get("logical_name").and_then(|value| value.as_str()) == Some("obom.cdx.json") {
                validate_vm_cyclonedx_obom_bytes(&bytes, &local_path)?;
            }
        }
    }
    if asset_files
        .iter()
        .any(|item| item.get("logical_name").and_then(|value| value.as_str()) == Some("obom.cdx.json"))
        && vm_oboms.is_empty()
    {
        return Err(anyhow!("health.json missing VM OBOM evidence"));
    }

    let mut saw_host_sbom_attestation = false;
    let mut saw_vm_asset_attestation = false;
    let mut host_package_attestation_subjects = BTreeSet::new();
    for attestation in attestations {
        let attestation_name = attestation
            .get("name")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json attestation name missing"))?;
        if let Some((expected_scope, expected_workflow)) = expected_attestation_rail(attestation_name) {
            let scope = attestation
                .get("scope")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json attestation scope missing"))?;
            if scope != expected_scope {
                return Err(anyhow!(
                    "health.json {} scope mismatch",
                    attestation_rail_label(attestation_name)
                ));
            }
            let workflow = attestation
                .get("workflow")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json attestation workflow missing"))?;
            if workflow != expected_workflow {
                return Err(anyhow!(
                    "health.json {} workflow mismatch",
                    attestation_rail_label(attestation_name)
                ));
            }
        }
        attestation
            .get("predicate_type")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("health.json attestation predicate_type missing"))?;
        let verify_command = attestation
            .get("verify_command")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("health.json attestation verify_command missing"))?;
        if !verify_command.contains("gh attestation verify") {
            return Err(anyhow!(
                "health.json attestation verify_command must use gh attestation verify"
            ));
        }
        if attestation_name == "github_attestations_host_sbom" {
            saw_host_sbom_attestation = true;
            let predicate_url = attestation
                .get("predicate_url")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json host SBOM attestation predicate_url missing"))?;
            if !host_sboms
                .iter()
                .any(|item| item.get("url").and_then(|value| value.as_str()) == Some(predicate_url))
            {
                return Err(anyhow!(
                    "health.json host SBOM attestation predicate {predicate_url} missing from host SBOM evidence"
                ));
            }
        }
        if attestation_name == "github_attestations_vm_assets" {
            let predicate_url = attestation
                .get("predicate_url")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("health.json VM asset attestation predicate_url missing"))?;
            if !vm_oboms
                .iter()
                .any(|item| item.get("url").and_then(|value| value.as_str()) == Some(predicate_url))
            {
                return Err(anyhow!(
                    "health.json VM asset attestation predicate {predicate_url} missing from VM OBOM evidence"
                ));
            }
        }
        let subjects = attestation
            .get("subjects")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("health.json attestation subjects missing"))?;
        if subjects.is_empty() {
            return Err(anyhow!("health.json attestation subjects empty"));
        }
        for subject in subjects {
            let subject_url = subject
                .as_str()
                .ok_or_else(|| anyhow!("health.json attestation subject is not a string"))?;
            if attestation_name == "github_attestations_host" {
                host_package_attestation_subjects.insert(subject_url.to_string());
            }
            if current_asset_subjects.contains(subject_url) {
                saw_vm_asset_attestation = true;
            }
        }
    }
    if !host_sboms.is_empty() && !saw_host_sbom_attestation {
        return Err(anyhow!("health.json host SBOM attestation evidence missing"));
    }
    for subject in &package_urls {
        if !host_package_attestation_subjects.contains(subject) {
            return Err(anyhow!(
                "health.json host package attestation subjects missing {subject}"
            ));
        }
    }
    if !current_asset_subjects.is_empty() && !saw_vm_asset_attestation {
        return Err(anyhow!("health.json VM asset attestation evidence missing"));
    }
    Ok(())
}

/// health.json's runtime summary must name exactly the graph's runtime.
pub(super) fn validate_assets_channel_graph_health_runtime(
    manifest: &serde_json::Value,
    health: &serde_json::Value,
) -> Result<()> {
    let Some(runtime) = graph_runtime(manifest) else {
        if json_path(health, &["runtime"]).is_some_and(|runtime| !runtime.is_null()) {
            return Err(anyhow!("health.json publishes a runtime the graph does not carry"));
        }
        require_json_null(
            health,
            &["updates", "runtime", "current"],
            "health.json runtime update must be null before a runtime is published",
        )?;
        return require_json_str(
            health,
            &["updates", "runtime", "state"],
            "not_published",
            "health.json runtime update state mismatch",
        );
    };
    let revision = require_json_string(runtime, &["revision"])?;
    for (path, expected, message) in [
        (
            &["runtime", "revision"][..],
            revision.as_str(),
            "health.json runtime revision does not match manifest runtime",
        ),
        (&["runtime", "state"], "current", "health.json runtime state mismatch"),
        (
            &["runtime", "source"],
            "manifest.runtime",
            "health.json runtime source mismatch",
        ),
        (
            &["updates", "runtime", "latest"],
            revision.as_str(),
            "health.json runtime update latest target does not match manifest runtime",
        ),
        (
            &["updates", "runtime", "current"],
            revision.as_str(),
            "health.json runtime update current target does not match manifest runtime",
        ),
        (
            &["updates", "runtime", "state"],
            "current",
            "health.json runtime update state mismatch",
        ),
        (
            &["updates", "runtime", "source"],
            "manifest.runtime",
            "health.json runtime update source mismatch",
        ),
    ] {
        require_json_str(health, path, expected, message)?;
    }
    Ok(())
}

pub(super) fn validate_assets_channel_graph_index_state(
    index_html: &str,
    channel: &str,
    manifest: &serde_json::Value,
    health: &serde_json::Value,
) -> Result<()> {
    let generated_at = require_json_string(health, &["generated_at"])?;
    let manifest_version = require_json_string(manifest, &["version"])?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let expected = [
        ("generated timestamp", generated_at.as_str()),
        ("manifest version", manifest_version.as_str()),
        ("channel manifest", channel_manifest.as_str()),
    ];
    for (label, value) in expected {
        if !index_html.contains(&escape_html(value)) {
            return Err(anyhow!("asset channel index missing {label} {value}"));
        }
    }
    Ok(())
}

pub(super) fn validate_assets_channel_catalog_manifest_digest(
    dist: &Path,
    channel: &str,
    manifest_content: &str,
) -> Result<()> {
    let channels_path = dist.join("channels.json");
    let channels: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&channels_path).with_context(|| format!("read {}", channels_path.display()))?,
    )
    .with_context(|| format!("parse {}", channels_path.display()))?;
    let manifest_url = format!("/assets/{channel}/manifest.json");
    let records = channels
        .get("channels")
        .and_then(|value| value.get(channel))
        .and_then(|value| value.get("manifests"))
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("channels.json missing {channel} manifest records"))?;
    let record = records
        .iter()
        .find(|record| {
            record.get("status").and_then(|value| value.as_str()) == Some("current")
                && record.get("url").and_then(|value| value.as_str()) == Some(manifest_url.as_str())
        })
        .ok_or_else(|| anyhow!("channels.json missing current manifest record for {channel}"))?;
    let expected_sha256 = require_json_string(record, &["digest", "sha256"])?;
    let actual_sha256 = format!("{:x}", Sha256::digest(manifest_content.as_bytes()));
    if actual_sha256 != expected_sha256 {
        return Err(anyhow!("channels.json manifest sha256 mismatch"));
    }
    let expected_blake3 = require_json_string(record, &["digest", "blake3"])?;
    let actual_blake3 = blake3::hash(manifest_content.as_bytes()).to_hex().to_string();
    if actual_blake3 != expected_blake3 {
        return Err(anyhow!("channels.json manifest blake3 mismatch"));
    }
    Ok(())
}

pub(super) fn validate_assets_channel_graph_page_state(
    channel_html: &str,
    channel: &str,
    manifest: &serde_json::Value,
    health: &serde_json::Value,
) -> Result<()> {
    let generated_at = require_json_string(health, &["generated_at"])?;
    let manifest_version = require_json_string(manifest, &["version"])?;
    let current_binary = require_json_string(health, &["current", "binary"])?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let mut expected = vec![
        ("generated timestamp", generated_at.as_str()),
        ("manifest version", manifest_version.as_str()),
        ("channel manifest", channel_manifest.as_str()),
    ];
    if !require_json_array(health, &["evidence", "host_binary_files"])?.is_empty() {
        expected.push(("current binary", current_binary.as_str()));
    }
    for (label, value) in expected {
        if !channel_html.contains(&escape_html(value)) {
            return Err(anyhow!("asset channel page missing {label} {value}"));
        }
    }
    if let Some(runtime) = graph_runtime(manifest) {
        let revision = require_json_string(runtime, &["revision"])?;
        if !channel_html.contains(&escape_html(&revision)) {
            return Err(anyhow!("asset channel page missing runtime revision {revision}"));
        }
    }
    Ok(())
}

pub(super) fn root_health_belongs_to_other_channel(root_health_path: &Path, channel: &str) -> bool {
    let Ok(content) = fs::read_to_string(root_health_path) else {
        return false;
    };
    let Ok(health) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    health
        .get("channel")
        .and_then(|value| value.as_str())
        .is_some_and(|root_channel| root_channel != channel)
}

pub(super) fn validate_assets_channel_index_html(index_html: &str, channel: &str) -> Result<()> {
    let expected = [
        "Channels",
        "Manifest revision",
        "Updated",
        "Coverage",
        "/channels.json",
        "Manifest URL",
    ];
    for needle in expected {
        if !index_html.contains(needle) {
            return Err(anyhow!("asset channel index missing {needle}"));
        }
    }
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    if !index_html.contains(&channel_manifest) {
        return Err(anyhow!("asset channel index missing {channel_manifest}"));
    }
    for forbidden in ["Selected manifest", ">Status<", ">Records<"] {
        if index_html.contains(forbidden) {
            return Err(anyhow!("asset channel index still contains {forbidden}"));
        }
    }
    if index_html.contains(&format!("/manifests/{channel}/")) {
        return Err(anyhow!(
            "asset channel index must not publish legacy graph manifest URLs"
        ));
    }
    Ok(())
}

pub(super) fn validate_assets_channel_page_html(channel_html: &str, channel: &str) -> Result<()> {
    let expected = ["Current Manifest", "Manifest History", "Capsem Packages", "Runtime"];
    for needle in expected {
        if !channel_html.contains(needle) {
            return Err(anyhow!("asset channel page missing {needle}"));
        }
    }
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    if !channel_html.contains(&channel_manifest) {
        return Err(anyhow!("asset channel page missing {channel_manifest}"));
    }
    if channel_html.contains("Capsem Binaries") {
        return Err(anyhow!("asset channel page must not flatten package-owned binaries"));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn write_test_assets_channel_index_fixture(dist: &Path, channel: &str) -> Result<()> {
    let health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dist.join("health.json")).context("read test health.json")?)
            .context("parse test health.json")?;
    let channel_manifest = format!("/assets/{channel}/manifest.json");
    let manifest_path = dist.join(channel_manifest.trim_start_matches('/'));
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).with_context(|| format!("read test {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parse test {}", manifest_path.display()))?;
    let manifest_version = require_json_string(&manifest, &["version"])?;
    let generated_at = require_json_string(&health, &["generated_at"])?;
    let runtime_revision = graph_runtime(&manifest)
        .map(|runtime| require_json_string(runtime, &["revision"]))
        .transpose()?
        .unwrap_or_else(|| "not published".to_string());
    let asset_base = require_json_string(&health, &["urls", "asset_base"])?;
    let binary = require_json_string(&health, &["current", "binary"])?;
    let assets = require_json_string(&health, &["current", "assets"])?;
    let date = health
        .get("asset_releases")
        .and_then(|value| value.as_array())
        .and_then(|releases| releases.first())
        .and_then(|release| release.get("date"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let html = format!(
        "<!doctype html><html><body><main><h1>Capsem Release Channels</h1>\
        <h2>Channels</h2><h2>Manifest revision</h2><h2>Updated</h2><h2>Coverage</h2>\
        <a href=\"/channels.json\">/channels.json</a>\
        <p>Manifest URL <a href=\"{channel_manifest}\">{channel_manifest}</a></p>\
        <p>{manifest_version} {binary} {assets} {generated_at} {date}</p>\
        <p>Current asset base {asset_base}</p>\
        <p>{runtime_revision}</p><h2>Binaries</h2><h2>Runtime</h2>\
        <h2>Capsem Binaries</h2><h2>Asset Release History</h2></main></body></html>",
        channel_manifest = escape_html(&channel_manifest),
        manifest_version = escape_html(&manifest_version),
        binary = escape_html(&binary),
        assets = escape_html(&assets),
        generated_at = escape_html(&generated_at),
        date = escape_html(date),
        asset_base = escape_html(&asset_base),
        runtime_revision = escape_html(&runtime_revision),
    );
    fs::write(dist.join("index.html"), html).context("write test release index fixture")?;
    let channel_dir = dist.join("channels").join(channel);
    fs::create_dir_all(&channel_dir).with_context(|| format!("create test channel page {}", channel_dir.display()))?;
    let channel_html = format!(
        "<!doctype html><html><body><main><h1>{channel}</h1>\
        <h2>Current Manifest</h2><h2>Manifest History</h2><h2>Capsem Packages</h2>\
        <h3>Package target Linux arm64</h3>\
        <a href=\"/channels/{channel}/packages/capsem-test-arm64-deb/\">Package detail</a>\
        <h2>Runtime</h2><p>SBOM</p>\
        <p>{generated_at}</p><p>{manifest_version}</p><p>{binary}</p><p>{assets}</p>\
        <a href=\"{channel_manifest}\">{channel_manifest}</a>\
        <p>{runtime_revision}</p>\
        </main></body></html>",
        channel = escape_html(channel),
        generated_at = escape_html(&generated_at),
        manifest_version = escape_html(&manifest_version),
        binary = escape_html(&binary),
        assets = escape_html(&assets),
        channel_manifest = escape_html(&channel_manifest),
        runtime_revision = escape_html(&runtime_revision),
    );
    fs::write(channel_dir.join("index.html"), channel_html).context("write test release channel page fixture")
}

pub(super) fn expected_attestation_rail(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "github_attestations_host" => Some(("host_binaries", ".github/workflows/release.yaml")),
        "github_attestations_host_sbom" => Some(("host_sbom", ".github/workflows/release.yaml")),
        "github_attestations_vm_assets" => Some(("vm_assets", ".github/workflows/release-assets.yaml")),
        _ => None,
    }
}

pub(super) fn attestation_rail_label(name: &str) -> &'static str {
    match name {
        "github_attestations_host" => "host attestation",
        "github_attestations_host_sbom" => "host SBOM attestation",
        "github_attestations_vm_assets" => "VM asset attestation",
        _ => "attestation",
    }
}

pub(super) fn require_json_str(root: &serde_json::Value, path: &[&str], expected: &str, message: &str) -> Result<()> {
    if json_path(root, path).and_then(|value| value.as_str()) != Some(expected) {
        return Err(anyhow!("{message}"));
    }
    Ok(())
}

pub(super) fn require_json_bool(root: &serde_json::Value, path: &[&str], expected: bool, message: &str) -> Result<()> {
    if json_path(root, path).and_then(|value| value.as_bool()) != Some(expected) {
        return Err(anyhow!("{message}"));
    }
    Ok(())
}

pub(super) fn require_json_string(root: &serde_json::Value, path: &[&str]) -> Result<String> {
    json_path(root, path)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("health.json missing {}", path.join(".")))
}

pub(super) fn require_json_absent(root: &serde_json::Value, path: &[&str], message: &str) -> Result<()> {
    if json_path(root, path).is_some() {
        return Err(anyhow!("{message}"));
    }
    Ok(())
}

pub(super) fn require_json_null(value: &serde_json::Value, path: &[&str], message: &str) -> Result<()> {
    let actual = value
        .pointer(&format!("/{}", path.join("/")))
        .ok_or_else(|| anyhow!("health.json missing {}", path.join(".")))?;
    if !actual.is_null() {
        return Err(anyhow!("{message}: got {actual}"));
    }
    Ok(())
}

pub(super) fn require_json_array<'a>(root: &'a serde_json::Value, path: &[&str]) -> Result<&'a Vec<serde_json::Value>> {
    json_path(root, path)
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("health.json missing {}", path.join(".")))
}

pub(super) fn json_path<'a>(root: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    let mut value = root;
    for key in path {
        value = value.get(*key)?;
    }
    Some(value)
}

use super::*;

/// Convert the public release graph into the runtime-only v2 view in memory.
///
/// The installed `manifest.json` remains the exact public document. This
/// adapter exists only because the boot resolver needs the compact v2 asset
/// index; it must never be serialized back over the installed manifest.
pub(super) fn manifest_v2_from_release_graph(value: &serde_json::Value) -> Result<ManifestV2> {
    let state = release_graph_runtime_state(value)?.context("release graph contains no runtime")?;
    if state.status.eq_ignore_ascii_case("revoked") {
        bail!("release graph runtime {} is revoked", state.revision);
    }
    let runtime = &value["runtime"];
    let min_binary = runtime
        .get("min_capsem_version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut arches = HashMap::new();
    for architecture in runtime["architectures"].as_array().into_iter().flatten() {
        let arch = architecture["architecture"].as_str().unwrap_or_default();
        arches.insert(arch.to_string(), runtime_architecture_boot_assets(arch, architecture)?);
    }
    let asset_version = state.revision;

    let packages = value
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let binary_version = packages
        .iter()
        .filter(|package| package.get("status").and_then(serde_json::Value::as_str) != Some("revoked"))
        .find_map(|package| package.get("version").and_then(serde_json::Value::as_str))
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string();

    Ok(ManifestV2 {
        format: 2,
        catalogs: value
            .get("catalogs")
            .filter(|value| !value.is_null())
            .map(capsem_proto::catalogs::CatalogSources::deserialize)
            .transpose()
            .context("invalid release graph catalog sources")?,
        refresh_policy: value
            .get("refresh_policy")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("24h")
            .to_string(),
        asset_base: None,
        assets: AssetsSection {
            current: asset_version.clone(),
            releases: HashMap::from([(
                asset_version.clone(),
                AssetRelease {
                    date: String::new(),
                    deprecated: false,
                    deprecated_date: None,
                    min_binary,
                    arches,
                },
            )]),
        },
        binaries: BinariesSection {
            current: binary_version.clone(),
            releases: HashMap::from([(
                binary_version.clone(),
                BinaryRelease {
                    date: String::new(),
                    deprecated: false,
                    deprecated_date: None,
                    min_assets: asset_version,
                    version: binary_version,
                    files: Vec::new(),
                },
            )]),
        },
    })
}

/// One runtime architecture's boot assets: logical asset name -> entry.
fn runtime_architecture_boot_assets(
    arch: &str,
    architecture: &serde_json::Value,
) -> Result<HashMap<String, AssetEntry>> {
    let images = architecture
        .get("images")
        .and_then(serde_json::Value::as_array)
        .context("release graph runtime architecture is missing images")?;
    let mut assets = HashMap::new();
    for image in images
        .iter()
        .filter(|image| image.get("status").and_then(serde_json::Value::as_str) != Some("revoked"))
    {
        let Some(kind) = image.get("kind").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !matches!(kind, "kernel" | "initrd" | "rootfs") {
            continue;
        }
        let name = image
            .get("name")
            .and_then(serde_json::Value::as_str)
            .context("release graph image is missing name")?;
        let digest = image.get("digest").context("release graph image is missing digest")?;
        assets.insert(
            name.to_string(),
            AssetEntry {
                hash: digest
                    .get("blake3")
                    .and_then(serde_json::Value::as_str)
                    .context("release graph image is missing BLAKE3")?
                    .to_string(),
                sha256: digest
                    .get("sha256")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                size: image
                    .get("bytes")
                    .and_then(serde_json::Value::as_u64)
                    .context("release graph image is missing byte size")?,
            },
        );
    }
    for required in ["vmlinuz", "initrd.img", "rootfs.erofs"] {
        if !assets.contains_key(required) {
            bail!("release graph runtime architecture {arch} is missing {required}");
        }
    }
    Ok(assets)
}

/// Parse and fingerprint the runtime a public graph carries.
///
/// `None` means the graph publishes no runtime yet (a binary-only channel).
/// Semantically unordered artifact lists and object keys are canonicalized so
/// formatting or JSON key order cannot manufacture an update; evidence and
/// image identity are fingerprinted separately so a caller can tell an
/// evidence-only republish from new boot images.
pub fn release_graph_runtime_state(value: &serde_json::Value) -> Result<Option<ReleaseGraphRuntimeState>> {
    let runtime = match value.get("runtime") {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(runtime) => runtime,
    };
    ensure!(runtime.is_object(), "release graph runtime must be an object");
    let revision = runtime
        .get("revision")
        .and_then(serde_json::Value::as_str)
        .context("release graph runtime is missing revision")?
        .to_string();
    validate_version(&revision).with_context(|| format!("release graph runtime revision {revision} is invalid"))?;
    let status = runtime
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("current")
        .to_string();
    let revoked = status.eq_ignore_ascii_case("revoked");
    let architectures = runtime
        .get("architectures")
        .and_then(serde_json::Value::as_array)
        .context("release graph runtime is missing architectures")?;
    if architectures.is_empty() && !revoked {
        bail!("release graph runtime {revision} contains no architectures");
    }

    let mut architecture_names = BTreeSet::new();
    let mut evidence_scope = BTreeMap::new();
    let mut images_scope = BTreeMap::new();
    for architecture in architectures {
        let arch = architecture
            .get("architecture")
            .and_then(serde_json::Value::as_str)
            .context("release graph runtime architecture is missing architecture")?;
        validate_filename(arch).with_context(|| format!("invalid runtime architecture key {arch:?}"))?;
        if !architecture_names.insert(arch.to_string()) {
            bail!("release graph runtime repeats architecture {arch}");
        }
        let image_revision = architecture
            .get("image_revision")
            .and_then(serde_json::Value::as_str)
            .context("release graph runtime architecture is missing image_revision")?;
        if image_revision != revision {
            bail!(
                "release graph runtime architecture {arch} image revision {image_revision} is not runtime {revision}"
            );
        }
        let evidence = canonical_active_artifacts(
            architecture.get("evidence"),
            &format!("release graph runtime architecture {arch} evidence"),
            false,
        )?;
        let images = canonical_active_artifacts(
            architecture.get("images"),
            &format!("release graph runtime architecture {arch} images"),
            true,
        )?;
        if !revoked {
            let image_kinds: BTreeSet<&str> = images
                .iter()
                .filter_map(|image| image.get("kind").and_then(serde_json::Value::as_str))
                .collect();
            for required in ["kernel", "initrd", "rootfs"] {
                if !image_kinds.contains(required) {
                    bail!("release graph runtime architecture {arch} is missing {required} image");
                }
            }
        }
        evidence_scope.insert(arch.to_string(), evidence);
        images_scope.insert(arch.to_string(), images);
    }

    Ok(Some(ReleaseGraphRuntimeState {
        evidence_revision: state_revision(
            "evidence",
            &serde_json::to_value(&evidence_scope).context("serialize runtime evidence state")?,
        ),
        images_revision: state_revision(
            "images",
            &serde_json::json!({"revision": revision, "architectures": images_scope}),
        ),
        revision,
        status,
        architectures: architecture_names.into_iter().collect(),
    }))
}

fn canonical_active_artifacts(
    value: Option<&serde_json::Value>,
    context: &str,
    validate_images: bool,
) -> Result<Vec<serde_json::Value>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let artifacts = value
        .as_array()
        .with_context(|| format!("{context} must be an array"))?;
    let mut canonical = Vec::new();
    for artifact in artifacts.iter().filter(|artifact| {
        artifact
            .get("status")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|status| !status.eq_ignore_ascii_case("revoked"))
    }) {
        let digest = artifact
            .get("digest")
            .with_context(|| format!("{context} artifact is missing digest"))?;
        let blake3 = digest
            .get("blake3")
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("{context} artifact is missing BLAKE3"))?;
        validate_hash(blake3)?;
        let sha256 = digest
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("{context} artifact is missing SHA-256"))?;
        validate_sha256(sha256)?;
        artifact
            .get("bytes")
            .and_then(serde_json::Value::as_u64)
            .with_context(|| format!("{context} artifact is missing byte size"))?;
        if validate_images {
            artifact
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("{context} image is missing kind"))?;
            artifact
                .get("name")
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("{context} image is missing name"))?;
        }
        canonical.push(artifact.clone());
    }
    canonical.sort_by_key(canonical_json);
    Ok(canonical)
}

fn state_revision(prefix: &str, value: &serde_json::Value) -> String {
    let hash = hash(canonical_json(value).as_bytes()).to_hex().to_string();
    format!("{prefix}-{}", &hash[..16])
}

fn canonical_json(value: &serde_json::Value) -> String {
    fn write(value: &serde_json::Value, output: &mut String) {
        match value {
            serde_json::Value::Null => output.push_str("null"),
            serde_json::Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
            serde_json::Value::Number(value) => output.push_str(&value.to_string()),
            serde_json::Value::String(value) => {
                output.push_str(&serde_json::to_string(value).expect("JSON strings serialize"));
            }
            serde_json::Value::Array(values) => {
                output.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    write(value, output);
                }
                output.push(']');
            }
            serde_json::Value::Object(values) => {
                output.push('{');
                let mut entries = values.iter().collect::<Vec<_>>();
                entries.sort_by_key(|(key, _)| key.as_str());
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push_str(&serde_json::to_string(key).expect("JSON keys serialize"));
                    output.push(':');
                    write(value, output);
                }
                output.push('}');
            }
        }
    }

    let mut output = String::new();
    write(value, &mut output);
    output
}

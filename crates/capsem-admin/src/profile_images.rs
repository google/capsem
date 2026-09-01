use super::*;

pub(super) fn image_build_workspace_path(source_profile: &ProfileConfigFile, arch: Option<&str>) -> PathBuf {
    PathBuf::from("cache/target/build")
        .join("image-workspace")
        .join(&source_profile.id)
        .join(arch.unwrap_or("all"))
}

pub(super) fn image_build_command(args: ImageBuildArgs) -> Result<()> {
    let source_profile = load_profile(&args.profile)?;
    let workspace = image_build_workspace_path(&source_profile, args.arch.as_deref());
    let workspace_report = materialize_image_workspace(&ImageWorkspaceArgs {
        profile: args.profile.clone(),
        config_root: args.config_root.clone(),
        guest_dir: args.guest_dir.clone(),
        output: workspace,
        arch: args.arch.clone(),
        json: true,
    })?;
    let plan = image_build_plan(&ImageBuildArgs {
        profile: PathBuf::from(&workspace_report.profile_path),
        config_root: PathBuf::from(&workspace_report.config_root),
        guest_dir: PathBuf::from(&workspace_report.workspace).join("guest"),
        output: args.output.clone(),
        arch: args.arch.clone(),
        template: args.template,
        clean: args.clean,
        json: args.json,
    })?;
    if plan.clean {
        clean_image_outputs(&plan)?;
    }
    for command in &plan.commands {
        run_command(command)?;
    }
    print_image_build_plan(&plan, args.json)?;
    Ok(())
}

pub(super) fn image_workspace_command(args: ImageWorkspaceArgs) -> Result<()> {
    let json = args.json;
    let report = materialize_image_workspace(&args)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "profile {} rev {} -> {}",
            report.profile_id, report.profile_revision, report.workspace
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProfilePinMode {
    Source,
    Materialized,
}

pub(super) fn validate_profile(path: &Path, config_root: Option<&Path>) -> Result<ProfileValidationReport> {
    validate_profile_with_pin_mode(path, config_root, ProfilePinMode::Source)
}

pub(super) fn validate_materialized_profile(
    path: &Path,
    config_root: Option<&Path>,
) -> Result<ProfileValidationReport> {
    validate_profile_with_pin_mode(path, config_root, ProfilePinMode::Materialized)
}

pub(super) fn validate_profile_with_pin_mode(
    path: &Path,
    config_root: Option<&Path>,
    pin_mode: ProfilePinMode,
) -> Result<ProfileValidationReport> {
    let content = fs::read_to_string(path).with_context(|| format!("read profile {}", path.display()))?;
    let profile: ProfileConfigFile =
        toml::from_str(&content).with_context(|| format!("parse profile {}", path.display()))?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", path.display()))?;
    match pin_mode {
        ProfilePinMode::Source => ensure_source_profile_unpinned(&profile, path)?,
        ProfilePinMode::Materialized => ensure_materialized_profile_pinned(&profile, path)?,
    }

    let config_root = match config_root {
        Some(root) => root.to_path_buf(),
        None => infer_config_root(path)?,
    };
    let rules = profile
        .compile_security_rule_set_from_files(&config_root, SecurityRuleSource::User)
        .map_err(|error| {
            anyhow!(
                "compile profile rule files for {} with config root {}: {error}",
                path.display(),
                config_root.display()
            )
        })?;

    Ok(ProfileValidationReport {
        schema: "capsem.admin.profile_validation.v1",
        ok: true,
        profile_id: profile.id,
        path: path.display().to_string(),
        config_root: config_root.display().to_string(),
        compiled_rules: rules.rules().len(),
    })
}

pub(super) fn ensure_source_profile_unpinned(profile: &ProfileConfigFile, path: &Path) -> Result<()> {
    let location = path.display();
    if profile.obom.is_some() {
        return Err(anyhow!(
            "source profile {location} must not contain generated obom pins"
        ));
    }
    for (arch, assets) in &profile.assets.arch {
        for (kind, descriptor) in [
            ("kernel", &assets.kernel),
            ("initrd", &assets.initrd),
            ("rootfs", &assets.rootfs),
        ] {
            if descriptor.hash.is_some() || descriptor.size.is_some() {
                return Err(anyhow!(
                    "source profile {location} must not contain hash/size pins for assets.arch.{arch}.{kind}"
                ));
            }
        }
    }
    for (kind, descriptor) in profile.files.iter() {
        if descriptor.hash.is_some() || descriptor.size.is_some() {
            return Err(anyhow!(
                "source profile {location} must not contain hash/size pins for files.{kind}"
            ));
        }
    }
    Ok(())
}

pub(super) fn ensure_materialized_profile_pinned(profile: &ProfileConfigFile, path: &Path) -> Result<()> {
    let location = path.display();
    for (arch, assets) in &profile.assets.arch {
        for (kind, descriptor) in [
            ("kernel", &assets.kernel),
            ("initrd", &assets.initrd),
            ("rootfs", &assets.rootfs),
        ] {
            descriptor
                .resolved_hash(&format!("profile.assets.arch.{arch}.{kind}"))
                .map_err(|error| anyhow!("materialized profile {location}: {error}"))?;
            descriptor
                .resolved_size(&format!("profile.assets.arch.{arch}.{kind}"))
                .map_err(|error| anyhow!("materialized profile {location}: {error}"))?;
        }
    }
    for (kind, descriptor) in profile.files.iter() {
        descriptor
            .resolved_hash(&format!("profile.files.{kind}"))
            .map_err(|error| anyhow!("materialized profile {location}: {error}"))?;
        descriptor
            .resolved_size(&format!("profile.files.{kind}"))
            .map_err(|error| anyhow!("materialized profile {location}: {error}"))?;
    }
    Ok(())
}

pub(super) fn check_profile(args: &ProfileCheckArgs) -> Result<ProfileCheckReport> {
    let validation = validate_profile(&args.path, args.config_root.as_deref())?;
    let profile = load_profile(&args.path)?;
    let config_root = match &args.config_root {
        Some(root) => root.clone(),
        None => infer_config_root(&args.path)?,
    };
    let assets: Vec<LocalAssetCheckReport> = Vec::new();
    let arches = selected_profile_arches(&profile, args.arch.as_deref())?;
    for arch in arches {
        let arch_assets = profile
            .assets
            .arch
            .get(&arch)
            .expect("arch came from selected_profile_arches");
        for descriptor in [&arch_assets.kernel, &arch_assets.initrd, &arch_assets.rootfs] {
            if descriptor.url.starts_with("file://") && (descriptor.hash.is_some() || descriptor.size.is_some()) {
                return Err(anyhow!(
                    "source profile {} must not contain file:// asset pins for {arch}/{}",
                    args.path.display(),
                    descriptor.name
                ));
            }
        }
    }
    fail_if_local_asset_checks_failed("profile file:// asset pin check", &assets)?;
    let profile_files = check_profile_payload_files(&profile, &config_root)?;
    fail_if_local_asset_checks_failed("profile payload file pin check", &profile_files)?;
    Ok(ProfileCheckReport {
        schema: "capsem.admin.profile_check.v1",
        ok: true,
        validation,
        assets,
        profile_files,
    })
}

pub(super) fn check_profile_payload_files(
    profile: &ProfileConfigFile,
    config_root: &Path,
) -> Result<Vec<LocalAssetCheckReport>> {
    let mut reports = Vec::new();
    for (kind, descriptor) in profile.files.iter() {
        let path = config_root.join(&descriptor.path);
        let present = path.is_file();
        reports.push(LocalAssetCheckReport {
            arch: "profile".to_string(),
            logical_name: kind.to_string(),
            expected_hash: "unpinned-source".to_string(),
            expected_size: 0,
            path: Some(path.display().to_string()),
            present,
            size_ok: None,
            blake3_ok: None,
        });
        if !present {
            continue;
        }
        validate_profile_payload_semantics(kind, &path)?;
        if kind == "root_manifest" {
            reports.extend(check_profile_root_manifest(&path)?);
        }
    }
    Ok(reports)
}

pub(super) fn validate_profile_payload_semantics(kind: &str, path: &Path) -> Result<()> {
    match kind {
        "mcp" => validate_profile_mcp_file(path),
        "apt_packages" | "python_requirements" | "npm_packages" => read_profile_package_lines(path).map(|_| ()),
        "python_requirements_lock" => validate_python_requirements_lock(path, None).map(|_| ()),
        "npm_package_lock" => validate_npm_package_lock(path, None).map(|_| ()),
        _ => Ok(()),
    }
}

pub(super) fn normalized_python_name(name: &str) -> String {
    name.to_ascii_lowercase().replace(['_', '.'], "-")
}

pub(super) fn exact_python_dependencies(packages: &[String]) -> Result<BTreeMap<String, String>> {
    packages
        .iter()
        .map(|package| {
            let (name, version) = package
                .split_once("==")
                .ok_or_else(|| anyhow!("Python requirement {package} must select one exact version"))?;
            if name.is_empty()
                || version.is_empty()
                || version.contains(['=', ';', '@'])
                || version.contains(char::is_whitespace)
            {
                return Err(anyhow!("Python requirement {package} must select one exact version"));
            }
            Ok((normalized_python_name(name), version.to_string()))
        })
        .collect()
}

pub(super) fn validate_python_requirements_lock(
    path: &Path,
    expected: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, String>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read Python requirements lock {}", path.display()))?;
    let mut dependencies = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut current_hashed = false;
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        if line.starts_with(char::is_whitespace) {
            if line.trim_start().starts_with("--hash=sha256:") {
                current_hashed = true;
            }
            continue;
        }
        if current.is_some() && !current_hashed {
            return Err(anyhow!(
                "Python requirements lock {} entry {} has no SHA-256 hash",
                path.display(),
                current.as_deref().unwrap_or_default()
            ));
        }
        let (name, version) = line
            .trim_end_matches(" \\")
            .split_once("==")
            .ok_or_else(|| anyhow!("Python lock entry {line} is not exact"))?;
        let normalized = normalized_python_name(name);
        dependencies.insert(normalized.clone(), version.to_string());
        current = Some(normalized);
        current_hashed = line.contains("--hash=sha256:");
    }
    if dependencies.is_empty() {
        return Err(anyhow!(
            "Python requirements lock {} must contain exact requirements",
            path.display()
        ));
    }
    if !current_hashed {
        return Err(anyhow!(
            "Python requirements lock {} entry {} has no SHA-256 hash",
            path.display(),
            current.as_deref().unwrap_or_default()
        ));
    }
    if expected.is_some_and(|wanted| {
        wanted
            .iter()
            .any(|(name, version)| dependencies.get(name) != Some(version))
    }) {
        return Err(anyhow!(
            "Python requirements lock {} does not match the profile's exact direct packages",
            path.display()
        ));
    }
    Ok(dependencies)
}

pub(super) fn exact_npm_dependencies(packages: &[String]) -> Result<BTreeMap<String, String>> {
    packages
        .iter()
        .map(|package| {
            let (name, version) = package
                .rsplit_once('@')
                .ok_or_else(|| anyhow!("npm package {package} must select one exact version"))?;
            if name.is_empty()
                || version.is_empty()
                || version
                    .chars()
                    .next()
                    .is_some_and(|prefix| matches!(prefix, '^' | '~' | '>' | '<' | '='))
                || version.contains(char::is_whitespace)
            {
                return Err(anyhow!("npm package {package} must select one exact version"));
            }
            Ok((name.to_string(), version.to_string()))
        })
        .collect()
}

pub(super) fn validate_npm_package_lock(
    path: &Path,
    expected: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, String>> {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(path).with_context(|| format!("read npm lock {}", path.display()))?)
            .with_context(|| format!("parse npm lock {}", path.display()))?;
    if value.get("lockfileVersion").and_then(serde_json::Value::as_u64) != Some(3) {
        return Err(anyhow!("npm lock {} must use lockfileVersion 3", path.display()));
    }
    let packages = value
        .get("packages")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow!("npm lock {} has no packages object", path.display()))?;
    let root = packages
        .get("")
        .and_then(|entry| entry.get("dependencies"))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow!("npm lock {} has no root dependencies", path.display()))?;
    let dependencies: BTreeMap<String, String> = root
        .iter()
        .map(|(name, version)| {
            let version = version
                .as_str()
                .ok_or_else(|| anyhow!("npm lock dependency {name} is not a string"))?;
            Ok((name.clone(), version.to_string()))
        })
        .collect::<Result<_>>()?;
    if expected.is_some_and(|wanted| wanted != &dependencies) {
        return Err(anyhow!(
            "npm lock {} does not match the profile's exact direct packages",
            path.display()
        ));
    }
    for (name, package) in packages.iter().filter(|(name, _)| !name.is_empty()) {
        let integrity = package
            .get("integrity")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !integrity.starts_with("sha512-") {
            return Err(anyhow!(
                "npm lock {} package {name} has no SHA-512 integrity",
                path.display()
            ));
        }
    }
    Ok(dependencies)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileMcpJsonConfig {
    #[serde(rename = "mcpServers")]
    mcp_servers: BTreeMap<String, serde_json::Value>,
}

pub(super) fn validate_profile_mcp_file(path: &Path) -> Result<()> {
    let content = fs::read_to_string(path).with_context(|| format!("read profile MCP config {}", path.display()))?;
    let config: ProfileMcpJsonConfig =
        serde_json::from_str(&content).with_context(|| format!("parse profile MCP config {}", path.display()))?;
    if config.mcp_servers.is_empty() {
        return Err(anyhow!(
            "profile MCP config {} must declare at least one server",
            path.display()
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileRootManifest {
    pub(super) format: String,
    pub(super) files: Vec<ProfileRootManifestFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileRootManifestFile {
    pub(super) path: String,
    pub(super) hash: String,
    pub(super) size: u64,
}

pub(super) fn check_profile_root_manifest(path: &Path) -> Result<Vec<LocalAssetCheckReport>> {
    let content = fs::read_to_string(path).with_context(|| format!("read profile root manifest {}", path.display()))?;
    let manifest: ProfileRootManifest =
        serde_json::from_str(&content).with_context(|| format!("parse profile root manifest {}", path.display()))?;
    if manifest.format != "capsem.profile-root.v1" {
        return Err(anyhow!(
            "profile root manifest {} has unsupported format {}",
            path.display(),
            manifest.format
        ));
    }
    if manifest.files.is_empty() {
        return Err(anyhow!(
            "profile root manifest {} must list at least one file",
            path.display()
        ));
    }
    let root_dir = path
        .parent()
        .ok_or_else(|| anyhow!("profile root manifest has no parent: {}", path.display()))?
        .join("root");
    let mut listed_files = BTreeSet::new();
    for entry in &manifest.files {
        validate_relative_manifest_path("profile root manifest file", &entry.path)?;
        if !listed_files.insert(entry.path.clone()) {
            return Err(anyhow!(
                "profile root manifest {} lists duplicate payload file {}",
                path.display(),
                entry.path
            ));
        }
        if entry.size == 0 {
            return Err(anyhow!(
                "profile root manifest {} entry {} has zero size",
                path.display(),
                entry.path
            ));
        }
    }
    let actual_files = collect_profile_root_files(&root_dir)?;
    if let Some(unlisted) = actual_files.difference(&listed_files).next() {
        return Err(anyhow!(
            "unlisted profile root payload file {} under {}",
            unlisted,
            root_dir.display()
        ));
    }
    if let Some(missing) = listed_files.difference(&actual_files).next() {
        return Err(anyhow!(
            "profile root manifest {} lists missing payload file {}",
            path.display(),
            missing
        ));
    }
    let mut reports = Vec::new();
    for entry in manifest.files {
        validate_profile_root_payload_content(&root_dir.join(&entry.path), &entry.path)?;
        reports.push(check_exact_local_asset(
            &root_dir.join(&entry.path),
            "profile-root",
            &entry.path,
            normalized_blake3(&entry.hash)?,
            entry.size,
        )?);
    }
    Ok(reports)
}

pub(super) fn validate_profile_root_payload_content(path: &Path, logical_name: &str) -> Result<()> {
    let payload = fs::read(path).with_context(|| format!("read profile root payload {}", path.display()))?;
    let text = String::from_utf8_lossy(&payload);
    for forbidden in [
        "127.0.0.1:11434",
        "localhost:11434",
        "CAPSEM_MOCK_SERVER",
        "\"provider\": \"ollama\"",
        "\"baseUrl\": \"http://127.0.0.1:11434\"",
    ] {
        if text.contains(forbidden) {
            return Err(anyhow!(
                "profile root provider override {} contains forbidden test/local provider fragment {}",
                logical_name,
                forbidden
            ));
        }
    }
    Ok(())
}

pub(super) fn collect_profile_root_files(root_dir: &Path) -> Result<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    if !root_dir.is_dir() {
        return Err(anyhow!("profile root directory {} is missing", root_dir.display()));
    }
    collect_profile_root_files_into(root_dir, root_dir, &mut files)?;
    Ok(files)
}

pub(super) fn collect_profile_root_files_into(
    root_dir: &Path,
    current: &Path,
    files: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in fs::read_dir(current).with_context(|| format!("read profile root directory {}", current.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", current.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat profile root payload {}", path.display()))?;
        if file_type.is_dir() {
            collect_profile_root_files_into(root_dir, &path, files)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(anyhow!("profile root payload {} is not a regular file", path.display()));
        }
        let relative = path
            .strip_prefix(root_dir)
            .with_context(|| format!("strip profile root prefix for {}", path.display()))?;
        let relative = relative.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        validate_relative_manifest_path("profile root payload file", &relative)?;
        files.insert(relative);
    }
    Ok(())
}

pub(super) fn materialize_profile_config(args: &ProfileMaterializeArgs) -> Result<ProfileMaterializeReport> {
    check_config_root(&args.config_root, args.arch.as_deref())?;
    if args.output_root == args.config_root {
        return Err(anyhow!(
            "output root {} must differ from source config root {}",
            args.output_root.display(),
            args.config_root.display()
        ));
    }
    if args.clean && args.output_root.exists() {
        fs::remove_dir_all(&args.output_root).with_context(|| format!("remove {}", args.output_root.display()))?;
    }
    if !args.output_root.exists() {
        copy_dir_recursive(&args.config_root, &args.output_root)?;
    }

    let mut profile = load_profile(&args.profile)?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", args.profile.display()))?;

    let selected_arches = selected_profile_arches(&profile, args.arch.as_deref())?;
    if args.arch.is_some() {
        profile
            .assets
            .arch
            .retain(|arch, _| selected_arches.iter().any(|selected| selected == arch));
    }

    let manifest_bytes = read_manifest_url(&args.manifest)?;
    let manifest_content = std::str::from_utf8(&manifest_bytes)
        .with_context(|| format!("manifest URL did not return UTF-8 JSON: {}", args.manifest))?;
    let materialize_manifest = load_profile_materialize_manifest(
        &args.manifest,
        manifest_content,
        &manifest_bytes,
        &profile.id,
        &selected_arches,
    )
    .with_context(|| format!("parse manifest from {}", args.manifest))?;
    let manifest = materialize_manifest.manifest;
    let current_release = manifest.assets.releases.get(&manifest.assets.current).ok_or_else(|| {
        anyhow!(
            "manifest {} current asset release {} is missing",
            args.manifest,
            manifest.assets.current
        )
    })?;

    copy_profile_descriptor_files(&profile, &args.config_root, &args.output_root)?;
    materialize_profile_file_descriptors(&mut profile, &args.output_root)?;

    let mut materialized_assets = Vec::new();
    let mut materialized_obom = Vec::new();
    for arch in selected_arches {
        let manifest_assets = current_release.arches.get(&arch).ok_or_else(|| {
            anyhow!(
                "manifest {} current release {} does not contain profile arch {arch}",
                args.manifest,
                manifest.assets.current
            )
        })?;
        let asset_inputs = ProfileAssetMaterializeInputs {
            assets_dir: &args.assets_dir,
            manifest_url: &args.manifest,
            asset_version: &manifest.assets.current,
            arch: &arch,
            manifest_assets,
            asset_urls: &materialize_manifest.asset_urls,
        };
        let rootfs_hash = {
            let profile_assets = profile
                .assets
                .arch
                .get_mut(&arch)
                .expect("arch came from selected_profile_arches");
            materialize_profile_asset_descriptor(asset_inputs, &mut profile_assets.kernel, &mut materialized_assets)?;
            materialize_profile_asset_descriptor(asset_inputs, &mut profile_assets.initrd, &mut materialized_assets)?;
            materialize_profile_asset_descriptor(asset_inputs, &mut profile_assets.rootfs, &mut materialized_assets)?;
            profile_assets
                .rootfs
                .hash
                .clone()
                .ok_or_else(|| anyhow!("materialized {arch} rootfs hash is unresolved"))?
        };
        materialize_profile_obom_descriptor(asset_inputs, rootfs_hash, &mut profile, &mut materialized_obom)?;
    }

    let output_profile_path = args.output_root.join("profiles").join(&profile.id).join("profile.toml");
    fs::create_dir_all(
        output_profile_path
            .parent()
            .ok_or_else(|| anyhow!("materialized profile path has no parent"))?,
    )
    .with_context(|| format!("create parent for {}", output_profile_path.display()))?;
    fs::write(
        &output_profile_path,
        toml::to_string_pretty(&profile).context("serialize materialized profile")?,
    )
    .with_context(|| format!("write {}", output_profile_path.display()))?;

    let manifest_output = args.output_root.join("assets/manifest.json");
    fs::create_dir_all(
        manifest_output
            .parent()
            .ok_or_else(|| anyhow!("materialized manifest path has no parent"))?,
    )
    .with_context(|| format!("create parent for {}", manifest_output.display()))?;
    fs::write(&manifest_output, &materialize_manifest.manifest_bytes)
        .with_context(|| format!("write {}", manifest_output.display()))?;

    let copied_validation = validate_materialized_profile(&output_profile_path, Some(&args.output_root))?;
    if copied_validation.profile_id != profile.id {
        return Err(anyhow!(
            "materialized profile id drifted: expected {}, got {}",
            profile.id,
            copied_validation.profile_id
        ));
    }

    Ok(ProfileMaterializeReport {
        schema: "capsem.admin.profile_materialize.v1",
        ok: true,
        profile_id: profile.id,
        profile_revision: profile.revision,
        source_config_root: args.config_root.display().to_string(),
        output_config_root: args.output_root.display().to_string(),
        profile_path: output_profile_path.display().to_string(),
        manifest: manifest_output.display().to_string(),
        asset_version: manifest.assets.current,
        materialized_assets,
        materialized_obom,
    })
}

pub(super) struct ProfileMaterializeManifest {
    manifest: ManifestV2,
    manifest_bytes: Vec<u8>,
    asset_urls: HashMap<(String, String), String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseChannelProfileManifest {
    profiles: BTreeMap<String, ReleaseChannelProfileDocument>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseChannelProfileDocument {
    revision: String,
    #[serde(default)]
    status: String,
    /// The binary floor the graph profile declares.
    ///
    /// Read here because the runtime manifest this projects into has the same
    /// field under another name, and dropping it produced a manifest that
    /// re-authoring a channel from could not name a floor at all: the glow-up
    /// hands its paired runtime manifest back to `assets channel build`, which
    /// copies `min_binary` onto every graph profile as `min_capsem_version`,
    /// and `record-binary` then refuses an empty semver. That is the whole
    /// release-lane glow-up, failing on a field nobody had carried across.
    #[serde(default)]
    min_capsem_version: String,
    #[serde(default)]
    architectures: Vec<ReleaseChannelProfileArchitecture>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseChannelProfileArchitecture {
    architecture: String,
    #[serde(default)]
    images: Vec<ReleaseChannelProfileArtifact>,
    #[serde(default)]
    evidence: Vec<ReleaseChannelProfileArtifact>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseChannelProfileArtifact {
    kind: String,
    #[serde(default)]
    name: String,
    url: String,
    #[serde(rename = "bytes")]
    size: u64,
    digest: ReleaseChannelProfileDigest,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseChannelProfileDigest {
    sha256: String,
    blake3: String,
}

pub(super) fn load_profile_materialize_manifest(
    manifest_url: &str,
    manifest_content: &str,
    manifest_bytes: &[u8],
    profile_id: &str,
    selected_arches: &[String],
) -> Result<ProfileMaterializeManifest> {
    let release_graph = serde_json::from_str::<serde_json::Value>(manifest_content)
        .ok()
        .and_then(|document| document.get("profiles").cloned())
        .is_some_and(|profiles| profiles.is_object());
    if release_graph {
        return profile_materialize_manifest_from_release_channel(
            manifest_url,
            manifest_content,
            profile_id,
            selected_arches,
        );
    }
    if let Ok(manifest) = ManifestV2::from_json(manifest_content) {
        return Ok(ProfileMaterializeManifest {
            manifest,
            manifest_bytes: manifest_bytes.to_vec(),
            asset_urls: HashMap::new(),
        });
    }

    profile_materialize_manifest_from_release_channel(manifest_url, manifest_content, profile_id, selected_arches)
}

pub(super) fn profile_materialize_manifest_from_release_channel(
    manifest_url: &str,
    manifest_content: &str,
    profile_id: &str,
    selected_arches: &[String],
) -> Result<ProfileMaterializeManifest> {
    let document: ReleaseChannelProfileManifest =
        serde_json::from_str(manifest_content).context("failed to parse release channel profile manifest JSON")?;
    let profile = document
        .profiles
        .get(profile_id)
        .ok_or_else(|| anyhow!("release channel manifest does not contain profile {profile_id}"))?;
    if release_channel_status_is_revoked(&profile.status) {
        anyhow::bail!("release channel profile {profile_id} is revoked");
    }

    let mut arch_entries: HashMap<String, HashMap<String, capsem_assets::asset_manager::AssetEntry>> = HashMap::new();
    let mut asset_urls = HashMap::new();
    for arch in selected_arches {
        let architecture = profile
            .architectures
            .iter()
            .find(|candidate| candidate.architecture == *arch)
            .ok_or_else(|| anyhow!("release channel profile {profile_id} does not contain architecture {arch}"))?;
        let mut assets = HashMap::new();
        for artifact in architecture.images.iter().chain(architecture.evidence.iter()) {
            if release_channel_status_is_revoked(&artifact.status) {
                continue;
            }
            let Some(logical_name) = release_channel_profile_artifact_logical_name(artifact) else {
                continue;
            };
            validate_release_channel_digest(&artifact.digest)
                .with_context(|| format!("validate {arch} {logical_name} digest"))?;
            assets.insert(
                logical_name.to_string(),
                capsem_assets::asset_manager::AssetEntry {
                    hash: artifact.digest.blake3.clone(),
                    sha256: artifact.digest.sha256.clone(),
                    size: artifact.size,
                },
            );
            asset_urls.insert(
                (arch.clone(), logical_name.to_string()),
                resolve_release_channel_artifact_url(manifest_url, &artifact.url)?,
            );
        }
        for required in ["vmlinuz", "initrd.img", "rootfs.erofs"] {
            if !assets.contains_key(required) {
                anyhow::bail!(
                    "release channel profile {profile_id} revision {} architecture {arch} missing {required} image",
                    profile.revision
                );
            }
        }
        arch_entries.insert(arch.clone(), assets);
    }

    let binary_version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = ManifestV2 {
        format: 2,
        refresh_policy: "24h".to_string(),
        asset_base: None,
        assets: capsem_assets::asset_manager::AssetsSection {
            current: profile.revision.clone(),
            releases: HashMap::from([(
                profile.revision.clone(),
                capsem_assets::asset_manager::AssetRelease {
                    date: String::new(),
                    deprecated: false,
                    deprecated_date: None,
                    // The graph profile's declared floor, under the name a
                    // runtime manifest gives it. See the field's own comment.
                    min_binary: profile.min_capsem_version.clone(),
                    arches: arch_entries,
                },
            )]),
        },
        binaries: capsem_assets::asset_manager::BinariesSection {
            current: binary_version.clone(),
            releases: HashMap::from([(
                binary_version.clone(),
                capsem_assets::asset_manager::BinaryRelease {
                    date: String::new(),
                    deprecated: false,
                    deprecated_date: None,
                    min_assets: profile.revision.clone(),
                    version: binary_version,
                    files: Vec::new(),
                },
            )]),
        },
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).context("serialize converted asset manifest")?;
    let manifest_json = std::str::from_utf8(&manifest_bytes).context("converted manifest JSON is UTF-8")?;
    ManifestV2::from_json(manifest_json).context("validate converted asset manifest")?;

    Ok(ProfileMaterializeManifest {
        manifest,
        manifest_bytes,
        asset_urls,
    })
}

pub(super) fn release_channel_profile_artifact_logical_name(
    artifact: &ReleaseChannelProfileArtifact,
) -> Option<&'static str> {
    match artifact.kind.as_str() {
        "kernel" => Some("vmlinuz"),
        "initrd" => Some("initrd.img"),
        "rootfs" => Some("rootfs.erofs"),
        "abom" => Some("abom.cdx.json"),
        "obom" => Some("obom.cdx.json"),
        "software_inventory" => Some("software-inventory.json"),
        _ if artifact.name == "obom.cdx.json" => Some("obom.cdx.json"),
        _ => None,
    }
}

pub(super) fn release_channel_status_is_revoked(status: &str) -> bool {
    status.eq_ignore_ascii_case("revoked")
}

pub(super) fn validate_release_channel_digest(digest: &ReleaseChannelProfileDigest) -> Result<()> {
    if !is_64_hex(&digest.blake3) {
        anyhow::bail!("profile image blake3 must be a 64-character hex digest");
    }
    if !is_64_hex(&digest.sha256) {
        anyhow::bail!("profile image sha256 must be a 64-character hex digest");
    }
    Ok(())
}

pub(super) fn is_64_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

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

pub(super) fn materialize_profile_asset_descriptor(
    inputs: ProfileAssetMaterializeInputs<'_>,
    descriptor: &mut capsem_core::net::policy_config::ProfileAssetDescriptor,
    reports: &mut Vec<ProfileMaterializedAssetReport>,
) -> Result<()> {
    let entry = inputs.manifest_assets.get(&descriptor.name).ok_or_else(|| {
        anyhow!(
            "manifest current release arch {} is missing {}",
            inputs.arch,
            descriptor.name
        )
    })?;
    descriptor.url = materialized_profile_asset_url(inputs, &descriptor.name, &entry.hash, entry.size)?;
    descriptor.hash = Some(format!("blake3:{}", entry.hash));
    descriptor.size = Some(entry.size);
    reports.push(ProfileMaterializedAssetReport {
        arch: inputs.arch.to_string(),
        logical_name: descriptor.name.clone(),
        url: descriptor.url.clone(),
        hash: descriptor.hash.clone().expect("materialized asset hash was just set"),
        size: descriptor.size.expect("materialized asset size was just set"),
    });
    Ok(())
}

pub(super) fn materialize_profile_file_descriptors(profile: &mut ProfileConfigFile, config_root: &Path) -> Result<()> {
    fn pin(
        descriptor: Option<&mut capsem_core::net::policy_config::ProfileFileDescriptor>,
        config_root: &Path,
    ) -> Result<()> {
        let Some(descriptor) = descriptor else {
            return Ok(());
        };
        let path = config_root.join(&descriptor.path);
        let hash = hash_file(&path).with_context(|| format!("hash profile payload {}", path.display()))?;
        let size = fs::metadata(&path)
            .with_context(|| format!("stat profile payload {}", path.display()))?
            .len();
        if size == 0 {
            return Err(anyhow!("profile payload {} must not be empty", path.display()));
        }
        descriptor.hash = Some(format!("blake3:{hash}"));
        descriptor.size = Some(size);
        Ok(())
    }

    pin(profile.files.enforcement.as_mut(), config_root)?;
    pin(profile.files.detection.as_mut(), config_root)?;
    pin(profile.files.mcp.as_mut(), config_root)?;
    pin(profile.files.apt_packages.as_mut(), config_root)?;
    pin(profile.files.python_requirements.as_mut(), config_root)?;
    pin(profile.files.python_requirements_lock.as_mut(), config_root)?;
    pin(profile.files.npm_packages.as_mut(), config_root)?;
    pin(profile.files.npm_package_lock.as_mut(), config_root)?;
    pin(profile.files.build.as_mut(), config_root)?;
    pin(profile.files.tips.as_mut(), config_root)?;
    pin(profile.files.root_manifest.as_mut(), config_root)?;
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct ProfileAssetMaterializeInputs<'a> {
    assets_dir: &'a Path,
    manifest_url: &'a str,
    asset_version: &'a str,
    arch: &'a str,
    manifest_assets: &'a std::collections::HashMap<String, capsem_assets::asset_manager::AssetEntry>,
    asset_urls: &'a HashMap<(String, String), String>,
}

pub(super) fn materialize_profile_obom_descriptor(
    inputs: ProfileAssetMaterializeInputs<'_>,
    rootfs_hash: String,
    profile: &mut ProfileConfigFile,
    reports: &mut Vec<ProfileMaterializedObomReport>,
) -> Result<()> {
    let Some(entry) = inputs.manifest_assets.get("obom.cdx.json") else {
        return Ok(());
    };
    let obom_url = materialized_profile_asset_url(inputs, "obom.cdx.json", &entry.hash, entry.size)?;
    let parsed_obom_url =
        reqwest::Url::parse(&obom_url).with_context(|| format!("parse materialized OBOM URL {obom_url}"))?;
    let (generator, generator_version) = if parsed_obom_url.scheme() == "file" {
        let obom_path = parsed_obom_url
            .to_file_path()
            .map_err(|_| anyhow!("materialized OBOM file URL must be absolute: {obom_url}"))?;
        let obom_path = obom_path
            .canonicalize()
            .with_context(|| format!("canonicalize {}", obom_path.display()))?;
        read_obom_generator(&obom_path)?
    } else {
        ("remote".to_string(), "unknown".to_string())
    };
    let descriptor = ProfileObomDescriptor {
        name: "obom.cdx.json".to_string(),
        url: obom_url,
        hash: format!("blake3:{}", entry.hash),
        size: entry.size,
        generator: generator.clone(),
        generator_version: generator_version.clone(),
    };
    profile
        .obom
        .get_or_insert_with(|| ProfileObomConfig {
            format: "cyclonedx-obom.v1".to_string(),
            arch: BTreeMap::new(),
        })
        .arch
        .insert(inputs.arch.to_string(), descriptor.clone());
    reports.push(ProfileMaterializedObomReport {
        arch: inputs.arch.to_string(),
        url: descriptor.url,
        hash: descriptor.hash,
        size: descriptor.size,
        generator,
        generator_version,
        rootfs_hash,
        scope: "base_image",
    });
    Ok(())
}

pub(super) fn materialized_profile_asset_url(
    inputs: ProfileAssetMaterializeInputs<'_>,
    logical_name: &str,
    hash: &str,
    size: u64,
) -> Result<String> {
    if let Some(url) = inputs
        .asset_urls
        .get(&(inputs.arch.to_string(), logical_name.to_string()))
    {
        return Ok(url.clone());
    }
    materialized_asset_url(
        inputs.assets_dir,
        inputs.manifest_url,
        inputs.asset_version,
        inputs.arch,
        logical_name,
        hash,
        size,
    )
}

pub(super) fn materialized_asset_url(
    assets_dir: &Path,
    manifest_url: &str,
    asset_version: &str,
    arch: &str,
    logical_name: &str,
    hash: &str,
    size: u64,
) -> Result<String> {
    if let Some(asset_base_url) = capsem_assets::asset_manager::asset_release_base_url_from_manifest_url(manifest_url) {
        return Ok(capsem_assets::asset_manager::asset_download_url_with_base(
            &asset_base_url,
            asset_version,
            arch,
            logical_name,
        ));
    }

    let check = check_local_asset(assets_dir, arch, logical_name, hash, size)?;
    fail_if_local_asset_checks_failed("profile materialize asset check", &[check])?;
    let asset_path = assets_dir.join(arch).join(logical_name);
    let asset_path = asset_path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", asset_path.display()))?;
    Ok(format!("file://{}", asset_path.display()))
}

pub(super) fn read_obom_generator(path: &Path) -> Result<(String, String)> {
    let content = fs::read_to_string(path).with_context(|| format!("read CycloneDX OBOM {}", path.display()))?;
    let document: serde_json::Value =
        serde_json::from_str(&content).with_context(|| format!("parse CycloneDX OBOM {}", path.display()))?;
    let metadata = document
        .get("metadata")
        .ok_or_else(|| anyhow!("CycloneDX OBOM {} is missing metadata", path.display()))?;
    let tools = metadata
        .get("tools")
        .ok_or_else(|| anyhow!("CycloneDX OBOM {} is missing metadata.tools", path.display()))?;
    let candidates: Vec<&serde_json::Value> = tools
        .get("components")
        .and_then(|components| components.as_array())
        .map(|components| components.iter().collect())
        .or_else(|| tools.as_array().map(|tools| tools.iter().collect()))
        .unwrap_or_default();
    let preferred = candidates
        .iter()
        .copied()
        .find(|candidate| {
            candidate
                .get("name")
                .and_then(|name| name.as_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("cdxgen"))
        })
        .or_else(|| {
            candidates.iter().copied().find(|candidate| {
                candidate.get("name").and_then(|name| name.as_str()).is_some()
                    && candidate.get("version").and_then(|version| version.as_str()).is_some()
            })
        })
        .ok_or_else(|| {
            anyhow!(
                "CycloneDX OBOM {} must record a generator name and version in metadata.tools",
                path.display()
            )
        })?;
    let name = preferred
        .get("name")
        .and_then(|name| name.as_str())
        .ok_or_else(|| anyhow!("CycloneDX OBOM {} generator is missing name", path.display()))?;
    let version = preferred
        .get("version")
        .and_then(|version| version.as_str())
        .ok_or_else(|| anyhow!("CycloneDX OBOM {} generator is missing version", path.display()))?;
    Ok((name.to_string(), version.to_string()))
}

pub(super) fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).with_context(|| format!("create {}", destination.display()))?;
    for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", source_path.display()))?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
            }
            fs::copy(&source_path, &destination_path)
                .with_context(|| format!("copy {} to {}", source_path.display(), destination_path.display()))?;
        }
    }
    Ok(())
}

pub(super) fn load_profile(path: &Path) -> Result<ProfileConfigFile> {
    let content = fs::read_to_string(path).with_context(|| format!("read profile {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("parse profile {}", path.display()))
}

pub(super) fn validate_settings(path: &Path) -> Result<SettingsValidationReport> {
    let content = fs::read_to_string(path).with_context(|| format!("read settings {}", path.display()))?;
    let settings: SettingsConfigFile =
        toml::from_str(&content).with_context(|| format!("parse settings {}", path.display()))?;
    settings
        .validate()
        .map_err(|error| anyhow!("validate settings {}: {error}", path.display()))?;
    Ok(SettingsValidationReport {
        schema: "capsem.admin.settings_validation.v1",
        ok: true,
        path: path.display().to_string(),
        app: SettingsAppReport {
            auto_update: settings.app.auto_update,
            notifications: settings.app.notifications,
            start_service_at_login: settings.app.start_service_at_login,
        },
        appearance: SettingsAppearanceReport {
            theme: settings.appearance.theme,
            font_size: settings.appearance.font_size,
            reduced_motion: settings.appearance.reduced_motion,
        },
    })
}

impl SettingsConfigFile {
    fn validate(&self) -> Result<(), String> {
        match self.appearance.theme.as_str() {
            "system" | "light" | "dark" => {}
            other => {
                return Err(format!("appearance.theme must be system, light, or dark, got {other}"));
            }
        }
        if !(8..=32).contains(&self.appearance.font_size) {
            return Err(format!(
                "appearance.font_size must be between 8 and 32, got {}",
                self.appearance.font_size
            ));
        }
        Ok(())
    }
}

pub(super) fn image_build_plan(args: &ImageBuildArgs) -> Result<ImageBuildPlan> {
    let profile = load_profile(&args.profile)?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", args.profile.display()))?;
    profile
        .compile_security_rule_set_from_files(&args.config_root, SecurityRuleSource::User)
        .map_err(|error| {
            anyhow!(
                "compile profile rule files for {} with config root {}: {error}",
                args.profile.display(),
                args.config_root.display()
            )
        })?;

    let mut arches = profile.assets.arch.keys().cloned().collect::<Vec<_>>();
    arches.sort();
    if let Some(arch) = &args.arch {
        if !profile.assets.arch.contains_key(arch) {
            return Err(anyhow!("profile {} does not define assets for arch {arch}", profile.id));
        }
        arches = vec![arch.clone()];
    }
    if arches.is_empty() {
        return Err(anyhow!("profile {} defines no asset architectures", profile.id));
    }

    let mut arch_plans = Vec::new();
    let mut commands = Vec::new();
    for arch in &arches {
        let assets = profile.assets.arch.get(arch).expect("arch came from profile asset map");
        arch_plans.push(ImageBuildArchPlan {
            arch: arch.clone(),
            kernel: assets.kernel.name.clone(),
            initrd: assets.initrd.name.clone(),
            rootfs: assets.rootfs.name.clone(),
        });
        if matches!(args.template, ImageBuildTemplate::All | ImageBuildTemplate::Kernel) {
            commands.push(CommandReport {
                step: "kernel".to_string(),
                arch: Some(arch.clone()),
                env: BTreeMap::new(),
                argv: vec![
                    "uv".to_string(),
                    "run".to_string(),
                    "python".to_string(),
                    "-m".to_string(),
                    "capsem_builder.image.image_build_backend".to_string(),
                    args.guest_dir.display().to_string(),
                    "--arch".to_string(),
                    arch.clone(),
                    "--template".to_string(),
                    "kernel".to_string(),
                    "--output".to_string(),
                    format!("{}/", args.output.display()),
                ],
            });
        }
        if matches!(args.template, ImageBuildTemplate::All | ImageBuildTemplate::Rootfs) {
            let mut env = BTreeMap::new();
            env.insert("CAPSEM_BUILD_EXPERIMENTAL_EROFS".to_string(), "1".to_string());
            env.insert("CAPSEM_BUILD_EROFS_COMPRESSION".to_string(), "lz4hc".to_string());
            env.insert("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL".to_string(), "12".to_string());
            commands.push(CommandReport {
                step: "rootfs".to_string(),
                arch: Some(arch.clone()),
                env,
                argv: vec![
                    "uv".to_string(),
                    "run".to_string(),
                    "python".to_string(),
                    "-m".to_string(),
                    "capsem_builder.image.image_build_backend".to_string(),
                    args.guest_dir.display().to_string(),
                    "--arch".to_string(),
                    arch.clone(),
                    "--template".to_string(),
                    "rootfs".to_string(),
                    "--output".to_string(),
                    format!("{}/", args.output.display()),
                ],
            });
        }
    }
    if !matches!(args.template, ImageBuildTemplate::Kernel) {
        commands.push(manifest_generate_command_report(&ManifestGenerateArgs {
            assets_dir: args.output.clone(),
            version: None,
            json: false,
        }));
    }

    Ok(ImageBuildPlan {
        schema: "capsem.admin.image_build_plan.v1",
        profile_id: profile.id,
        profile_revision: profile.revision,
        guest_dir: args.guest_dir.display().to_string(),
        output: args.output.display().to_string(),
        clean: args.clean,
        template: match args.template {
            ImageBuildTemplate::All => "all",
            ImageBuildTemplate::Kernel => "kernel",
            ImageBuildTemplate::Rootfs => "rootfs",
        },
        arches: arch_plans,
        commands,
    })
}

#[cfg(test)]
pub(super) fn verify_image_outputs(args: &ImageVerifyArgs) -> Result<ImageVerifyReport> {
    let profile = load_profile(&args.profile)?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", args.profile.display()))?;
    profile
        .compile_security_rule_set_from_files(&args.config_root, SecurityRuleSource::User)
        .map_err(|error| {
            anyhow!(
                "compile profile rule files for {} with config root {}: {error}",
                args.profile.display(),
                args.config_root.display()
            )
        })?;

    let manifest_path = args
        .manifest
        .clone()
        .unwrap_or_else(|| args.output.join("manifest.json"));
    let manifest = load_manifest(&manifest_path)?;
    let current_release = manifest.assets.releases.get(&manifest.assets.current).ok_or_else(|| {
        anyhow!(
            "manifest {} current asset release {} is missing",
            manifest_path.display(),
            manifest.assets.current
        )
    })?;

    let mut arches = Vec::new();
    for arch in selected_profile_arches(&profile, args.arch.as_deref())? {
        let manifest_assets = current_release.arches.get(&arch).ok_or_else(|| {
            anyhow!(
                "manifest {} current release {} does not contain profile arch {arch}",
                manifest_path.display(),
                manifest.assets.current
            )
        })?;
        let profile_assets = profile
            .assets
            .arch
            .get(&arch)
            .expect("arch came from selected_profile_arches");
        let mut asset_reports = Vec::new();
        for descriptor in [&profile_assets.kernel, &profile_assets.initrd, &profile_assets.rootfs] {
            let entry = manifest_assets.get(&descriptor.name).ok_or_else(|| {
                anyhow!(
                    "manifest {} current release {} arch {arch} is missing {}",
                    manifest_path.display(),
                    manifest.assets.current,
                    descriptor.name
                )
            })?;
            asset_reports.push(check_local_asset(
                &args.output,
                &arch,
                &descriptor.name,
                &entry.hash,
                entry.size,
            )?);
        }
        fail_if_local_asset_checks_failed("image output verify", &asset_reports)?;
        arches.push(ImageVerifyArchReport {
            arch,
            assets: asset_reports,
        });
    }

    Ok(ImageVerifyReport {
        schema: "capsem.admin.image_verify.v1",
        ok: true,
        profile_id: profile.id,
        profile_revision: profile.revision,
        output: args.output.display().to_string(),
        manifest: manifest_path.display().to_string(),
        arches,
    })
}

pub(super) fn materialize_image_workspace(args: &ImageWorkspaceArgs) -> Result<ImageWorkspaceReport> {
    check_config_root(&args.config_root, args.arch.as_deref())?;
    check_profile(&ProfileCheckArgs {
        path: args.profile.clone(),
        config_root: Some(args.config_root.clone()),
        arch: args.arch.clone(),
        json: true,
    })?;
    let profile = load_profile(&args.profile)?;
    profile
        .validate()
        .map_err(|error| anyhow!("validate profile {}: {error}", args.profile.display()))?;
    profile
        .compile_security_rule_set_from_files(&args.config_root, SecurityRuleSource::User)
        .map_err(|error| {
            anyhow!(
                "compile profile rule files for {} with config root {}: {error}",
                args.profile.display(),
                args.config_root.display()
            )
        })?;
    let arches = selected_profile_arches(&profile, args.arch.as_deref())?;

    let workspace = &args.output;
    if workspace.exists() {
        fs::remove_dir_all(workspace)
            .with_context(|| format!("remove stale image workspace {}", workspace.display()))?;
    }
    let workspace_config_root = workspace.join("config");
    let workspace_guest_dir = workspace.join("guest");
    let workspace_profile_path = workspace_config_root
        .join("profiles")
        .join(&profile.id)
        .join("profile.toml");
    let workspace_rules_root = workspace_config_root.join("profiles").join(&profile.id);
    fs::create_dir_all(
        workspace_profile_path
            .parent()
            .expect("workspace profile path has parent"),
    )
    .with_context(|| format!("create {}", workspace_profile_path.display()))?;
    fs::create_dir_all(&workspace_rules_root).with_context(|| format!("create {}", workspace_rules_root.display()))?;

    let profile_toml = fs::read(&args.profile).with_context(|| format!("read {}", args.profile.display()))?;
    fs::write(&workspace_profile_path, &profile_toml)
        .with_context(|| format!("write {}", workspace_profile_path.display()))?;

    let mut rule_files = Vec::new();
    copy_profile_rule_file(
        &args.config_root,
        &workspace_config_root,
        profile.rule_files.enforcement.as_deref(),
        "enforcement",
        &mut rule_files,
    )?;
    copy_profile_rule_file(
        &args.config_root,
        &workspace_config_root,
        profile.rule_files.sigma.as_deref(),
        "sigma",
        &mut rule_files,
    )?;
    copy_profile_descriptor_files(&profile, &args.config_root, &workspace_config_root)?;
    materialize_profile_guest_inputs(&profile, &args.config_root, &args.guest_dir, &workspace_guest_dir)?;

    let copied_check = check_profile(&ProfileCheckArgs {
        path: workspace_profile_path.clone(),
        config_root: Some(workspace_config_root.clone()),
        arch: args.arch.clone(),
        json: true,
    })?;
    if copied_check.validation.profile_id != profile.id {
        return Err(anyhow!(
            "workspace profile id drifted: expected {}, got {}",
            profile.id,
            copied_check.validation.profile_id
        ));
    }

    let plan = image_build_plan(&ImageBuildArgs {
        profile: workspace_profile_path.clone(),
        config_root: workspace_config_root.clone(),
        guest_dir: workspace_guest_dir,
        output: workspace.join("assets"),
        arch: args.arch.clone(),
        template: ImageBuildTemplate::All,
        clean: false,
        json: true,
    })?;
    let build_plan_path = workspace.join("build-plan.json");
    fs::write(&build_plan_path, serde_json::to_vec_pretty(&plan)?)
        .with_context(|| format!("write {}", build_plan_path.display()))?;

    let report = ImageWorkspaceReport {
        schema: "capsem.admin.image_workspace.v1",
        ok: true,
        profile_id: profile.id,
        profile_revision: profile.revision,
        workspace: workspace.display().to_string(),
        config_root: workspace_config_root.display().to_string(),
        profile_path: workspace_profile_path.display().to_string(),
        profile_blake3: blake3::hash(&profile_toml).to_hex().to_string(),
        build_plan_path: build_plan_path.display().to_string(),
        rule_files,
        arches: plan
            .arches
            .into_iter()
            .filter(|arch| arches.iter().any(|selected| selected == &arch.arch))
            .collect(),
    };
    fs::write(workspace.join("workspace.json"), serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("write {}", workspace.join("workspace.json").display()))?;
    Ok(report)
}

pub(super) fn copy_profile_descriptor_files(
    profile: &ProfileConfigFile,
    source_config_root: &Path,
    destination_config_root: &Path,
) -> Result<()> {
    for (kind, descriptor) in profile.files.iter() {
        validate_relative_manifest_path("profile file descriptor path", &descriptor.path)?;
        let source = source_config_root.join(&descriptor.path);
        let destination = destination_config_root.join(&descriptor.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::copy(&source, &destination)
            .with_context(|| format!("copy profile {kind} {} to {}", source.display(), destination.display()))?;

        if kind == "root_manifest" {
            let source_root = source
                .parent()
                .ok_or_else(|| anyhow!("profile root manifest has no parent"))?
                .join("root");
            let destination_root = destination
                .parent()
                .ok_or_else(|| anyhow!("workspace profile root manifest has no parent"))?
                .join("root");
            if destination_root.exists() {
                fs::remove_dir_all(&destination_root)
                    .with_context(|| format!("remove {}", destination_root.display()))?;
            }
            copy_dir_recursive(&source_root, &destination_root)?;
        }
    }
    Ok(())
}

pub(super) fn materialize_profile_guest_inputs(
    profile: &ProfileConfigFile,
    config_root: &Path,
    source_guest_dir: &Path,
    workspace_guest_dir: &Path,
) -> Result<()> {
    let source_config = config_root.join("docker").join("image");
    let workspace_config = workspace_guest_dir.join("config");
    fs::create_dir_all(&workspace_config).with_context(|| format!("create {}", workspace_config.display()))?;
    for relative in ["build.toml", "manifest.toml"] {
        let source = source_config.join(relative);
        let destination = workspace_config.join(relative);
        fs::copy(&source, &destination)
            .with_context(|| format!("copy {} to {}", source.display(), destination.display()))?;
    }
    copy_dir_recursive(&source_config.join("kernel"), &workspace_config.join("kernel"))?;
    copy_dir_recursive(&source_config.join("security"), &workspace_config.join("security"))?;
    copy_dir_recursive(&source_config.join("vm"), &workspace_config.join("vm"))?;
    write_profile_vm_resources_toml(&workspace_config.join("vm").join("resources.toml"), profile)?;
    copy_dir_recursive(
        &source_guest_dir.join("artifacts"),
        &workspace_guest_dir.join("artifacts"),
    )?;

    let packages_dir = workspace_config.join("packages");
    fs::create_dir_all(&packages_dir).with_context(|| format!("create {}", packages_dir.display()))?;
    if let Some(descriptor) = profile.files.apt_packages.as_ref() {
        let packages = read_profile_package_lines(&config_root.join(&descriptor.path))?;
        write_profile_package_toml(
            &packages_dir.join("apt.toml"),
            "apt",
            "System Packages",
            "apt",
            "apt-get install -y --no-install-recommends",
            &packages,
        )?;
    }
    if let (Some(descriptor), Some(lock_descriptor)) = (
        profile.files.python_requirements.as_ref(),
        profile.files.python_requirements_lock.as_ref(),
    ) {
        let packages = read_profile_package_lines(&config_root.join(&descriptor.path))?;
        write_profile_package_toml(
            &packages_dir.join("python.toml"),
            "python",
            "Python Packages",
            "uv",
            "uv pip install --system --break-system-packages",
            &packages,
        )?;
        let lock_source = config_root.join(&lock_descriptor.path);
        let expected = exact_python_dependencies(&packages)?;
        validate_python_requirements_lock(&lock_source, Some(&expected))?;
        fs::copy(&lock_source, packages_dir.join("python-requirements.lock"))
            .with_context(|| format!("copy Python requirements lock {}", lock_source.display()))?;
    }
    if let (Some(descriptor), Some(lock_descriptor)) = (
        profile.files.npm_packages.as_ref(),
        profile.files.npm_package_lock.as_ref(),
    ) {
        let packages = read_profile_package_lines(&config_root.join(&descriptor.path))?;
        write_profile_package_toml(
            &packages_dir.join("npm.toml"),
            "npm",
            "Node Packages",
            "npm",
            "npm install -g --prefix /opt/ai-clis",
            &packages,
        )?;
        let expected = exact_npm_dependencies(&packages)?;
        let lock_source = config_root.join(&lock_descriptor.path);
        validate_npm_package_lock(&lock_source, Some(&expected))?;
        fs::copy(&lock_source, packages_dir.join("npm-package-lock.json"))
            .with_context(|| format!("copy npm package lock {}", lock_source.display()))?;
        fs::write(
            packages_dir.join("npm-package.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "name": "capsem-profile-ai-clis",
                "private": true,
                "dependencies": expected,
            }))?,
        )?;
    }
    if let Some(descriptor) = profile.files.build.as_ref() {
        let source = config_root.join(&descriptor.path);
        let destination = workspace_guest_dir.join("profile-build.sh");
        fs::copy(&source, &destination)
            .with_context(|| format!("copy {} to {}", source.display(), destination.display()))?;
    }
    if let Some(descriptor) = profile.files.tips.as_ref() {
        let source = config_root.join(&descriptor.path);
        let artifacts_dir = workspace_guest_dir.join("artifacts");
        fs::create_dir_all(&artifacts_dir).with_context(|| format!("create {}", artifacts_dir.display()))?;
        fs::copy(&source, artifacts_dir.join("tips.txt"))
            .with_context(|| format!("copy profile tips {}", source.display()))?;
    }
    if let Some(descriptor) = profile.files.root_manifest.as_ref() {
        let manifest_path = config_root.join(&descriptor.path);
        let source_root = manifest_path
            .parent()
            .ok_or_else(|| anyhow!("profile root manifest has no parent"))?
            .join("root");
        copy_dir_recursive(&source_root, &workspace_guest_dir.join("profile-root"))?;
    }
    Ok(())
}

pub(super) fn write_profile_vm_resources_toml(path: &Path, profile: &ProfileConfigFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let content = format!(
        "[resources]\n\
         cpu_count = {}\n\
         ram_gb = {}\n\
         scratch_disk_size_gb = {}\n\
         log_bodies = false\n\
         max_body_capture = 4096\n\
         retention_days = 30\n\
         max_sessions = 100\n\
         min_content_sessions = 25\n\
         max_disk_gb = 100\n\
         terminated_retention_days = 365\n",
        profile.vm.cpu_count, profile.vm.ram_gb, profile.vm.scratch_disk_size_gb
    );
    fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

pub(super) fn read_profile_package_lines(path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(path).with_context(|| format!("read package list {}", path.display()))?;
    let packages = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if packages.is_empty() {
        return Err(anyhow!("package list {} is empty", path.display()));
    }
    Ok(packages)
}

pub(super) fn write_profile_package_toml(
    path: &Path,
    key: &str,
    name: &str,
    manager: &str,
    install_cmd: &str,
    packages: &[String],
) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("package TOML path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let packages = packages
        .iter()
        .map(|package| format!("    {package:?}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let content = format!(
        r#"[{key}]
name = {name:?}
manager = {manager:?}
install_cmd = {install_cmd:?}
packages = [
{packages},
]
"#
    );
    fs::write(path, content).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub(super) fn copy_profile_rule_file(
    config_root: &Path,
    workspace_config_root: &Path,
    rule_file: Option<&str>,
    kind: &'static str,
    reports: &mut Vec<ImageWorkspaceRuleFileReport>,
) -> Result<()> {
    let Some(rule_file) = rule_file else {
        return Ok(());
    };
    if Path::new(rule_file).is_absolute() {
        return Err(anyhow!(
            "image workspace requires profile rule files to be relative, got {rule_file}"
        ));
    }
    let source_path = resolve_profile_rule_file_path(config_root, rule_file);
    let destination_path = workspace_config_root.join(rule_file);
    fs::create_dir_all(
        destination_path
            .parent()
            .ok_or_else(|| anyhow!("rule file destination has no parent"))?,
    )
    .with_context(|| format!("create parent for {}", destination_path.display()))?;
    let bytes = fs::read(&source_path).with_context(|| format!("read rule file {}", source_path.display()))?;
    fs::write(&destination_path, &bytes).with_context(|| format!("write rule file {}", destination_path.display()))?;
    reports.push(ImageWorkspaceRuleFileReport {
        kind,
        source: source_path.display().to_string(),
        path: destination_path.display().to_string(),
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        size: bytes.len() as u64,
    });
    Ok(())
}

pub(super) fn manifest_generate_command_report(args: &ManifestGenerateArgs) -> CommandReport {
    let version_expr = match &args.version {
        Some(version) => format!("{version:?}"),
        None => "get_project_version(Path('.'))".to_string(),
    };
    CommandReport {
        step: "manifest".to_string(),
        arch: None,
        env: BTreeMap::new(),
        argv: vec![
            "uv".to_string(),
            "run".to_string(),
            "python3".to_string(),
            "-c".to_string(),
            format!(
                "from pathlib import Path; from capsem_builder.image.docker import generate_checksums, get_project_version; v = {version_expr}; generate_checksums(Path({:?}), v); print(f'manifest.json generated (v{{v}})')",
                args.assets_dir.display().to_string()
            ),
        ],
    }
}

pub(super) fn selected_profile_arches(profile: &ProfileConfigFile, only_arch: Option<&str>) -> Result<Vec<String>> {
    let mut arches = profile.assets.arch.keys().cloned().collect::<Vec<_>>();
    arches.sort();
    if let Some(arch) = only_arch {
        if !profile.assets.arch.contains_key(arch) {
            return Err(anyhow!("profile {} does not define assets for arch {arch}", profile.id));
        }
        arches = vec![arch.to_string()];
    }
    if arches.is_empty() {
        return Err(anyhow!("profile {} defines no asset architectures", profile.id));
    }
    Ok(arches)
}

pub(super) fn check_local_asset(
    assets_dir: &Path,
    arch: &str,
    logical_name: &str,
    expected_hash: &str,
    expected_size: u64,
) -> Result<LocalAssetCheckReport> {
    let path = assets_dir.join(arch).join(logical_name);
    check_exact_local_asset(&path, arch, logical_name, expected_hash, expected_size)
}

pub(super) fn check_exact_local_asset(
    path: &Path,
    arch: &str,
    logical_name: &str,
    expected_hash: &str,
    expected_size: u64,
) -> Result<LocalAssetCheckReport> {
    if !path.is_file() {
        return Ok(LocalAssetCheckReport {
            arch: arch.to_string(),
            logical_name: logical_name.to_string(),
            expected_hash: expected_hash.to_string(),
            expected_size,
            path: Some(path.display().to_string()),
            present: false,
            size_ok: None,
            blake3_ok: None,
        });
    }
    let metadata = fs::metadata(path).with_context(|| format!("stat local asset {}", path.display()))?;
    let digest = hash_file(path)?;
    Ok(LocalAssetCheckReport {
        arch: arch.to_string(),
        logical_name: logical_name.to_string(),
        expected_hash: expected_hash.to_string(),
        expected_size,
        path: Some(path.display().to_string()),
        present: true,
        size_ok: Some(metadata.len() == expected_size),
        blake3_ok: Some(digest == expected_hash),
    })
}

pub(super) fn fail_if_local_asset_checks_failed(context: &str, assets: &[LocalAssetCheckReport]) -> Result<()> {
    let failures = assets
        .iter()
        .filter(|asset| !asset.present || asset.size_ok.is_some_and(|ok| !ok) || asset.blake3_ok.is_some_and(|ok| !ok))
        .map(|asset| {
            format!(
                "{}:{} present={} size_ok={} blake3_ok={} path={}",
                asset.arch,
                asset.logical_name,
                asset.present,
                asset
                    .size_ok
                    .map(|ok| ok.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                asset
                    .blake3_ok
                    .map(|ok| ok.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                asset.path.as_deref().unwrap_or("n/a"),
            )
        })
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        return Err(anyhow!("{context} failed: {}", failures.join("; ")));
    }
    Ok(())
}

pub(super) fn normalized_blake3(value: &str) -> Result<&str> {
    value
        .strip_prefix("blake3:")
        .ok_or_else(|| anyhow!("expected blake3:<hash>, got {value}"))
}

pub(super) fn validate_relative_manifest_path(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('/')
        || value.starts_with("file://")
        || value.contains("..")
        || value.contains('\\')
        || value.trim() != value
    {
        return Err(anyhow!("{field} must be a relative path without traversal: {value}"));
    }
    Ok(())
}

pub(super) fn print_image_build_plan(plan: &ImageBuildPlan, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }
    println!(
        "profile {} rev {} -> {}",
        plan.profile_id, plan.profile_revision, plan.output
    );
    for arch in &plan.arches {
        println!("  {}: {}, {}, {}", arch.arch, arch.kernel, arch.initrd, arch.rootfs);
    }
    for command in &plan.commands {
        let env = if command.env.is_empty() {
            String::new()
        } else {
            format!(
                "{} ",
                command
                    .env
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        println!("  {}{}", env, command.argv.join(" "));
    }
    Ok(())
}

pub(super) fn clean_image_outputs(plan: &ImageBuildPlan) -> Result<()> {
    let output = PathBuf::from(&plan.output);
    for arch in &plan.arches {
        let path = output.join(&arch.arch);
        if !path.exists() {
            continue;
        }
        match plan.template {
            "all" => {
                fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
            }
            "kernel" => {
                for name in [&arch.kernel, &arch.initrd] {
                    let file = path.join(name);
                    if file.exists() {
                        fs::remove_file(&file).with_context(|| format!("remove {}", file.display()))?;
                    }
                }
            }
            "rootfs" => {
                for name in [
                    arch.rootfs.as_str(),
                    "rootfs.squashfs",
                    "obom.cdx.json",
                    "software-inventory.json",
                    "build-ledger.log",
                    "tool-versions.txt",
                ] {
                    let file = path.join(name);
                    if file.exists() {
                        fs::remove_file(&file).with_context(|| format!("remove {}", file.display()))?;
                    }
                }
            }
            other => return Err(anyhow!("unsupported image build template {other}")),
        }
    }
    if plan.arches.len() > 1 {
        for name in ["manifest.json", "B3SUMS"] {
            let path = output.join(name);
            if path.exists() {
                fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            }
        }
    }
    Ok(())
}

pub(super) fn run_command(command: &CommandReport) -> Result<()> {
    let (program, args) = command
        .argv
        .split_first()
        .ok_or_else(|| anyhow!("empty command for step {}", command.step))?;
    let status = Command::new(program)
        .args(args)
        .envs(&command.env)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("run image build step {}", command.step))?;
    if !status.success() {
        return Err(anyhow!("image build step {} failed with status {status}", command.step));
    }
    Ok(())
}

pub(super) fn compile_rule_file(kind: &'static str, path: &Path, source: RuleFileSourceArg) -> Result<RuleFileReport> {
    let content = fs::read_to_string(path).with_context(|| format!("read {kind} {}", path.display()))?;
    let profile = match kind {
        "enforcement" => SecurityRuleProfile::parse_toml(&content)
            .map_err(|error| anyhow!("parse enforcement {}: {error}", path.display()))?,
        "detection" => SecurityRuleProfile::parse_sigma_yaml(&content)
            .map_err(|error| anyhow!("parse detection {}: {error}", path.display()))?,
        other => return Err(anyhow!("unsupported rule file kind: {other}")),
    };
    let source = source.into_security_rule_source();
    let rule_set = SecurityRuleSet::compile_profile(&profile, source)
        .map_err(|error| anyhow!("compile {kind} {}: {error}", path.display()))?;
    let rules = rule_set.rules().iter().map(compiled_rule_report).collect::<Vec<_>>();
    Ok(RuleFileReport {
        schema: "capsem.admin.rule_file_report.v1",
        ok: true,
        kind,
        source: match source {
            SecurityRuleSource::User => "user",
            SecurityRuleSource::Corp => "corp",
            SecurityRuleSource::BuiltinDefault => "builtin_default",
        },
        path: path.display().to_string(),
        compiled_rules: rules.len(),
        rules,
    })
}

pub(super) fn compiled_rule_report(rule: &CompiledSecurityRule) -> CompiledRuleReport {
    CompiledRuleReport {
        rule_id: rule.rule_id.clone(),
        provider: rule.provider.clone(),
        namespace: rule.namespace.clone(),
        rule_key: rule.rule_key.clone(),
        default_rule: rule.default_rule,
        name: rule.name.clone(),
        action: rule.action.as_str(),
        detection_level: rule.detection_level.map(|level| level.as_str()),
        priority: rule.priority,
        condition: rule.condition.clone(),
        reason: rule.reason.clone(),
        corp_locked: rule.corp_locked,
    }
}

pub(super) fn load_manifest(path: &Path) -> Result<ManifestV2> {
    let content = fs::read_to_string(path).with_context(|| format!("read manifest {}", path.display()))?;
    ManifestV2::from_json(&content).with_context(|| format!("parse manifest {}", path.display()))
}

pub(super) fn read_manifest_url(source: &str) -> Result<Vec<u8>> {
    read_url_bytes(source, "manifest")
}

pub(super) fn read_url_bytes(source: &str, label: &str) -> Result<Vec<u8>> {
    let url = reqwest::Url::parse(source).with_context(|| {
        format!("{label} must be a URL: use https://..., http://..., or file:///absolute/path, got {source}")
    })?;
    match url.scheme() {
        "http" | "https" => {
            let response = reqwest::blocking::Client::builder()
                .user_agent("capsem-admin")
                .build()
                .with_context(|| format!("build {label} HTTP client"))?
                .get(url)
                .send()
                .with_context(|| format!("fetch {label} {source}"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(anyhow!("{label} fetch failed: HTTP {status} for {source}"));
            }
            Ok(response
                .bytes()
                .with_context(|| format!("read {label} response body"))?
                .to_vec())
        }
        "file" => {
            let path = url
                .to_file_path()
                .map_err(|_| anyhow!("{label} file URL must be absolute: {source}"))?;
            fs::read(&path).with_context(|| format!("read {label} {}", path.display()))
        }
        scheme => Err(anyhow!(
            "unsupported {label} URL scheme {scheme}: use https://, http://, or file://"
        )),
    }
}

pub(super) fn manifest_report(
    path: &Path,
    manifest: &ManifestV2,
    assets_dir: Option<&Path>,
    only_arch: Option<&str>,
) -> Result<ManifestReport> {
    let mut arches = Vec::new();
    for (asset_version, release) in &manifest.assets.releases {
        for (arch, assets) in &release.arches {
            if only_arch.is_some_and(|only| only != arch) {
                continue;
            }
            let mut asset_reports = Vec::new();
            let mut names = assets.keys().collect::<Vec<_>>();
            names.sort();
            for name in names {
                let entry = assets.get(name).expect("asset name from keys");
                let (path, present, size_ok, blake3_ok) = match assets_dir {
                    Some(dir) => {
                        let file_path = dir.join(arch).join(name);
                        if !file_path.is_file() {
                            (Some(file_path.display().to_string()), false, None, None)
                        } else {
                            let metadata = fs::metadata(&file_path)
                                .with_context(|| format!("stat manifest asset {}", file_path.display()))?;
                            let digest = hash_file(&file_path)?;
                            (
                                Some(file_path.display().to_string()),
                                true,
                                Some(metadata.len() == entry.size),
                                Some(digest == entry.hash),
                            )
                        }
                    }
                    None => (None, false, None, None),
                };
                asset_reports.push(ManifestAssetReport {
                    logical_name: name.clone(),
                    hash: entry.hash.clone(),
                    size: entry.size,
                    path,
                    present,
                    size_ok,
                    blake3_ok,
                });
            }
            arches.push(ManifestArchReport {
                asset_version: asset_version.clone(),
                arch: arch.clone(),
                assets: asset_reports,
            });
        }
    }
    arches.sort_by(|left, right| {
        left.asset_version
            .cmp(&right.asset_version)
            .then_with(|| left.arch.cmp(&right.arch))
    });
    if let Some(only_arch) = only_arch {
        if arches.is_empty() {
            return Err(anyhow!("manifest {} does not contain arch {only_arch}", path.display()));
        }
    }
    Ok(ManifestReport {
        schema: "capsem.admin.manifest_report.v1",
        ok: true,
        path: path.display().to_string(),
        blake3: hash_file(path)?,
        refresh_policy: manifest.refresh_policy.clone(),
        asset_version: manifest.assets.current.clone(),
        binary_version: manifest.binaries.current.clone(),
        releases: manifest.assets.releases.len(),
        arches,
    })
}

pub(super) fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub(super) fn infer_config_root(profile_path: &Path) -> Result<PathBuf> {
    let parent = profile_path.parent().ok_or_else(|| {
        anyhow!(
            "cannot infer config root for profile path without parent: {}",
            profile_path.display()
        )
    })?;
    if profile_path.file_name().is_some_and(|name| name == "profile.toml")
        && parent
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "profiles")
    {
        return parent
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("cannot infer config root from profile path {}", profile_path.display()));
    }
    if parent.file_name().is_some_and(|name| name == "profiles") {
        return parent
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("cannot infer config root from profile path {}", profile_path.display()));
    }
    Ok(parent.to_path_buf())
}

use super::*;

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
        // Site-root relative. A `file://` channel is a release-site tree on
        // disk whose root is the directory holding `assets/`, not `/`.
        let mut root = base;
        if root.scheme() == "file" {
            if let Some(dist) = root.path().rfind("/assets/") {
                let path = format!("{}{trimmed}", &root.path()[..dist]);
                root.set_path(&path);
                root.set_query(None);
                root.set_fragment(None);
                return Ok(root.to_string());
            }
        }
        root.set_path(trimmed);
        root.set_query(None);
        root.set_fragment(None);
        return Ok(root.to_string());
    }
    base.join(trimmed)
        .with_context(|| format!("resolve release channel artifact {trimmed} against {channel_source}"))
        .map(|url| url.to_string())
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

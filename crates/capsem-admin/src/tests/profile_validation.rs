use super::*;

#[test]
fn graph_channel_page_validates_each_mixed_profile_revision() {
    let manifest = serde_json::json!({
        "version": "1.0.0",
        "profiles": {
            "co-work": {"revision": "2026.06.08.7"},
            "code": {"revision": "2026.06.08.8"},
        },
    });
    let health = serde_json::json!({
        "generated_at": "2026-07-28T00:00:00Z",
        "current": {"binary": "1.6.0"},
        "profiles": {"revision": "profiles-derived-set-identity"},
        "evidence": {"host_binary_files": []},
    });
    let complete_page = concat!(
        "2026-07-28T00:00:00Z 1.0.0 /assets/nightly/manifest.json ",
        "co-work 2026.06.08.7 code 2026.06.08.8"
    );

    validate_assets_channel_graph_page_state(complete_page, "nightly", &manifest, &health)
        .expect("all manifest-owned profile revisions are rendered");

    let missing_code_revision = concat!(
        "2026-07-28T00:00:00Z 1.0.0 /assets/nightly/manifest.json ",
        "co-work 2026.06.08.7 code profiles-derived-set-identity"
    );
    let error = validate_assets_channel_graph_page_state(missing_code_revision, "nightly", &manifest, &health)
        .expect_err("aggregate identity cannot replace a missing profile revision");
    assert!(
        error.to_string().contains("missing profile revision code 2026.06.08.8"),
        "{error:#}"
    );
}

#[test]
fn cli_accepts_materialized_profile_validation() {
    let cli = Cli::parse_from([
        "capsem-admin",
        "profile",
        "validate",
        "target/config/profiles/co-work/profile.toml",
        "--config-root",
        "target/config",
        "--materialized",
    ]);
    match cli.command {
        Commands::Profile(ProfileCommand {
            command: ProfileSubcommand::Validate(args),
        }) => assert!(args.materialized),
        _ => panic!("expected profile validate"),
    }
}

#[test]
fn validates_checked_in_code_profile_through_security_rule_set() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let config_root = repo_root.join("config");
    let profile_path = config_root.join("profiles/code/profile.toml");

    let report = validate_profile(&profile_path, Some(&config_root)).expect("profile validates");

    assert!(report.ok);
    assert_eq!(report.profile_id, "code");
    assert!(report.compiled_rules >= 7);
}

#[test]
fn source_profile_validation_rejects_generated_pins() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let config_root = repo_root.join("config");
    let source = fs::read_to_string(config_root.join("profiles/code/profile.toml")).expect("read source profile");
    let pinned = source.replace(
        "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\n",
        "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\nhash = \"blake3:aa933a569fe27ed014ae76b58eb278d72fbde8a3cbd4c06a23da2987e70d0bd1\"\nsize = 8786432\n",
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let profile_path = temp.path().join("profile.toml");
    fs::write(&profile_path, pinned).expect("write pinned profile");

    let error = validate_profile(&profile_path, Some(&config_root)).expect_err("source profile pins rejected");

    assert!(
        error.to_string().contains("source profile") && error.to_string().contains("hash/size pins"),
        "{error:#}"
    );
}

#[test]
fn validates_checked_in_settings_file() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let path = repo_root.join("config/settings/settings.toml");

    let report = validate_settings(&path).expect("settings validates");

    assert!(report.ok);
    assert!(report.app.auto_update);
    assert_eq!(report.appearance.theme, "system");
}

#[test]
fn settings_validation_rejects_runtime_profile_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("settings.toml");
    fs::write(
        &path,
        r#"
[app]
auto_update = true
notifications = true
start_service_at_login = true

[appearance]
theme = "system"
font_size = 14
reduced_motion = false

[profiles]
code = true
"#,
    )
    .expect("settings");

    let error = validate_settings(&path).expect_err("profile fields rejected");

    assert!(format!("{error:#}").contains("unknown field `profiles`"), "{error:#}");
}

#[test]
fn checked_in_config_root_passes_admin_lint() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");

    let report = check_config_root(&repo_root.join("config"), Some("arm64")).expect("config root checks");

    assert!(report.ok);
    assert!(report
        .profiles
        .iter()
        .any(|profile| profile.validation.profile_id == "code"));
    assert!(report
        .profiles
        .iter()
        .any(|profile| profile.validation.profile_id == "co-work"));
}

#[test]
fn config_root_lint_rejects_profile_id_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path().join("config");
    fs::create_dir_all(config_root.join("profiles/wrong")).expect("profile dir");
    fs::create_dir_all(config_root.join("settings")).expect("settings dir");
    fs::create_dir_all(config_root.join("corp")).expect("corp dir");
    fs::write(
        config_root.join("settings/settings.toml"),
        include_str!("../../../../config/settings/settings.toml"),
    )
    .expect("settings");
    fs::write(config_root.join("corp/corp.toml"), "refresh_policy = \"24h\"\n").expect("corp");
    fs::write(
        config_root.join("profiles/wrong/profile.toml"),
        include_str!("../../../../config/profiles/code/profile.toml"),
    )
    .expect("profile");

    let error = check_config_root(&config_root, Some("arm64")).expect_err("catalog id mismatch rejected");

    assert!(format!("{error:#}").contains("id mismatch"), "{error:#}");
}

#[test]
fn rejects_profile_rule_files_with_old_policy_syntax() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path();
    fs::create_dir_all(config_root.join("profiles/code")).expect("profile rules dir");
    let old_table = "policy".to_string() + ".http.block_old";
    fs::write(
        config_root.join("profiles/code/enforcement.toml"),
        r#"
[__OLD_TABLE__]
on = ["http.request"]
if = "http.host == 'evil.test'"
decision = "block"
"#
        .replace("__OLD_TABLE__", &old_table),
    )
    .expect("old policy file");
    fs::write(
        config_root.join("profiles/code/profile.toml"),
        r#"
id = "code"
name = "Code"
description = "Optimized for coding and long-running agents."
revision = "2026.06.08.3"
refresh_policy = "24h"

[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "https://example.test/vmlinuz"

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "https://example.test/initrd.img"

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://example.test/rootfs.erofs"

[rule_files]
enforcement = "profiles/code/enforcement.toml"
"#,
    )
    .expect("profile");

    let error = validate_profile(&config_root.join("profiles/code/profile.toml"), Some(config_root))
        .expect_err("old policy syntax rejected");

    assert!(
        error.to_string().contains("unknown field `policy`") || format!("{error:#}").contains("unknown field `policy`"),
        "{error:#}"
    );
}

#[test]
fn compiles_checked_in_enforcement_file() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let path = repo_root.join("config/profiles/code/enforcement.toml");

    let report = compile_rule_file("enforcement", &path, RuleFileSourceArg::User).expect("compile");

    assert_eq!(report.kind, "enforcement");
    let rule_ids = report
        .rules
        .iter()
        .map(|rule| rule.rule_id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        rule_ids,
        BTreeSet::from([
            "profiles.rules.capsem_mock_server",
            "profiles.rules.default_http",
            "profiles.rules.default_dns",
            "profiles.rules.default_mcp",
            "profiles.rules.default_model",
            "profiles.rules.default_unknown_model_provider",
            "profiles.rules.default_unknown_mcp_server",
            "profiles.rules.default_file",
            "profiles.rules.default_process",
        ])
    );
    assert_eq!(report.compiled_rules, rule_ids.len());
    assert_eq!(
        report
            .rules
            .iter()
            .filter(|rule| !rule.default_rule)
            .map(|rule| rule.rule_id.as_str())
            .collect::<Vec<_>>(),
        vec!["profiles.rules.capsem_mock_server"]
    );
    assert!(report.rules.iter().all(|rule| rule.action == "allow"));
    assert!(report.rules.iter().all(|rule| rule.priority > 0));
    assert_eq!(
        report
            .rules
            .iter()
            .filter(|rule| rule.detection_level.is_some())
            .map(|rule| (rule.rule_id.as_str(), rule.detection_level))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            ("profiles.rules.default_unknown_model_provider", Some("informational")),
            ("profiles.rules.default_unknown_mcp_server", Some("informational")),
        ])
    );
}

#[test]
fn compiles_checked_in_detection_file() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let path = repo_root.join("config/profiles/code/detection.yaml");

    let report = compile_rule_file("detection", &path, RuleFileSourceArg::User).expect("compile");

    assert_eq!(report.kind, "detection");
    assert_eq!(report.compiled_rules, 1);
    assert_eq!(report.rules[0].rule_id, "profiles.rules.skill_loaded");
    assert_eq!(report.rules[0].detection_level, Some("informational"));
}

#[test]
fn checked_in_profile_build_wraps_agy_with_skip_permissions() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let path = repo_root.join("config/profiles/code/build.sh");
    let content = fs::read_to_string(path).expect("profile build script");

    assert!(
        content.contains("/usr/local/bin/agy-real"),
        "profile build script must preserve the real AGY binary behind a wrapper"
    );
    assert!(
        content.contains("--dangerously-skip-permissions"),
        "profile-owned AGY wrapper must opt into the Capsem permission model"
    );
    assert!(
        content.contains("CAPSEM_OLLAMA_URL")
            && content.contains("CAPSEM_OLLAMA_SHA256")
            && content.contains("sha256sum"),
        "profile build script must install the exact digest-bound Ollama archive"
    );
    assert!(!content.contains("https://ollama.com/install.sh"));
}

#[test]
fn enforcement_compile_rejects_old_on_if_decision_shape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("old.toml");
    fs::write(
        &path,
        r#"
[profiles.rules.old_http]
name = "old_http"
on = ["http.request"]
if = "http.host == 'evil.test'"
decision = "block"
"#,
    )
    .expect("old rule");

    let error = compile_rule_file("enforcement", &path, RuleFileSourceArg::User).expect_err("old shape rejected");

    assert!(format!("{error:#}").contains("missing field `action`"), "{error:#}");
}

#[test]
fn infers_config_root_for_profiles_directory() {
    let root = PathBuf::from("/tmp/capsem-config");
    let path = root.join("profiles/code/profile.toml");
    assert_eq!(infer_config_root(&path).unwrap(), root);
}

#[test]
fn checks_manifest_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("manifest.json");
    fs::write(&path, minimal_manifest_json(None, true)).expect("manifest");

    let manifest = load_manifest(&path).expect("manifest parses");
    let report = manifest_report(&path, &manifest, None, None).expect("report");

    assert_eq!(
        report.blake3,
        blake3::hash(fs::read(&path).unwrap().as_slice()).to_hex().to_string()
    );
    assert_eq!(report.refresh_policy, "24h");
    assert_eq!(report.asset_version, "2026.0607.1");
    assert!(report.arches.iter().any(|arch| arch.arch == "arm64"));
}

#[test]
fn manifest_check_rejects_missing_refresh_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("manifest.json");
    fs::write(&path, minimal_manifest_json(None, false)).expect("manifest");

    let error = load_manifest(&path).expect_err("refresh policy required");

    assert!(format!("{error:#}").contains("refresh_policy"), "{error:#}");
}

#[test]
fn manifest_verify_checks_literal_sibling_assets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let payload = b"capsem test asset";
    let hash = blake3::hash(payload).to_hex().to_string();
    let manifest_path = temp.path().join("manifest.json");
    fs::write(&manifest_path, minimal_manifest_json(Some(&hash), true)).expect("manifest");
    let assets_root = temp.path().join("assets");
    let assets_dir = assets_root.join("arm64");
    fs::create_dir_all(&assets_dir).expect("assets dir");
    fs::write(assets_dir.join("rootfs.erofs"), payload).expect("asset");

    let manifest = load_manifest(&manifest_path).expect("manifest");
    let report =
        manifest_report(&manifest_path, &manifest, Some(&assets_root), Some("arm64")).expect("manifest verify");

    let asset = &report.arches[0].assets[0];
    assert!(asset.present);
    assert_eq!(asset.size_ok, Some(true));
    assert_eq!(asset.blake3_ok, Some(true));
}

#[test]
fn profile_check_verifies_only_declared_file_urls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.rule_files.enforcement = None;
    profile.rule_files.sigma = None;
    profile.files = Default::default();
    profile.assets.arch.retain(|arch, _| arch == "arm64");
    let arch_assets = profile.assets.arch.get_mut("arm64").expect("arm64 assets");
    for descriptor in [
        &mut arch_assets.kernel,
        &mut arch_assets.initrd,
        &mut arch_assets.rootfs,
    ] {
        let payload = format!("{} bytes", descriptor.name);
        let path = temp.path().join(&descriptor.name);
        fs::write(&path, payload.as_bytes()).expect("asset");
        descriptor.url = format!("file://{}", path.display());
    }
    let profile_path = temp.path().join("profile.toml");
    fs::write(&profile_path, toml::to_string(&profile).expect("serialize profile")).expect("profile");

    let report = check_profile(&ProfileCheckArgs {
        path: profile_path,
        config_root: Some(temp.path().to_path_buf()),
        arch: Some("arm64".to_string()),
        json: true,
    })
    .expect("profile check");

    assert!(report.assets.is_empty());
    assert!(report.profile_files.is_empty());
}

#[test]
fn profile_check_validates_profile_payload_files_and_root_manifest() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let profile_path = repo_root.join("config/profiles/code/profile.toml");

    let report = check_profile(&ProfileCheckArgs {
        path: profile_path,
        config_root: Some(repo_root.join("config")),
        arch: Some("arm64".to_string()),
        json: true,
    })
    .expect("checked-in profile payload files validate");

    assert!(report.profile_files.iter().any(|file| file.logical_name == "mcp"));
    assert!(report
        .profile_files
        .iter()
        .any(|file| file.logical_name == "root/.codex/config.toml"));
    assert!(report.profile_files.iter().all(|file| file.present));
    assert!(report
        .profile_files
        .iter()
        .any(|file| file.size_ok == Some(true) && file.blake3_ok == Some(true)));
}

#[test]
fn release_graph_publishes_every_manifested_profile_root_payload() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let config_root = repo_root.join("config");
    let profile = load_profile(&config_root.join("profiles/code/profile.toml")).expect("load profile");
    let root_manifest: ProfileRootManifest = serde_json::from_slice(
        &fs::read(config_root.join("profiles/code/root.manifest.json")).expect("read root manifest"),
    )
    .expect("parse root manifest");
    let mut copies = Vec::new();

    let rows = graph_profile_config_refs(
        &profile,
        &config_root,
        "nightly",
        &profile.revision,
        "x86_64",
        &mut copies,
    )
    .expect("build profile config graph");
    let root_rows = rows
        .iter()
        .filter(|row| row["kind"].as_str() == Some("root_payload"))
        .collect::<Vec<_>>();

    assert_eq!(root_rows.len(), root_manifest.files.len());
    let expected_paths = root_manifest
        .files
        .iter()
        .map(|entry| format!("profiles/code/root/{}", entry.path))
        .collect::<BTreeSet<_>>();
    let actual_paths = root_rows
        .iter()
        .map(|row| row["path"].as_str().expect("root payload path").to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_paths, expected_paths);
    let expected_digests = root_rows
        .iter()
        .map(|row| row["digest"]["blake3"].as_str().expect("root payload BLAKE3"))
        .collect::<BTreeSet<_>>();
    let actual_urls = root_rows
        .iter()
        .map(|row| row["url"].as_str().expect("root payload URL"))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_urls.len(), expected_digests.len());
    let urls_by_digest = root_rows
        .iter()
        .fold(BTreeMap::<&str, BTreeSet<&str>>::new(), |mut grouped, row| {
            grouped
                .entry(row["digest"]["blake3"].as_str().expect("root payload BLAKE3"))
                .or_default()
                .insert(row["url"].as_str().expect("root payload URL"));
            grouped
        });
    assert!(
        urls_by_digest.values().all(|urls| urls.len() == 1),
        "identical root payload bytes must reuse one immutable URL"
    );
    for row in root_rows {
        let path = row["path"].as_str().expect("root payload path");
        assert!(copies.iter().any(|copy| {
            copy.url == row["url"].as_str().expect("root payload URL") && copy.source == config_root.join(path)
        }));
    }
}

#[test]
fn profile_check_rejects_missing_profile_payload_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path().join("config");
    let profile_dir = config_root.join("profiles/code");
    fs::create_dir_all(&profile_dir).expect("profile dir");
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.rule_files.enforcement = None;
    profile.rule_files.sigma = None;
    profile.assets.arch.retain(|arch, _| arch == "arm64");
    profile.files = Default::default();
    profile.files.mcp = Some(capsem_core::net::policy_config::ProfileFileDescriptor {
        path: "profiles/code/mcp.json".to_string(),
        hash: None,
        size: None,
    });
    let profile_path = profile_dir.join("profile.toml");
    fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

    let error = check_profile(&ProfileCheckArgs {
        path: profile_path,
        config_root: Some(config_root),
        arch: Some("arm64".to_string()),
        json: true,
    })
    .expect_err("missing payload file rejected");
    assert!(error.to_string().contains("profile payload file pin check"));
}

#[test]
fn profile_check_rejects_malformed_profile_mcp_file_even_when_hash_matches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path().join("config");
    let profile_dir = config_root.join("profiles/code");
    fs::create_dir_all(&profile_dir).expect("profile dir");
    let mcp = "{ definitely not json";
    fs::write(profile_dir.join("mcp.json"), mcp).expect("mcp");
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.rule_files.enforcement = None;
    profile.rule_files.sigma = None;
    profile.assets.arch.retain(|arch, _| arch == "arm64");
    profile.files = Default::default();
    profile.files.mcp = Some(capsem_core::net::policy_config::ProfileFileDescriptor {
        path: "profiles/code/mcp.json".to_string(),
        hash: None,
        size: None,
    });
    let profile_path = profile_dir.join("profile.toml");
    fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

    let error = check_profile(&ProfileCheckArgs {
        path: profile_path,
        config_root: Some(config_root),
        arch: Some("arm64".to_string()),
        json: true,
    })
    .expect_err("malformed MCP config rejected");

    assert!(format!("{error:#}").contains("parse profile MCP config"), "{error:#}");
}

#[test]
fn profile_check_rejects_empty_profile_package_file_even_when_hash_matches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path().join("config");
    let profile_dir = config_root.join("profiles/code");
    fs::create_dir_all(&profile_dir).expect("profile dir");
    let packages = "# intentionally empty\n";
    fs::write(profile_dir.join("python-requirements.txt"), packages).expect("packages");
    fs::write(
        profile_dir.join("python-requirements.lock"),
        "pytest==9.1.1 \\\n    --hash=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    )
    .expect("lock");
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.rule_files.enforcement = None;
    profile.rule_files.sigma = None;
    profile.assets.arch.retain(|arch, _| arch == "arm64");
    profile.files = Default::default();
    profile.files.python_requirements = Some(capsem_core::net::policy_config::ProfileFileDescriptor {
        path: "profiles/code/python-requirements.txt".to_string(),
        hash: None,
        size: None,
    });
    profile.files.python_requirements_lock = Some(capsem_core::net::policy_config::ProfileFileDescriptor {
        path: "profiles/code/python-requirements.lock".to_string(),
        hash: None,
        size: None,
    });
    let profile_path = profile_dir.join("profile.toml");
    fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

    let error = check_profile(&ProfileCheckArgs {
        path: profile_path,
        config_root: Some(config_root),
        arch: Some("arm64".to_string()),
        json: true,
    })
    .expect_err("empty package file rejected");

    assert!(format!("{error:#}").contains("package list"), "{error:#}");
}

#[test]
fn profile_dependency_locks_reject_direct_version_drift() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_lock = temp.path().join("python-requirements.lock");
    fs::write(
        &python_lock,
        "pytest==9.1.0 \\\n    --hash=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    )
    .expect("python lock");
    let expected_python = BTreeMap::from([("pytest".to_string(), "9.1.1".to_string())]);
    let python_error = validate_python_requirements_lock(&python_lock, Some(&expected_python))
        .expect_err("direct Python version drift must be rejected");
    assert!(
        format!("{python_error:#}").contains("does not match the profile's exact direct packages"),
        "{python_error:#}"
    );

    let npm_lock = temp.path().join("npm-package-lock.json");
    fs::write(
        &npm_lock,
        serde_json::to_vec(&serde_json::json!({
            "name": "capsem-profile-ai-clis",
            "lockfileVersion": 3,
            "packages": {
                "": {"dependencies": {"@openai/codex": "0.146.0"}},
                "node_modules/@openai/codex": {
                    "version": "0.146.0",
                    "integrity": "sha512-dGVzdA=="
                }
            }
        }))
        .expect("npm lock json"),
    )
    .expect("npm lock");
    let expected_npm = BTreeMap::from([("@openai/codex".to_string(), "0.147.0".to_string())]);
    let npm_error = validate_npm_package_lock(&npm_lock, Some(&expected_npm))
        .expect_err("direct npm version drift must be rejected");
    assert!(
        format!("{npm_error:#}").contains("does not match the profile's exact direct packages"),
        "{npm_error:#}"
    );
}

#[test]
fn profile_check_rejects_profile_root_manifest_escape_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path().join("config");
    let profile_dir = config_root.join("profiles/code");
    fs::create_dir_all(&profile_dir).expect("profile dir");
    let root_manifest = r#"{
  "format": "capsem.profile-root.v1",
  "files": [
    {
      "path": "../outside",
      "hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "size": 1
    }
  ]
}
"#;
    fs::write(profile_dir.join("root.manifest.json"), root_manifest).expect("root manifest");
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.rule_files.enforcement = None;
    profile.rule_files.sigma = None;
    profile.assets.arch.retain(|arch, _| arch == "arm64");
    profile.files = Default::default();
    profile.files.root_manifest = Some(capsem_core::net::policy_config::ProfileFileDescriptor {
        path: "profiles/code/root.manifest.json".to_string(),
        hash: None,
        size: None,
    });
    let profile_path = profile_dir.join("profile.toml");
    fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

    let error = check_profile(&ProfileCheckArgs {
        path: profile_path,
        config_root: Some(config_root),
        arch: Some("arm64".to_string()),
        json: true,
    })
    .expect_err("root manifest escape rejected");

    assert!(error.to_string().contains("profile root manifest file"), "{error:#}");
}

#[test]
fn profile_check_rejects_unpinned_profile_root_payload_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path().join("config");
    let profile_dir = config_root.join("profiles/code");
    let profile_root = profile_dir.join("root");
    fs::create_dir_all(profile_root.join("root/.codex")).expect("profile root");
    fs::create_dir_all(profile_root.join("root/.antigravity")).expect("agy root");
    let codex_payload = b"[mcp_servers.capsem]\ncommand = \"/run/capsem-mcp-server\"\n";
    fs::write(profile_root.join("root/.codex/config.toml"), codex_payload).expect("codex config");
    fs::write(
        profile_root.join("root/.antigravity/antigravity-oauth-token"),
        b"secret",
    )
    .expect("unlisted token");
    let root_manifest = format!(
        r#"{{
  "format": "capsem.profile-root.v1",
  "files": [
    {{
      "path": "root/.codex/config.toml",
      "hash": "blake3:{}",
      "size": {}
    }}
  ]
}}
"#,
        blake3::hash(codex_payload).to_hex(),
        codex_payload.len()
    );
    fs::write(profile_dir.join("root.manifest.json"), root_manifest).expect("root manifest");
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.rule_files.enforcement = None;
    profile.rule_files.sigma = None;
    profile.assets.arch.retain(|arch, _| arch == "arm64");
    profile.files = Default::default();
    profile.files.root_manifest = Some(capsem_core::net::policy_config::ProfileFileDescriptor {
        path: "profiles/code/root.manifest.json".to_string(),
        hash: None,
        size: None,
    });
    let profile_path = profile_dir.join("profile.toml");
    fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

    let error = check_profile(&ProfileCheckArgs {
        path: profile_path,
        config_root: Some(config_root),
        arch: Some("arm64".to_string()),
        json: true,
    })
    .expect_err("unlisted profile root payload rejected");

    assert!(
        format!("{error:#}").contains("unlisted profile root payload file"),
        "{error:#}"
    );
}

#[cfg(unix)]
#[test]
fn profile_check_rejects_symlinked_profile_root_payloads() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let profile_dir = temp.path().join("profiles/code");
    let profile_root = profile_dir.join("root/root");
    fs::create_dir_all(&profile_root).expect("profile root");
    let outside = temp.path().join("outside");
    fs::write(&outside, b"outside").expect("outside payload");
    symlink(&outside, profile_root.join(".profile")).expect("payload symlink");
    let root_manifest = format!(
        r#"{{
  "format": "capsem.profile-root.v1",
  "files": [
    {{
      "path": "root/.profile",
      "hash": "blake3:{}",
      "size": {}
    }}
  ]
}}
"#,
        blake3::hash(b"outside").to_hex(),
        b"outside".len()
    );
    let manifest_path = profile_dir.join("root.manifest.json");
    fs::write(&manifest_path, root_manifest).expect("root manifest");

    let error = check_profile_root_manifest(&manifest_path).expect_err("payload symlink rejected");

    assert!(format!("{error:#}").contains("not a regular file"), "{error:#}");
}

#[test]
fn profile_check_rejects_local_model_provider_profile_root_payloads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_root = temp.path().join("config");
    let profile_dir = config_root.join("profiles/code");
    let profile_root = profile_dir.join("root");
    fs::create_dir_all(profile_root.join("root/.gemini/config")).expect("profile root");
    let payload = br#"{
  "ai": {
    "provider": "ollama",
    "baseUrl": "http://127.0.0.1:11434",
    "model": "gemma4:latest"
  }
}
"#;
    fs::write(profile_root.join("root/.gemini/config/config.json"), payload).expect("agy config");
    let root_manifest = format!(
        r#"{{
  "format": "capsem.profile-root.v1",
  "files": [
    {{
      "path": "root/.gemini/config/config.json",
      "hash": "blake3:{}",
      "size": {}
    }}
  ]
}}
"#,
        blake3::hash(payload).to_hex(),
        payload.len()
    );
    fs::write(profile_dir.join("root.manifest.json"), root_manifest).expect("root manifest");
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.rule_files.enforcement = None;
    profile.rule_files.sigma = None;
    profile.assets.arch.retain(|arch, _| arch == "arm64");
    profile.files = Default::default();
    profile.files.root_manifest = Some(capsem_core::net::policy_config::ProfileFileDescriptor {
        path: "profiles/code/root.manifest.json".to_string(),
        hash: None,
        size: None,
    });
    let profile_path = profile_dir.join("profile.toml");
    fs::write(&profile_path, toml::to_string(&profile).unwrap()).expect("profile");

    let error = check_profile(&ProfileCheckArgs {
        path: profile_path,
        config_root: Some(config_root),
        arch: Some("arm64".to_string()),
        json: true,
    })
    .expect_err("local provider profile root payload rejected");

    assert!(
        format!("{error:#}").contains("profile root provider override"),
        "{error:#}"
    );
}

#[test]
fn image_verify_rejects_profile_manifest_pin_drift() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("assets");
    let arch_dir = output.join("arm64");
    fs::create_dir_all(&arch_dir).expect("asset dir");
    let kernel = b"kernel";
    let initrd = b"initrd";
    let rootfs = b"rootfs";
    fs::write(arch_dir.join("vmlinuz"), kernel).expect("kernel");
    fs::write(arch_dir.join("initrd.img"), initrd).expect("initrd");
    fs::write(arch_dir.join("rootfs.erofs"), rootfs).expect("rootfs");
    let kernel_hash = blake3::hash(kernel).to_hex().to_string();
    let rootfs_hash = blake3::hash(rootfs).to_hex().to_string();
    let wrong_initrd_hash = "1111111111111111111111111111111111111111111111111111111111111111";
    fs::write(
        output.join("manifest.json"),
        format!(
            r#"{{
  "format": 2,
  "refresh_policy": "24h",
  "assets": {{
    "current": "2030.0101.1",
    "releases": {{
      "2030.0101.1": {{
        "date": "2030-01-01",
        "deprecated": false,
        "min_binary": "1.0.0",
        "arches": {{
          "arm64": {{
            "vmlinuz": {{"hash": "{kernel_hash}", "size": {kernel_size}}},
            "initrd.img": {{"hash": "{wrong_initrd_hash}", "size": {initrd_size}}},
            "rootfs.erofs": {{"hash": "{rootfs_hash}", "size": {rootfs_size}}}
          }}
        }}
      }}
    }}
  }},
  "binaries": {{
    "current": "1.0.0",
    "releases": {{"1.0.0": {{"date": "2030-01-01", "deprecated": false, "min_assets": "2030.0101.1"}}}}
  }}
}}"#,
            kernel_size = kernel.len(),
            initrd_size = initrd.len(),
            rootfs_size = rootfs.len(),
        ),
    )
    .expect("manifest");

    let mut profile = ProfileConfigFile::builtin_primary();
    profile.rule_files.enforcement = None;
    profile.rule_files.sigma = None;
    profile.assets.arch.retain(|arch, _| arch == "arm64");
    let profile_path = temp.path().join("profile.toml");
    fs::write(&profile_path, toml::to_string(&profile).expect("serialize profile")).expect("profile");

    let error = verify_image_outputs(&ImageVerifyArgs {
        profile: profile_path,
        config_root: temp.path().to_path_buf(),
        output,
        manifest: None,
        arch: Some("arm64".to_string()),
    })
    .expect_err("manifest/output drift rejected");

    assert!(format!("{error:#}").contains("image output verify failed"), "{error:#}");
}

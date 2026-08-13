use super::*;
use std::fs;

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
    let error = validate_assets_channel_graph_page_state(
        missing_code_revision,
        "nightly",
        &manifest,
        &health,
    )
    .expect_err("aggregate identity cannot replace a missing profile revision");
    assert!(
        error
            .to_string()
            .contains("missing profile revision code 2026.06.08.8"),
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
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let config_root = repo_root.join("config");
    let profile_path = config_root.join("profiles/code/profile.toml");

    let report =
        validate_profile(&profile_path, Some(&config_root)).expect("profile validates");

    assert!(report.ok);
    assert_eq!(report.profile_id, "code");
    assert!(report.compiled_rules >= 7);
}

#[test]
fn source_profile_validation_rejects_generated_pins() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let config_root = repo_root.join("config");
    let source = fs::read_to_string(config_root.join("profiles/code/profile.toml"))
        .expect("read source profile");
    let pinned = source.replace(
        "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\n",
        "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\nhash = \"blake3:aa933a569fe27ed014ae76b58eb278d72fbde8a3cbd4c06a23da2987e70d0bd1\"\nsize = 8786432\n",
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let profile_path = temp.path().join("profile.toml");
    fs::write(&profile_path, pinned).expect("write pinned profile");

    let error = validate_profile(&profile_path, Some(&config_root))
        .expect_err("source profile pins rejected");

    assert!(
        error.to_string().contains("source profile")
            && error.to_string().contains("hash/size pins"),
        "{error:#}"
    );
}

#[test]
fn validates_checked_in_settings_file() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
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

    assert!(
        format!("{error:#}").contains("unknown field `profiles`"),
        "{error:#}"
    );
}

#[test]
fn checked_in_config_root_passes_admin_lint() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");

    let report = check_config_root(&repo_root.join("config"), Some("arm64"))
        .expect("config root checks");

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
        include_str!("../../../config/settings/settings.toml"),
    )
    .expect("settings");
    fs::write(
        config_root.join("corp/corp.toml"),
        "refresh_policy = \"24h\"\n",
    )
    .expect("corp");
    fs::write(
        config_root.join("profiles/wrong/profile.toml"),
        include_str!("../../../config/profiles/code/profile.toml"),
    )
    .expect("profile");

    let error = check_config_root(&config_root, Some("arm64"))
        .expect_err("catalog id mismatch rejected");

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

    let error = validate_profile(
        &config_root.join("profiles/code/profile.toml"),
        Some(config_root),
    )
    .expect_err("old policy syntax rejected");

    assert!(
        error.to_string().contains("unknown field `policy`")
            || format!("{error:#}").contains("unknown field `policy`"),
        "{error:#}"
    );
}

#[test]
fn compiles_checked_in_enforcement_file() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let path = repo_root.join("config/profiles/code/enforcement.toml");

    let report =
        compile_rule_file("enforcement", &path, RuleFileSourceArg::User).expect("compile");

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
            (
                "profiles.rules.default_unknown_model_provider",
                Some("informational")
            ),
            (
                "profiles.rules.default_unknown_mcp_server",
                Some("informational")
            ),
        ])
    );
}

#[test]
fn compiles_checked_in_detection_file() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let path = repo_root.join("config/profiles/code/detection.yaml");

    let report =
        compile_rule_file("detection", &path, RuleFileSourceArg::User).expect("compile");

    assert_eq!(report.kind, "detection");
    assert_eq!(report.compiled_rules, 1);
    assert_eq!(report.rules[0].rule_id, "profiles.rules.skill_loaded");
    assert_eq!(report.rules[0].detection_level, Some("informational"));
}

#[test]
fn checked_in_profile_build_wraps_agy_with_skip_permissions() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
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

    let error = compile_rule_file("enforcement", &path, RuleFileSourceArg::User)
        .expect_err("old shape rejected");

    assert!(
        format!("{error:#}").contains("missing field `action`"),
        "{error:#}"
    );
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
        blake3::hash(fs::read(&path).unwrap().as_slice())
            .to_hex()
            .to_string()
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
    let report = manifest_report(&manifest_path, &manifest, Some(&assets_root), Some("arm64"))
        .expect("manifest verify");

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
    fs::write(
        &profile_path,
        toml::to_string(&profile).expect("serialize profile"),
    )
    .expect("profile");

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
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let profile_path = repo_root.join("config/profiles/code/profile.toml");

    let report = check_profile(&ProfileCheckArgs {
        path: profile_path,
        config_root: Some(repo_root.join("config")),
        arch: Some("arm64".to_string()),
        json: true,
    })
    .expect("checked-in profile payload files validate");

    assert!(report
        .profile_files
        .iter()
        .any(|file| file.logical_name == "mcp"));
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
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let config_root = repo_root.join("config");
    let profile =
        load_profile(&config_root.join("profiles/code/profile.toml")).expect("load profile");
    let root_manifest: ProfileRootManifest = serde_json::from_slice(
        &fs::read(config_root.join("profiles/code/root.manifest.json"))
            .expect("read root manifest"),
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
        .map(|row| {
            row["digest"]["blake3"]
                .as_str()
                .expect("root payload BLAKE3")
        })
        .collect::<BTreeSet<_>>();
    let actual_urls = root_rows
        .iter()
        .map(|row| row["url"].as_str().expect("root payload URL"))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_urls.len(), expected_digests.len());
    let urls_by_digest = root_rows.iter().fold(
        BTreeMap::<&str, BTreeSet<&str>>::new(),
        |mut grouped, row| {
            grouped
                .entry(
                    row["digest"]["blake3"]
                        .as_str()
                        .expect("root payload BLAKE3"),
                )
                .or_default()
                .insert(row["url"].as_str().expect("root payload URL"));
            grouped
        },
    );
    assert!(
        urls_by_digest.values().all(|urls| urls.len() == 1),
        "identical root payload bytes must reuse one immutable URL"
    );
    for row in root_rows {
        let path = row["path"].as_str().expect("root payload path");
        assert!(copies.iter().any(|copy| {
            copy.url == row["url"].as_str().expect("root payload URL")
                && copy.source == config_root.join(path)
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

    assert!(
        format!("{error:#}").contains("parse profile MCP config"),
        "{error:#}"
    );
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
    profile.files.python_requirements =
        Some(capsem_core::net::policy_config::ProfileFileDescriptor {
            path: "profiles/code/python-requirements.txt".to_string(),
            hash: None,
            size: None,
        });
    profile.files.python_requirements_lock =
        Some(capsem_core::net::policy_config::ProfileFileDescriptor {
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
    profile.files.root_manifest =
        Some(capsem_core::net::policy_config::ProfileFileDescriptor {
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

    assert!(
        error.to_string().contains("profile root manifest file"),
        "{error:#}"
    );
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
    fs::write(profile_root.join("root/.codex/config.toml"), codex_payload)
        .expect("codex config");
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
    profile.files.root_manifest =
        Some(capsem_core::net::policy_config::ProfileFileDescriptor {
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

    let error =
        check_profile_root_manifest(&manifest_path).expect_err("payload symlink rejected");

    assert!(
        format!("{error:#}").contains("not a regular file"),
        "{error:#}"
    );
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
    fs::write(
        profile_root.join("root/.gemini/config/config.json"),
        payload,
    )
    .expect("agy config");
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
    profile.files.root_manifest =
        Some(capsem_core::net::policy_config::ProfileFileDescriptor {
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
    fs::write(
        &profile_path,
        toml::to_string(&profile).expect("serialize profile"),
    )
    .expect("profile");

    let error = verify_image_outputs(&ImageVerifyArgs {
        profile: profile_path,
        config_root: temp.path().to_path_buf(),
        output,
        manifest: None,
        arch: Some("arm64".to_string()),
    })
    .expect_err("manifest/output drift rejected");

    assert!(
        format!("{error:#}").contains("image output verify failed"),
        "{error:#}"
    );
}

#[test]
fn image_build_requires_profile_argument() {
    let error = Cli::try_parse_from(["capsem-admin", "image", "build"])
        .expect_err("profile is required");

    assert!(error.to_string().contains("--profile"), "{error}");
}

#[test]
fn image_workspace_is_a_supported_command_with_required_inputs() {
    let error = Cli::try_parse_from(["capsem-admin", "image", "workspace"])
        .expect_err("workspace inputs are required");

    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    let rendered = error.to_string();
    assert!(rendered.contains("--profile"), "{rendered}");
    assert!(rendered.contains("--output"), "{rendered}");
}

#[test]
fn image_build_workspaces_are_isolated_by_profile_and_architecture() {
    let profile = ProfileConfigFile::builtin_primary();

    let arm64 = image_build_workspace_path(&profile, Some("arm64"));
    let x86_64 = image_build_workspace_path(&profile, Some("x86_64"));

    assert_ne!(arm64, x86_64);
    assert_eq!(arm64, PathBuf::from("target/image-workspace/code/arm64"));
    assert_eq!(x86_64, PathBuf::from("target/image-workspace/code/x86_64"));
}

#[test]
fn image_build_rejects_dry_run_escape_hatch() {
    let error = Cli::try_parse_from([
        "capsem-admin",
        "image",
        "build",
        "--profile",
        "config/profiles/code/profile.toml",
        "--dry-run",
    ])
    .expect_err("dry-run is not a public product rail");

    assert!(
        error
            .to_string()
            .contains("unexpected argument '--dry-run'"),
        "{error}"
    );
}

#[test]
fn removed_admin_authoring_commands_are_not_parseable() {
    for argv in [
        ["capsem-admin", "profile", "init"],
        ["capsem-admin", "settings", "init"],
        ["capsem-admin", "enforcement", "compile"],
        ["capsem-admin", "detection", "compile"],
        ["capsem-admin", "manifest", "verify"],
        ["capsem-admin", "image", "plan"],
        ["capsem-admin", "image", "verify"],
    ] {
        let error = Cli::try_parse_from(argv).expect_err("removed command rejected");
        assert!(
            error.to_string().contains("unrecognized subcommand"),
            "{error}"
        );
    }
}

#[test]
fn image_plan_is_profile_derived_and_uses_erofs_lz4hc() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let args = ImageBuildArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: repo_root.join("assets"),
        arch: Some("arm64".to_string()),
        template: ImageBuildTemplate::All,
        clean: true,
        json: true,
    };

    let plan = image_build_plan(&args).expect("image plan");

    assert_eq!(plan.profile_id, "code");
    assert_eq!(plan.arches.len(), 1);
    assert_eq!(plan.arches[0].arch, "arm64");
    assert_eq!(plan.arches[0].rootfs, "rootfs.erofs");
    assert_eq!(plan.commands.len(), 3);
    assert_eq!(plan.commands[0].step, "kernel");
    assert_eq!(
        plan.commands[0].argv[0..5]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "uv",
            "run",
            "python",
            "-m",
            "capsem.builder.image_build_backend",
        ]
    );
    assert!(!plan.commands[0]
        .argv
        .windows(2)
        .any(|window| window[0] == "capsem-builder" && window[1] == "build"));
    assert_eq!(plan.commands[1].step, "rootfs");
    assert_eq!(
        plan.commands[1].argv[0..5]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "uv",
            "run",
            "python",
            "-m",
            "capsem.builder.image_build_backend",
        ]
    );
    assert!(!plan.commands[1]
        .argv
        .windows(2)
        .any(|window| window[0] == "capsem-builder" && window[1] == "build"));
    assert_eq!(
        plan.commands[1].env.get("CAPSEM_BUILD_EROFS_COMPRESSION"),
        Some(&"lz4hc".to_string())
    );
    assert_eq!(
        plan.commands[1]
            .env
            .get("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL"),
        Some(&"12".to_string())
    );
    assert_eq!(plan.commands[2].step, "manifest");
}

#[test]
fn image_plan_kernel_only_does_not_generate_manifest() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let args = ImageBuildArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: repo_root.join("assets"),
        arch: Some("arm64".to_string()),
        template: ImageBuildTemplate::Kernel,
        clean: true,
        json: true,
    };

    let plan = image_build_plan(&args).expect("image plan");

    assert_eq!(
        plan.commands
            .iter()
            .map(|command| command.step.as_str())
            .collect::<Vec<_>>(),
        vec!["kernel"]
    );
}

#[test]
fn image_clean_rootfs_preserves_kernel_and_initrd() {
    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join("arm64");
    fs::create_dir_all(&arch_dir).expect("arch dir");
    fs::write(arch_dir.join("vmlinuz"), b"kernel").expect("kernel");
    fs::write(arch_dir.join("initrd.img"), b"initrd").expect("initrd");
    fs::write(arch_dir.join("rootfs.erofs"), b"rootfs").expect("rootfs");
    fs::write(arch_dir.join("obom.cdx.json"), b"obom").expect("obom");

    clean_image_outputs(&ImageBuildPlan {
        schema: "test",
        profile_id: "code".to_string(),
        profile_revision: "test".to_string(),
        guest_dir: "guest".to_string(),
        output: temp.path().display().to_string(),
        clean: true,
        template: "rootfs",
        arches: vec![ImageBuildArchPlan {
            arch: "arm64".to_string(),
            kernel: "vmlinuz".to_string(),
            initrd: "initrd.img".to_string(),
            rootfs: "rootfs.erofs".to_string(),
        }],
        commands: Vec::new(),
    })
    .expect("rootfs clean");

    assert!(arch_dir.join("vmlinuz").is_file());
    assert!(arch_dir.join("initrd.img").is_file());
    assert!(!arch_dir.join("rootfs.erofs").exists());
    assert!(!arch_dir.join("obom.cdx.json").exists());
}

#[test]
fn image_clean_kernel_preserves_rootfs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join("arm64");
    fs::create_dir_all(&arch_dir).expect("arch dir");
    fs::write(arch_dir.join("vmlinuz"), b"kernel").expect("kernel");
    fs::write(arch_dir.join("initrd.img"), b"initrd").expect("initrd");
    fs::write(arch_dir.join("rootfs.erofs"), b"rootfs").expect("rootfs");

    clean_image_outputs(&ImageBuildPlan {
        schema: "test",
        profile_id: "code".to_string(),
        profile_revision: "test".to_string(),
        guest_dir: "guest".to_string(),
        output: temp.path().display().to_string(),
        clean: true,
        template: "kernel",
        arches: vec![ImageBuildArchPlan {
            arch: "arm64".to_string(),
            kernel: "vmlinuz".to_string(),
            initrd: "initrd.img".to_string(),
            rootfs: "rootfs.erofs".to_string(),
        }],
        commands: Vec::new(),
    })
    .expect("kernel clean");

    assert!(!arch_dir.join("vmlinuz").exists());
    assert!(!arch_dir.join("initrd.img").exists());
    assert!(arch_dir.join("rootfs.erofs").is_file());
}

#[test]
fn image_plan_rejects_arch_missing_from_profile() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let args = ImageBuildArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: repo_root.join("assets"),
        arch: Some("riscv64".to_string()),
        template: ImageBuildTemplate::All,
        clean: false,
        json: false,
    };

    let error = image_build_plan(&args).expect_err("unknown arch rejected");

    assert!(
        error
            .to_string()
            .contains("does not define assets for arch riscv64"),
        "{error:#}"
    );
}

#[test]
fn image_workspace_materializes_self_contained_profile_config() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let args = ImageWorkspaceArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: temp.path().join("workspace"),
        arch: Some("arm64".to_string()),
        json: true,
    };

    let report = materialize_image_workspace(&args).expect("workspace");

    assert_eq!(report.profile_id, "code");
    assert_eq!(report.arches.len(), 1);
    assert_eq!(report.arches[0].arch, "arm64");
    assert_eq!(report.rule_files.len(), 2);
    let workspace_profile = args.output.join("config/profiles/code/profile.toml");
    assert!(workspace_profile.is_file());
    assert!(args
        .output
        .join("config/profiles/code/enforcement.toml")
        .is_file());
    assert!(args
        .output
        .join("config/profiles/code/detection.yaml")
        .is_file());
    assert!(args.output.join("build-plan.json").is_file());
    assert!(args.output.join("workspace.json").is_file());
    let generated_config = args.output.join("guest").join("config");
    assert!(generated_config.join("packages/apt.toml").is_file());
    let source_build = repo_root.join("config/docker/image/build.toml");
    let generated_build = generated_config.join("build.toml");
    assert_eq!(
        fs::read(&generated_build).expect("materialized build config"),
        fs::read(&source_build).expect("source build config"),
        "the image workspace must copy the one authoritative build contract byte-for-byte"
    );
    let build_config: toml::Value = toml::from_str(
        &fs::read_to_string(&generated_build).expect("read materialized build config"),
    )
    .expect("parse materialized build config");
    let kernel = build_config["build"]["kernel"]
        .as_table()
        .expect("one common kernel source table");
    assert!(kernel.get("version").is_some());
    assert!(kernel.get("sha256").is_some());
    let apt_packages = fs::read_to_string(generated_config.join("packages/apt.toml"))
        .expect("materialized apt packages");
    assert!(
        apt_packages.contains("\"zstd\""),
        "Ollama's official installer consumes .tar.zst payloads, so shipped profiles must include zstd"
    );
    assert!(generated_config.join("packages/python.toml").is_file());
    assert!(generated_config
        .join("packages/python-requirements.lock")
        .is_file());
    assert!(generated_config.join("packages/npm.toml").is_file());
    assert!(generated_config.join("packages/npm-package.json").is_file());
    assert!(generated_config
        .join("packages/npm-package-lock.json")
        .is_file());
    let resources = fs::read_to_string(generated_config.join("vm/resources.toml"))
        .expect("materialized VM resources");
    assert!(resources.contains("ram_gb = 12"));
    assert!(resources.contains("scratch_disk_size_gb = 64"));
    assert!(args.output.join("guest/profile-build.sh").is_file());
    let profile_build = fs::read_to_string(args.output.join("guest/profile-build.sh"))
        .expect("materialized profile build script");
    assert!(profile_build.contains("CAPSEM_OLLAMA_SHA256"));
    assert!(!profile_build.contains("https://ollama.com/install.sh"));
    assert!(args
        .output
        .join("guest/profile-root/root/.codex/config.toml")
        .is_file());
    assert!(args.output.join("guest/artifacts/tips.txt").is_file());
    let build_plan: serde_json::Value =
        serde_json::from_slice(&fs::read(args.output.join("build-plan.json")).unwrap())
            .unwrap();
    assert!(build_plan["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == args.output.join("guest").display().to_string().as_str())));

    let copied = check_profile(&ProfileCheckArgs {
        path: workspace_profile,
        config_root: Some(args.output.join("config")),
        arch: None,
        json: true,
    })
    .expect("copied workspace profile validates and owns every pinned payload");
    assert_eq!(copied.validation.profile_id, "code");
    assert!(copied.profile_files.iter().all(|file| file.present));
}

#[test]
fn image_workspace_removes_stale_profile_root_payloads_before_materializing() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("workspace");
    let stale_profile_root = output.join("guest/profile-root/root/.gemini/config/config.json");
    fs::create_dir_all(stale_profile_root.parent().unwrap()).expect("stale parent");
    fs::write(
        &stale_profile_root,
        r#"{"ai":{"provider":"ollama","baseUrl":"http://127.0.0.1:11434"}}"#,
    )
    .expect("stale provider override");
    let stale_deleted_file = output.join("guest/profile-root/root/.stale-local-provider.json");
    fs::write(&stale_deleted_file, r#"{"provider":"ollama"}"#).expect("stale file");

    let args = ImageWorkspaceArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: output.clone(),
        arch: Some("arm64".to_string()),
        json: true,
    };

    materialize_image_workspace(&args).expect("workspace");

    let materialized_config =
        fs::read_to_string(&stale_profile_root).expect("materialized AGY provider config");
    assert_eq!(materialized_config.trim(), "{}");
    assert!(
        !stale_deleted_file.exists(),
        "removed profile-root payloads must not survive into rebuilt image workspaces"
    );
}

#[test]
fn profile_materialize_writes_generated_config_from_manifest() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let assets_dir = temp.path().join("assets");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let output_root = temp.path().join("target/config");
    let source_profile = repo_root.join("config/profiles/code/profile.toml");
    let original_source = fs::read_to_string(&source_profile).expect("read source profile");

    let report = materialize_profile_config(&ProfileMaterializeArgs {
        profile: source_profile.clone(),
        config_root: repo_root.join("config"),
        manifest: file_url(&manifest_path),
        assets_dir: assets_dir.clone(),
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: true,
        json: true,
    })
    .expect("materialize profile config");

    assert_eq!(report.profile_id, "code");
    assert_eq!(report.materialized_assets.len(), 3);
    assert_eq!(report.materialized_obom.len(), 1);
    assert!(output_root.join("settings/settings.toml").is_file());
    assert!(output_root.join("corp/corp.toml").is_file());
    assert!(output_root.join("assets/manifest.json").is_file());
    assert!(output_root.join("profiles/code/enforcement.toml").is_file());
    assert!(output_root.join("profiles/code/detection.yaml").is_file());

    let generated_profile_path = output_root.join("profiles/code/profile.toml");
    let generated: ProfileConfigFile =
        toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated"))
            .expect("parse generated profile");
    let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
    assert!(arm64.kernel.url.starts_with("file://"));
    assert!(arm64.initrd.url.starts_with("file://"));
    assert!(arm64.rootfs.url.starts_with("file://"));
    assert_eq!(
        arm64.kernel.hash,
        Some(format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex()))
    );
    assert_eq!(arm64.initrd.size, Some(b"initrd-arm64".len() as u64));
    assert_eq!(arm64.rootfs.name, "rootfs.erofs");
    assert!(generated
        .files
        .iter()
        .all(|(_, descriptor)| descriptor.hash.is_some() && descriptor.size.is_some()));
    let obom = generated
        .obom
        .as_ref()
        .expect("materialized profile has base-image OBOM")
        .arch
        .get("arm64")
        .expect("arm64 OBOM");
    assert!(obom.url.starts_with("file://"));
    assert_eq!(
        obom.hash,
        format!(
            "blake3:{}",
            blake3::hash(test_obom_json().as_bytes()).to_hex()
        )
    );
    assert_eq!(obom.generator, "cdxgen");
    assert_eq!(obom.generator_version, "11.0.0");

    let validation = validate_materialized_profile(&generated_profile_path, Some(&output_root))
        .expect("valid materialized output");
    assert_eq!(validation.profile_id, "code");
    assert_eq!(
        fs::read_to_string(source_profile).expect("read source profile after"),
        original_source,
        "materialization must not mutate checked-in source profile"
    );
}

#[test]
fn profile_materialize_remote_manifest_derives_release_site_asset_urls() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let manifest_json = fs::read_to_string(&manifest_path).expect("manifest");
    let manifest_url = serve_manifest_once(manifest_json);
    let output_root = temp.path().join("target/config");

    materialize_profile_config(&ProfileMaterializeArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        manifest: manifest_url.clone(),
        assets_dir: temp.path().join("no-local-assets"),
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: true,
        json: true,
    })
    .expect("remote manifest materializes without local asset blobs");

    let generated_profile_path = output_root.join("profiles/code/profile.toml");
    let generated: ProfileConfigFile =
        toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated"))
            .expect("parse generated profile");
    let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
    let expected_base = manifest_url.replace(
        "/assets/stable/manifest.json",
        "/assets/releases/2030.0101.1",
    );
    assert_eq!(arm64.kernel.url, format!("{expected_base}/arm64-vmlinuz"));
    assert_eq!(
        arm64.initrd.url,
        format!("{expected_base}/arm64-initrd.img")
    );
    assert_eq!(
        arm64.rootfs.url,
        format!("{expected_base}/arm64-rootfs.erofs")
    );
    assert_eq!(
        arm64.kernel.hash,
        Some(format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex()))
    );
    let obom = generated
        .obom
        .as_ref()
        .expect("remote OBOM descriptor")
        .arch
        .get("arm64")
        .expect("arm64 OBOM");
    assert_eq!(obom.url, format!("{expected_base}/arm64-obom.cdx.json"));
    assert_eq!(obom.generator, "remote");
    assert_eq!(obom.generator_version, "unknown");
}

#[test]
fn profile_materialize_release_channel_manifest_uses_profile_image_urls() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let output_root = temp.path().join("target/config");
    let local_obom = temp.path().join("resolved-obom.cdx.json");
    fs::write(&local_obom, test_obom_json()).expect("write resolved OBOM");
    let local_obom_url = file_url(&local_obom);
    let software_inventory = test_software_inventory_json("arm64");
    let local_software_inventory = temp.path().join("resolved-software-inventory.json");
    fs::write(&local_software_inventory, &software_inventory)
        .expect("write resolved software inventory");
    let local_software_inventory_url = file_url(&local_software_inventory);
    let digest = |bytes: &[u8]| {
        serde_json::json!({
            "sha256": format!("{:x}", Sha256::digest(bytes)),
            "blake3": blake3::hash(bytes).to_hex().to_string(),
        })
    };
    let manifest_json = serde_json::to_string_pretty(&serde_json::json!({
        "version": "1.5.0+assets.2030.0101.1",
        "status": "current",
        "packages": [],
        "profiles": {
            "code": {
                "version": "2030.0101.1",
                "id": "code",
                "name": "Code",
                "revision": "2030.0101.1",
                "status": "current",
                "min_capsem_version": "1.5.0",
                "architectures": [
                    {
                        "architecture": "arm64",
                        "images": [
                            {
                                "kind": "kernel",
                                "name": "vmlinuz",
                                "url": "/profiles/releases/2030.0101.1/code/arm64/vmlinuz",
                                "bytes": b"kernel-arm64".len(),
                                "digest": digest(b"kernel-arm64"),
                                "status": "current"
                            },
                            {
                                "kind": "initrd",
                                "name": "initrd.img",
                                "url": "https://cdn.example.test/initrd.img",
                                "bytes": b"initrd-arm64".len(),
                                "digest": digest(b"initrd-arm64"),
                                "status": "current"
                            },
                            {
                                "kind": "rootfs",
                                "name": "rootfs.erofs",
                                "url": "/profiles/releases/2030.0101.1/code/arm64/rootfs.erofs",
                                "bytes": b"rootfs-arm64".len(),
                                "digest": digest(b"rootfs-arm64"),
                                "status": "current"
                            }
                        ],
                        "evidence": [
                            {
                                "kind": "obom",
                                "name": "obom.cdx.json",
                                "url": local_obom_url,
                                "bytes": test_obom_json().len(),
                                "digest": digest(test_obom_json().as_bytes()),
                                "status": "current"
                            },
                            {
                                "kind": "software_inventory",
                                "url": local_software_inventory_url,
                                "bytes": software_inventory.len(),
                                "digest": digest(software_inventory.as_bytes()),
                                "status": "current"
                            }
                        ]
                    }
                ]
            }
        }
    }))
    .expect("release channel manifest");
    let manifest_url = serve_manifest_once(manifest_json);

    materialize_profile_config(&ProfileMaterializeArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        manifest: manifest_url.clone(),
        assets_dir: temp.path().join("no-local-assets"),
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: true,
        json: true,
    })
    .expect("release channel manifest materializes without local asset blobs");

    let generated_profile_path = output_root.join("profiles/code/profile.toml");
    let generated: ProfileConfigFile =
        toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated"))
            .expect("parse generated profile");
    let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
    let expected_origin = manifest_url.replace("/assets/stable/manifest.json", "");
    assert_eq!(
        arm64.kernel.url,
        format!("{expected_origin}/profiles/releases/2030.0101.1/code/arm64/vmlinuz")
    );
    assert_eq!(arm64.initrd.url, "https://cdn.example.test/initrd.img");
    assert_eq!(
        arm64.rootfs.url,
        format!("{expected_origin}/profiles/releases/2030.0101.1/code/arm64/rootfs.erofs")
    );
    assert_eq!(
        arm64.rootfs.hash,
        Some(format!("blake3:{}", blake3::hash(b"rootfs-arm64").to_hex()))
    );

    let obom = generated
        .obom
        .as_ref()
        .expect("release channel OBOM descriptor")
        .arch
        .get("arm64")
        .expect("arm64 OBOM");
    assert_eq!(obom.url, local_obom_url);
    assert_eq!(obom.generator, "cdxgen");
    assert_eq!(obom.generator_version, "11.0.0");

    let converted_manifest_path = output_root.join("assets/manifest.json");
    let converted = ManifestV2::from_json(
        &fs::read_to_string(&converted_manifest_path).expect("read converted manifest"),
    )
    .expect("converted release channel manifest is raw v2");
    assert_eq!(converted.format, 2);
    assert_eq!(converted.assets.current, "2030.0101.1");
    let converted_assets = converted.assets.releases["2030.0101.1"]
        .arches
        .get("arm64")
        .expect("converted arm64 assets");
    assert!(converted_assets.contains_key("obom.cdx.json"));
    assert!(converted_assets.contains_key("software-inventory.json"));
    assert_eq!(
        converted_assets["software-inventory.json"].sha256,
        format!("{:x}", Sha256::digest(software_inventory.as_bytes()))
    );
    assert_eq!(
        converted_assets["initrd.img"].sha256,
        format!("{:x}", Sha256::digest(b"initrd-arm64"))
    );
}

#[test]
fn profile_materialize_preserves_previous_profiles_in_same_output_catalog() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let assets_dir = temp.path().join("assets");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let output_root = temp.path().join("target/config");
    let config_root = repo_root.join("config");

    materialize_profile_config(&ProfileMaterializeArgs {
        profile: config_root.join("profiles/co-work/profile.toml"),
        config_root: config_root.clone(),
        manifest: file_url(&manifest_path),
        assets_dir: assets_dir.clone(),
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: true,
        json: true,
    })
    .expect("materialize co-work");

    materialize_profile_config(&ProfileMaterializeArgs {
        profile: config_root.join("profiles/code/profile.toml"),
        config_root,
        manifest: file_url(&manifest_path),
        assets_dir,
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: false,
        json: true,
    })
    .expect("materialize code");

    for profile_id in ["co-work", "code"] {
        let generated_profile_path = output_root
            .join("profiles")
            .join(profile_id)
            .join("profile.toml");
        let generated: ProfileConfigFile = toml::from_str(
            &fs::read_to_string(&generated_profile_path).expect("read generated profile"),
        )
        .expect("generated profile parses");
        let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
        assert_eq!(
            arm64.kernel.hash,
            Some(format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex())),
            "{profile_id} kernel pin must remain generated"
        );
        assert_eq!(
            arm64.initrd.hash,
            Some(format!("blake3:{}", blake3::hash(b"initrd-arm64").to_hex())),
            "{profile_id} initrd pin must remain generated"
        );
        assert_eq!(
            arm64.rootfs.hash,
            Some(format!("blake3:{}", blake3::hash(b"rootfs-arm64").to_hex())),
            "{profile_id} rootfs pin must remain generated"
        );
        assert!(arm64.kernel.url.starts_with("file://"));
        assert!(arm64.initrd.url.starts_with("file://"));
        assert!(arm64.rootfs.url.starts_with("file://"));
    }
}

#[test]
fn profile_materialize_rejects_arch_missing_from_manifest() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");

    let error = materialize_profile_config(&ProfileMaterializeArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        manifest: file_url(&manifest_path),
        assets_dir: temp.path().join("assets"),
        output_root: temp.path().join("target/config"),
        arch: Some("x86_64".to_string()),
        clean: true,
        json: false,
    })
    .expect_err("missing manifest arch rejected");

    assert!(
        format!("{error:#}").contains("does not contain profile arch x86_64"),
        "{error:#}"
    );
}

#[test]
fn profile_materialize_manifest_source_must_be_url() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");

    let error = materialize_profile_config(&ProfileMaterializeArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        manifest: manifest_path.display().to_string(),
        assets_dir: temp.path().join("assets"),
        output_root: temp.path().join("target/config"),
        arch: Some("arm64".to_string()),
        clean: true,
        json: false,
    })
    .expect_err("bare manifest path rejected");

    assert!(
        format!("{error:#}").contains("manifest must be a URL"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_build_writes_manifest_under_channel_assets_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let manifest_url = file_url(&manifest_path);
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    let out_dir = temp.path().join("target/release-channel");

    let report = build_assets_channel(
        &manifest_url,
        &assets_dir,
        &profiles_dir,
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let channel_manifest = out_dir.join("assets/stable/manifest.json");
    let release_dir = out_dir.join("assets/releases/2030.0101.1");
    assert_eq!(report.manifest, channel_manifest.display().to_string());
    assert_eq!(report.copied_assets, 5);
    assert!(
        !out_dir.join("index.html").exists(),
        "human release pages are built by release-site Astro, not capsem-admin"
    );
    assert!(out_dir.join("health.json").is_file());
    assert!(channel_manifest.is_file());
    assert_eq!(
        fs::read(release_dir.join("arm64-vmlinuz")).expect("published kernel"),
        b"kernel-arm64"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let source = fs::metadata(assets_dir.join("arm64/vmlinuz")).unwrap();
        let release = fs::metadata(release_dir.join("arm64-vmlinuz")).unwrap();
        assert_ne!(
            source.ino(),
            release.ino(),
            "an external fixture is unclassified and must copy rather than fail open"
        );
    }
    assert!(release_dir.join("arm64-initrd.img").is_file());
    assert!(release_dir.join("arm64-rootfs.erofs").is_file());
    assert!(release_dir.join("arm64-obom.cdx.json").is_file());
    assert!(release_dir.join("arm64-software-inventory.json").is_file());
    let channel_manifest_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&channel_manifest).expect("channel manifest"))
            .expect("channel manifest json");
    let source_manifest_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("source manifest"))
            .expect("source manifest json");
    let kernel_artifact = channel_manifest_json["profiles"]
        .as_object()
        .expect("profiles object")
        .values()
        .flat_map(|profile| {
            profile["architectures"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|image| image["architecture"].as_str() == Some("arm64"))
                .flat_map(|image| image["images"].as_array().into_iter().flatten())
        })
        .find(|artifact| artifact["kind"].as_str() == Some("kernel"))
        .expect("arm64 kernel artifact");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let published = out_dir.join(
            kernel_artifact["url"]
                .as_str()
                .expect("kernel publication URL")
                .trim_start_matches('/'),
        );
        assert_ne!(
            fs::metadata(assets_dir.join("arm64/vmlinuz"))
                .unwrap()
                .ino(),
            fs::metadata(published).unwrap().ino(),
            "an external fixture must remain independent from published output"
        );
    }
    assert_eq!(
        kernel_artifact["digest"]["blake3"],
        source_manifest_json["assets"]["releases"]["2030.0101.1"]["arches"]["arm64"]["vmlinuz"]
            ["hash"]
    );
    assert!(
        kernel_artifact["digest"]["sha256"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64),
        "channel manifest must hydrate VM asset SHA-256"
    );
    let health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("health.json")).unwrap())
            .expect("health json parses");
    assert_eq!(
        health["schema"].as_str(),
        Some("capsem.assets_channel.health.v1")
    );
    assert_eq!(health["current"]["assets"].as_str(), Some("2030.0101.1"));
    assert_eq!(
        health["urls"]["manifest"].as_str(),
        Some("/assets/stable/manifest.json")
    );
    assert_eq!(
        health["urls"]["asset_base"].as_str(),
        Some("/assets/releases")
    );
    assert_eq!(
        health["assets"]["files"][0]["url"].as_str(),
        Some("/assets/releases/2030.0101.1/arm64-initrd.img")
    );
    assert!(
        health["updates"]["assets"]["files"].is_null(),
        "VM asset file inventory belongs under assets.files, not updates.assets.files"
    );
    assert_eq!(
        health["assets"]["compatibility"]["min_binary"].as_str(),
        Some("1.0.0")
    );
    assert_eq!(
        health["assets"]["requires_newer"]["binary"].as_bool(),
        Some(false)
    );
    assert_eq!(
        health["asset_releases"][0]["date"].as_str(),
        Some("2030-01-01")
    );
    assert_eq!(
        health["evidence"]["vm_oboms"][0]["url"].as_str(),
        Some("/assets/releases/2030.0101.1/arm64-obom.cdx.json")
    );
    assert_eq!(
        health["evidence"]["host_sboms"][0]["name"].as_str(),
        Some("capsem-sbom.spdx.json")
    );
    assert_eq!(
        health["evidence"]["host_binary_files"][1]["name"].as_str(),
        Some("capsem-sbom.spdx.json")
    );
    assert_eq!(
        health["evidence"]["attestations"][0]["name"].as_str(),
        Some("github_attestations_host")
    );
    assert_eq!(
        health["evidence"]["attestations"][0]["predicate_type"].as_str(),
        Some("https://slsa.dev/provenance/v1")
    );
    assert_eq!(
        health["evidence"]["attestations"][0]["verify_command"].as_str(),
        Some("gh attestation verify <subject-url> --owner google")
    );
    assert_eq!(
        health["evidence"]["attestations"][1]["name"].as_str(),
        Some("github_attestations_host_sbom")
    );
    assert_eq!(
        health["evidence"]["attestations"][1]["predicate_type"].as_str(),
        Some("https://spdx.dev/Document/v2.3")
    );
    assert_eq!(
        health["evidence"]["attestations"][1]["predicate_url"].as_str(),
        Some("https://github.com/google/capsem/releases/download/v1.0.0/capsem-sbom.spdx.json")
    );
    assert_eq!(
        health["evidence"]["attestations"][1]["subjects"][0].as_str(),
        Some("https://github.com/google/capsem/releases/download/v1.0.0/capsem-1.0.0.pkg")
    );
    assert_eq!(
        health["evidence"]["attestations"][2]["name"].as_str(),
        Some("github_attestations_vm_assets")
    );
    assert_eq!(
        health["evidence"]["attestations"][2]["predicate_url"].as_str(),
        Some("/assets/releases/2030.0101.1/arm64-obom.cdx.json")
    );
    assert_eq!(
        health["evidence"]["attestations"][2]["subjects"][0].as_str(),
        Some("/assets/releases/2030.0101.1/arm64-initrd.img")
    );
    assert_eq!(
        health["updates"]["binary"]["latest"].as_str(),
        health["current"]["binary"].as_str()
    );
    assert_eq!(
        health["updates"]["binary"]["current"].as_str(),
        health["current"]["binary"].as_str()
    );
    assert_eq!(
        health["updates"]["binary"]["source"].as_str(),
        Some("manifest.binaries.current")
    );
    assert_eq!(
        health["updates"]["assets"]["latest"].as_str(),
        Some("2030.0101.1")
    );
    assert_eq!(
        health["updates"]["assets"]["current"].as_str(),
        Some("2030.0101.1")
    );
    assert_eq!(
        health["updates"]["assets"]["manifest"].as_str(),
        Some("/assets/stable/manifest.json")
    );
    assert_eq!(
        health["updates"]["assets"]["asset_base"].as_str(),
        Some("/assets/releases")
    );
    assert_eq!(
        health["updates"]["assets"]["compatibility"]["min_binary"].as_str(),
        Some("1.0.0")
    );
    assert_eq!(
        health["updates"]["assets"]["requires_newer"]["binary"].as_bool(),
        Some(false)
    );
    assert_eq!(
        health["profiles"]["revision"].as_str(),
        health["updates"]["profiles"]["latest"].as_str()
    );
    assert!(
        health["profiles"]["compatibility"].is_null(),
        "profiles must not publish channel compatibility"
    );
    assert_eq!(health["profiles"]["min_binary"].as_str(), Some("1.0.0"));
    assert!(
        health["updates"]["profiles"]["compatibility"].is_null(),
        "profile update metadata must not publish channel compatibility"
    );
    assert_eq!(
        health["updates"]["profiles"]["state"].as_str(),
        Some("current")
    );
    assert_eq!(
        health["profiles"]["source"].as_str(),
        Some("manifest.profiles")
    );
    assert!(health["profiles"]["hash"].is_null());
    assert_eq!(
        health["updates"]["profiles"]["source"].as_str(),
        Some("manifest.profiles")
    );
    assert!(health["updates"]["profiles"]["hash"].is_null());
    assert_eq!(health["updates"]["images"]["latest"].as_str(), None);
    assert!(
        health["updates"]["images"]["latest"].is_null(),
        "unpublished image latest should be explicit null"
    );
    assert_eq!(
        health["updates"]["images"]["state"].as_str(),
        Some("not_published")
    );
    assert_eq!(
        health["updates"]["images"]["source"].as_str(),
        Some("manifest.profiles.images")
    );

    let check = check_assets_channel(&out_dir, "stable").expect("asset channel checks");
    assert_eq!(check.channel, "stable");
    assert_eq!(check.manifest, channel_manifest.display().to_string());
}

#[test]
fn release_graph_manifest_version_is_independent_from_package_and_assets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("target/release-channel");

    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let channels: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("channels.json")).unwrap())
            .expect("channels json");
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(out_dir.join("assets/stable/manifest.json")).unwrap(),
    )
    .expect("graph manifest json");

    assert_eq!(
        channels["channels"]["stable"]["manifests"][0]["version"].as_str(),
        Some("1.0.2")
    );
    assert_eq!(manifest["version"].as_str(), Some("1.0.2"));
    assert_eq!(manifest["packages"][0]["version"].as_str(), Some("1.0.0"));
    assert_eq!(
        manifest["profiles"]["code"]["architectures"][0]["image_revision"].as_str(),
        Some("2030.0101.1")
    );
}

#[test]
fn asset_attestation_predicate_uses_published_obom_url_shape() {
    let files = vec![
        AssetsChannelAssetFile {
            arch: "arm64".to_string(),
            logical_name: "initrd.img".to_string(),
            url: "/assets/releases/2030.0101.1/arm64-initrd.img".to_string(),
            hash: "1".repeat(64),
            size: 1,
        },
        AssetsChannelAssetFile {
            arch: "arm64".to_string(),
            logical_name: "arm64-obom.cdx.json".to_string(),
            url: "/assets/releases/2030.0101.1/arm64-obom.cdx.json".to_string(),
            hash: "2".repeat(64),
            size: 1,
        },
    ];

    let attestations = current_asset_attestations(&files);

    assert_eq!(attestations.len(), 1);
    assert_eq!(
        attestations[0].predicate_url.as_deref(),
        Some("/assets/releases/2030.0101.1/arm64-obom.cdx.json")
    );
}

#[test]
fn assets_channel_build_preserves_existing_channels_when_adding_nightly() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let manifest_url = file_url(&manifest_path);
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    let out_dir = temp.path().join("target/release-channel");

    build_assets_channel(
        &manifest_url,
        &assets_dir,
        &profiles_dir,
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("stable channel builds");
    let stable_channels: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("channels.json")).unwrap())
            .expect("stable channels json");
    let stable_manifest_url = stable_channels["channels"]["stable"]["manifests"][0]["url"]
        .as_str()
        .expect("stable manifest url")
        .to_string();

    build_assets_channel(
        &manifest_url,
        &assets_dir,
        &profiles_dir,
        "nightly",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("nightly channel builds without erasing stable");

    let channels: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("channels.json")).unwrap())
            .expect("merged channels json");
    let channel_ids = channels["channels"]
        .as_object()
        .expect("channels object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        channel_ids,
        vec!["nightly".to_string(), "stable".to_string()]
    );
    assert_eq!(
        channels["channels"]["stable"]["manifests"][0]["url"].as_str(),
        Some(stable_manifest_url.as_str())
    );
    assert!(out_dir
        .join(stable_manifest_url.trim_start_matches('/'))
        .is_file());
    assert!(out_dir.join("assets/stable/manifest.json").is_file());
    assert!(out_dir.join("assets/nightly/manifest.json").is_file());
    let nightly_manifest_url = channels["channels"]["nightly"]["manifests"][0]["url"]
        .as_str()
        .expect("nightly manifest url");
    assert!(out_dir
        .join(nightly_manifest_url.trim_start_matches('/'))
        .is_file());

    check_assets_channel(&out_dir, "stable").expect("merged stable channel checks");
    fs::remove_file(out_dir.join("index.html")).expect("remove stable test index fixture");
    check_assets_channel(&out_dir, "nightly").expect("merged nightly channel checks");
}

#[test]
fn assets_channel_build_bootstraps_without_binary_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
            .expect("manifest json");
    manifest["binaries"]["releases"]["1.0.0"]
        .as_object_mut()
        .expect("binary release")
        .remove("files");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    let out_dir = temp.path().join("target/release-channel");

    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("first asset channel builds before binary evidence exists");

    let health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("health.json")).unwrap())
            .expect("health json parses");
    assert_eq!(
        health["evidence"]["host_binary_files"],
        serde_json::json!([])
    );
    assert_eq!(health["evidence"]["host_sboms"], serde_json::json!([]));
    assert!(health["evidence"]["attestations"]
        .as_array()
        .expect("attestations")
        .iter()
        .any(|item| item["name"] == "github_attestations_vm_assets"));

    check_assets_channel(&out_dir, "stable")
        .expect("first asset channel checks before binary evidence exists");
}

#[test]
fn assets_channel_headers_split_mutable_and_immutable_paths() {
    let headers = render_assets_channel_headers("stable");

    assert!(headers.contains("/\n  Cache-Control: no-cache, must-revalidate"));
    assert!(headers.contains("/index.html\n  Cache-Control: no-cache, must-revalidate"));
    assert!(headers.contains("/health.json\n  Cache-Control: no-cache, must-revalidate"));
    assert!(headers.contains("/assets/stable/*\n  Cache-Control: no-cache, must-revalidate"));
    assert!(!headers.contains("/profiles/stable/*\n  Cache-Control: no-cache"));
    assert!(headers
        .contains("/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable"));
    assert!(headers.contains(
        "/profiles/releases/*\n  Cache-Control: public, max-age=31536000, immutable"
    ));
    assert!(!headers.contains("/assets/*\n  Cache-Control: no-cache"));
    assert!(!headers.contains("/profiles/*\n  Cache-Control: no-cache"));
}

#[test]
fn host_spdx_requires_sha256_file_checksums() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sbom_path = temp.path().join("capsem-sbom.spdx.json");
    let sha1_only = br#"{
          "spdxVersion": "SPDX-2.3",
          "files": [
            {
              "SPDXID": "SPDXRef-File-capsem-gateway",
              "checksums": [
                {
                  "algorithm": "SHA1",
                  "checksumValue": "2a2bebeee60f894f3599e06c755c91944f1c3cc8"
                }
              ]
            }
          ]
        }"#;

    let error = validate_host_spdx_sbom_bytes(sha1_only, &sbom_path)
        .expect_err("SHA1-only SPDX file checksums rejected");

    assert!(
        format!("{error:#}").contains("missing SHA256 checksum"),
        "{error:#}"
    );
}

#[test]
fn host_spdx_accepts_sha256_file_checksums() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sbom_path = temp.path().join("capsem-sbom.spdx.json");
    let with_sha256 = br#"{
          "spdxVersion": "SPDX-2.3",
          "files": [
            {
              "SPDXID": "SPDXRef-File-capsem-gateway",
              "checksums": [
                {
                  "algorithm": "SHA256",
                  "checksumValue": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                }
              ]
            }
          ]
        }"#;

    validate_host_spdx_sbom_bytes(with_sha256, &sbom_path)
        .expect("SPDX file with SHA256 checksum validates");
}

#[test]
fn binary_files_from_deb_records_contained_executable_inventory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let deb_path = temp.path().join("Capsem_1.4.1234567890_arm64.deb");
    let executable = b"real capsem executable bytes";
    write_minimal_deb_with_file(
        &deb_path,
        "usr/bin/capsem-app",
        executable,
        release_graph::PackageArchitecture::Arm64,
    );

    let files = binary_files_from_artifacts(&[deb_path]).expect("binary files");

    assert_eq!(files.len(), 1);
    let package = &files[0];
    assert_eq!(package.name, "Capsem_1.4.1234567890_arm64.deb");
    assert_eq!(package.binaries.len(), 1);
    let binary = &package.binaries[0];
    assert_eq!(binary.name, "capsem-app");
    assert_eq!(binary.installed_path, "/usr/bin/capsem-app");
    assert_eq!(binary.size, executable.len() as u64);
    assert_eq!(binary.sha256, format!("{:x}", Sha256::digest(executable)));
    assert_eq!(binary.blake3, blake3::hash(executable).to_hex().to_string());
    assert_eq!(binary.sbom_component_ref, "SPDXRef-File-capsem-app");
}

#[test]
fn binary_files_from_deb_rejects_filename_control_architecture_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let deb_path = temp.path().join("Capsem_1.4.1234567890_amd64.deb");
    write_minimal_deb_with_file(
        &deb_path,
        "usr/bin/capsem-app",
        b"executable",
        release_graph::PackageArchitecture::Arm64,
    );

    let error = binary_files_from_artifacts(&[deb_path])
        .expect_err("filename/control architecture mismatch rejected");

    assert!(
        format!("{error:#}")
            .contains("filename architecture amd64 does not match control Architecture arm64"),
        "{error:#}"
    );
}

#[test]
fn binary_files_from_deb_rejects_missing_control_architecture() {
    let temp = tempfile::tempdir().expect("tempdir");
    let deb_path = temp.path().join("Capsem_1.4.1234567890_amd64.deb");
    write_minimal_deb_with_control(
        &deb_path,
        "usr/bin/capsem-app",
        b"executable",
        b"Package: capsem\nVersion: 1.0.0\n",
    );

    let error = binary_files_from_artifacts(&[deb_path])
        .expect_err("missing control Architecture rejected");

    assert!(
        format!("{error:#}").contains("missing Architecture"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_record_binary_updates_manifest_without_changing_assets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let original: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
            .expect("json");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
    let deb_path = artifacts_dir.join("Capsem_1.4.1234567890_arm64.deb");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &pkg_path,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"pkg executable bytes",
    );
    write_minimal_deb_with_file(
        &deb_path,
        "usr/bin/capsem-app",
        b"deb executable bytes",
        release_graph::PackageArchitecture::Arm64,
    );
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let report = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        None,
        &[pkg_path.clone(), deb_path.clone(), sbom_path.clone()],
        "2030-02-03",
    )
    .expect("record binary release");

    assert_eq!(
        report.schema,
        "capsem.admin.assets_channel_record_binary.v1"
    );
    assert_eq!(report.version, "1.4.1234567890");
    assert_eq!(report.min_assets, "2030.0101.1");
    assert_eq!(report.files.len(), 3);
    let updated: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
            .expect("json");
    assert_eq!(updated["assets"], original["assets"]);
    assert_eq!(updated["binaries"]["current"], "1.4.1234567890");
    let release = &updated["binaries"]["releases"]["1.4.1234567890"];
    assert_eq!(release["date"], "2030-02-03");
    assert_eq!(release["deprecated"], false);
    assert_eq!(release["min_assets"], "2030.0101.1");
    assert_eq!(release["version"], "1.4.1234567890");
    assert_eq!(release["files"].as_array().expect("files").len(), 3);
    assert_eq!(release["files"][0]["name"], "Capsem-1.4.1234567890.pkg");
    assert_eq!(
        release["files"][0]["sha256"],
        format!(
            "{:x}",
            Sha256::digest(fs::read(&pkg_path).expect("pkg bytes"))
        )
    );
    assert_eq!(
        release["files"][1]["binaries"][0]["installed_path"].as_str(),
        Some("/usr/bin/capsem-app")
    );
    assert_eq!(release["files"][2]["name"], "capsem-sbom.spdx.json");
}

#[test]
fn assets_channel_record_binary_updates_graph_manifest_without_changing_profiles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_release_graph_manifest(temp.path());
    let original: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
            .expect("json");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
    let deb_path = artifacts_dir.join("Capsem_1.4.1234567890_amd64.deb");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &pkg_path,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"pkg executable bytes",
    );
    write_minimal_deb_with_file(
        &deb_path,
        "usr/bin/capsem-app",
        b"deb executable bytes",
        release_graph::PackageArchitecture::Amd64,
    );
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let report = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        None,
        &[pkg_path.clone(), deb_path.clone(), sbom_path.clone()],
        "2030-02-03",
    )
    .expect("record graph binary release");

    assert_eq!(report.version, "1.4.1234567890");
    assert_eq!(report.min_assets, "2030.0101.1");
    let updated: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
            .expect("json");
    assert_eq!(updated["profiles"], original["profiles"]);
    assert!(updated.get("assets").is_none());
    assert!(updated.get("binaries").is_none());
    assert_eq!(updated["packages"].as_array().expect("packages").len(), 2);
    assert_eq!(updated["packages"][0]["name"], "Capsem-1.4.1234567890.pkg");
    assert_eq!(updated["packages"][0]["version"], "1.4.1234567890");
    assert_eq!(updated["packages"][0]["status"], "current");
    assert_eq!(updated["packages"][0]["platform"], "macos");
    assert_eq!(updated["packages"][0]["architecture"], "arm64");
    assert_eq!(
        updated["packages"][0]["digest"]["sha256"],
        format!(
            "{:x}",
            Sha256::digest(fs::read(&pkg_path).expect("pkg bytes"))
        )
    );
    assert_eq!(
        updated["packages"][0]["binaries"][0]["installed_path"].as_str(),
        Some("/Applications/Capsem.app/Contents/MacOS/capsem-app")
    );
    assert_eq!(
        updated["packages"][0]["evidence"][0]["name"].as_str(),
        Some("capsem-sbom.spdx.json")
    );
    assert_eq!(
        updated["packages"][0]["evidence"][0]["url"].as_str(),
        Some("https://github.com/google/capsem/releases/download/v1.4.1234567890/capsem-sbom.spdx.json")
    );
    assert_eq!(
        updated["packages"][1]["binaries"][0]["installed_path"].as_str(),
        Some("/usr/bin/capsem-app")
    );
    assert_eq!(
        updated["packages"][1]["name"],
        "Capsem_1.4.1234567890_amd64.deb"
    );
    assert_eq!(updated["packages"][1]["architecture"], "amd64");
}

#[test]
fn staged_profile_then_binary_activation_enforces_bounds_without_rebuilding_profile() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_release_graph_manifest(temp.path());
    let mut staged: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
            .expect("json");
    staged["profiles"]["co-work"]["version"] =
        serde_json::Value::String("2030.0203.1".to_string());
    staged["profiles"]["co-work"]["revision"] =
        serde_json::Value::String("2030.0203.1".to_string());
    staged["profiles"]["co-work"]["min_capsem_version"] =
        serde_json::Value::String("1.4.1234567890".to_string());
    staged["profiles"]["co-work"]["max_capsem_version"] =
        serde_json::Value::String("1.4.1234567890".to_string());
    fs::write(
        &manifest_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&staged).expect("staged manifest")
        ),
    )
    .expect("write staged manifest");
    let staged_bytes = fs::read(&manifest_path).expect("staged bytes");
    let staged_profile = staged["profiles"]["co-work"].clone();

    assert!(
        !graph_profile_matches_current_binary(&staged_profile, &staged)
            .expect("old binary compatibility"),
        "the staged profile must remain private while the public binary is too old"
    );
    let error = build_assets_channel_from_graph(
        staged.clone(),
        "stable",
        "1.0.2",
        &temp.path().join("incompatible-dist"),
        "2030-02-03T04:05:06Z",
    )
    .expect_err("incompatible staged source cannot become a public distribution");
    assert!(
        format!("{error:#}").contains("co-work"),
        "the rejection must identify the incompatible profile: {error:#}"
    );

    let too_new_dir = temp.path().join("too-new-binary");
    fs::create_dir_all(&too_new_dir).expect("too-new artifact dir");
    let too_new_pkg = too_new_dir.join("Capsem-2.0.0.pkg");
    let too_new_deb = too_new_dir.join("Capsem_2.0.0_amd64.deb");
    let too_new_sbom = too_new_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &too_new_pkg,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"too new pkg executable bytes",
    );
    write_minimal_deb_with_file(
        &too_new_deb,
        "usr/bin/capsem-app",
        b"too new deb executable bytes",
        release_graph::PackageArchitecture::Amd64,
    );
    fs::write(&too_new_sbom, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("too-new SBOM");
    let error = record_binary_release_metadata(
        &manifest_path,
        "2.0.0",
        None,
        &[too_new_pkg, too_new_deb, too_new_sbom],
        "2030-02-03",
    )
    .expect_err("binary newer than the staged profile maximum must be rejected");
    assert!(
        format!("{error:#}").contains("co-work"),
        "the rejection must identify the incompatible profile: {error:#}"
    );
    assert_eq!(
        fs::read(&manifest_path).expect("manifest after rejected binary"),
        staged_bytes,
        "a rejected binary must not mutate the staged source manifest"
    );

    let compatible_dir = temp.path().join("compatible-binary");
    fs::create_dir_all(&compatible_dir).expect("compatible artifact dir");
    let compatible_pkg = compatible_dir.join("Capsem-1.4.1234567890.pkg");
    let compatible_deb = compatible_dir.join("Capsem_1.4.1234567890_amd64.deb");
    let compatible_sbom = compatible_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &compatible_pkg,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"compatible pkg executable bytes",
    );
    write_minimal_deb_with_file(
        &compatible_deb,
        "usr/bin/capsem-app",
        b"compatible deb executable bytes",
        release_graph::PackageArchitecture::Amd64,
    );
    fs::write(&compatible_sbom, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("compatible SBOM");
    record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        None,
        &[compatible_pkg, compatible_deb, compatible_sbom],
        "2030-02-03",
    )
    .expect("compatible binary activates staged profile");

    let activated: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("activated manifest"))
            .expect("activated json");
    assert_eq!(
        activated["profiles"]["co-work"], staged_profile,
        "binary activation must reuse the exact staged profile instead of rebuilding it"
    );
    assert!(graph_profile_matches_current_binary(
        &activated["profiles"]["co-work"],
        &activated
    )
    .expect("activated compatibility"));
    build_assets_channel_from_graph(
        activated,
        "stable",
        "1.0.2",
        &temp.path().join("activated-dist"),
        "2030-02-03T04:05:06Z",
    )
    .expect("compatible staged profile and binary can become public");
}

#[test]
fn assets_channel_record_binary_rejects_sbom_without_host_package() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        None,
        &[sbom_path],
        "2030-02-03",
    )
    .expect_err("SBOM-only binary metadata rejected");

    assert!(
        format!("{error:#}")
            .contains("binary release metadata must include a host package artifact"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_record_binary_rejects_non_package_host_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let readme_path = artifacts_dir.join("release-notes.txt");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    fs::write(&readme_path, b"not an installable package").expect("readme");
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        None,
        &[readme_path, sbom_path],
        "2030-02-03",
    )
    .expect_err("non-package host artifact rejected");

    assert!(
        format!("{error:#}")
            .contains("binary release metadata must include a .pkg or .deb artifact"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_record_binary_rejects_empty_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    fs::write(&pkg_path, []).expect("empty pkg");
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        None,
        &[pkg_path, sbom_path],
        "2030-02-03",
    )
    .expect_err("empty binary artifact rejected");

    assert!(
        format!("{error:#}").contains("binary release artifact is empty"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_record_binary_rejects_package_version_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.0000000000.pkg");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &pkg_path,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"pkg executable bytes",
    );
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        None,
        &[pkg_path, sbom_path],
        "2030-02-03",
    )
    .expect_err("mismatched package version rejected");

    assert!(
        format!("{error:#}")
            .contains("binary release package artifact name must match version"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_record_binary_rejects_noncanonical_sbom_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
    let sbom_path = artifacts_dir.join("host-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &pkg_path,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"pkg executable bytes",
    );
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        None,
        &[pkg_path, sbom_path],
        "2030-02-03",
    )
    .expect_err("noncanonical SBOM artifact rejected");

    assert!(
        format!("{error:#}").contains("capsem-sbom.spdx.json"),
        "{error:#}"
    );
}

fn write_minimal_pkg_with_file(path: &Path, file_path: &str, contents: &[u8]) {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("pkg root");
        let payload_path = root.path().join(file_path);
        fs::create_dir_all(payload_path.parent().expect("payload parent"))
            .expect("payload parent dir");
        fs::write(&payload_path, contents).expect("payload file");
        fs::set_permissions(&payload_path, fs::Permissions::from_mode(0o755))
            .expect("payload executable");

        let output = Command::new("pkgbuild")
            .arg("--root")
            .arg(root.path())
            .arg("--identifier")
            .arg("org.capsem.test")
            .arg("--version")
            .arg("1.4.1234567890")
            .arg(path)
            .output()
            .expect("run pkgbuild");
        assert!(
            output.status.success(),
            "pkgbuild failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(not(target_os = "macos"))]
    {
        use flate2::{write::GzEncoder, Compression};
        use tar::{Builder, Header};

        let mut pkg = Vec::new();
        {
            let encoder = GzEncoder::new(&mut pkg, Compression::default());
            let mut builder = Builder::new(encoder);
            let mut header = Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    format!("capsem.pkg/Payload/{file_path}"),
                    contents,
                )
                .expect("append pkg executable");
            let encoder = builder.into_inner().expect("finish tar");
            encoder.finish().expect("finish gzip");
        }
        fs::write(path, pkg).expect("write synthetic pkg");
    }
}

fn write_minimal_deb_with_file(
    path: &Path,
    file_path: &str,
    contents: &[u8],
    architecture: release_graph::PackageArchitecture,
) {
    let control = format!(
        "Package: capsem\nVersion: 1.0.0\nArchitecture: {}\n",
        architecture.as_str()
    );
    write_minimal_deb_with_control(path, file_path, contents, control.as_bytes());
}

fn write_minimal_deb_with_control(
    path: &Path,
    file_path: &str,
    contents: &[u8],
    control: &[u8],
) {
    use flate2::{write::GzEncoder, Compression};
    use tar::{Builder, Header};

    let mut control_tar_gz = Vec::new();
    {
        let encoder = GzEncoder::new(&mut control_tar_gz, Compression::default());
        let mut builder = Builder::new(encoder);
        let mut header = Header::new_gnu();
        header.set_size(control.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "control", control)
            .expect("append Debian control file");
        let encoder = builder.into_inner().expect("finish control tar");
        encoder.finish().expect("finish control gzip");
    }

    let mut data_tar_gz = Vec::new();
    {
        let encoder = GzEncoder::new(&mut data_tar_gz, Compression::default());
        let mut builder = Builder::new(encoder);
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, file_path, contents)
            .expect("append executable");
        let encoder = builder.into_inner().expect("finish tar");
        encoder.finish().expect("finish gzip");
    }

    let mut deb = Vec::new();
    deb.extend_from_slice(b"!<arch>\n");
    append_ar_member(&mut deb, "debian-binary", b"2.0\n");
    append_ar_member(&mut deb, "control.tar.gz", &control_tar_gz);
    append_ar_member(&mut deb, "data.tar.gz", &data_tar_gz);
    fs::write(path, deb).expect("write deb");
}

fn append_ar_member(out: &mut Vec<u8>, name: &str, contents: &[u8]) {
    use std::io::Write;

    let header = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
        format!("{name}/"),
        0,
        0,
        0,
        0o100644,
        contents.len()
    );
    assert_eq!(header.len(), 60);
    out.write_all(header.as_bytes()).expect("ar header");
    out.write_all(contents).expect("ar contents");
    if !contents.len().is_multiple_of(2) {
        out.write_all(b"\n").expect("ar padding");
    }
}

#[test]
fn assets_channel_build_externalizes_shared_blobs_but_owns_profile_blobs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("target/release-channel");
    let asset_base =
        "https://github.com/google/capsem/releases/download/assets-v{asset_version}";

    let report = build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-02-03T00:00:00Z",
        Some(asset_base),
    )
    .expect("externalized channel builds without local blobs");

    assert_eq!(report.copied_assets, 0);
    assert!(!out_dir.join("assets/releases").exists());
    assert!(out_dir.join("profiles/releases/stable").is_dir());
    let channel_manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(out_dir.join("assets/stable/manifest.json")).unwrap(),
    )
    .expect("channel manifest parses");
    let health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("health.json")).unwrap())
            .expect("health parses");
    let rootfs_url = "https://github.com/google/capsem/releases/download/assets-v2030.0101.1/arm64-rootfs.erofs";
    assert_eq!(health["urls"]["asset_base"].as_str(), Some(asset_base));
    let health_files = health["assets"]["files"].as_array().expect("asset files");
    assert!(health_files
        .iter()
        .any(|file| file["url"].as_str() == Some(rootfs_url)));
    assert!(
        serde_json::to_string(&channel_manifest)
            .expect("serialize channel manifest")
            .contains("/profiles/releases/stable/code/"),
        "selected channel manifest should carry channel/profile-owned URLs"
    );
    check_assets_channel(&out_dir, "stable").expect("externalized channel checks");
}

#[test]
fn assets_channel_check_rejects_bad_health_schema() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &assets_dir,
        &profiles_dir,
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
            .expect("health json");
    health["schema"] = serde_json::Value::String("capsem.bad_schema".to_string());
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write bad health");

    let error =
        check_assets_channel(&out_dir, "stable").expect_err("bad health schema rejected");

    assert!(
        format!("{error:#}").contains("health.json schema mismatch"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_allows_package_owned_sbom_without_host_sbom_summary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
            .expect("health json");
    health["evidence"]["host_sboms"] = serde_json::json!([]);
    health["evidence"]["attestations"]
        .as_array_mut()
        .expect("attestations")
        .retain(|attestation| {
            attestation.get("name").and_then(|name| name.as_str())
                != Some("github_attestations_host_sbom")
        });
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without host SBOM");

    check_assets_channel(&out_dir, "stable").expect("package-owned SBOMs are allowed");
}

#[test]
fn assets_channel_check_rejects_missing_asset_release_date() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
            .expect("health json");
    health["asset_releases"][0]
        .as_object_mut()
        .expect("asset release object")
        .remove("date");
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without asset release date");

    let error =
        check_assets_channel(&out_dir, "stable").expect_err("missing release date rejected");

    assert!(
        format!("{error:#}").contains("health.json asset release date mismatch"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_evidence_vm_obom() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
            .expect("health json");
    health["evidence"]["vm_oboms"] = serde_json::json!([]);
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without VM OBOM");

    let error = check_assets_channel(&out_dir, "stable").expect_err("missing VM OBOM rejected");

    assert!(
        format!("{error:#}").contains("health.json missing VM OBOM evidence"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_evidence_vm_attestation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
            .expect("health json");
    let attestations = health["evidence"]["attestations"]
        .as_array()
        .expect("attestations")
        .iter()
        .filter(|attestation| {
            attestation.get("name").and_then(|name| name.as_str())
                != Some("github_attestations_vm_assets")
        })
        .cloned()
        .collect::<Vec<_>>();
    health["evidence"]["attestations"] = serde_json::Value::Array(attestations);
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without VM attestation");

    let error =
        check_assets_channel(&out_dir, "stable").expect_err("missing VM attestation rejected");

    assert!(
        format!("{error:#}").contains("health.json VM asset attestation evidence missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_vm_attestation_predicate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
            .expect("health json");
    let attestations = health["evidence"]["attestations"]
        .as_array_mut()
        .expect("attestations");
    let vm_attestation = attestations
        .iter_mut()
        .find(|attestation| {
            attestation.get("name").and_then(|name| name.as_str())
                == Some("github_attestations_vm_assets")
        })
        .expect("VM asset attestation");
    vm_attestation
        .as_object_mut()
        .expect("attestation object")
        .remove("predicate_url");
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without VM predicate");

    let error = check_assets_channel(&out_dir, "stable")
        .expect_err("missing VM attestation predicate rejected");

    assert!(
        format!("{error:#}").contains("health.json VM asset attestation predicate_url missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_host_sbom_attestation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
            .expect("health json");
    let attestations = health["evidence"]["attestations"]
        .as_array()
        .expect("attestations")
        .iter()
        .filter(|attestation| {
            attestation.get("name").and_then(|name| name.as_str())
                != Some("github_attestations_host_sbom")
        })
        .cloned()
        .collect::<Vec<_>>();
    health["evidence"]["attestations"] = serde_json::Value::Array(attestations);
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without host SBOM attestation");

    let error = check_assets_channel(&out_dir, "stable")
        .expect_err("missing host SBOM attestation rejected");

    assert!(
        format!("{error:#}").contains("health.json host SBOM attestation evidence missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_host_sbom_attestation_missing_package_subject() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
            .expect("manifest json");
    manifest["binaries"]["releases"]["1.0.0"]["files"]
        .as_array_mut()
        .expect("binary files")
        .push(serde_json::json!({
            "name": "Capsem_1.0.0_arm64.deb",
            "size": 789,
            "sha256": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210",
            "blake3": "3333333333333333333333333333333333333333333333333333333333333333",
            "binaries": [
                {
                    "name": "capsem-tray",
                    "installed_path": "/usr/bin/capsem-tray",
                    "size": 19,
                    "sha256": "4444444444444444444444444444444444444444444444444444444444444444",
                    "blake3": "5555555555555555555555555555555555555555555555555555555555555555",
                    "sbom_component_ref": "SPDXRef-File-capsem-tray"
                }
            ]
        }));
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("manifest json"),
    )
    .expect("write manifest with deb");
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
            .expect("health json");
    let host_attestation = health["evidence"]["attestations"]
        .as_array_mut()
        .expect("attestations")
        .iter_mut()
        .find(|attestation| {
            attestation.get("name").and_then(|name| name.as_str())
                == Some("github_attestations_host")
        })
        .expect("host package attestation");
    let subjects = host_attestation["subjects"]
        .as_array_mut()
        .expect("host package subjects");
    *subjects = vec![serde_json::json!(
        "https://github.com/google/capsem/releases/download/v1.0.0/not-a-package"
    )];
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without deb SBOM subject");

    let error = check_assets_channel(&out_dir, "stable")
        .expect_err("missing host package SBOM subject rejected");

    assert!(
        format!("{error:#}").contains("health.json host package attestation subjects missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_attestation_without_verification_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health"))
            .expect("health json");
    health["evidence"]["attestations"][0]
        .as_object_mut()
        .expect("attestation object")
        .remove("verify_command");
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without verification metadata");

    let error = check_assets_channel(&out_dir, "stable")
        .expect_err("missing attestation verification metadata rejected");

    assert!(
        format!("{error:#}").contains("health.json attestation verify_command missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_current_asset_blob() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    let out_dir = temp.path().join("target/release-channel");
    build_assets_channel(
        &file_url(&manifest_path),
        &assets_dir,
        &profiles_dir,
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");
    fs::remove_file(out_dir.join("assets/releases/2030.0101.1/arm64-rootfs.erofs"))
        .expect("remove published rootfs");

    let error =
        check_assets_channel(&out_dir, "stable").expect_err("missing asset blob rejected");

    assert!(
        format!("{error:#}").contains("arm64-rootfs.erofs"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_rejects_unsafe_channel_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let manifest_url = file_url(&manifest_path);
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    for channel in ["../stable", "stable.v1", "stable channel", "<stable>"] {
        let error = build_assets_channel(
            &manifest_url,
            &assets_dir,
            &profiles_dir,
            channel,
            "1.0.2",
            &temp.path().join("target/release-channel"),
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect_err("unsafe channel rejected");

        assert!(error.to_string().contains("invalid asset channel name"));
    }
}

#[test]
fn assets_channel_manifest_source_must_be_url() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let error = build_assets_channel(
        &manifest_path.display().to_string(),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &temp.path().join("target/release-channel"),
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect_err("bare manifest path rejected");

    assert!(
        format!("{error:#}").contains("manifest must be a URL"),
        "{error:#}"
    );
}

#[test]
fn profile_release_commands_publish_report_is_lane_scoped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stable_manifest = temp.path().join("stable-manifest.json");
    let nightly_manifest = temp.path().join("nightly-manifest.json");
    write_profile_release_manifest(&stable_manifest, "1.4.0", "1.0.0", "deprecated");
    write_profile_release_manifest(
        &nightly_manifest,
        "1.5.0-nightly.20300101",
        "2026.7.2-2",
        "supported",
    );

    let args = ReleaseArgs {
        manifest_path: Some(nightly_manifest.clone()),
        candidate_manifest: None,
        publication_base: None,
        channel: "nightly".to_string(),
        manifest_version: Some("1.5.0-nightly.20300101".to_string()),
        profile: "co-work".to_string(),
        profile_version: Some("2026.7.2-2".to_string()),
        config_root: repo_config_profiles_dir()
            .parent()
            .expect("config root")
            .to_path_buf(),
        status: ProfileReleaseStatusArg::Current,
        bootstrap_from_manifest: None,
        bootstrap_output: None,
        dry_run: false,
        json: true,
    };

    let report = apply_profile_release_status(&args).expect("publish profile release");

    assert_eq!(report.schema, "capsem.admin.profile_release.v1");
    assert_eq!(report.action, "release");
    assert_eq!(report.status, release_graph::Status::Current);
    assert_eq!(report.changed_channels, vec!["nightly"]);
    assert_eq!(report.changed_manifests, vec!["1.5.0-nightly.20300101"]);
    assert_eq!(report.changed_profiles, vec!["co-work"]);
    assert_eq!(report.changed_config_refs, 1);
    assert_eq!(report.changed_image_artifacts, 3);

    let nightly: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&nightly_manifest).expect("nightly manifest"))
            .expect("nightly json");
    assert_eq!(
        nightly["profiles"]["co-work"]["status"].as_str(),
        Some("current")
    );
    assert_eq!(
        nightly["profiles"]["co-work"]["architectures"][0]["config"][0]["status"].as_str(),
        Some("current")
    );
    assert_eq!(
        nightly["profiles"]["co-work"]["architectures"][0]["images"][0]["status"].as_str(),
        Some("current")
    );

    let stable: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&stable_manifest).expect("stable manifest"))
            .expect("stable json");
    assert_eq!(
        stable["profiles"]["co-work"]["status"].as_str(),
        Some("deprecated"),
        "publishing nightly co-work must not mutate stable"
    );
}

#[test]
fn profile_release_commands_require_enum_status_values() {
    let error = Cli::try_parse_from([
        "capsem-admin",
        "release",
        "--manifest-path",
        "manifest.json",
        "--channel",
        "nightly",
        "--manifest-version",
        "1.5.0-nightly.20300101",
        "--profile",
        "co-work",
        "--profile-version",
        "2026.7.2-2",
        "--status",
        "removed",
    ])
    .expect_err("removed is not a release status");

    assert!(error.to_string().contains("invalid value"), "{error}");
}

#[test]
fn profile_release_paths_are_channel_qualified() {
    let stable = profile_release_url("stable", "code", "2026.06.08.7", "arm64", "rootfs.erofs")
        .expect("stable profile URL");
    let nightly =
        profile_release_url("nightly", "code", "2026.06.08.7", "arm64", "rootfs.erofs")
            .expect("nightly profile URL");

    assert_eq!(
        stable,
        "/profiles/releases/stable/code/2026.06.08.7/arm64/rootfs.erofs"
    );
    assert_eq!(
        nightly,
        "/profiles/releases/nightly/code/2026.06.08.7/arm64/rootfs.erofs"
    );
    assert_ne!(stable, nightly);
}

#[test]
fn release_command_has_one_operator_shape() {
    let cli = Cli::parse_from([
        "capsem-admin",
        "release",
        "--channel",
        "nightly",
        "--profile",
        "code",
        "--dry-run",
    ]);
    match cli.command {
        Commands::Release(args) => {
            assert_eq!(args.channel, "nightly");
            assert_eq!(args.profile, "code");
            assert!(args.manifest_path.is_none());
            assert!(args.dry_run);
        }
        _ => panic!("expected release command"),
    }
    assert_eq!(
        profile_publication_identity("nightly", "code", "2026.06.08.7")
            .expect("publication identity"),
        "profile-nightly-code-2026.06.08.7"
    );
    assert!(
        profile_publication_identity("nightly", "code", "revision/escape").is_err(),
        "publication identities must be safe immutable GitHub release tags"
    );
}

#[derive(Default)]
struct RecordingProfileWorkflowRunner {
    listings: std::collections::VecDeque<String>,
    calls: Vec<Vec<String>>,
    waits: usize,
    fail_watch: bool,
}

impl ProfileWorkflowRunner for RecordingProfileWorkflowRunner {
    fn run(&mut self, args: &[String]) -> Result<()> {
        self.calls.push(args.to_vec());
        if self.fail_watch && args.first().map(String::as_str) == Some("run") {
            return Err(anyhow!("watched profile workflow failed"));
        }
        Ok(())
    }

    fn output(&mut self, args: &[String]) -> Result<String> {
        self.calls.push(args.to_vec());
        self.listings
            .pop_front()
            .ok_or_else(|| anyhow!("unexpected workflow listing"))
    }

    fn wait_before_poll(&mut self) {
        self.waits += 1;
    }
}

#[test]
fn profile_release_dispatch_waits_for_its_exact_workflow_run() {
    let mut runner = RecordingProfileWorkflowRunner {
        listings: [
            "[]".to_string(),
            serde_json::json!([{
                "databaseId": 42,
                "displayTitle": "Release profile nightly/code dispatch-7",
            }])
            .to_string(),
        ]
        .into(),
        ..Default::default()
    };

    let run_id = dispatch_profile_workflow(
        &mut runner,
        "release-assets.yaml",
        "nightly",
        "code",
        "dispatch-7",
    )
    .expect("dispatch is found and watched");

    assert_eq!(run_id, 42);
    assert_eq!(runner.waits, 1);
    assert_eq!(
        runner.calls[0],
        [
            "workflow",
            "run",
            "release-assets.yaml",
            "--ref",
            "main",
            "-f",
            "channel=nightly",
            "-f",
            "profile=code",
            "-f",
            "dry_run=false",
            "-f",
            "dispatch_id=dispatch-7",
        ]
    );
    assert_eq!(
        runner.calls.last().expect("watch call"),
        &["run", "watch", "42", "--exit-status"]
    );
}

#[test]
fn profile_release_dispatch_ignores_an_unrelated_pending_run() {
    let mut runner = RecordingProfileWorkflowRunner {
        listings: [
            serde_json::json!([{
                "databaseId": 9,
                "displayTitle": "Release profile nightly/co-work somebody-else",
            }])
            .to_string(),
            serde_json::json!([{
                "databaseId": 10,
                "displayTitle": "Release profile nightly/code ours",
            }])
            .to_string(),
        ]
        .into(),
        ..Default::default()
    };

    let run_id = dispatch_profile_workflow(
        &mut runner,
        "release-assets.yaml",
        "nightly",
        "code",
        "ours",
    )
    .expect("the correlated run is selected");

    assert_eq!(run_id, 10);
    assert_eq!(runner.calls.last().expect("watch call")[2], "10");
}

#[test]
fn profile_release_dispatch_propagates_the_exact_run_failure() {
    let mut runner = RecordingProfileWorkflowRunner {
        listings: [serde_json::json!([{
            "databaseId": 11,
            "displayTitle": "Release profile nightly/code ours",
        }])
        .to_string()]
        .into(),
        fail_watch: true,
        ..Default::default()
    };

    let error = dispatch_profile_workflow(
        &mut runner,
        "release-assets.yaml",
        "nightly",
        "code",
        "ours",
    )
    .expect_err("the public command must fail with its exact workflow run");

    assert!(format!("{error:#}").contains("watched profile workflow failed"));
    assert_eq!(runner.calls.last().expect("watch call")[2], "11");
}

#[test]
fn profile_release_merges_only_selected_profile_and_reports_compatibility() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/capsem-release/fixtures/release-graph-stable-nightly.json");
    let graph: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture).expect("fixture"))
            .expect("fixture json");
    let base = graph["manifests"]["nightly"]["1.0.2"].clone();
    let mut candidate = base.clone();
    candidate["profiles"]["code"]["revision"] =
        serde_json::Value::String("2026.07.24.1".to_string());
    candidate["profiles"]["code"]["version"] =
        serde_json::Value::String("2026.07.24.1".to_string());
    candidate["profiles"]["code"]["min_capsem_version"] =
        serde_json::Value::String("9.0.0".to_string());
    let base_path = temp.path().join("base.json");
    let candidate_path = temp.path().join("candidate.json");
    fs::write(
        &base_path,
        serde_json::to_vec_pretty(&base).expect("base json"),
    )
    .expect("write base");
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&candidate).expect("candidate json"),
    )
    .expect("write candidate");
    let args = ReleaseArgs {
        channel: "nightly".to_string(),
        profile: "code".to_string(),
        config_root: PathBuf::from("config"),
        manifest_path: Some(base_path.clone()),
        candidate_manifest: Some(candidate_path),
        publication_base: Some(
            "https://github.com/google/capsem/releases/download/profile-nightly-code-2026.07.24.1"
                .to_string(),
        ),
        manifest_version: Some("1.0.2".to_string()),
        profile_version: Some("2026.07.24.1".to_string()),
        status: ProfileReleaseStatusArg::Current,
        bootstrap_from_manifest: None,
        bootstrap_output: None,
        dry_run: false,
        json: true,
    };

    let report = apply_profile_release_status(&args).expect("merge selected profile");
    let merged: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(base_path).expect("merged"))
            .expect("merged json");

    assert!(!report.compatible_with_current_binary);
    assert_eq!(report.changed_profiles, vec!["code"]);
    assert_eq!(merged["packages"], base["packages"]);
    assert_eq!(merged["profiles"]["co-work"], base["profiles"]["co-work"]);
    assert_eq!(
        merged["profiles"]["code"]["revision"].as_str(),
        Some("2026.07.24.1")
    );
    assert_eq!(
        merged["profiles"]["code"]["architectures"][0]["config"][0]["url"].as_str(),
        Some(
            "https://github.com/google/capsem/releases/download/profile-nightly-code-2026.07.24.1/arm64-profile.toml"
        )
    );
    assert_eq!(
        merged["profiles"]["code"]["architectures"][0]["images"][0]["url"].as_str(),
        Some(
            "https://github.com/google/capsem/releases/download/profile-nightly-code-2026.07.24.1/arm64-vmlinuz"
        )
    );
    assert!(merged["profiles"]["code"]["architectures"][0]["software"]
        .as_array()
        .expect("software rows")
        .iter()
        .all(|row| row["evidence"].as_str()
            == Some(
                "https://github.com/google/capsem/releases/download/profile-nightly-code-2026.07.24.1/arm64-software-inventory.json"
            )));
    assert!(merged["profiles"]["code"]["architectures"][0]["evidence"]
        .as_array()
        .expect("evidence rows")
        .iter()
        .all(|row| !row["url"]
            .as_str()
            .expect("evidence URL")
            .contains("/arm64-arm64-")));
}

fn write_profile_release_manifest(
    path: &Path,
    manifest_version: &str,
    profile_revision: &str,
    status: &str,
) {
    fs::write(
        path,
        format!(
            r#"{{
	  "version": "{manifest_version}",
	  "status": "current",
	  "packages": [],
	  "profiles": {{
    "co-work": {{
      "version": "{profile_revision}",
      "id": "co-work",
      "name": "Co-work",
      "revision": "{profile_revision}",
      "status": "{status}",
	      "min_capsem_version": "1.4.0",
	      "architectures": [
	        {{
	          "architecture": "arm64",
	          "software": [
	            {{
	              "name": "python",
	              "version": "3.12.11",
	              "source": "apt",
	              "architecture": "arm64",
	              "evidence": "/profiles/releases/{profile_revision}/co-work/arm64/apt-packages.txt",
	              "digest": {digest}
	            }}
	          ],
	          "config": [
	            {{
	              "kind": "mcp",
	              "path": "profiles/co-work/mcp.json",
	              "url": "/profiles/releases/{profile_revision}/co-work/arm64/mcp.json",
	              "bytes": 12,
	              "digest": {digest},
	              "status": "{status}"
	            }}
	          ],
		          "images": [
		            {{
		              "kind": "kernel",
		              "name": "vmlinuz",
		              "url": "/profiles/releases/{profile_revision}/co-work/arm64/vmlinuz",
		              "bytes": 42,
		              "digest": {digest},
		              "status": "{status}"
		            }},
		            {{
		              "kind": "initrd",
		              "name": "initrd.img",
		              "url": "/profiles/releases/{profile_revision}/co-work/arm64/initrd.img",
		              "bytes": 42,
		              "digest": {digest},
		              "status": "{status}"
		            }},
		            {{
		              "kind": "rootfs",
		              "name": "rootfs.erofs",
		              "url": "/profiles/releases/{profile_revision}/co-work/arm64/rootfs.erofs",
	              "bytes": 42,
	              "digest": {digest},
	              "status": "{status}"
	            }}
	          ],
	          "evidence": [
	            {{
	              "kind": "abom",
	              "url": "/profiles/releases/{profile_revision}/co-work/arm64/abom.cdx.json",
	              "digest": {digest}
	            }},
	            {{
	              "kind": "sbom",
	              "url": "/profiles/releases/{profile_revision}/co-work/arm64/sbom.cdx.json",
	              "digest": {digest}
	            }}
	          ]
	        }}
	      ]
    }}
  }}
}}"#,
            manifest_version = manifest_version,
            profile_revision = profile_revision,
            status = status,
            digest = serde_json::json!({
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "blake3": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            }),
        ),
    )
    .expect("profile release manifest");
}

fn file_url(path: &Path) -> String {
    let path = path.canonicalize().expect("canonical test path");
    format!("file://{}", path.display())
}

fn repo_config_profiles_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .join("config/profiles")
}

fn serve_manifest_once(body: String) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test manifest server");
    let addr = listener.local_addr().expect("manifest server addr");
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept manifest request");
        let mut buffer = [0_u8; 4096];
        let _ = stream.read(&mut buffer);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write manifest response");
    });
    format!("http://{addr}/assets/stable/manifest.json")
}

fn minimal_manifest_json(hash: Option<&str>, include_refresh_policy: bool) -> String {
    let hash =
        hash.unwrap_or("1111111111111111111111111111111111111111111111111111111111111111");
    format!(
        r#"{{
  "format": 2,
  {refresh}
  "assets": {{
    "current": "2026.0607.1",
    "releases": {{
      "2026.0607.1": {{
        "arches": {{
          "arm64": {{
            "rootfs.erofs": {{
              "hash": "{hash}",
              "size": 17
            }}
          }}
        }}
      }}
    }}
  }},
  "binaries": {{
    "current": "1.0.0",
    "releases": {{
      "1.0.0": {{
        "min_assets": "2026.0607.1"
      }}
    }}
  }}
}}"#,
        refresh = if include_refresh_policy {
            r#""refresh_policy": "24h","#
        } else {
            ""
        },
        hash = hash,
    )
}

fn write_test_assets_manifest(root: &Path, arch: &str) -> PathBuf {
    let assets_dir = root.join("assets").join(arch);
    fs::create_dir_all(&assets_dir).expect("assets dir");
    let kernel = format!("kernel-{arch}");
    let initrd = format!("initrd-{arch}");
    let rootfs = format!("rootfs-{arch}");
    let obom = test_obom_json();
    let software_inventory = test_software_inventory_json(arch);
    let pkg_sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let sbom_sha256 = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let pkg_blake3 = "1111111111111111111111111111111111111111111111111111111111111111";
    let sbom_blake3 = "2222222222222222222222222222222222222222222222222222222222222222";
    fs::write(assets_dir.join("vmlinuz"), kernel.as_bytes()).expect("kernel");
    fs::write(assets_dir.join("initrd.img"), initrd.as_bytes()).expect("initrd");
    fs::write(assets_dir.join("rootfs.erofs"), rootfs.as_bytes()).expect("rootfs");
    fs::write(assets_dir.join("obom.cdx.json"), obom.as_bytes()).expect("obom");
    fs::write(
        assets_dir.join("software-inventory.json"),
        software_inventory.as_bytes(),
    )
    .expect("software inventory");
    let manifest_path = root.join("assets/manifest.json");
    fs::write(
        &manifest_path,
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
          "{arch}": {{
            "vmlinuz": {{"hash": "{kernel_hash}", "size": {kernel_size}}},
            "initrd.img": {{"hash": "{initrd_hash}", "size": {initrd_size}}},
            "rootfs.erofs": {{"hash": "{rootfs_hash}", "size": {rootfs_size}}},
            "obom.cdx.json": {{"hash": "{obom_hash}", "size": {obom_size}}},
            "software-inventory.json": {{"hash": "{software_inventory_hash}", "size": {software_inventory_size}}}
          }}
        }}
      }}
    }}
  }},
  "binaries": {{
    "current": "1.0.0",
    "releases": {{
      "1.0.0": {{
        "date": "2030-01-01",
        "deprecated": false,
        "min_assets": "2030.0101.1",
        "files": [
          {{"name": "capsem-1.0.0.pkg", "size": 123, "sha256": "{pkg_sha256}", "blake3": "{pkg_blake3}", "binaries": [
            {{
              "name": "capsem-app",
              "installed_path": "/Applications/Capsem.app/Contents/MacOS/capsem-app",
              "size": 17,
              "sha256": "{binary_sha256}",
              "blake3": "{binary_blake3}",
              "sbom_component_ref": "SPDXRef-File-capsem-app"
            }}
          ]}},
          {{"name": "capsem-sbom.spdx.json", "size": 456, "sha256": "{sbom_sha256}", "blake3": "{sbom_blake3}"}}
        ]
      }}
    }}
  }}
}}"#,
            arch = arch,
            kernel_hash = blake3::hash(kernel.as_bytes()).to_hex(),
            kernel_size = kernel.len(),
            initrd_hash = blake3::hash(initrd.as_bytes()).to_hex(),
            initrd_size = initrd.len(),
            rootfs_hash = blake3::hash(rootfs.as_bytes()).to_hex(),
            rootfs_size = rootfs.len(),
            obom_hash = blake3::hash(obom.as_bytes()).to_hex(),
            obom_size = obom.len(),
            software_inventory_hash = blake3::hash(software_inventory.as_bytes()).to_hex(),
            software_inventory_size = software_inventory.len(),
            pkg_sha256 = pkg_sha256,
            sbom_sha256 = sbom_sha256,
            pkg_blake3 = pkg_blake3,
            sbom_blake3 = sbom_blake3,
            binary_sha256 =
                "3333333333333333333333333333333333333333333333333333333333333333",
            binary_blake3 =
                "4444444444444444444444444444444444444444444444444444444444444444",
        ),
    )
    .expect("manifest");
    manifest_path
}

fn write_test_release_graph_manifest(root: &Path) -> PathBuf {
    let manifest_path = root.join("graph-manifest.json");
    fs::write(
        &manifest_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "version": "1.0.2",
                "channel": "stable",
                "status": "current",
                "packages": [
                    {
                        "id": "old-capsem-pkg",
                        "kind": "macos_pkg",
                        "name": "Capsem-1.0.0.pkg",
                        "version": "1.0.0",
                        "platform": "macos",
                        "architecture": "arm64",
                        "url": "https://github.com/google/capsem/releases/download/v1.0.0/Capsem-1.0.0.pkg",
                        "bytes": 123,
                        "digest": {
                            "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                            "blake3": "1111111111111111111111111111111111111111111111111111111111111111",
                        },
                        "binaries": [
                            {
                                "name": "capsem-app",
                                "description": "",
                                "version": "1.0.0",
                                "installed_path": "/Applications/Capsem.app/Contents/MacOS/capsem-app",
                                "platform": "macos",
                                "architecture": "arm64",
                                "bytes": 17,
                                "digest": {
                                    "sha256": "2222222222222222222222222222222222222222222222222222222222222222",
                                    "blake3": "3333333333333333333333333333333333333333333333333333333333333333",
                                },
                                "status": "current",
                                "sbom_component_ref": "SPDXRef-File-capsem-app",
                            }
                        ],
                        "evidence": [],
                        "status": "current",
                    }
                ],
                "profiles": {
                    "co-work": {
                        "version": "1.0.0",
                        "id": "co-work",
                        "name": "Co-work",
                        "revision": "2030.0101.1",
                        "status": "current",
                        "min_capsem_version": "1.0.0",
                        "architectures": [
                            {
                                "architecture": "arm64",
                                "software": [],
                                "config": [
                                    {
                                        "kind": "profile",
                                        "path": "profiles/co-work/profile.toml",
                                        "url": "/profiles/releases/2030.0101.1/co-work/arm64/profile.toml",
                                        "bytes": 42,
                                        "digest": {
                                            "sha256": "4444444444444444444444444444444444444444444444444444444444444444",
                                            "blake3": "5555555555555555555555555555555555555555555555555555555555555555",
                                        },
                                        "status": "current",
                                    }
                                ],
                                "images": [
                                    {
                                        "kind": "rootfs",
                                        "name": "rootfs.erofs",
                                        "url": "https://github.com/google/capsem/releases/download/assets-v2030.0101.1/arm64-rootfs.erofs",
                                        "bytes": 777,
                                        "digest": {
                                            "sha256": "6666666666666666666666666666666666666666666666666666666666666666",
                                            "blake3": "7777777777777777777777777777777777777777777777777777777777777777",
                                        },
                                        "status": "current",
                                    }
                                ],
                                "evidence": [],
                            }
                        ],
                    }
                },
            }))
            .expect("graph manifest")
        ),
    )
    .expect("manifest");
    manifest_path
}

fn test_software_inventory_json(arch: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "schema": "capsem.profile_software_inventory.v1",
            "architecture": arch,
            "packages": [
                {
                    "name": "python3",
                    "version": "3.12.1-1",
                    "source": "apt",
                    "architecture": arch
                },
                {
                    "name": "@openai/codex",
                    "version": "1.2.3",
                    "source": "npm",
                    "architecture": "all"
                }
            ]
        })
    )
}

fn test_obom_json() -> String {
    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "metadata": {
            "tools": {
                "components": [
                    {"name": "cdxgen", "version": "11.0.0", "type": "application"}
                ]
            },
            "component": {
                "name": "capsem-code-rootfs",
                "type": "operating-system"
            }
        },
        "components": [
            {"name": "bash", "version": "5.2", "type": "library"}
        ]
    })
    .to_string()
}

// -- Revision validation is where corp-authored profiles meet the rule -------

#[test]
fn a_dated_profile_revision_is_rejected_at_release_time() {
    // The scheme being retired. It is URL-path safe, so path validation alone
    // waved it through -- which is how a June date shipped on a July build.
    // Each profile's own revision is what must be semver; the collapsed
    // release identifier may still be a `profiles-<hash>` when one release
    // spans profiles sitting at different versions, which independent
    // versioning makes normal rather than exceptional.
    let profiles = vec![profile_config_file("code", "2026.06.08.9")];

    // Formatted as the operator sees it: anyhow's alternate form prints the
    // whole chain, so the message names both the profile and the value.
    let error = format!(
        "{:#}",
        profile_release_revision(&profiles, ProfileRevisionPolicyArg::Strict).unwrap_err()
    );

    assert!(
        error.contains("2026.06.08.9") && error.contains("code"),
        "rejection must name both the profile and the offending revision: {error}"
    );
}

#[test]
fn a_semver_profile_revision_is_accepted_at_release_time() {
    let profiles = vec![profile_config_file("code", "0.6.0")];

    assert_eq!(
        profile_release_revision(&profiles, ProfileRevisionPolicyArg::Strict).unwrap(),
        "0.6.0"
    );
}

#[test]
fn selected_input_policy_imports_a_legacy_published_revision() {
    let profiles = vec![profile_config_file("co-work", "2026.06.08.7")];

    assert_eq!(
        profile_release_revision(&profiles, ProfileRevisionPolicyArg::SelectedInput).unwrap(),
        "2026.06.08.7"
    );
}

#[test]
fn profiles_at_different_semver_revisions_collapse_to_a_hash_identifier() {
    // Independent versioning means a multi-profile release has no single
    // revision to name. That identifier is not itself semver, and must not be
    // held to it.
    let profiles = vec![
        profile_config_file("code", "0.6.0"),
        profile_config_file("co-work", "0.3.2"),
    ];

    let revision =
        profile_release_revision(&profiles, ProfileRevisionPolicyArg::Strict).unwrap();

    assert!(
        revision.starts_with("profiles-"),
        "differing revisions must collapse to a content identifier: {revision}"
    );
    assert!(validate_profile_revision_path(&revision).is_ok());
}

#[test]
fn profile_revision_validation_still_rejects_unsafe_paths() {
    // Semver enforcement must not displace the path check it joins.
    assert!(validate_profile_revision_path("../etc/passwd").is_err());
    assert!(validate_profile_revision_path("0.6.0/../..").is_err());

    let profiles = vec![profile_config_file("code", "../etc/passwd")];
    assert!(
        profile_release_revision(&profiles, ProfileRevisionPolicyArg::SelectedInput).is_err()
    );
}

/// A minimal profile carrying just an id and a revision.
///
/// Built through serde so the fixture tracks the schema: a field gaining a
/// default here should not need a test edit, and a field losing one should
/// fail loudly rather than silently defaulting.
fn profile_config_file(id: &str, revision: &str) -> ProfileConfigFile {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "name": id,
        "description": id,
        "revision": revision,
        "refresh_policy": "manual",
        "assets": { "format": "erofs", "refresh_policy": "manual", "arch": {} }
    }))
    .expect("minimal profile fixture must match ProfileConfigFile")
}

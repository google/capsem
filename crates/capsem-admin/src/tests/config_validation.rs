use super::*;

#[test]
fn graph_channel_page_names_the_runtime_revision() {
    let manifest = serde_json::json!({
        "version": "1.0.0",
        "runtime": {"revision": "0.7.0-0123456789ab"},
    });
    let health = serde_json::json!({
        "generated_at": "2026-07-28T00:00:00Z",
        "current": {"binary": "1.6.0"},
        "evidence": {"host_binary_files": []},
    });
    let complete_page = "2026-07-28T00:00:00Z 1.0.0 /assets/nightly/manifest.json runtime 0.7.0-0123456789ab";

    validate_assets_channel_graph_page_state(complete_page, "nightly", &manifest, &health)
        .expect("the manifest-owned runtime revision is rendered");

    let missing_revision = "2026-07-28T00:00:00Z 1.0.0 /assets/nightly/manifest.json runtime";
    let error = validate_assets_channel_graph_page_state(missing_revision, "nightly", &manifest, &health)
        .expect_err("the page must name the runtime it publishes");
    assert!(
        error
            .to_string()
            .contains("missing runtime revision 0.7.0-0123456789ab"),
        "{error:#}"
    );

    let binary_only = serde_json::json!({"version": "1.0.0"});
    validate_assets_channel_graph_page_state(missing_revision, "nightly", &binary_only, &health)
        .expect("a channel without a runtime has no runtime revision to render");
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
fn settings_validation_rejects_sections_it_does_not_own() {
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

    let error = validate_settings(&path).expect_err("unowned section rejected");

    assert!(format!("{error:#}").contains("unknown field `profiles`"), "{error:#}");
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
fn compiles_checked_in_corp_rule_files() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let corp_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .join("config/corp");

    let enforcement = compile_rule_file(
        "enforcement",
        &corp_root.join("enforcement.toml"),
        RuleFileSourceArg::Corp,
    )
    .expect("compile enforcement");
    assert_eq!(enforcement.kind, "enforcement");
    assert_eq!(enforcement.compiled_rules, 1);
    assert_eq!(enforcement.rules[0].action, "block");
    assert_eq!(enforcement.rules[0].detection_level, Some("high"));

    let detection = compile_rule_file("detection", &corp_root.join("detection.yaml"), RuleFileSourceArg::Corp)
        .expect("compile detection");
    assert_eq!(detection.kind, "detection");
    assert_eq!(detection.compiled_rules, 1);
    assert_eq!(detection.rules[0].detection_level, Some("informational"));
}

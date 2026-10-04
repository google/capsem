use super::*;

fn release_args(manifest_path: &Path, manifest_version: &str, runtime_revision: &str) -> ReleaseArgs {
    ReleaseArgs {
        source_commit: source_commit(),
        manifest_path: Some(manifest_path.to_path_buf()),
        candidate_manifest: None,
        publication_base: None,
        channel: "nightly".to_string(),
        manifest_version: Some(manifest_version.to_string()),
        runtime_revision: Some(runtime_revision.to_string()),
        status: ReleaseStatusArg::Current,
        bootstrap_from_manifest: None,
        bootstrap_retired_manifest: None,
        bootstrap_retired_sha256: None,
        bootstrap_output: None,
        dry_run: false,
        json: true,
    }
}

fn write_runtime_release_manifest(path: &Path, channel: &str, manifest_version: &str, revision: &str, status: &str) {
    let mut manifest = test_runtime_graph(channel, revision);
    manifest["version"] = serde_json::json!(manifest_version);
    manifest["runtime"]["status"] = serde_json::json!(status);
    for architecture in manifest["runtime"]["architectures"]
        .as_array_mut()
        .expect("architectures")
    {
        for image in architecture["images"].as_array_mut().expect("images") {
            image["status"] = serde_json::json!(status);
        }
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(&manifest).expect("runtime release manifest"),
    )
    .expect("write runtime release manifest");
}

#[test]
fn runtime_release_status_is_channel_scoped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stable_manifest = temp.path().join("stable-manifest.json");
    let nightly_manifest = temp.path().join("nightly-manifest.json");
    write_runtime_release_manifest(&stable_manifest, "stable", "1.4.0", "1.0.0", "deprecated");
    write_runtime_release_manifest(
        &nightly_manifest,
        "nightly",
        "1.5.0-nightly.20300101",
        "0.7.0-0123456789ab",
        "supported",
    );

    let args = release_args(&nightly_manifest, "1.5.0-nightly.20300101", "0.7.0-0123456789ab");
    let report = apply_runtime_release_status(&args).expect("publish runtime release");

    assert_eq!(report.schema, "capsem.admin.runtime_release.v1");
    assert_eq!(report.action, "release");
    assert_eq!(report.status, release_graph::Status::Current);
    assert_eq!(report.runtime_revision, "0.7.0-0123456789ab");
    assert_eq!(report.publication_identity, "runtime-nightly-0.7.0-0123456789ab");
    assert_eq!(report.changed_channels, vec!["nightly"]);
    assert_eq!(report.changed_manifests, vec!["1.5.0-nightly.20300101"]);
    assert_eq!(report.changed_image_artifacts, 3);
    let report_json = serde_json::to_value(&report).expect("report json");
    for retired in ["profile", "profile_version", "changed_profiles", "changed_config_refs"] {
        assert!(report_json.get(retired).is_none(), "{retired} left the release report");
    }

    let nightly: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&nightly_manifest).expect("nightly manifest")).expect("nightly json");
    assert_eq!(nightly["runtime"]["status"].as_str(), Some("current"));
    assert_eq!(
        nightly["runtime"]["architectures"][0]["images"][0]["status"].as_str(),
        Some("current")
    );

    let stable: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&stable_manifest).expect("stable manifest")).expect("stable json");
    assert_eq!(
        stable["runtime"]["status"].as_str(),
        Some("deprecated"),
        "publishing the nightly runtime must not mutate stable"
    );
}

#[test]
fn runtime_release_status_refuses_a_revision_the_manifest_does_not_carry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = temp.path().join("manifest.json");
    write_runtime_release_manifest(&manifest, "nightly", "1.0.0", "0.7.0-0123456789ab", "current");

    let error = apply_runtime_release_status(&release_args(&manifest, "1.0.0", "0.7.0-fedcba987654"))
        .expect_err("a different runtime revision is not this manifest's runtime");

    assert!(
        format!("{error:#}").contains("expected 0.7.0-fedcba987654"),
        "{error:#}"
    );
}

#[test]
fn runtime_release_commands_require_enum_status_values() {
    let error = Cli::try_parse_from([
        "capsem-admin",
        "release",
        "--manifest-path",
        "manifest.json",
        "--channel",
        "nightly",
        "--manifest-version",
        "1.5.0-nightly.20300101",
        "--source-commit",
        "0123456789abcdef0123456789abcdef01234567",
        "--runtime-revision",
        "0.7.0-0123456789ab",
        "--status",
        "removed",
    ])
    .expect_err("removed is not a release status");

    assert!(error.to_string().contains("invalid value"), "{error}");
}

#[test]
fn retired_graph_authoring_verifies_the_exact_input_bytes() {
    let bytes = b"known retired graph";
    let expected = format!("{:x}", Sha256::digest(bytes))
        .parse::<channel_bootstrap::RetiredGraphSha256>()
        .expect("canonical digest");

    verify_retired_graph_sha256(bytes, &expected).expect("exact payload accepted");
    let error = verify_retired_graph_sha256(b"substituted graph", &expected).expect_err("substitution rejected");
    assert!(format!("{error:#}").contains("sha256 mismatch"), "{error:#}");
}

#[test]
fn runtime_release_paths_are_channel_qualified() {
    let stable = runtime_release_url("stable", "0.7.0-0123456789ab", "arm64", "rootfs.erofs").expect("stable URL");
    let nightly = runtime_release_url("nightly", "0.7.0-0123456789ab", "arm64", "rootfs.erofs").expect("nightly URL");

    assert_eq!(stable, "/runtime/releases/stable/0.7.0-0123456789ab/arm64/rootfs.erofs");
    assert_eq!(
        nightly,
        "/runtime/releases/nightly/0.7.0-0123456789ab/arm64/rootfs.erofs"
    );
    for (arch, file) in [("..", "rootfs.erofs"), ("arm64", "../rootfs.erofs"), ("arm64", "")] {
        assert!(
            runtime_release_url("stable", "0.7.0-0123456789ab", arch, file).is_err(),
            "{arch}/{file} must not form a runtime path"
        );
    }
}

#[test]
fn runtime_revision_is_the_workspace_version_and_the_commit_prefix() {
    let commit = source_commit();

    assert_eq!(
        commit.runtime_revision(),
        format!("{}-0123456789ab", env!("CARGO_PKG_VERSION"))
    );
    let selection = validate_release_selection("nightly", &commit).expect("selection validates");
    assert_eq!(selection.runtime_revision, commit.runtime_revision());
    assert_eq!(
        selection.publication_identity,
        format!("runtime-nightly-{}", commit.runtime_revision())
    );
    assert!(validate_release_selection("night/ly", &commit).is_err());
}

#[test]
fn validate_and_release_take_only_channel_and_source_commit() {
    let cli = Cli::parse_from([
        "capsem-admin",
        "release",
        "--channel",
        "nightly",
        "--source-commit",
        "0123456789abcdef0123456789abcdef01234567",
        "--dry-run",
    ]);
    match cli.command {
        Commands::Release(args) => {
            assert_eq!(args.channel, "nightly");
            assert_eq!(args.source_commit.as_str(), "0123456789abcdef0123456789abcdef01234567");
            assert!(args.manifest_path.is_none());
            assert!(args.dry_run);
        }
        _ => panic!("expected release command"),
    }
    let cli = Cli::parse_from([
        "capsem-admin",
        "validate",
        "--channel",
        "stable",
        "--source-commit",
        "0123456789abcdef0123456789abcdef01234567",
        "--json",
    ]);
    assert!(matches!(cli.command, Commands::Validate(_)));
    for retired in [["--profile", "code"], ["--config-root", "config"]] {
        for command in ["validate", "release"] {
            Cli::try_parse_from([
                "capsem-admin",
                command,
                "--channel",
                "stable",
                "--source-commit",
                "0123456789abcdef0123456789abcdef01234567",
                retired[0],
                retired[1],
            ])
            .expect_err("profiles are not a release unit");
        }
    }
    assert_eq!(
        runtime_publication_identity("nightly", "0.7.0-0123456789ab").expect("publication identity"),
        "runtime-nightly-0.7.0-0123456789ab"
    );
    assert!(
        runtime_publication_identity("nightly", "revision/escape").is_err(),
        "publication identities must be safe immutable GitHub release tags"
    );
}

#[derive(Default)]
struct RecordingReleaseWorkflowRunner {
    listings: std::collections::VecDeque<String>,
    calls: Vec<Vec<String>>,
    waits: usize,
    fail_watch: bool,
}

impl ReleaseWorkflowRunner for RecordingReleaseWorkflowRunner {
    fn run(&mut self, args: &[String]) -> Result<()> {
        self.calls.push(args.to_vec());
        if self.fail_watch && args.first().map(String::as_str) == Some("run") {
            return Err(anyhow!("watched runtime workflow failed"));
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

fn workflow_run(id: u64, title: &str, commit: &SourceCommit, status: &str, conclusion: &str) -> serde_json::Value {
    serde_json::json!({
        "databaseId": id,
        "displayTitle": title,
        "headSha": commit,
        "headBranch": format!("capsem-source-{commit}"),
        "status": status,
        "conclusion": conclusion,
    })
}

#[test]
fn runtime_release_dispatch_waits_for_its_exact_workflow_run() {
    let commit = source_commit();
    let source_ref = format!("capsem-source-{commit}");
    let title = "Release runtime nightly dispatch-7";
    let mut runner = RecordingReleaseWorkflowRunner {
        listings: [
            "[]".to_string(),
            serde_json::json!([workflow_run(42, title, &commit, "in_progress", "")]).to_string(),
            workflow_run(42, title, &commit, "completed", "success").to_string(),
        ]
        .into(),
        ..Default::default()
    };

    let run_id = dispatch_release_workflow(&mut runner, "release-assets.yaml", "nightly", &commit, "dispatch-7")
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
            &source_ref,
            "-f",
            "channel=nightly",
            "-f",
            "dry_run=false",
            "-f",
            "dispatch_id=dispatch-7",
            "-f",
            &format!("source_commit={commit}"),
        ]
    );
    assert_eq!(&runner.calls[3], &["run", "watch", "42", "--exit-status"]);
}

#[test]
fn runtime_release_dispatch_ignores_an_unrelated_pending_run() {
    let commit = source_commit();
    let ours = "Release runtime nightly ours";
    let mut runner = RecordingReleaseWorkflowRunner {
        listings: [
            serde_json::json!([workflow_run(
                9,
                "Release runtime nightly somebody-else",
                &commit,
                "in_progress",
                ""
            )])
            .to_string(),
            serde_json::json!([workflow_run(10, ours, &commit, "in_progress", "")]).to_string(),
            workflow_run(10, ours, &commit, "completed", "success").to_string(),
        ]
        .into(),
        ..Default::default()
    };

    let run_id = dispatch_release_workflow(&mut runner, "release-assets.yaml", "nightly", &commit, "ours")
        .expect("the correlated run is selected");

    assert_eq!(run_id, 10);
    assert_eq!(runner.calls.last().expect("watch call")[2], "10");
}

#[test]
fn runtime_release_dispatch_propagates_the_exact_run_failure() {
    let commit = source_commit();
    let mut runner = RecordingReleaseWorkflowRunner {
        listings: [serde_json::json!([workflow_run(
            11,
            "Release runtime nightly ours",
            &commit,
            "in_progress",
            ""
        )])
        .to_string()]
        .into(),
        fail_watch: true,
        ..Default::default()
    };

    let error = dispatch_release_workflow(&mut runner, "release-assets.yaml", "nightly", &commit, "ours")
        .expect_err("the public command must fail with its exact workflow run");

    assert!(format!("{error:#}").contains("watched runtime workflow failed"));
    assert_eq!(runner.calls.last().expect("watch call")[2], "11");
}

#[test]
fn runtime_release_merges_the_candidate_runtime_and_reports_compatibility() {
    let temp = tempfile::tempdir().expect("tempdir");
    let revision = source_commit().runtime_revision();
    let mut base = test_runtime_graph("nightly", "0.6.9-aaaaaaaaaaaa");
    base["version"] = serde_json::json!("1.0.2");
    let mut candidate = test_runtime_graph("nightly", &revision);
    candidate["version"] = serde_json::json!("1.0.2");
    candidate["runtime"]["min_capsem_version"] = serde_json::json!("9.0.0");
    let base_path = temp.path().join("base.json");
    let candidate_path = temp.path().join("candidate.json");
    fs::write(&base_path, serde_json::to_vec_pretty(&base).expect("base json")).expect("write base");
    let publication_base = format!("https://github.com/google/capsem/releases/download/runtime-nightly-{revision}");
    let args = ReleaseArgs {
        candidate_manifest: Some(candidate_path.clone()),
        publication_base: Some(publication_base.clone()),
        ..release_args(&base_path, "1.0.2", &revision)
    };

    candidate["runtime"]["source_commit"] = serde_json::Value::String("f".repeat(40));
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&candidate).expect("candidate json"),
    )
    .expect("write mismatched candidate");
    let error = apply_runtime_release_status(&args).expect_err("wrong source commit rejected");
    assert!(format!("{error:#}").contains("was built from"), "{error:#}");
    candidate["runtime"]
        .as_object_mut()
        .expect("runtime object")
        .remove("source_commit");
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&candidate).expect("candidate json"),
    )
    .expect("write candidate");

    let report = apply_runtime_release_status(&args).expect("merge candidate runtime");
    let merged: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&base_path).expect("merged")).expect("merged json");

    assert!(!report.compatible_with_current_binary);
    assert_eq!(report.changed_image_artifacts, 0);
    assert_eq!(merged["packages"], base["packages"]);
    assert!(merged.get("profiles").is_none());
    assert_eq!(merged["runtime"]["source_commit"], source_commit().as_str());
    assert!(merged.get("source_commit").is_none());
    assert_eq!(merged["runtime"]["revision"].as_str(), Some(revision.as_str()));
    assert_eq!(
        merged["runtime"]["architectures"][0]["images"][0]["url"].as_str(),
        Some(format!("{publication_base}/arm64-vmlinuz").as_str())
    );
    assert!(merged["runtime"]["architectures"][0]["software"]
        .as_array()
        .expect("software rows")
        .iter()
        .all(|row| row["evidence"].as_str()
            == Some(format!("{publication_base}/arm64-software-inventory.json").as_str())));
    assert!(merged["runtime"]["architectures"][0]["evidence"]
        .as_array()
        .expect("evidence rows")
        .iter()
        .all(|row| !row["url"].as_str().expect("evidence URL").contains("/arm64-arm64-")));
    validate_assets_channel_graph_manifest(&merged, "nightly").expect("merged graph validates");
}

#[test]
fn runtime_release_refuses_a_candidate_without_a_runtime() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base = test_runtime_graph("nightly", "0.6.9-aaaaaaaaaaaa");
    let mut candidate = base.clone();
    candidate.as_object_mut().expect("graph").remove("runtime");
    let base_path = temp.path().join("base.json");
    let candidate_path = temp.path().join("candidate.json");
    fs::write(&base_path, serde_json::to_vec_pretty(&base).expect("base json")).expect("write base");
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&candidate).expect("candidate json"),
    )
    .expect("write candidate");
    let args = ReleaseArgs {
        candidate_manifest: Some(candidate_path),
        ..release_args(&base_path, "1.0.2", "0.7.0-0123456789ab")
    };

    let error = apply_runtime_release_status(&args).expect_err("nothing to publish");

    assert!(format!("{error:#}").contains("does not publish a runtime"), "{error:#}");
}

#[test]
fn corporate_manifest_carries_the_corporations_runtime_under_its_base() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base = "https://corp.example/runtime/";
    let official = test_runtime_graph("stable", "1.0.0");
    let mut runtime_source = test_runtime_graph("acme-stable", "2030.0101.1");
    runtime_source["packages"] = serde_json::json!([]);
    rewrite_runtime_publication_urls(&mut runtime_source["runtime"], &format!("{base}2030.0101.1"))
        .expect("corporate publication URLs");
    let official_path = temp.path().join("official.json");
    let runtime_path = temp.path().join("runtime.json");
    fs::write(&official_path, serde_json::to_vec(&official).expect("official json")).expect("write official");
    fs::write(
        &runtime_path,
        serde_json::to_vec(&runtime_source).expect("runtime json"),
    )
    .expect("write runtime");
    let args = |runtime_base: &str| ManifestCorporateArgs {
        corporation: "acme".to_string(),
        channel: "acme-stable".to_string(),
        official_manifest: official_path.clone(),
        runtime_manifest: runtime_path.clone(),
        runtime_base: runtime_base.to_string(),
        binary: "latest".to_string(),
        source_commit: source_commit(),
        output_root: temp.path().join("out"),
        manifest_version: "1.0.0".to_string(),
        json: true,
    };

    let report = author_corporate_manifest(&args(base)).expect("corporate manifest authored");

    assert_eq!(report.runtime_revision, "2030.0101.1");
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report.output_manifest).expect("output")).expect("output json");
    assert_eq!(written["packages"], official["packages"]);
    assert_eq!(written["runtime"]["source_commit"], source_commit().as_str());
    assert!(written.get("profiles").is_none());

    let error = author_corporate_manifest(&args("https://other.example/runtime/"))
        .expect_err("runtime references must live under the owned base");
    assert!(
        format!("{error:#}").contains("outside the owned runtime base"),
        "{error:#}"
    );

    let mut no_runtime = runtime_source.clone();
    no_runtime.as_object_mut().expect("graph").remove("runtime");
    fs::write(&runtime_path, serde_json::to_vec(&no_runtime).expect("runtime json")).expect("write runtime");
    let error = author_corporate_manifest(&args(base)).expect_err("a corporate graph needs its runtime");
    assert!(format!("{error:#}").contains("must contain a runtime"), "{error:#}");
}

use super::*;

#[test]
fn profile_release_commands_publish_report_is_lane_scoped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stable_manifest = temp.path().join("stable-manifest.json");
    let nightly_manifest = temp.path().join("nightly-manifest.json");
    write_profile_release_manifest(&stable_manifest, "1.4.0", "1.0.0", "deprecated");
    write_profile_release_manifest(&nightly_manifest, "1.5.0-nightly.20300101", "2026.7.2-2", "supported");

    let args = ReleaseArgs {
        source_commit: source_commit(),
        manifest_path: Some(nightly_manifest.clone()),
        candidate_manifest: None,
        publication_base: None,
        channel: "nightly".to_string(),
        manifest_version: Some("1.5.0-nightly.20300101".to_string()),
        profile: "co-work".to_string(),
        profile_version: Some("2026.7.2-2".to_string()),
        config_root: repo_config_profiles_dir().parent().expect("config root").to_path_buf(),
        status: ProfileReleaseStatusArg::Current,
        bootstrap_from_manifest: None,
        bootstrap_retired_manifest: None,
        bootstrap_retired_sha256: None,
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
        serde_json::from_str(&fs::read_to_string(&nightly_manifest).expect("nightly manifest")).expect("nightly json");
    assert_eq!(nightly["profiles"]["co-work"]["status"].as_str(), Some("current"));
    assert_eq!(
        nightly["profiles"]["co-work"]["architectures"][0]["config"][0]["status"].as_str(),
        Some("current")
    );
    assert_eq!(
        nightly["profiles"]["co-work"]["architectures"][0]["images"][0]["status"].as_str(),
        Some("current")
    );

    let stable: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&stable_manifest).expect("stable manifest")).expect("stable json");
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
        "--source-commit",
        "0123456789abcdef0123456789abcdef01234567",
        "--profile-version",
        "2026.7.2-2",
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
fn profile_release_paths_are_channel_qualified() {
    let stable =
        profile_release_url("stable", "code", "2026.06.08.7", "arm64", "rootfs.erofs").expect("stable profile URL");
    let nightly =
        profile_release_url("nightly", "code", "2026.06.08.7", "arm64", "rootfs.erofs").expect("nightly profile URL");

    assert_eq!(stable, "/profiles/releases/stable/code/2026.06.08.7/arm64/rootfs.erofs");
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
        "--source-commit",
        "0123456789abcdef0123456789abcdef01234567",
        "--dry-run",
    ]);
    match cli.command {
        Commands::Release(args) => {
            assert_eq!(args.channel, "nightly");
            assert_eq!(args.profile, "code");
            assert_eq!(args.source_commit.as_str(), "0123456789abcdef0123456789abcdef01234567");
            assert!(args.manifest_path.is_none());
            assert!(args.dry_run);
        }
        _ => panic!("expected release command"),
    }
    assert_eq!(
        profile_publication_identity("nightly", "code", "2026.06.08.7").expect("publication identity"),
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
    let commit = source_commit();
    let source_ref = format!("capsem-source-{commit}");
    let mut runner = RecordingProfileWorkflowRunner {
        listings: [
            "[]".to_string(),
            serde_json::json!([{
                "databaseId": 42,
                "displayTitle": "Release profile nightly/code dispatch-7",
                "headSha": commit,
                "headBranch": source_ref,
                "status": "in_progress",
                "conclusion": "",
            }])
            .to_string(),
            serde_json::json!({
                "databaseId": 42,
                "displayTitle": "Release profile nightly/code dispatch-7",
                "headSha": commit,
                "headBranch": source_ref,
                "status": "completed",
                "conclusion": "success",
            })
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
        &commit,
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
            &source_ref,
            "-f",
            "channel=nightly",
            "-f",
            "profile=code",
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
fn profile_release_dispatch_ignores_an_unrelated_pending_run() {
    let commit = source_commit();
    let source_ref = format!("capsem-source-{commit}");
    let mut runner = RecordingProfileWorkflowRunner {
        listings: [
            serde_json::json!([{
                "databaseId": 9,
                "displayTitle": "Release profile nightly/co-work somebody-else",
                "headSha": commit,
                "headBranch": source_ref,
                "status": "in_progress",
                "conclusion": "",
            }])
            .to_string(),
            serde_json::json!([{
                "databaseId": 10,
                "displayTitle": "Release profile nightly/code ours",
                "headSha": commit,
                "headBranch": source_ref,
                "status": "in_progress",
                "conclusion": "",
            }])
            .to_string(),
            serde_json::json!({
                "databaseId": 10,
                "displayTitle": "Release profile nightly/code ours",
                "headSha": commit,
                "headBranch": source_ref,
                "status": "completed",
                "conclusion": "success",
            })
            .to_string(),
        ]
        .into(),
        ..Default::default()
    };

    let run_id = dispatch_profile_workflow(&mut runner, "release-assets.yaml", "nightly", "code", &commit, "ours")
        .expect("the correlated run is selected");

    assert_eq!(run_id, 10);
    assert_eq!(runner.calls.last().expect("watch call")[2], "10");
}

#[test]
fn profile_release_dispatch_propagates_the_exact_run_failure() {
    let commit = source_commit();
    let mut runner = RecordingProfileWorkflowRunner {
        listings: [serde_json::json!([{
            "databaseId": 11,
            "displayTitle": "Release profile nightly/code ours",
            "headSha": commit,
            "headBranch": format!("capsem-source-{commit}"),
            "status": "in_progress",
            "conclusion": "",
        }])
        .to_string()]
        .into(),
        fail_watch: true,
        ..Default::default()
    };

    let error = dispatch_profile_workflow(&mut runner, "release-assets.yaml", "nightly", "code", &commit, "ours")
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
        serde_json::from_str(&fs::read_to_string(fixture).expect("fixture")).expect("fixture json");
    let base = graph["manifests"]["nightly"]["1.0.2"].clone();
    let mut candidate = base.clone();
    candidate["profiles"]["code"]["revision"] = serde_json::Value::String("2026.07.24.1".to_string());
    candidate["profiles"]["code"]["version"] = serde_json::Value::String("2026.07.24.1".to_string());
    candidate["profiles"]["code"]["min_capsem_version"] = serde_json::Value::String("9.0.0".to_string());
    let base_path = temp.path().join("base.json");
    let candidate_path = temp.path().join("candidate.json");
    fs::write(&base_path, serde_json::to_vec_pretty(&base).expect("base json")).expect("write base");
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&candidate).expect("candidate json"),
    )
    .expect("write candidate");
    let args = ReleaseArgs {
        source_commit: source_commit(),
        channel: "nightly".to_string(),
        profile: "code".to_string(),
        config_root: PathBuf::from("config"),
        manifest_path: Some(base_path.clone()),
        candidate_manifest: Some(candidate_path),
        publication_base: Some(
            "https://github.com/google/capsem/releases/download/profile-nightly-code-2026.07.24.1".to_string(),
        ),
        manifest_version: Some("1.0.2".to_string()),
        profile_version: Some("2026.07.24.1".to_string()),
        status: ProfileReleaseStatusArg::Current,
        bootstrap_from_manifest: None,
        bootstrap_retired_manifest: None,
        bootstrap_retired_sha256: None,
        bootstrap_output: None,
        dry_run: false,
        json: true,
    };

    candidate["profiles"]["code"]["source_commit"] = serde_json::Value::String("f".repeat(40));
    fs::write(
        args.candidate_manifest.as_ref().expect("candidate path"),
        serde_json::to_vec_pretty(&candidate).expect("candidate json"),
    )
    .expect("write mismatched candidate");
    let error = apply_profile_release_status(&args).expect_err("wrong source commit rejected");
    assert!(format!("{error:#}").contains("was built from"), "{error:#}");
    candidate["profiles"]["code"]
        .as_object_mut()
        .expect("profile object")
        .remove("source_commit");
    fs::write(
        args.candidate_manifest.as_ref().expect("candidate path"),
        serde_json::to_vec_pretty(&candidate).expect("candidate json"),
    )
    .expect("write candidate");

    let report = apply_profile_release_status(&args).expect("merge selected profile");
    let merged: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(base_path).expect("merged")).expect("merged json");

    assert!(!report.compatible_with_current_binary);
    assert_eq!(report.changed_profiles, vec!["code"]);
    assert_eq!(merged["packages"], base["packages"]);
    assert_eq!(merged["profiles"]["co-work"], base["profiles"]["co-work"]);
    assert_eq!(merged["profiles"]["code"]["source_commit"], source_commit().as_str());
    assert!(merged.get("source_commit").is_none());
    assert_eq!(merged["profiles"]["code"]["revision"].as_str(), Some("2026.07.24.1"));
    assert_eq!(
        merged["profiles"]["code"]["architectures"][0]["config"][0]["url"].as_str(),
        Some("https://github.com/google/capsem/releases/download/profile-nightly-code-2026.07.24.1/arm64-profile.toml")
    );
    assert_eq!(
        merged["profiles"]["code"]["architectures"][0]["images"][0]["url"].as_str(),
        Some("https://github.com/google/capsem/releases/download/profile-nightly-code-2026.07.24.1/arm64-vmlinuz")
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
        .all(|row| !row["url"].as_str().expect("evidence URL").contains("/arm64-arm64-")));
}

fn write_profile_release_manifest(path: &Path, manifest_version: &str, profile_revision: &str, status: &str) {
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

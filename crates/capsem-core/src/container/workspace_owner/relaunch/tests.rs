use super::*;
use crate::container::{stage::EnvironmentLaunchContext, workload_resources};

fn context(number: u128) -> EnvironmentLaunchContext {
    EnvironmentLaunchContext {
        session_id: uuid::Uuid::from_u128(number),
        owner_generation: uuid::Uuid::from_u128(number + 1),
        environment_id: uuid::Uuid::from_u128(20),
        revision_id: uuid::Uuid::from_u128(number + 2),
    }
}

fn fixture() -> (tempfile::TempDir, String, String, String) {
    let directory = tempfile::tempdir().unwrap();
    let share = crate::session::prepare_image_share(directory.path()).unwrap();
    let config = br#"{"config":{"Labels":{}}}"#;
    let config_digest = format!("sha256:{:x}", capsem_crypto::Sha256::digest(config));
    let manifest = serde_json::to_vec(&serde_json::json!({"config":{"digest":config_digest},"layers":[]})).unwrap();
    let digest = format!("sha256:{:x}", capsem_crypto::Sha256::digest(&manifest));
    std::fs::write(share.join(config_digest.strip_prefix("sha256:").unwrap()), config).unwrap();
    std::fs::write(share.join(digest.strip_prefix("sha256:").unwrap()), manifest).unwrap();
    let root = b"verified carried root";
    let root_digest = format!("sha256:{:x}", capsem_crypto::Sha256::digest(root));
    let integrity = format!("blake3:{}", capsem_crypto::blake3::hash(root));
    std::fs::write(share.join(root_digest.strip_prefix("sha256:").unwrap()), root).unwrap();
    let original = context(1);
    let startup = capsem_config::environment::EnvironmentStartup {
        script: "exit 99".into(),
        timeout_seconds: 1,
    };
    let plan = stage::stage_plan(
        stage::StageImage {
            manifest: &digest,
            rootfs: Some(stage::RootfsFile {
                digest: &root_digest,
                integrity: &integrity,
                size: root.len() as u64,
            }),
            environment: Some(&original),
            startup: Some(&startup),
        },
        &["old-command".into()],
        &BTreeMap::from([("OLD_ONLY".into(), "old".into())]),
        workload_resources(2048, 2).unwrap(),
        stage::DeclaredSurface::Terminal,
    )
    .unwrap();
    for file in plan {
        crate::session::publish_launch_control(directory.path(), &file.name, &file.bytes).unwrap();
    }
    (directory, digest, config_digest, root_digest)
}

#[test]
fn a_new_revision_builds_fresh_controls_and_verifies_carried_bytes_without_publishing() {
    let (directory, digest, _, _) = fixture();
    let options_path = crate::session::image_share_path(directory.path()).join("options.json");
    let original = std::fs::read(&options_path).unwrap();
    let selected = context(5);
    let startup = capsem_config::environment::EnvironmentStartup {
        script: "printf approved".into(),
        timeout_seconds: 600,
    };
    let resources = workload_resources(4096, 4).unwrap();
    let plan = plan_environment(
        directory.path(),
        &digest,
        &selected,
        &BTreeMap::from([("NEW_ONLY".into(), "approved".into())]),
        Some(&startup),
        resources,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&options_path).unwrap(),
        original,
        "planning must pass through service import policy before publishing"
    );
    let options: serde_json::Value =
        serde_json::from_slice(&plan.iter().find(|file| file.name == "options.json").unwrap().bytes).unwrap();
    assert_eq!(options["environment"], serde_json::to_value(selected).unwrap());
    assert_eq!(options["env"], serde_json::json!({"NEW_ONLY":"approved"}));
    assert_eq!(options["args"], serde_json::json!([]));
    assert_eq!(options["startup"], serde_json::to_value(startup).unwrap());
    assert_eq!(options["resources"], serde_json::to_value(resources).unwrap());
    assert_eq!(options["manifest"], digest);
    assert_eq!(plan.len(), 1 + crate::container::LAUNCHER_FILES.len());
    assert!(
        crate::container::workspace_owner::rebind_environment(directory.path(), &selected).is_err(),
        "metadata-only retargeting must stay refused"
    );
}

#[test]
fn same_owner_is_unchanged_and_a_new_owner_can_remove_old_startup_and_variables() {
    let (directory, digest, _, _) = fixture();
    assert!(plan_environment(
        directory.path(),
        &digest,
        &context(1),
        &BTreeMap::new(),
        None,
        workload_resources(2048, 2).unwrap()
    )
    .unwrap()
    .is_empty());
    let plan = plan_environment(
        directory.path(),
        &digest,
        &context(5),
        &BTreeMap::new(),
        None,
        workload_resources(2048, 2).unwrap(),
    )
    .unwrap();
    let options: serde_json::Value = serde_json::from_slice(&plan[0].bytes).unwrap();
    assert!(options.get("startup").is_none());
    assert_eq!(options["env"], serde_json::json!({}));
    assert_eq!(options["args"], serde_json::json!([]));
}

#[test]
fn corrupt_carried_metadata_rootfs_or_selection_never_changes_old_controls() {
    for kind in ["manifest", "config", "rootfs", "symlink", "selection", "identity"] {
        let (directory, digest, config, root) = fixture();
        let share = crate::session::image_share_path(directory.path());
        let original = std::fs::read(share.join("options.json")).unwrap();
        let mut selected = context(5);
        let mut expected = digest.clone();
        match kind {
            "manifest" => std::fs::write(share.join(digest.strip_prefix("sha256:").unwrap()), b"{}").unwrap(),
            "config" => std::fs::write(share.join(config.strip_prefix("sha256:").unwrap()), b"{}").unwrap(),
            "rootfs" => std::fs::write(share.join(root.strip_prefix("sha256:").unwrap()), b"corrupt").unwrap(),
            "symlink" => {
                let path = share.join(config.strip_prefix("sha256:").unwrap());
                std::fs::remove_file(&path).unwrap();
                std::os::unix::fs::symlink("options.json", path).unwrap();
            }
            "selection" => expected = format!("sha256:{}", "a".repeat(64)),
            "identity" => selected.owner_generation = uuid::Uuid::nil(),
            _ => unreachable!(),
        }
        assert!(
            plan_environment(
                directory.path(),
                &expected,
                &selected,
                &BTreeMap::new(),
                None,
                workload_resources(2048, 2).unwrap()
            )
            .is_err(),
            "{kind}"
        );
        assert_eq!(std::fs::read(share.join("options.json")).unwrap(), original, "{kind}");
    }
}

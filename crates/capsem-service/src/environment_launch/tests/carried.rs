//! A retained next revision must prepare its own controls from carried bytes.
use super::*;
use capsem_core::container::{stage, workload_resources};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_boot_revision_recomposes_controls_without_resolving_head_or_pulling() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    for behavior in ["allow", "denied", "replaced"] {
        recompose(behavior).await;
    }
}

async fn recompose(behavior: &'static str) {
    let directory = tempfile::tempdir().unwrap();
    let (_paths, settings, _) = install_empty_settings_env(&directory);
    let state = make_test_state();
    let vm = id(90);
    let session = directory.path().join(&vm);
    std::fs::create_dir_all(&session).unwrap();
    std::fs::create_dir_all(
        session
            .join(capsem_core::GUEST_SHARE_DIR)
            .join(capsem_core::session::WORKSPACE_DIR),
    )
    .unwrap();
    let share = capsem_core::session::prepare_image_share(&session).unwrap();
    let config = serde_json::to_vec(&serde_json::json!({"config":{"Labels":{}}})).unwrap();
    let config_digest = format!("sha256:{:x}", capsem_crypto::Sha256::digest(&config));
    let manifest = serde_json::to_vec(&serde_json::json!({"config":{"digest":config_digest},"layers":[]})).unwrap();
    let digest = format!("sha256:{:x}", capsem_crypto::Sha256::digest(&manifest));
    std::fs::write(share.join(config_digest.strip_prefix("sha256:").unwrap()), config).unwrap();
    std::fs::write(share.join(digest.strip_prefix("sha256:").unwrap()), manifest).unwrap();
    for number in [2, 3, 4] {
        let mut saved = revision(number, (number > 2).then_some(number - 1), "block");
        saved.specification.image = Some(capsem_config::environment::EnvironmentImage {
            reference: "registry.example/app:1".into(),
            resolved_digest: digest.clone(),
            update_policy: Default::default(),
        });
        saved.specification.startup = Some(capsem_config::environment::EnvironmentStartup {
            script: format!("printf revision-{number}"),
            timeout_seconds: if number == 3 { 600 } else { 1 },
        });
        state.environments.commit(saved, number as i64).wait().await.unwrap();
    }
    std::fs::write(
        &settings,
        format!("[images]\ncatalog=false\nsources=['registry.example/app']\nadmit=['registry.example/app@{digest}']\n"),
    )
    .unwrap();
    insert_fake_instance_with_session_dir(&state, &vm, std::process::id(), session.clone());
    let generation = {
        let mut instances = state.instances.lock().unwrap();
        let instance = instances.get_mut(&vm).unwrap();
        let mut binding = EnvironmentBinding::starting(reference(3), "acknowledged".into());
        binding.acknowledged = true;
        instance.environment = Some(binding);
        let generation = instance.generation;
        drop(instances);
        generation
    };
    let previous = stage::EnvironmentLaunchContext {
        session_id: uuid::Uuid::parse_str(&id(80)).unwrap(),
        owner_generation: uuid::Uuid::parse_str(&id(81)).unwrap(),
        environment_id: uuid::Uuid::parse_str(&id(1)).unwrap(),
        revision_id: uuid::Uuid::parse_str(&id(2)).unwrap(),
    };
    let startup = capsem_config::environment::EnvironmentStartup {
        script: "exit 99".into(),
        timeout_seconds: 1,
    };
    for file in stage::stage_plan(
        stage::StageImage {
            manifest: &digest,
            rootfs: None,
            environment: Some(&previous),
            startup: Some(&startup),
        },
        &["old-override".into()],
        &std::collections::BTreeMap::from([("OLD_ONLY".into(), "forbidden".into())]),
        workload_resources(2048, 2).unwrap(),
        stage::DeclaredSurface::Terminal,
    )
    .unwrap()
    {
        capsem_core::session::publish_launch_control(&session, &file.name, &file.bytes).unwrap();
    }
    std::fs::write(
        session.join("container.json"),
        serde_json::to_vec(&serde_json::json!({
            "image":"registry.example/app:1","digest":digest,"manifest":digest,
            "resolved":format!("registry.example/app@{digest}")
        }))
        .unwrap(),
    )
    .unwrap();
    let socket = state.instances.lock().unwrap().get(&vm).unwrap().uds_path.clone();
    let original_options = std::fs::read(share.join("options.json")).unwrap();
    let expected = if behavior == "allow" {
        1 + capsem_core::container::LAUNCHER_FILES.len()
    } else {
        1
    };
    let changed_state = state.clone();
    let changed_vm = vm.clone();
    let changed_session = session.clone();
    let peer = spawn_fake_process(&socket, expected, move |message| {
        let crate::ServiceToProcess::LogFileBoundary { id, action, .. } = message else {
            panic!("only approved file imports are expected: {message:?}");
        };
        assert_eq!(*action, capsem_proto::ipc::FileBoundaryAction::Import);
        if behavior == "replaced" {
            insert_fake_instance_with_session_dir(
                &changed_state,
                &changed_vm,
                std::process::id(),
                changed_session.clone(),
            );
        }
        let reply = crate::ProcessToService::LogFileBoundaryResult {
            id: *id,
            success: behavior != "denied",
            data: None,
            error: (behavior == "denied").then(|| "fixture import policy refused control".into()),
        };
        Box::pin(async move { Some(reply) })
    });
    let result = prepare_image(&state, &vm, false).await;
    if behavior != "allow" {
        assert!(result.is_err(), "{behavior}: {result:?}");
        peer.await.unwrap();
        assert_eq!(
            std::fs::read(share.join("options.json")).unwrap(),
            original_options,
            "{behavior}: refused preparation cannot publish controls"
        );
        return;
    }
    assert!(
        result.is_ok(),
        "a retained new revision needs fresh controls: {result:?}"
    );
    assert!(!result.unwrap(), "the carried image must not start another pull task");
    peer.await.unwrap();
    let options: serde_json::Value =
        serde_json::from_slice(&std::fs::read(share.join("options.json")).unwrap()).unwrap();
    assert_eq!(options["environment"]["revision_id"], id(3));
    assert_eq!(options["environment"]["session_id"], vm);
    assert_eq!(options["environment"]["owner_generation"], generation.to_string());
    assert_eq!(options["env"], serde_json::json!({"REVISION":"revision-3"}));
    assert_eq!(
        options["startup"],
        serde_json::json!({"script":"printf revision-3","timeout_seconds":600})
    );
    assert_eq!(options["args"], serde_json::json!([]));
    assert_eq!(options["manifest"], digest);
    assert!(!state.run_dir.join("container-pulls").join("blobs").exists());
}

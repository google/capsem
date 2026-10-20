use super::*;

#[test]
fn credential_policy_cache_reuses_exact_inputs_and_observes_same_size_changes() {
    let make = |action| {
        let corp = format!("[corp.rules.target]\nname = 'target'\naction = '{action}'\npriority = -10\nmatch = 'http.host == \"example.test\"'\n");
        Arc::new((Default::default(), toml::from_str(&corp).unwrap()))
    };
    let policy = Arc::new(ActivePolicyFile::from_settings_and_corp(&Default::default(), &Default::default()).unwrap());
    let capability = capsem_credentials::GrantCapability::new(
        "probe".into(),
        std::collections::BTreeSet::from(["openid".into()]),
        vec![capsem_credentials::GrantTarget::new("GET", "https://example.test/probe").unwrap()],
    )
    .unwrap();
    let inputs = make("allow");
    let original = compute(
        Applied {
            policy: Arc::clone(&policy),
            cached: None,
        },
        Arc::clone(&inputs),
        std::slice::from_ref(&capability),
    )
    .unwrap_or_else(|_| panic!("valid target policy"));
    assert!(original.admits(&capability));
    let cached = Applied {
        policy,
        cached: Some(Cached {
            inputs,
            admission: Arc::clone(&original),
        }),
    };
    let repeated =
        compute(cached.clone(), make("allow"), std::slice::from_ref(&capability)).unwrap_or_else(|_| panic!());
    assert!(Arc::ptr_eq(&original, &repeated));
    let changed = compute(cached, make("block"), std::slice::from_ref(&capability)).unwrap_or_else(|_| panic!());
    assert!(!Arc::ptr_eq(&original, &changed));
    assert!(!changed.admits(&capability));
}

#[tokio::test]
async fn credential_policy_confirmation_requires_exact_successful_digest_ack() {
    let (state, _root) = crate::tests::make_test_state_with_tempdir();
    let id = capsem_crypto::uuid_v4().to_string();
    let directory = state.run_dir.join("sessions").join(&id);
    std::fs::create_dir_all(directory.join(ACTIVE_POLICY_DIR)).unwrap();
    let policy_path = directory.join(ACTIVE_POLICY_DIR).join(ACTIVE_POLICY_FILE);
    let policy = ActivePolicyFile::from_settings_and_corp(&Default::default(), &Default::default()).unwrap();
    let bytes = toml::to_string(&policy).unwrap();
    capsem_foundation::unix::fs::atomic_write_private(&policy_path, bytes.as_bytes()).unwrap();
    crate::tests::insert_fake_instance_with_session_dir(&state, &id, std::process::id(), directory);
    let generation = state.instances.lock().unwrap()[&id].generation;
    let uds_path = capsem_foundation::uds::instance_socket_path(&state.run_dir, &id).unwrap();
    state.instances.lock().unwrap().get_mut(&id).unwrap().uds_path = uds_path.clone();
    let mut command = tokio::process::Command::new("sleep");
    command.arg("30").kill_on_drop(true);
    let (mut child, mut retirement) = state.retirements.spawn(&id, generation, &mut command).unwrap();
    // The in-process IPC fixture's kernel peer is this process; retirement
    // remains owned by the separately spawned child.
    let proxy = crate::proxy_worker::tests::fake("normal").await.unwrap();
    state.register_proxy_worker(&id, generation, proxy).unwrap();
    let session = *uuid::Uuid::parse_str(&id).unwrap().as_bytes();
    let context = Context {
        runtime: VmBinding::new(id.clone(), generation).unwrap(),
        managed: None,
        binding: SessionBinding::new(
            capsem_credentials::GrantSession::new(session, session, *generation.as_bytes()).unwrap(),
            SystemTime::now() + std::time::Duration::from_secs(60),
        )
        .unwrap(),
        policy_path,
        uds_path: uds_path.clone(),
    };
    for (digest, error, success) in [
        (Some(active_policy_digest(bytes.as_bytes())), None, true),
        (Some("blake3:wrong".into()), None, false),
        (None, Some("reload refused".into()), false),
    ] {
        state.connections.applied.lock().unwrap().clear();
        let expected_policy = bytes.as_bytes().to_vec();
        let owner = crate::tests::spawn_fake_process(&uds_path, 1, move |message| {
            let reply = match message {
                ServiceToProcess::ReloadConfig { id, active_policy } => {
                    assert_eq!(active_policy, &expected_policy);
                    Some(ProcessToService::ConfigReloadResult {
                        id: *id,
                        active_policy_digest: digest.clone(),
                        error: error.clone(),
                    })
                }
                _ => None,
            };
            Box::pin(async move { reply })
        });
        assert_eq!(confirm(&state, &context).await.is_ok(), success);
        assert_eq!(
            state
                .connections
                .applied
                .lock()
                .unwrap()
                .contains_key(&(id.clone(), generation)),
            success
        );
        assert!(crate::tests::only_reloads(&owner.await.unwrap(), 1));
    }
    child.start_kill().unwrap();
    retirement.wait_child(&mut child).await.unwrap();
    retirement.complete();
    std::fs::remove_file(&uds_path).unwrap();
    std::fs::remove_file(uds_path.with_extension("ready")).unwrap();
}

#[tokio::test]
async fn credential_policy_receipts_require_original_live_owner_and_are_fenced_on_stop() {
    let (state, _root) = crate::tests::make_test_state_with_tempdir();
    let id = capsem_crypto::uuid_v4().to_string();
    let directory = state.run_dir.join("sessions").join(&id);
    crate::tests::insert_fake_instance_with_session_dir(&state, &id, std::process::id(), directory);
    let generation = state.instances.lock().unwrap()[&id].generation;
    let binding = VmBinding::new(id.clone(), generation).unwrap();
    let policy = Arc::new(ActivePolicyFile::from_settings_and_corp(&Default::default(), &Default::default()).unwrap());
    assert!(matches!(
        record(&state, &binding, Arc::clone(&policy)),
        Err(CredentialError(api::CredentialErrorCode::OwnerChanged, _))
    ));
    let mut command = tokio::process::Command::new("sleep");
    command.arg("30").kill_on_drop(true);
    let (mut child, mut retirement) = state.retirements.spawn(&id, generation, &mut command).unwrap();
    state.instances.lock().unwrap().get_mut(&id).unwrap().pid = child.id().unwrap();
    assert!(record(&state, &binding, Arc::clone(&policy)).is_ok());
    assert!(state
        .connections
        .applied
        .lock()
        .unwrap()
        .contains_key(&(id.clone(), generation)));
    state
        .connections
        .stop_runtime(&id, generation, capsem_credentials::GrantStopPolicy::Preserve)
        .await
        .unwrap();
    assert!(state.connections.applied.lock().unwrap().is_empty());
    record(&state, &binding, Arc::clone(&policy)).unwrap_or_else(|_| panic!());
    state.connections.retire_session(&id).await.unwrap();
    assert!(state.connections.applied.lock().unwrap().is_empty());
    record(&state, &binding, Arc::clone(&policy)).unwrap_or_else(|_| panic!());
    state.instances.lock().unwrap().get_mut(&id).unwrap().generation = capsem_crypto::uuid_v4();
    assert!(matches!(
        record(&state, &binding, policy),
        Err(CredentialError(api::CredentialErrorCode::OwnerChanged, _))
    ));
    state
        .connections
        .fence_runtime(&id, generation, capsem_credentials::GrantStopPolicy::Preserve);
    assert!(state.connections.applied.lock().unwrap().is_empty());
    child.start_kill().unwrap();
    retirement.wait_child(&mut child).await.unwrap();
    retirement.complete();
}

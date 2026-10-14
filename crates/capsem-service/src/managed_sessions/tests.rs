use super::*;
use crate::tests::make_test_state;

#[tokio::test]
async fn failed_network_retirement_keeps_managed_close_pending_and_original_exit_retryable() {
    use capsem_core::managed_sessions::{Capability, LeaseClock, LeasePolicy, Registry, Reservation, State};
    let state = make_test_state();
    let registry = Registry::open(&state.run_dir.join("managed-ownership")).unwrap();
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([63; 32]);
    let Reservation::New(ticket) = registry.reserve(request, &cap).unwrap() else {
        panic!()
    };
    let clock = LeaseClock::new(100, std::time::Instant::now());
    registry
        .start_lease(
            &ticket,
            LeasePolicy::new(std::time::Duration::from_secs(10)).unwrap(),
            clock,
        )
        .unwrap();
    registry.begin_create(&ticket, clock).unwrap();
    let binding = VmBinding::new(new_persistent_vm_id(), capsem_crypto::uuid_v4()).unwrap();
    registry.prepare_spawn(&ticket, binding.clone(), clock).unwrap();
    registry.bind_created(&ticket, binding.clone(), clock).unwrap();
    let snapshot = registry.close(request, &cap).unwrap();
    let grants = Arc::new(ControlledGrants {
        fail: false.into(),
        calls: 0.into(),
    });
    let adapter = ManagedLifecycle::for_recovery(Arc::clone(&state), &snapshot, Arc::new(Arc::clone(&grants))).unwrap();
    let session = state.run_dir.join("sessions").join(binding.id());
    std::fs::create_dir_all(&session).unwrap();
    crate::instance::persist_spawn_identity(&session, binding.id(), binding.generation()).unwrap();
    let future = i64::MAX / 2;
    let network = state.networks.lock().await.create("cleanup", future).await.unwrap();
    let address = state
        .networks
        .lock()
        .await
        .attach(network.id, binding.id(), future)
        .await
        .unwrap();
    assert!(adapter.cleanup(ticket.clone(), binding.clone()).await.is_err());
    assert!(
        session.exists(),
        "failed networking cannot acknowledge deleted session bytes"
    );
    assert_eq!(
        registry.inspect(request, &cap).unwrap().unwrap().state(),
        State::Closing
    );
    assert_eq!(
        state.networks.lock().await.address_of(network.id, binding.id()),
        Some(address)
    );
    let receipts = capsem_core::session::spawn_owner::RetirementStore::open(&state.native_retirements_path()).unwrap();
    assert!(receipts.contains(&binding).unwrap());
    assert_eq!(grants.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    state
        .networks
        .lock()
        .await
        .vm_deleted(binding.id(), future + 1)
        .await
        .unwrap();
    adapter.cleanup(ticket.clone(), binding.clone()).await.unwrap();
    registry.complete_cleanup(&ticket, &binding).unwrap();
    assert_eq!(registry.inspect(request, &cap).unwrap().unwrap().state(), State::Closed);
    assert!(state.networks.lock().await.summary(network.id).is_none());
    assert_eq!(grants.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(!session.exists());
}

#[test]
fn recovery_native_child_fixture() {
    use std::io::Read;
    let Ok(path) = std::env::var("CAPSEM_TEST_RECOVERY_ROOT") else {
        return;
    };
    let binding: VmBinding = serde_json::from_str(&std::env::var("CAPSEM_TEST_RECOVERY_BINDING").unwrap()).unwrap();
    let parent = capsem_foundation::unix::process::parent_process_id().unwrap().get();
    let _owner = capsem_core::session::spawn_owner::enter(std::path::Path::new(&path), &binding, parent).unwrap();
    std::io::stdin().read_to_end(&mut Vec::new()).unwrap();
}

#[tokio::test]
async fn recovery_reopens_original_cleanup_after_session_deletion_before_closed_commit() {
    use capsem_core::managed_sessions::{Capability, LeaseClock, LeasePolicy, Registry, Reservation, State};
    for ordinary_delete in [false, true] {
        let (state, _files) = crate::tests::make_test_state_with_tempdir();
        let registry_path = state.run_dir.join("managed-ownership");
        let registry = Registry::open(&registry_path).unwrap();
        let request = capsem_crypto::uuid_v4();
        let cap = Capability::from_bytes([62; 32]);
        let Reservation::New(ticket) = registry.reserve(request, &cap).unwrap() else {
            panic!()
        };
        let clock = LeaseClock::new(100, std::time::Instant::now());
        registry
            .start_lease(
                &ticket,
                LeasePolicy::new(std::time::Duration::from_secs(10)).unwrap(),
                clock,
            )
            .unwrap();
        registry.begin_create(&ticket, clock).unwrap();
        let binding = VmBinding::new(new_persistent_vm_id(), capsem_crypto::uuid_v4()).unwrap();
        registry.prepare_spawn(&ticket, binding.clone(), clock).unwrap();
        registry.bind_created(&ticket, binding.clone(), clock).unwrap();
        let snapshot = registry.close(request, &cap).unwrap();
        let grants = Arc::new(ControlledGrants {
            fail: false.into(),
            calls: 0.into(),
        });
        let adapter =
            ManagedLifecycle::for_recovery(Arc::clone(&state), &snapshot, Arc::new(Arc::clone(&grants))).unwrap();
        let session = state.run_dir.join("sessions").join(binding.id());
        std::fs::create_dir_all(&session).unwrap();
        crate::instance::persist_spawn_identity(&session, binding.id(), binding.generation()).unwrap();
        // The effects have completed, but the ownership commit has not happened.
        // Drop them and reopen both runtime effects and persistence from disk.
        if ordinary_delete {
            state.delete_session_dir(&session).unwrap();
        } else {
            adapter.cleanup(ticket, binding.clone()).await.unwrap();
        }
        assert!(!session.exists());
        assert_eq!(
            registry.inspect(request, &cap).unwrap().unwrap().state(),
            State::Closing
        );
        drop(adapter);
        drop(registry);
        let run_dir = state.run_dir.clone();
        let assets_dir = state.assets_dir.clone();
        drop(state);
        let restarted = Arc::new(crate::tests::test_state(run_dir, assets_dir, None));
        let registry = Registry::open(&registry_path).unwrap();
        let snapshot = registry.inspect(request, &cap).unwrap().unwrap();
        let adapter =
            ManagedLifecycle::for_recovery(Arc::clone(&restarted), &snapshot, Arc::new(Arc::clone(&grants))).unwrap();
        let controller = capsem_core::managed_sessions::controller::Controller::new(
            registry,
            capsem_core::managed_sessions::controller::EffectBounds::new(
                std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(2),
            )
            .unwrap(),
        );
        let receipt = restarted
            .native_retirements_path()
            .join(format!("{}.json", binding.generation()));
        let bytes = std::fs::read(&receipt).unwrap();
        std::fs::write(&receipt, b"corrupt").unwrap();
        assert!(controller.recover(request, Arc::new(adapter.clone())).await.is_err());
        assert_eq!(
            controller.inspect(request, cap.clone()).await.unwrap().unwrap().state(),
            State::Closing
        );
        assert_eq!(
            grants.calls.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(!ordinary_delete)
        );
        std::fs::write(&receipt, bytes).unwrap();
        assert_eq!(
            controller.recover(request, Arc::new(adapter)).await.unwrap().state(),
            State::Closed
        );
        assert_eq!(
            grants.calls.load(std::sync::atomic::Ordering::SeqCst),
            if ordinary_delete { 1 } else { 2 }
        );
        assert!(restarted.instances.lock().unwrap().is_empty());
        assert!(!session.exists());
    }
}

#[tokio::test]
async fn recovery_fences_preboot_and_awaits_original_native_exit_before_closing() {
    use capsem_core::managed_sessions::{Capability, LeaseClock, LeasePolicy, Registry, Reservation, State};
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for admitted in [false, true] {
        let state = make_test_state();
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(&root.path().join("ownership")).unwrap();
        let request = capsem_crypto::uuid_v4();
        let cap = Capability::from_bytes([61; 32]);
        let Reservation::New(ticket) = registry.reserve(request, &cap).unwrap() else {
            panic!()
        };
        let clock = LeaseClock::new(100, std::time::Instant::now());
        registry
            .start_lease(
                &ticket,
                LeasePolicy::new(std::time::Duration::from_secs(10)).unwrap(),
                clock,
            )
            .unwrap();
        registry.begin_create(&ticket, clock).unwrap();
        let binding = VmBinding::new(new_persistent_vm_id(), capsem_crypto::uuid_v4()).unwrap();
        registry.prepare_spawn(&ticket, binding.clone(), clock).unwrap();
        let snapshot = registry.inspect(request, &cap).unwrap().unwrap();
        let grants = Arc::new(ControlledGrants {
            fail: false.into(),
            calls: 0.into(),
        });
        let adapter =
            ManagedLifecycle::for_recovery(Arc::clone(&state), &snapshot, Arc::new(Arc::clone(&grants))).unwrap();
        let session = state.run_dir.join("sessions").join(binding.id());
        std::fs::create_dir_all(&session).unwrap();
        crate::instance::persist_spawn_identity(&session, binding.id(), binding.generation()).unwrap();
        let owner = capsem_core::managed_sessions::controller::Controller::new(
            registry,
            capsem_core::managed_sessions::controller::EffectBounds::new(
                std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(2),
            )
            .unwrap(),
        );
        if admitted {
            let mut child = Child(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "managed_sessions::tests::recovery_native_child_fixture",
                        "--nocapture",
                    ])
                    .env_clear()
                    .env("CAPSEM_TEST_RECOVERY_ROOT", &session)
                    .env("CAPSEM_TEST_RECOVERY_BINDING", serde_json::to_string(&binding).unwrap())
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                let record: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(session.join("spawn-owner.json")).unwrap()).unwrap();
                if record["pid"].as_u64() == Some(u64::from(child.0.id())) {
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "child witness not published");
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            assert!(owner.recover(request, Arc::new(adapter.clone())).await.is_err());
            assert_eq!(
                owner.inspect(request, cap.clone()).await.unwrap().unwrap().state(),
                State::Unknown
            );
            assert!(session.exists());
            assert!(
                state.delete_session_dir(&session).is_err(),
                "ordinary deletion cannot discard a live native owner"
            );
            assert!(child.0.try_wait().unwrap().is_none());
            assert_eq!(grants.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
            child.0.kill().unwrap();
            child.0.wait().unwrap();
        }
        assert_eq!(
            owner.recover(request, Arc::new(adapter)).await.unwrap().state(),
            State::Closed
        );
        assert!(!session.exists());
        assert_eq!(grants.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

struct ControlledGrants {
    fail: std::sync::atomic::AtomicBool,
    calls: std::sync::atomic::AtomicUsize,
}
impl GrantRetirement for Arc<ControlledGrants> {
    fn revoke(&self, _ticket: Ticket, _binding: VmBinding) -> EffectFuture<()> {
        let grants = Arc::clone(self);
        Box::pin(async move {
            grants.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            anyhow::ensure!(
                !grants.fail.load(std::sync::atomic::Ordering::SeqCst),
                "fixture grant retirement refused"
            );
            Ok(())
        })
    }
}

#[tokio::test]
async fn grant_failure_prevents_session_cleanup_and_retry_awaits_original_retirement() {
    let state = make_test_state();
    let grants = Arc::new(ControlledGrants {
        fail: true.into(),
        calls: 0.into(),
    });
    let adapter = ManagedLifecycle::new(
        Arc::clone(&state),
        ProvisionRequest::default(),
        Arc::new(Arc::clone(&grants)),
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let registry = capsem_core::managed_sessions::Registry::open(&root.path().join("ownership")).unwrap();
    let cap = capsem_core::managed_sessions::Capability::from_bytes([42; 32]);
    let capsem_core::managed_sessions::Reservation::New(ticket) =
        registry.reserve(capsem_crypto::uuid_v4(), &cap).unwrap()
    else {
        panic!()
    };
    adapter.scope(&ticket).unwrap();
    let binding = adapter.binding.clone();
    let clock = capsem_core::managed_sessions::LeaseClock::new(100, std::time::Instant::now());
    registry
        .start_lease(
            &ticket,
            capsem_core::managed_sessions::LeasePolicy::new(std::time::Duration::from_secs(10)).unwrap(),
            clock,
        )
        .unwrap();
    registry.begin_create(&ticket, clock).unwrap();
    registry.prepare_spawn(&ticket, binding.clone(), clock).unwrap();
    registry.bind_created(&ticket, binding.clone(), clock).unwrap();
    let snapshot = registry.inspect(ticket.request(), &cap).unwrap().unwrap();
    let adapter = ManagedLifecycle::for_recovery(Arc::clone(&state), &snapshot, Arc::new(Arc::clone(&grants))).unwrap();
    assert!(adapter.request.is_none());
    assert_eq!(adapter.binding, binding);
    let session = state.run_dir.join("sessions").join(binding.id());
    std::fs::create_dir_all(&session).unwrap();
    crate::instance::persist_spawn_identity(&session, binding.id(), binding.generation()).unwrap();
    let child = tokio::process::Command::new("sh")
        .args(["-c", "exec sleep 30"])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    crate::tests::insert_fake_instance_with_session_dir(&state, binding.id(), child.id().unwrap(), session.clone());
    state
        .instances
        .lock()
        .unwrap()
        .get_mut(binding.id())
        .unwrap()
        .generation = binding.generation();
    let retirement = state.retirements.register(binding.id(), binding.generation()).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        binding.id().into(),
        binding.id().into(),
        Arc::clone(&state),
        state.instance_socket_path(binding.id()).unwrap(),
        session.clone(),
        retirement,
    );
    assert!(adapter.cleanup(ticket.clone(), binding.clone()).await.is_err());
    assert!(
        session.exists(),
        "failed grant retirement must not acknowledge deleted session bytes"
    );
    assert!(state.instances.lock().unwrap().get(binding.id()).is_none());
    grants.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    adapter.cleanup(ticket, binding).await.unwrap();
    reaper.await.unwrap();
    assert!(!session.exists());
    assert_eq!(grants.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test]
async fn interrupted_creation_join_waits_for_the_original_continuation() {
    let state = make_test_state();
    let adapter = ManagedLifecycle::new(state, ProvisionRequest::default(), Arc::new(RefuseGrants)).unwrap();
    let (done, _receiver) = watch::channel(None);
    *adapter.work.continuation.lock().await = Some(done.clone());
    let join = {
        let adapter = adapter.clone();
        tokio::spawn(async move { adapter.join_creation().await })
    };
    tokio::task::yield_now().await;
    assert!(!join.is_finished());
    assert!(adapter.work.cancel.is_cancelled());
    done.send_replace(Some(false));
    join.await.unwrap().unwrap();
}

struct RefuseGrants;
impl GrantRetirement for RefuseGrants {
    fn revoke(&self, _ticket: Ticket, _binding: VmBinding) -> EffectFuture<()> {
        Box::pin(async { anyhow::bail!("fixture grant retirement refused") })
    }
}

#[tokio::test]
async fn recovery_adapter_keeps_untracked_original_child_unknown_and_preserves_session() {
    use capsem_core::managed_sessions::{Capability, LeaseClock, LeasePolicy, Registry, Reservation, State};
    let state = make_test_state();
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::open(&root.path().join("ownership")).unwrap();
    let request = capsem_crypto::uuid_v4();
    let capability = Capability::from_bytes([54; 32]);
    let Reservation::New(ticket) = registry.reserve(request, &capability).unwrap() else {
        panic!()
    };
    let clock = LeaseClock::new(100, std::time::Instant::now());
    registry
        .start_lease(
            &ticket,
            LeasePolicy::new(std::time::Duration::from_secs(10)).unwrap(),
            clock,
        )
        .unwrap();
    registry.begin_create(&ticket, clock).unwrap();
    let binding = VmBinding::new(new_persistent_vm_id(), capsem_crypto::uuid_v4()).unwrap();
    registry.prepare_spawn(&ticket, binding.clone(), clock).unwrap();
    let snapshot = registry.inspect(request, &capability).unwrap().unwrap();
    let adapter = ManagedLifecycle::for_recovery(Arc::clone(&state), &snapshot, Arc::new(RefuseGrants)).unwrap();
    assert_eq!(adapter.binding, binding);
    assert!(adapter.request.is_none());
    adapter.scope(&ticket).unwrap();
    let session = state.run_dir.join("sessions").join(binding.id());
    std::fs::create_dir_all(&session).unwrap();
    let directory = capsem_foundation::unix::contained::ContainedDir::open_root(&session).unwrap();
    capsem_core::session::write_spawn_identity(&directory, &binding).unwrap();
    let owner = capsem_core::managed_sessions::controller::Controller::new(
        registry,
        capsem_core::managed_sessions::controller::EffectBounds::new(
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(2),
        )
        .unwrap(),
    );
    assert!(owner.recover(request, Arc::new(adapter.clone())).await.is_err());
    assert_eq!(
        owner.inspect(request, capability).await.unwrap().unwrap().state(),
        State::Unknown
    );
    assert!(session.exists(), "metadata alone cannot prove the old child exited");
    assert!(owner
        .create(
            capsem_crypto::uuid_v4(),
            Capability::from_bytes([55; 32]),
            LeasePolicy::new(std::time::Duration::from_secs(10)).unwrap(),
            Arc::new(adapter.clone()),
        )
        .await
        .is_err());
    assert!(state.instances.lock().unwrap().is_empty());
    assert!(adapter.work.continuation.lock().await.is_none());
}

#[tokio::test]
async fn rejected_pre_spawn_creation_reconciles_without_inventing_a_child() {
    let state = make_test_state();
    let grants = Arc::new(ControlledGrants {
        fail: false.into(),
        calls: 0.into(),
    });
    let adapter = ManagedLifecycle::new(
        Arc::clone(&state),
        ProvisionRequest {
            ram_mb: Some(1),
            ..Default::default()
        },
        Arc::new(Arc::clone(&grants)),
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let owner = capsem_core::managed_sessions::controller::Controller::new(
        capsem_core::managed_sessions::Registry::open(&root.path().join("ownership")).unwrap(),
        capsem_core::managed_sessions::controller::EffectBounds::new(
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(2),
        )
        .unwrap(),
    );
    let request = capsem_crypto::uuid_v4();
    let cap = capsem_core::managed_sessions::Capability::from_bytes([43; 32]);
    assert!(owner
        .create(
            request,
            cap.clone(),
            capsem_core::managed_sessions::LeasePolicy::new(std::time::Duration::from_secs(10)).unwrap(),
            Arc::new(adapter)
        )
        .await
        .is_err());
    assert_eq!(
        owner.close(request, cap).await.unwrap().state(),
        capsem_core::managed_sessions::State::Closed
    );
    assert!(state.instances.lock().unwrap().is_empty());
    assert!(grants.calls.load(std::sync::atomic::Ordering::SeqCst) >= 1);
}

#[test]
fn managed_adapter_refuses_named_or_persistent_requests_before_any_effect() {
    let state = make_test_state();
    for request in [
        ProvisionRequest {
            name: Some("named".into()),
            ..Default::default()
        },
        ProvisionRequest {
            persistent: true,
            ..Default::default()
        },
        ProvisionRequest {
            labels: Some([("invalid key!".into(), "v".into())].into()),
            ..Default::default()
        },
    ] {
        assert!(ManagedLifecycle::new(Arc::clone(&state), request, Arc::new(RefuseGrants)).is_err());
    }
    assert!(state.instances.lock().unwrap().is_empty());
    assert!(super::app_error(AppError::new(StatusCode::CONFLICT, "busy".into()))
        .to_string()
        .contains("409"));
}

#[tokio::test]
async fn managed_adapter_never_cleans_up_a_different_actual_generation() {
    let state = make_test_state();
    let adapter =
        ManagedLifecycle::new(Arc::clone(&state), ProvisionRequest::default(), Arc::new(RefuseGrants)).unwrap();
    let root = tempfile::tempdir().unwrap();
    let registry = capsem_core::managed_sessions::Registry::open(&root.path().join("ownership")).unwrap();
    let cap = capsem_core::managed_sessions::Capability::from_bytes([41; 32]);
    let capsem_core::managed_sessions::Reservation::New(ticket) =
        registry.reserve(capsem_crypto::uuid_v4(), &cap).unwrap()
    else {
        panic!()
    };
    adapter.scope(&ticket).unwrap();
    let mut replacement = crate::tests::test_instance();
    replacement.id = adapter.binding.id().into();
    replacement.pid = 0;
    let generation = replacement.generation;
    state
        .instances
        .lock()
        .unwrap()
        .insert(replacement.id.clone(), replacement);
    assert!(adapter.cleanup(ticket, adapter.binding.clone()).await.is_err());
    assert_eq!(
        state
            .instances
            .lock()
            .unwrap()
            .get(adapter.binding.id())
            .unwrap()
            .generation,
        generation
    );
}

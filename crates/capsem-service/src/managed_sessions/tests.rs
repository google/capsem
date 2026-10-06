use super::*;
use crate::tests::make_test_state;

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
    let capsem_core::managed_sessions::Reservation::New(ticket) = registry.reserve(uuid::Uuid::new_v4(), &cap).unwrap()
    else {
        panic!()
    };
    adapter.scope(&ticket).unwrap();
    let binding = adapter.binding.clone();
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
    let request = uuid::Uuid::new_v4();
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
    ] {
        assert!(ManagedLifecycle::new(Arc::clone(&state), request, Arc::new(RefuseGrants)).is_err());
    }
    assert!(state.instances.lock().unwrap().is_empty());
}

#[tokio::test]
async fn managed_adapter_never_cleans_up_a_different_actual_generation() {
    let state = make_test_state();
    let adapter =
        ManagedLifecycle::new(Arc::clone(&state), ProvisionRequest::default(), Arc::new(RefuseGrants)).unwrap();
    let root = tempfile::tempdir().unwrap();
    let registry = capsem_core::managed_sessions::Registry::open(&root.path().join("ownership")).unwrap();
    let cap = capsem_core::managed_sessions::Capability::from_bytes([41; 32]);
    let capsem_core::managed_sessions::Reservation::New(ticket) = registry.reserve(uuid::Uuid::new_v4(), &cap).unwrap()
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
